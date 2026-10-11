mod ai;
mod audio;
mod discord_rpc;
mod export;
mod paths;
mod project;
mod session;
mod transcription;

use audio::{start_audio_capture, stop_audio_capture, AudioCaptureHandle, AudioSources};
use discord_rpc::{get_channel_info, is_rpc_connected, load_tokens, save_tokens, DiscordRpcClient};
use export::{export_srt, export_vtt};
use log::{debug, warn};
use paths::{app_data_dir, discord_tokens_path, models_dir, projects_dir};
use project::{
    auto_save_project, delete_project, format_project_name, list_projects, list_projects_with_meta,
    load_project, purge_old_recent, save_project,
};
use session::{
    clear_live_segment_tx, flush_pending_if_elapsed, record_speaking_event, set_live_segment_tx,
    start_session, stop_session, SessionAudioPaths, SessionSegment, SessionState,
};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use transcription::{
    download_engine, download_model_with_progress, engine_dir, extract_segment, installed_engine,
    is_installed, list_installed_model_names, list_models, resolve_model_path, transcribe_via_api,
    write_wav_from_samples, EngineKind, RemoteTranscriptionConfig, WhisperServer,
};

#[tauri::command]
fn get_app_data_dir(app: tauri::AppHandle) -> Result<String, String> {
    app_data_dir(&app).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn get_log_file_path(app: tauri::AppHandle) -> Result<String, String> {
    paths::log_file_path(&app).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn get_projects_dir(app: tauri::AppHandle) -> Result<String, String> {
    projects_dir(&app).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn get_models_dir(app: tauri::AppHandle) -> Result<String, String> {
    models_dir(&app).map(|p| p.to_string_lossy().into_owned())
}

/// Forward speaking events from the Discord connection to the recorder. Ends when the
/// connection is dropped (its sender goes away).
fn spawn_speaking_forwarder(mut rx: mpsc::UnboundedReceiver<discord_rpc::SpeakingEvent>) {
    tokio::spawn(async move {
        while let Some(evt) = rx.recv().await {
            match evt {
                discord_rpc::SpeakingEvent::Start { user_id } => {
                    record_speaking_event(true, user_id);
                }
                discord_rpc::SpeakingEvent::Stop { user_id } => {
                    record_speaking_event(false, user_id);
                }
            }
        }
    });
}

#[tauri::command]
async fn discord_rpc_connect(
    app: tauri::AppHandle,
    client_id: String,
) -> Result<(), String> {
    let client = DiscordRpcClient::new(client_id.clone());
    let (tx, rx) = mpsc::unbounded_channel();
    spawn_speaking_forwarder(rx);
    let refresh_token = client.connect(tx).await?;
    if let Some(refresh) = refresh_token {
        let path = discord_tokens_path(&app)?;
        save_tokens(
            &path,
            &discord_rpc::DiscordTokens {
                client_id,
                refresh_token: refresh,
            },
        )?;
    }
    Ok(())
}

#[tauri::command]
async fn discord_rpc_auto_reconnect(app: tauri::AppHandle) -> Result<bool, String> {
    let path = discord_tokens_path(&app)?;
    let tokens = match load_tokens(&path)? {
        Some(t) => t,
        None => return Ok(false),
    };
    let client = DiscordRpcClient::new(tokens.client_id.clone());
    // Discord rotates refresh tokens: save the new one before anything else can fail.
    let (access_token, refresh) = client.refresh_tokens(&tokens.refresh_token).await?;
    save_tokens(
        &path,
        &discord_rpc::DiscordTokens {
            client_id: tokens.client_id,
            refresh_token: refresh,
        },
    )?;
    let (tx, rx) = mpsc::unbounded_channel();
    spawn_speaking_forwarder(rx);
    client.connect_with_access_token(tx, access_token).await?;
    Ok(true)
}

/// Discord desktop is running if one of its IPC pipes (discord-ipc-0..9) exists.
/// Lists the pipe namespace instead of opening pipes, so no connection is made.
async fn is_discord_running() -> bool {
    #[cfg(windows)]
    {
        std::fs::read_dir(r"\\.\pipe\")
            .map(|entries| {
                entries.flatten().any(|e| {
                    e.file_name()
                        .to_str()
                        .and_then(|n| n.strip_prefix("discord-ipc-"))
                        .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
                })
            })
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[tauri::command]
async fn discord_rpc_connection_state() -> Result<serde_json::Value, String> {
    let state = if get_channel_info().is_some() {
        "InVoice"
    } else if is_rpc_connected() {
        "Idle"
    } else {
        "Disconnected"
    };
    let discord_running = is_discord_running().await;
    Ok(serde_json::json!({
        "state": state,
        "discord_running": discord_running
    }))
}

static AUDIO_HANDLE: Mutex<Option<AudioCaptureHandle>> = Mutex::new(None);
static SESSION_AUDIO_PATHS: Mutex<Option<(String, String)>> = Mutex::new(None);
static LIVE_TRANSCRIPT_TEXTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WAS_LIVE_RECORDING: Mutex<bool> = Mutex::new(false);

#[tauri::command]
fn start_recording(
    app: tauri::AppHandle,
    output_path: String,
    mic_path: String,
    segment_merge_buffer_ms: Option<u64>,
    project_name_template: Option<String>,
    live_realtime: Option<bool>,
    live_model_path: Option<String>,
    live_transcription_mode: Option<String>,
    live_remote_base_url: Option<String>,
    live_remote_model: Option<String>,
    live_remote_api_key: Option<String>,
    live_language_code: Option<String>,
    audio_sources: Option<AudioSources>,
) -> Result<(), String> {
    let channel_info =
        get_channel_info().ok_or("Not connected to Discord. Connect in Settings first.")?;
    let user_labels: std::collections::HashMap<String, String> = channel_info.user_labels.clone();
    let buffer_ms = segment_merge_buffer_ms.unwrap_or(1000);
    let template =
        project_name_template.unwrap_or_else(|| "{guild}_{channel}_{timestamp}".to_string());
    let live = live_realtime.unwrap_or(false);
    let self_user_id = channel_info.self_user_id.clone();

    start_session(
        channel_info.guild_name,
        channel_info.guild_id,
        channel_info.channel_name,
        Some(channel_info.channel_id),
        channel_info.channel_type,
        self_user_id.clone(),
        user_labels.clone(),
        buffer_ms,
        template,
        live,
    );

    let (loopback_buf, mic_buf, loopback_path, mic_path_buf) = if live {
        *WAS_LIVE_RECORDING.lock().unwrap() = true;
        let lb = Arc::new(Mutex::new(audio::AudioBuffer::new()));
        let mb = Arc::new(Mutex::new(audio::AudioBuffer::new()));
        let lb_task = lb.clone();
        let mb_task = mb.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        set_live_segment_tx(tx);
        *LIVE_TRANSCRIPT_TEXTS.lock().unwrap() = Vec::new();

        let app_handle = app.clone();
        let use_remote = live_transcription_mode.as_deref() == Some("remote")
            && live_remote_base_url
                .as_ref()
                .map_or(false, |u| !u.trim().is_empty())
            && live_remote_model
                .as_ref()
                .map_or(false, |m| !m.trim().is_empty());
        let remote_config = use_remote.then(|| {
            RemoteTranscriptionConfig::new(
                live_remote_base_url.clone().unwrap_or_default(),
                live_remote_model.clone().unwrap_or_default(),
                live_remote_api_key.clone(),
            )
        });
        let model_path = live_model_path.clone();
        let language_code = live_language_code.clone();
        let engines_root = paths::engines_dir(&app)?;
        let server_log = paths::log_file_path(&app)?.with_file_name("whisper-server-live.log");
        let temp_dir = app_data_dir(&app).map(|d| d.join("transcribe_temp")).ok();

        // Spawn periodic flush so solo speakers get segments (pending is flushed after buffer_ms)
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                interval.tick().await;
                if !*WAS_LIVE_RECORDING.lock().unwrap() {
                    break;
                }
                flush_pending_if_elapsed();
            }
        });

        tauri::async_runtime::spawn(async move {
            let server = if use_remote {
                None
            } else {
                let model = model_path.as_ref().filter(|p| std::path::Path::new(p).exists());
                match (installed_engine(&engines_root), model) {
                    (Some((_, dir)), Some(m)) => {
                        match WhisperServer::start(&dir, std::path::Path::new(m), &server_log).await {
                            Ok(s) => Some(s),
                            Err(e) => {
                                warn!("[live] {}", e);
                                None
                            }
                        }
                    }
                    (None, _) => {
                        warn!("[live] No transcription engine installed. Transcribe once or download it in Settings.");
                        None
                    }
                    (_, None) => {
                        warn!("[live] Live model is missing: {:?}", model_path);
                        None
                    }
                }
            };
            let lang = language_code.clone().unwrap_or_else(|| "auto".into());
            while let Some(seg) = rx.recv().await {
                debug!(
                    "[live] segment received: {}..{} ms, user={}",
                    seg.start_ms, seg.end_ms, seg.user_id
                );
                if seg.end_ms <= seg.start_ms {
                    debug!("[live] skipping invalid segment (end <= start)");
                    continue;
                }
                // Small delay so the capture buffer has time to receive samples (session and buffer
                // can have a slight time offset since capture starts after session).
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let is_local = self_user_id.as_ref().map_or(false, |id| id == &seg.user_id);
                let buf = if is_local { &mb_task } else { &lb_task };
                let samples = {
                    let guard = buf.lock().unwrap();
                    guard.extract(seg.start_ms, seg.end_ms)
                };
                if samples.is_empty() {
                    warn!("[live] extract returned empty for {}..{} ms (buffer may not have samples yet)", seg.start_ms, seg.end_ms);
                    continue;
                }
                if transcription::rms(&samples) < SILENCE_RMS {
                    continue;
                }
                let temp_dir = match &temp_dir {
                    Some(d) => d.clone(),
                    None => {
                        warn!("[live] no temp_dir configured, skipping segment");
                        continue;
                    }
                };
                let _ = std::fs::create_dir_all(&temp_dir);
                let seg_path = temp_dir.join(format!(
                    "live_seg_{}.wav",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis()
                ));
                if write_wav_from_samples(&seg_path, &samples).is_err() {
                    warn!(
                        "[live] failed to write WAV for segment {}..{} ms",
                        seg.start_ms, seg.end_ms
                    );
                    continue;
                }
                let text = if use_remote {
                    match &remote_config {
                        Some(cfg) => transcribe_via_api(cfg, &seg_path).await.unwrap_or_default(),
                        None => String::new(),
                    }
                } else if let Some(ref server) = server {
                    match server.transcribe(&seg_path, &lang).await {
                        Ok(r) => r.text,
                        Err(e) => {
                            warn!("[live] {}", e);
                            String::new()
                        }
                    }
                } else {
                    String::new()
                };
                let idx = LIVE_TRANSCRIPT_TEXTS.lock().unwrap().len();
                LIVE_TRANSCRIPT_TEXTS.lock().unwrap().push(text.clone());
                debug!(
                    "[live] emitted transcript-segment idx={} len={} preview={:?}",
                    idx,
                    text.len(),
                    text.chars().take(50).collect::<String>()
                );
                let _ = app_handle.emit(
                    "transcript-segment",
                    serde_json::json!({ "segment": seg, "text": text, "index": idx }),
                );
                let _ = std::fs::remove_file(&seg_path);
            }
        });

        (Some(lb), Some(mb), output_path.clone(), mic_path.clone())
    } else {
        (None, None, output_path.clone(), mic_path.clone())
    };

    let handle = start_audio_capture(
        std::path::Path::new(&loopback_path),
        std::path::Path::new(&mic_path_buf),
        &audio_sources.unwrap_or_default(),
        loopback_buf,
        mic_buf,
    )?;
    *AUDIO_HANDLE.lock().unwrap() = Some(handle);
    *SESSION_AUDIO_PATHS.lock().unwrap() = Some((output_path, mic_path));
    if !live {
        *WAS_LIVE_RECORDING.lock().unwrap() = false;
    }
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
async fn list_audio_devices_command() -> Result<audio::AudioDevices, String> {
    tauri::async_runtime::spawn_blocking(audio::list_audio_devices)
        .await
        .map_err(|e| e.to_string())?
}

/// Peak level (0..1) of a source over 3 seconds, for the Test buttons in Settings.
#[cfg(windows)]
#[tauri::command]
async fn measure_audio_level_command(mic: bool, device_id: Option<String>) -> Result<f32, String> {
    tauri::async_runtime::spawn_blocking(move || audio::measure_level(mic, device_id, std::time::Duration::from_secs(3)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn stop_recording(_app: tauri::AppHandle) -> Result<Option<SessionState>, String> {
    let paths = SESSION_AUDIO_PATHS.lock().unwrap().take();
    if let Some(handle) = AUDIO_HANDLE.lock().unwrap().take() {
        stop_audio_capture(handle)?;
    }
    clear_live_segment_tx();
    let was_live = *WAS_LIVE_RECORDING.lock().unwrap();
    *WAS_LIVE_RECORDING.lock().unwrap() = false;
    let mut state = paths.and_then(|(loopback, microphone)| {
        stop_session(SessionAudioPaths {
            loopback: Some(loopback),
            microphone: Some(microphone),
        })
    });
    if was_live {
        let texts = std::mem::take(&mut *LIVE_TRANSCRIPT_TEXTS.lock().unwrap());
        if let Some(ref mut s) = state {
            let mut texts = texts;
            while texts.len() < s.segments.len() {
                texts.push(String::new());
            }
            s.live_transcript_texts = Some(texts.clone());
            s.transcript_texts = texts;
        }
    }
    Ok(state)
}

#[tauri::command]
fn get_channel_info_command() -> Result<Option<serde_json::Value>, String> {
    Ok(get_channel_info().map(|c| {
        let self_username = c
            .self_user_id
            .as_ref()
            .and_then(|id| c.user_labels.get(id))
            .cloned();
        serde_json::json!({
            "channel_id": c.channel_id,
            "channel_name": c.channel_name,
            "guild_id": c.guild_id,
            "guild_name": c.guild_name,
            "self_user_id": c.self_user_id,
            "self_username": self_username,
            // Everyone in the voice channel, including you.
            "member_count": c.user_labels.len(),
        })
    }))
}

#[tauri::command]
fn save_project_command(
    app: tauri::AppHandle,
    path: String,
    state: SessionState,
) -> Result<(), String> {
    save_project(&app, std::path::Path::new(&path), &state)
}

#[tauri::command]
fn load_project_command(path: String) -> Result<SessionState, String> {
    load_project(std::path::Path::new(&path))
}

#[tauri::command]
fn list_projects_command(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    list_projects(&app)
}

#[tauri::command]
fn list_projects_with_meta_command(
    app: tauri::AppHandle,
) -> Result<Vec<project::ProjectMeta>, String> {
    list_projects_with_meta(&app)
}

#[tauri::command]
fn auto_save_project_command(app: tauri::AppHandle, state: SessionState) -> Result<String, String> {
    auto_save_project(&app, &state)
}

#[tauri::command]
fn delete_project_command(path: String, delete_audio: bool) -> Result<(), String> {
    delete_project(std::path::Path::new(&path), delete_audio)
}

#[tauri::command]
fn purge_recent_command(
    app: tauri::AppHandle,
    retention_days: u64,
    max_bytes: u64,
    keep_path: Option<String>,
) -> Result<u32, String> {
    purge_old_recent(&app, retention_days, max_bytes, keep_path.as_deref())
}

#[tauri::command]
fn set_project_saved_command(app: tauri::AppHandle, path: String, saved: bool) -> Result<String, String> {
    project::set_project_saved(&app, std::path::Path::new(&path), saved)
}

#[tauri::command]
fn list_models_command(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let dir = models_dir(&app)?;
    let mut paths = Vec::new();
    if dir.exists() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "bin") {
                paths.push(path.to_string_lossy().into_owned());
            }
        }
    }
    Ok(paths)
}

#[tauri::command]
async fn download_model_command(
    app: tauri::AppHandle,
    model_name: String,
) -> Result<String, String> {
    let models_dir = models_dir(&app)?;
    let app_emit = app.clone();
    let model_name_emit = model_name.clone();
    download_model_with_progress(&models_dir, &model_name, move |downloaded, total| {
        let _ = app_emit.emit(
            "download-progress",
            serde_json::json!({
                "modelName": model_name_emit,
                "bytesDownloaded": downloaded,
                "totalBytes": total,
            }),
        );
    })
    .await
}

#[tauri::command]
fn resolve_model_path_command(
    app: tauri::AppHandle,
    model_name: String,
) -> Result<Option<String>, String> {
    let dir = models_dir(&app)?;
    Ok(resolve_model_path(&dir, &model_name).map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
fn list_installed_model_names_command(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let dir = models_dir(&app)?;
    Ok(list_installed_model_names(&dir))
}

#[tauri::command]
fn open_models_dir_command(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = models_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_remote_models_command(
    host: String,
    models_path: Option<String>,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    list_models(&host, models_path.as_deref(), api_key.as_deref()).await
}

/// Segments shorter than this are skipped: whisper mostly hallucinates on them.
const MIN_SEGMENT_MS: u64 = 250;
/// Segments quieter than this (RMS of 16-bit samples) are treated as silence and not sent to
/// whisper, which invents text like "Thank you." for silent audio.
const SILENCE_RMS: f32 = 100.0;
/// With an English model set, turns it detects as English at least this confidently keep its text.
const ENGLISH_THRESHOLD: f32 = 0.5;

/// The installed engine (GPU preferred). Downloads the small CPU engine if none is installed.
async fn ensure_engine(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let root = paths::engines_dir(app)?;
    if let Some((_, dir)) = installed_engine(&root) {
        return Ok(dir);
    }
    let app_emit = app.clone();
    download_engine(&root, EngineKind::Cpu, move |done, total| {
        let _ = app_emit.emit(
            "engine-download-progress",
            serde_json::json!({ "kind": "cpu", "bytesDownloaded": done, "totalBytes": total }),
        );
    })
    .await
}

#[tauri::command]
fn engine_status_command(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let root = paths::engines_dir(&app)?;
    let nvidia = std::path::Path::new(r"C:\Windows\System32\nvcuda.dll").exists();
    Ok(serde_json::json!({
        "cpu": is_installed(&root, EngineKind::Cpu),
        "cuda": is_installed(&root, EngineKind::Cuda),
        "nvidia": nvidia,
    }))
}

#[tauri::command]
async fn download_engine_command(app: tauri::AppHandle, kind: String) -> Result<(), String> {
    let kind = EngineKind::parse(&kind)?;
    let root = paths::engines_dir(&app)?;
    let app_emit = app.clone();
    let kind_name = format!("{:?}", kind).to_lowercase();
    download_engine(&root, kind, move |done, total| {
        let _ = app_emit.emit(
            "engine-download-progress",
            serde_json::json!({ "kind": kind_name, "bytesDownloaded": done, "totalBytes": total }),
        );
    })
    .await?;
    Ok(())
}

#[tauri::command]
fn remove_engine_command(app: tauri::AppHandle, kind: String) -> Result<(), String> {
    let kind = EngineKind::parse(&kind)?;
    let dir = engine_dir(&paths::engines_dir(&app)?, kind);
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
}

#[tauri::command]
async fn transcribe_session_command(
    app: tauri::AppHandle,
    state: SessionState,
    model_path: Option<String>,
    english_model_path: Option<String>,
    transcription_mode: String,
    remote_base_url: Option<String>,
    remote_model: Option<String>,
    remote_api_key: Option<String>,
    language_code: Option<String>,
    only_indices: Option<Vec<usize>>,
) -> Result<SessionState, String> {
    let loopback_path = state
        .audio_paths
        .loopback
        .as_ref()
        .map(std::path::Path::new)
        .ok_or("No loopback audio")?;
    let mic_path = state
        .audio_paths
        .microphone
        .as_ref()
        .map(std::path::Path::new)
        .ok_or("No microphone audio")?;

    let use_remote = transcription_mode == "remote"
        && remote_base_url
            .as_ref()
            .map_or(false, |u| !u.trim().is_empty())
        && remote_model
            .as_ref()
            .map_or(false, |m| !m.trim().is_empty());
    let remote_config = use_remote.then(|| {
        RemoteTranscriptionConfig::new(
            remote_base_url.clone().unwrap_or_default(),
            remote_model.clone().unwrap_or_default(),
            remote_api_key.clone(),
        )
    });
    let language = language_code.clone().unwrap_or_else(|| "auto".into());

    // Local: one server per model, started once for the whole recording.
    let (main_server, english_server) = if use_remote {
        (None, None)
    } else {
        let model = model_path.ok_or("No model selected. Download a model in Settings.")?;
        if !std::path::Path::new(&model).exists() {
            return Err(format!("Model not found: {}", model));
        }
        let engine = ensure_engine(&app).await?;
        let logs = paths::log_file_path(&app)?;
        let main = WhisperServer::start(&engine, std::path::Path::new(&model), &logs.with_file_name("whisper-server.log")).await?;
        let english = match english_model_path.filter(|p| language != "en" && p != &model) {
            Some(p) => {
                if !std::path::Path::new(&p).exists() {
                    return Err(format!("English model not found: {}", p));
                }
                Some(WhisperServer::start(&engine, std::path::Path::new(&p), &logs.with_file_name("whisper-server-en.log")).await?)
            }
            None => None,
        };
        (Some(main), english)
    };

    let temp_dir = app_data_dir(&app)?.join("transcribe_temp");
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let mut texts = state.transcript_texts.clone();
    texts.resize(state.segments.len(), String::new());

    let todo: Vec<usize> = (0..state.segments.len())
        .filter(|i| only_indices.as_ref().map_or(true, |only| only.contains(i)))
        .collect();
    log::info!(
        "[transcribe] {} segments, mode={}, language={}, english routing={}",
        todo.len(),
        if use_remote { "remote" } else { "local" },
        language,
        english_server.is_some()
    );

    for (n, &i) in todo.iter().enumerate() {
        let _ = app.emit("transcribe-progress", serde_json::json!({ "done": n, "total": todo.len() }));
        let seg = &state.segments[i];
        if seg.end_ms < seg.start_ms + MIN_SEGMENT_MS {
            texts[i] = String::new();
            continue;
        }
        let is_local = state.self_user_id.as_ref().map_or(false, |id| id == &seg.user_id);
        let source_path = if is_local { mic_path } else { loopback_path };
        let segment_path = temp_dir.join(format!("seg_{}.wav", i));
        let (level, samples) = extract_segment(source_path, &segment_path, seg.start_ms, seg.end_ms)?;
        if samples < (MIN_SEGMENT_MS * 16) as usize || level < SILENCE_RMS {
            let _ = std::fs::remove_file(&segment_path);
            texts[i] = String::new();
            continue;
        }

        let result = if let Some(config) = &remote_config {
            transcribe_via_api(config, &segment_path).await
        } else {
            let main = main_server.as_ref().expect("local mode has a server");
            match &english_server {
                Some(en) => match en.transcribe(&segment_path, "auto").await {
                    Ok(r) if r.english_prob >= ENGLISH_THRESHOLD => Ok(r.text),
                    Ok(_) => main.transcribe(&segment_path, &language).await.map(|r| r.text),
                    Err(e) => Err(e),
                },
                None => main.transcribe(&segment_path, &language).await.map(|r| r.text),
            }
        };
        let _ = std::fs::remove_file(&segment_path);

        match result {
            Ok(t) => texts[i] = t,
            Err(e) => {
                warn!("[transcribe] segment {}: {}", i, e);
                texts[i] = format!("[Transcription error: {}]", e);
            }
        }
    }
    let _ = app.emit("transcribe-progress", serde_json::json!({ "done": todo.len(), "total": todo.len() }));

    Ok(SessionState {
        transcript_texts: texts,
        ..state
    })
}

/// Runs the configured AI command with `input` on stdin and returns its stdout.
#[tauri::command]
async fn run_ai_command(app: tauri::AppHandle, command: String, input: String) -> Result<String, String> {
    let work_dir = paths::app_data_dir(&app)?.join("ai_workdir");
    ai::run_command(&command, &input, &work_dir).await
}

#[tauri::command]
fn format_project_name_command(
    template: String,
    guild: Option<String>,
    channel: Option<String>,
) -> Result<String, String> {
    Ok(format_project_name(
        &template,
        guild.as_deref(),
        channel.as_deref(),
    ))
}

#[tauri::command]
fn export_transcript(
    path: String,
    format: String,
    segments: Vec<SessionSegment>,
    texts: Vec<String>,
) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    match format.as_str() {
        "srt" => export_srt(p, &segments, &texts),
        "vtt" => export_vtt(p, &segments, &texts),
        _ => Err(format!("Unsupported format: {}", format)),
    }
}

/// Log directory in Roaming (with projects). Resolved without AppHandle.
fn log_dir_path() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .map(|p| std::path::PathBuf::from(p).join("d-scribe").join("logs"))
            .unwrap_or_else(|_| std::path::PathBuf::from(".").join("logs"))
    }
    #[cfg(not(windows))]
    {
        dirs::data_dir()
            .map(|d| d.join("d-scribe").join("logs"))
            .unwrap_or_else(|| std::path::PathBuf::from(".").join("logs"))
    }
}

fn init_logger() -> Result<std::path::PathBuf, fern::InitError> {
    let log_dir = log_dir_path();
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = log_dir.join("d-scribe.log");

    let format =
        |out: fern::FormatCallback<'_>, message: &std::fmt::Arguments<'_>, record: &log::Record| {
            out.finish(format_args!(
                "[{}][{}][{}][{:?}] {}",
                chrono::Local::now().format("%Y-%m-%d"),
                chrono::Local::now().format("%H:%M:%S"),
                record.target(),
                record.level(),
                message
            ))
        };

    fern::Dispatch::new()
        .format(format)
        .level(log::LevelFilter::Debug)
        .chain(
            fern::Dispatch::new()
                .filter(|m| !m.target().starts_with("wasapi"))
                .chain(std::io::stdout()),
        )
        .chain(fern::log_file(&log_file)?)
        .apply()?;

    Ok(log_file)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _log_path = init_logger().ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_log::Builder::default().skip_logger().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            paths::ensure_directories(app.handle())?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match discord_rpc_auto_reconnect(handle).await {
                    Ok(true) => log::info!("[d-scribe] Auto-reconnected to Discord"),
                    Ok(false) => log::info!("[d-scribe] No saved Discord login"),
                    Err(e) => log::warn!("[d-scribe] Discord auto-reconnect failed: {}", e),
                }
            });
            // Set window icon explicitly (helps in dev mode when PE icon may not show)
            let handle = app.handle().clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(win) = handle.get_webview_window("main") {
                    let icon_path = std::env::current_exe()
                        .ok()
                        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
                        .and_then(|target_dir| target_dir.parent().map(|p| p.to_path_buf()))
                        .map(|src_tauri| src_tauri.join("icons").join("icon.ico"));
                    if let Some(path) = icon_path.filter(|p| p.exists()) {
                        if let Ok(icon) = tauri::image::Image::from_path(&path) {
                            let _ = win.set_icon(icon.to_owned());
                        }
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            run_ai_command,
            get_app_data_dir,
            get_log_file_path,
            get_projects_dir,
            get_models_dir,
            discord_rpc_connect,
            discord_rpc_auto_reconnect,
            discord_rpc_connection_state,
            get_channel_info_command,
            start_recording,
            stop_recording,
            save_project_command,
            load_project_command,
            list_projects_command,
            list_projects_with_meta_command,
            auto_save_project_command,
            delete_project_command,
            purge_recent_command,
            set_project_saved_command,
            format_project_name_command,
            export_transcript,
            list_models_command,
            download_model_command,
            resolve_model_path_command,
            list_installed_model_names_command,
            open_models_dir_command,
            list_remote_models_command,
            transcribe_session_command,
            engine_status_command,
            list_audio_devices_command,
            measure_audio_level_command,
            download_engine_command,
            remove_engine_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

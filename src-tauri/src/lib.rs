mod audio;
mod discord_rpc;
mod export;
mod paths;
mod project;
mod session;
mod transcription;

use log::{debug, warn};
use tauri::Emitter;
use audio::{start_audio_capture, stop_audio_capture, AudioCaptureHandle};
use discord_rpc::{get_channel_info, save_tokens, load_tokens, DiscordRpcClient};
use export::{export_srt, export_vtt};
use paths::{app_data_dir, discord_tokens_path, models_dir, projects_dir};
use project::{auto_save_project, delete_project, format_project_name, list_projects, list_projects_with_meta, load_project, purge_old_recent, save_project};
use tauri_plugin_shell::ShellExt;
use transcription::{download_model, extract_segment, transcribe_via_api, write_wav_from_samples, RemoteTranscriptionConfig, WhisperCliBackend};
use session::{clear_live_segment_tx, flush_pending_if_elapsed, record_speaking_event, set_live_segment_tx, start_session, stop_session, SessionAudioPaths, SessionSegment, SessionState};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

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

#[tauri::command]
async fn discord_rpc_connect(
    app: tauri::AppHandle,
    client_id: String,
    client_secret: String,
    rpc_origin: String,
) -> Result<(), String> {
    let client = DiscordRpcClient::new(client_id.clone(), client_secret.clone(), rpc_origin.clone());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let refresh_token = client.connect(tx).await?;
    if let Some(refresh) = refresh_token {
        let path = discord_tokens_path(&app)?;
        save_tokens(
            &path,
            &discord_rpc::DiscordTokens {
                client_id,
                client_secret,
                rpc_origin,
                refresh_token: refresh,
            },
        )?;
    }
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
    Ok(())
}

#[tauri::command]
async fn discord_rpc_auto_reconnect(app: tauri::AppHandle) -> Result<bool, String> {
    let path = discord_tokens_path(&app)?;
    let tokens = match load_tokens(&path)? {
        Some(t) => t,
        None => return Ok(false),
    };
    let client = DiscordRpcClient::new(
        tokens.client_id.clone(),
        tokens.client_secret.clone(),
        tokens.rpc_origin.clone(),
    );
    let (tx, mut rx) = mpsc::unbounded_channel();
    let new_refresh = client
        .connect_with_refresh_token(tx, tokens.refresh_token)
        .await?;
    if let Some(refresh) = new_refresh {
        save_tokens(
            &path,
            &discord_rpc::DiscordTokens {
                client_id: tokens.client_id,
                client_secret: tokens.client_secret,
                rpc_origin: tokens.rpc_origin,
                refresh_token: refresh,
            },
        )?;
    }
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
    Ok(true)
}

#[tauri::command]
fn discord_rpc_connection_state() -> Result<String, String> {
    Ok(if get_channel_info().is_some() {
        "Connected".into()
    } else {
        "Disconnected".into()
    })
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
) -> Result<(), String> {
    let channel_info = get_channel_info().ok_or("Not connected to Discord. Connect in Settings first.")?;
    let user_labels: std::collections::HashMap<String, String> = channel_info.user_labels.clone();
    let buffer_ms = segment_merge_buffer_ms.unwrap_or(1000);
    let template = project_name_template.unwrap_or_else(|| "{guild}_{channel}_{timestamp}".to_string());
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
            && live_remote_base_url.as_ref().map_or(false, |u| !u.trim().is_empty())
            && live_remote_model.as_ref().map_or(false, |m| !m.trim().is_empty());
        let remote_config = use_remote.then(|| {
            RemoteTranscriptionConfig::new(
                live_remote_base_url.clone().unwrap_or_default(),
                live_remote_model.clone().unwrap_or_default(),
                live_remote_api_key.clone(),
            )
        });
        let model_path = live_model_path.clone();
        let whisper_path = (!use_remote).then(|| {
            std::env::current_exe().ok().and_then(|p| {
                let dir = p.parent()?;
                let exe = dir.join("whisper-cli.exe");
                if exe.exists() {
                    Some(exe)
                } else {
                    #[cfg(windows)]
                    {
                        let exe = dir.join("whisper-cli-x86_64-pc-windows-msvc.exe");
                        if exe.exists() {
                            return Some(exe);
                        }
                    }
                    None
                }
            })
        }).flatten();
        let use_sidecar = !use_remote && whisper_path.is_none() && app.shell().sidecar("whisper-cli").is_ok();
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
            while let Some(seg) = rx.recv().await {
                debug!("[live] segment received: {}..{} ms, user={}", seg.start_ms, seg.end_ms, seg.user_id);
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
                let temp_dir = match &temp_dir {
                    Some(d) => d.clone(),
                    None => {
                        warn!("[live] no temp_dir configured, skipping segment");
                        continue;
                    }
                };
                let _ = std::fs::create_dir_all(&temp_dir);
                let seg_path = temp_dir.join(format!("live_seg_{}.wav", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()));
                if write_wav_from_samples(&seg_path, &samples).is_err() {
                    warn!("[live] failed to write WAV for segment {}..{} ms", seg.start_ms, seg.end_ms);
                    continue;
                }
                let text = if use_remote {
                    match &remote_config {
                        Some(cfg) => transcribe_via_api(cfg, &seg_path).await.unwrap_or_default(),
                        None => String::new(),
                    }
                } else if let Some(ref exe) = whisper_path {
                    let model = model_path.as_ref().filter(|p| std::path::Path::new(p).exists());
                    if model.is_none() {
                        warn!("[live] no valid model path (missing or path does not exist), segment will have empty text");
                    }
                    if let Some(m) = model {
                        let exe = exe.clone();
                        let seg_path_buf = seg_path.clone();
                        let model_str = m.to_string();
                        tauri::async_runtime::spawn_blocking(move || {
                            let of_base = seg_path_buf.with_extension("");
                            let output = std::process::Command::new(&exe)
                                .args(["-m", &model_str, "-f", &seg_path_buf.to_string_lossy(), "-np", "-nt", "-otxt", "-of", &of_base.to_string_lossy()])
                                .output();
                            match output {
                                Ok(out) if out.status.success() => {
                                    let txt_path = seg_path_buf.with_extension("txt");
                                    let raw = std::fs::read_to_string(&txt_path).unwrap_or_default();
                                    let _ = std::fs::remove_file(&txt_path);
                                    raw.lines()
                                        .filter_map(|line| {
                                            let t = line.trim();
                                            if t.is_empty() { None }
                                            else if t.starts_with('[') && t.contains("-->") {
                                                t.find(']').map(|i| t[i + 1..].trim().to_string()).filter(|s| !s.is_empty())
                                            } else { Some(t.to_string()) }
                                        })
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                        .trim()
                                        .to_string()
                                }
                                _ => String::new(),
                            }
                        })
                        .await
                        .unwrap_or_default()
                    } else {
                        String::new()
                    }
                } else if use_sidecar {
                    if let Ok(sidecar) = app_handle.shell().sidecar("whisper-cli") {
                        let model = model_path.as_ref().filter(|p| std::path::Path::new(p).exists());
                        if let Some(m) = model {
                            let of_base = seg_path.with_extension("");
                            let output = sidecar
                                .args(["-m", m, "-f", &seg_path.to_string_lossy(), "-np", "-nt", "-otxt", "-of", &of_base.to_string_lossy()])
                                .output()
                                .await;
                            if let Ok(out) = output {
                                if out.status.success() {
                                    let txt_path = seg_path.with_extension("txt");
                                    let raw = std::fs::read_to_string(&txt_path).unwrap_or_default();
                                    let _ = std::fs::remove_file(&txt_path);
                                    raw.lines()
                                        .filter_map(|line| {
                                            let t = line.trim();
                                            if t.is_empty() { None }
                                            else if t.starts_with('[') && t.contains("-->") {
                                                t.find(']').map(|i| t[i + 1..].trim().to_string()).filter(|s| !s.is_empty())
                                            } else { Some(t.to_string()) }
                                        })
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                        .trim()
                                        .to_string()
                                } else { String::new() }
                            } else { String::new() }
                        } else { String::new() }
                    } else { String::new() }
                } else {
                    warn!("[live] no transcription backend (whisper-cli not found, sidecar unavailable)");
                    String::new()
                };
                let idx = LIVE_TRANSCRIPT_TEXTS.lock().unwrap().len();
                LIVE_TRANSCRIPT_TEXTS.lock().unwrap().push(text.clone());
                debug!("[live] emitted transcript-segment idx={} len={} preview={:?}", idx, text.len(), text.chars().take(50).collect::<String>());
                let _ = app_handle.emit("transcript-segment", serde_json::json!({ "segment": seg, "text": text, "index": idx }));
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
            s.transcript_texts = texts;
            while s.transcript_texts.len() < s.segments.len() {
                s.transcript_texts.push(String::new());
            }
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
fn list_projects_with_meta_command(app: tauri::AppHandle) -> Result<Vec<project::ProjectMeta>, String> {
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
fn purge_recent_command(app: tauri::AppHandle, retention_days: u64) -> Result<u32, String> {
    purge_old_recent(&app, retention_days)
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
    download_model(&models_dir, &model_name).await
}

#[tauri::command]
async fn transcribe_session_command(
    app: tauri::AppHandle,
    state: SessionState,
    model_path: Option<String>,
    transcription_mode: String,
    remote_base_url: Option<String>,
    remote_model: Option<String>,
    remote_api_key: Option<String>,
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
        && remote_base_url.as_ref().map_or(false, |u| !u.trim().is_empty())
        && remote_model.as_ref().map_or(false, |m| !m.trim().is_empty());

    let (model_path_buf, whisper_path, use_sidecar) = if use_remote {
        (std::path::PathBuf::new(), None, false)
    } else {
        let model_path = model_path.ok_or("No model path. Download a model (Settings) or select one.")?;
        let model_path_buf = std::path::Path::new(&model_path).to_path_buf();
        if !model_path_buf.exists() {
            return Err(format!("Model not found: {}", model_path));
        }
        let whisper_path = std::env::current_exe()
            .ok()
            .and_then(|p| {
                let dir = p.parent()?;
                let exe = dir.join("whisper-cli.exe");
                if exe.exists() {
                    Some(exe)
                } else {
                    #[cfg(windows)]
                    {
                        let exe = dir.join("whisper-cli-x86_64-pc-windows-msvc.exe");
                        if exe.exists() {
                            return Some(exe);
                        }
                    }
                    None
                }
            });
        let use_sidecar = whisper_path.is_none() && app.shell().sidecar("whisper-cli").is_ok();
        (model_path_buf, whisper_path, use_sidecar)
    };

    // Use app data dir instead of system temp - sidecar may have restricted access to %TEMP%
    let temp_dir = app_data_dir(&app)?.join("transcribe_temp");
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let mut texts = state.transcript_texts.clone();
    while texts.len() < state.segments.len() {
        texts.push(String::new());
    }

    let remote_config = use_remote.then(|| {
        RemoteTranscriptionConfig::new(
            remote_base_url.clone().unwrap_or_default(),
            remote_model.clone().unwrap_or_default(),
            remote_api_key.clone(),
        )
    });

    debug!(
        "[transcribe] START: {} segments, mode={}, temp_dir={}",
        state.segments.len(),
        if use_remote { "remote" } else { "integrated" },
        temp_dir.to_string_lossy()
    );

    let current_exe = std::env::current_exe().ok();
    debug!(
        "[transcribe] mode: whisper_path={:?}, use_sidecar={}, current_exe={:?}",
        whisper_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        use_sidecar,
        current_exe.as_ref().map(|p| p.to_string_lossy().to_string())
    );
    if !use_remote && whisper_path.is_none() && !use_sidecar {
        warn!(
            "[transcribe] No whisper binary found. Looked for whisper-cli.exe next to {:?}",
            current_exe.as_ref().and_then(|p| p.parent().map(|d| d.to_string_lossy().to_string()))
        );
    }

    for (i, seg) in state.segments.iter().enumerate() {
        let is_local = state
            .self_user_id
            .as_ref()
            .map_or(false, |id| id == &seg.user_id);
        let source_path = if is_local { mic_path } else { loopback_path };
        let segment_path = temp_dir.join(format!("seg_{}.wav", i));

        // Skip empty segments
        if seg.end_ms <= seg.start_ms {
            texts[i] = String::new();
            continue;
        }

        extract_segment(source_path, &segment_path, seg.start_ms, seg.end_ms)?;
        let seg_size = std::fs::metadata(&segment_path).ok().map(|m| m.len()).unwrap_or(0);
        let segment_path_str = segment_path.to_string_lossy().to_string();
        debug!(
            "[transcribe] segment {}: {} -> {} ms, source={:?}, seg_file={}, seg_size_bytes={}",
            i,
            seg.start_ms,
            seg.end_ms,
            source_path,
            segment_path_str,
            seg_size
        );

        let result = if use_remote {
            let config = remote_config.as_ref().ok_or("Remote config missing")?;
            transcribe_via_api(config, &segment_path).await
        } else if let Some(ref whisper_exe) = whisper_path {
            // Run whisper directly - same process, full file access
            debug!("[transcribe] segment {}: using direct Command, exe={:?}", i, whisper_exe);
            let txt_path = segment_path.with_extension("txt");
            let of_base = segment_path.with_extension("");
            let output = std::process::Command::new(whisper_exe)
                .args([
                    "-m",
                    model_path_buf.to_str().unwrap(),
                    "-f",
                    &segment_path_str,
                    "-np",
                    "-nt",
                    "-otxt",
                    "-of",
                    of_base.to_str().unwrap(),
                ])
                .output()
                .map_err(|e| format!("Failed to run whisper: {}", e))?;
            let exit = output.status.code().unwrap_or(-1);
            let stderr_s = String::from_utf8_lossy(&output.stderr);
            let stdout_s = String::from_utf8_lossy(&output.stdout);
            debug!(
                "[transcribe] segment {}: Whisper exit={}, stderr_len={}, stdout_len={}, txt_exists={}",
                i,
                exit,
                stderr_s.len(),
                stdout_s.len(),
                txt_path.exists()
            );
            if !output.status.success() {
                warn!(
                    "[transcribe] segment {}: Whisper failed. stderr={:?} stdout={:?}",
                    i,
                    stderr_s.chars().take(500).collect::<String>(),
                    stdout_s.chars().take(500).collect::<String>()
                );
            }
            if output.status.success() {
                let raw = std::fs::read_to_string(&txt_path).unwrap_or_default();
                debug!("[transcribe] segment {}: txt raw len={}, content={:?}", i, raw.len(), raw.chars().take(200).collect::<String>());
                let text = raw
                    .lines()
                    .filter_map(|line| {
                        let t = line.trim();
                        if t.is_empty() {
                            None
                        } else if t.starts_with('[') && t.contains("-->") {
                            t.find(']')
                                .map(|i| t[i + 1..].trim().to_string())
                                .filter(|s| !s.is_empty())
                        } else {
                            Some(t.to_string())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
                    .trim()
                    .to_string();
                let _ = std::fs::remove_file(&txt_path);
                debug!("[transcribe] segment {}: parsed text len={}, text={:?}", i, text.len(), text.chars().take(100).collect::<String>());
                Ok(text)
            } else {
                Err(format!("Whisper failed: {}", stderr_s.trim()))
            }
        } else if use_sidecar {
            debug!("[transcribe] segment {}: using sidecar", i);
            let sidecar = app.shell().sidecar("whisper-cli").map_err(|e| {
                format!(
                    "Whisper sidecar failed: {}. Place whisper-cli-x86_64-pc-windows-msvc.exe in src-tauri/binaries/ (see README there).",
                    e
                )
            })?;
            // Use -otxt -of to write to file: sidecar stdout capture can be unreliable
            let txt_path = segment_path.with_extension("txt");
            let output = sidecar
                .args([
                    "-m",
                    &model_path_buf.to_string_lossy(),
                    "-f",
                    &segment_path_str,
                    "-np",
                    "-nt",
                    "-otxt",
                    "-of",
                    &segment_path.with_extension("").to_string_lossy(),
                ])
                .output()
                .await
                .map_err(|e| format!("Failed to run whisper: {}", e))?;
            let exit = output.status.code().unwrap_or(-1);
            let stderr_s = String::from_utf8_lossy(&output.stderr);
            let stdout_s = String::from_utf8_lossy(&output.stdout);
            debug!(
                "[transcribe] segment {}: sidecar exit={}, txt_exists={}, stderr_len={}, stdout_len={}",
                i, exit, txt_path.exists(), stderr_s.len(), stdout_s.len()
            );
            if !output.status.success() {
                warn!(
                    "[transcribe] segment {}: sidecar failed. stderr={:?} stdout={:?}",
                    i,
                    stderr_s.chars().take(500).collect::<String>(),
                    stdout_s.chars().take(500).collect::<String>()
                );
            }
            if output.status.success() {
                let raw = std::fs::read_to_string(&txt_path).unwrap_or_default();
                debug!("[transcribe] segment {}: sidecar txt raw len={}, content={:?}", i, raw.len(), raw.chars().take(200).collect::<String>());
                let text = raw
                    .lines()
                    .filter_map(|line| {
                        let t = line.trim();
                        if t.is_empty() {
                            None
                        } else if t.starts_with('[') && t.contains("-->") {
                            t.find(']')
                                .map(|i| t[i + 1..].trim().to_string())
                                .filter(|s| !s.is_empty())
                        } else {
                            Some(t.to_string())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
                    .trim()
                    .to_string();
                let _ = std::fs::remove_file(&txt_path);
                debug!("[transcribe] segment {}: sidecar parsed text len={}, text={:?}", i, text.len(), text.chars().take(100).collect::<String>());
                Ok(text)
            } else {
                let err_msg = if stderr_s.trim().is_empty() && !stdout_s.trim().is_empty() {
                    format!("exit code {} (stdout: {})", exit, stdout_s.trim())
                } else if stderr_s.trim().is_empty() {
                    format!("exit code {} (no stderr)", exit)
                } else {
                    format!("{}", stderr_s.trim())
                };
                Err(format!("Whisper failed: {}", err_msg))
            }
        } else {
            debug!("[transcribe] segment {}: using WhisperCliBackend fallback", i);
            let backend = WhisperCliBackend::new(Some(model_path_buf.to_string_lossy().into_owned()), None);
            backend.transcribe_file(&segment_path)
        };

        match &result {
            Ok(t) => {
                debug!("[transcribe] segment {}: SUCCESS, text len={}, preview={:?}", i, t.len(), t.chars().take(80).collect::<String>());
                texts[i] = t.to_string();
            }
            Err(e) => {
                warn!("[transcribe] segment {}: FAILED: {}", i, e);
                let msg = if use_sidecar {
                    e.to_string()
                } else if e.contains("program not found") || e.contains("Failed to run whisper") {
                    format!(
                        "{}. Download whisper from https://github.com/ggml-org/whisper.cpp/releases, extract whisper-cli.exe, rename to whisper-cli-x86_64-pc-windows-msvc.exe, place in src-tauri/binaries/ (see README there).",
                        e
                    )
                } else {
                    e.to_string()
                };
                texts[i] = format!("[Transcription error: {}]", msg);
            }
        }
        let _ = std::fs::remove_file(&segment_path);
    }

    let non_empty: usize = texts.iter().filter(|t| !t.is_empty()).count();
    debug!(
        "[transcribe] DONE: {} segments, {} non-empty texts",
        texts.len(),
        non_empty
    );

    Ok(SessionState {
        transcript_texts: texts,
        ..state
    })
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

    let format = |out: fern::FormatCallback<'_>, message: &std::fmt::Arguments<'_>, record: &log::Record| {
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
        .plugin(tauri_plugin_log::Builder::default().skip_logger().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            paths::ensure_directories(app.handle())?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(true) = discord_rpc_auto_reconnect(handle).await {
                    log::info!("[d-scribe] Auto-reconnected to Discord");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            format_project_name_command,
            export_transcript,
            list_models_command,
            download_model_command,
            transcribe_session_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

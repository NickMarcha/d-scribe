mod audio;
mod discord_rpc;
mod export;
mod paths;
mod project;
mod session;
mod transcription;

use log::{debug, warn};
use audio::{start_audio_capture, stop_audio_capture, AudioCaptureHandle};
use discord_rpc::{get_channel_info, DiscordRpcClient};
use export::{export_srt, export_vtt};
use paths::{app_data_dir, models_dir, projects_dir};
use project::{format_project_name, list_projects, load_project, save_project};
use tauri_plugin_shell::ShellExt;
use transcription::{download_model, extract_segment, WhisperCliBackend};
use session::{record_speaking_event, start_session, stop_session, SessionAudioPaths, SessionSegment, SessionState};
use std::sync::Mutex;
use tokio::sync::mpsc;

#[tauri::command]
fn get_app_data_dir(app: tauri::AppHandle) -> Result<String, String> {
    app_data_dir(&app).map(|p| p.to_string_lossy().into_owned())
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
    client_id: String,
    client_secret: String,
    rpc_origin: String,
) -> Result<(), String> {
    let client = DiscordRpcClient::new(client_id, client_secret, rpc_origin);
    let (tx, mut rx) = mpsc::unbounded_channel();
    client.connect(tx).await?;
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
async fn discord_rpc_connection_state() -> Result<String, String> {
    Ok("Disconnected".into())
}

static AUDIO_HANDLE: Mutex<Option<AudioCaptureHandle>> = Mutex::new(None);
static SESSION_AUDIO_PATHS: Mutex<Option<(String, String)>> = Mutex::new(None);

#[tauri::command]
fn start_recording(
    output_path: String,
    mic_path: String,
    segment_merge_buffer_ms: Option<u64>,
) -> Result<(), String> {
    let channel_info = get_channel_info().ok_or("Not connected to Discord. Connect in Settings first.")?;
    let user_labels: std::collections::HashMap<String, String> = channel_info.user_labels.clone();
    let buffer_ms = segment_merge_buffer_ms.unwrap_or(1000);
    start_session(
        channel_info.guild_name,
        channel_info.channel_name,
        Some(channel_info.channel_id),
        channel_info.self_user_id,
        user_labels,
        buffer_ms,
    );
    let handle = start_audio_capture(
        std::path::Path::new(&output_path),
        std::path::Path::new(&mic_path),
    )?;
    *AUDIO_HANDLE.lock().unwrap() = Some(handle);
    *SESSION_AUDIO_PATHS.lock().unwrap() = Some((output_path, mic_path));
    Ok(())
}

#[tauri::command]
fn stop_recording(_app: tauri::AppHandle) -> Result<Option<SessionState>, String> {
    let paths = SESSION_AUDIO_PATHS.lock().unwrap().take();
    if let Some(handle) = AUDIO_HANDLE.lock().unwrap().take() {
        stop_audio_capture(handle)?;
    }
    let state = paths.and_then(|(loopback, microphone)| {
        stop_session(SessionAudioPaths {
            loopback: Some(loopback),
            microphone: Some(microphone),
        })
    });
    Ok(state)
}

#[tauri::command]
fn get_channel_info_command() -> Result<Option<serde_json::Value>, String> {
    Ok(get_channel_info().map(|c| {
        serde_json::json!({
            "channel_id": c.channel_id,
            "channel_name": c.channel_name,
            "guild_id": c.guild_id,
            "guild_name": c.guild_name,
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
    model_path: String,
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

    let model_path_buf = std::path::Path::new(&model_path);
    if !model_path_buf.exists() {
        return Err(format!("Model not found: {}", model_path));
    }

    // Use app data dir instead of system temp - sidecar may have restricted access to %TEMP%
    let temp_dir = app_data_dir(&app)?.join("transcribe_temp");
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let mut texts = state.transcript_texts.clone();
    while texts.len() < state.segments.len() {
        texts.push(String::new());
    }

    debug!(
        "[transcribe] START: {} segments, model={}, temp_dir={}",
        state.segments.len(),
        model_path,
        temp_dir.to_string_lossy()
    );

    // Prefer std::process::Command - sidecar can have sandbox/access issues on Windows
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

    let current_exe = std::env::current_exe().ok();
    debug!(
        "[transcribe] mode: whisper_path={:?}, use_sidecar={}, current_exe={:?}",
        whisper_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        use_sidecar,
        current_exe.as_ref().map(|p| p.to_string_lossy().to_string())
    );
    if whisper_path.is_none() && !use_sidecar {
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

        let result = if let Some(ref whisper_exe) = whisper_path {
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
            let backend = WhisperCliBackend::new(Some(model_path.clone()), None);
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            paths::ensure_directories(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_data_dir,
            get_projects_dir,
            get_models_dir,
            discord_rpc_connect,
            discord_rpc_connection_state,
            get_channel_info_command,
            start_recording,
            stop_recording,
            save_project_command,
            load_project_command,
            list_projects_command,
            format_project_name_command,
            export_transcript,
            list_models_command,
            download_model_command,
            transcribe_session_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

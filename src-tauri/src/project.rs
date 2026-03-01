//! Project file save/load.

use crate::paths;
use crate::session::{SessionAudioPaths, SessionSegment, SessionState};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Project file format (same as SessionState, for compatibility).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    pub session_id: String,
    pub created_at: u64,
    pub guild_name: Option<String>,
    #[serde(default)]
    pub guild_id: Option<String>,
    pub channel_name: Option<String>,
    pub channel_id: Option<String>,
    /// Discord channel type: 1=dm, 2=guild_voice, 3=group_dm
    #[serde(default)]
    pub channel_type: Option<u8>,
    #[serde(default)]
    pub live_mode_enabled: bool,
    #[serde(default)]
    pub self_user_id: Option<String>,
    #[serde(default)]
    pub user_labels: std::collections::HashMap<String, String>,
    pub segments: Vec<SessionSegment>,
    pub transcript_texts: Vec<String>,
    #[serde(default)]
    pub live_transcript_texts: Option<Vec<String>>,
    pub audio_paths: SessionAudioPaths,
}

impl From<SessionState> for ProjectFile {
    fn from(s: SessionState) -> Self {
        Self {
            session_id: s.session_id,
            created_at: s.created_at,
            guild_name: s.guild_name,
            guild_id: s.guild_id,
            channel_name: s.channel_name,
            channel_id: s.channel_id,
            channel_type: s.channel_type,
            live_mode_enabled: s.live_mode_enabled,
            self_user_id: s.self_user_id,
            user_labels: s.user_labels,
            segments: s.segments,
            transcript_texts: s.transcript_texts,
            live_transcript_texts: s.live_transcript_texts,
            audio_paths: s.audio_paths,
        }
    }
}

impl From<ProjectFile> for SessionState {
    fn from(p: ProjectFile) -> Self {
        Self {
            session_id: p.session_id,
            created_at: p.created_at,
            guild_name: p.guild_name,
            guild_id: p.guild_id,
            channel_name: p.channel_name,
            channel_id: p.channel_id,
            channel_type: p.channel_type,
            live_mode_enabled: p.live_mode_enabled,
            self_user_id: p.self_user_id,
            user_labels: p.user_labels,
            segments: p.segments,
            transcript_texts: p.transcript_texts,
            live_transcript_texts: p.live_transcript_texts,
            audio_paths: p.audio_paths,
        }
    }
}

pub fn save_project(_app: &tauri::AppHandle, path: &Path, state: &SessionState) -> Result<(), String> {
    let file = ProjectFile::from(state.clone());
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Auto-save session to recent folder. Uses session_id and created_at for uniqueness.
pub fn auto_save_project(app: &tauri::AppHandle, state: &SessionState) -> Result<String, String> {
    let dir = paths::recent_projects_dir(app)?;
    let safe_id: String = state
        .session_id
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let filename = format!("{}_{}.json", safe_id, state.created_at);
    let path = dir.join(&filename);
    save_project(app, &path, state)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Delete a project file and optionally its associated audio files.
pub fn delete_project(path: &Path, delete_audio: bool) -> Result<(), String> {
    if delete_audio {
        if let Ok(json) = std::fs::read_to_string(path) {
            if let Ok(file) = serde_json::from_str::<ProjectFile>(&json) {
                for p in [&file.audio_paths.loopback, &file.audio_paths.microphone] {
                    if let Some(ref pth) = p {
                        let p = Path::new(pth);
                        if p.exists() {
                            let _ = std::fs::remove_file(p);
                        }
                    }
                }
            }
        }
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Purge recent projects older than retention_days. Deletes JSON and associated audio files.
pub fn purge_old_recent(app: &tauri::AppHandle, retention_days: u64) -> Result<u32, String> {
    let dir = paths::recent_projects_dir(app)?;
    if !dir.exists() {
        return Ok(0);
    }
    let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
    let cutoff_secs = cutoff.timestamp() as u64;
    let mut purged = 0u32;
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "json" || e == "dscribe") {
            if let Ok(json) = std::fs::read_to_string(&path) {
                if let Ok(meta) = serde_json::from_str::<ProjectMetaPartial>(&json) {
                    if let Some(created) = meta.created_at {
                        if created < cutoff_secs {
                            let _ = delete_project(&path, true);
                            purged += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(purged)
}

pub fn load_project(path: &Path) -> Result<SessionState, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: ProjectFile = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(file.into())
}

pub fn list_projects(app: &tauri::AppHandle) -> Result<Vec<String>, String> {
    list_projects_with_meta(app).map(|v| v.into_iter().map(|p| p.name).collect())
}

/// Metadata for display in project list.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectMeta {
    pub name: String,
    pub path: String,
    pub guild_name: Option<String>,
    pub channel_name: Option<String>,
    pub created_at: u64,
}

/// Minimal struct for reading metadata without full deserialization.
#[derive(serde::Deserialize)]
struct ProjectMetaPartial {
    created_at: Option<u64>,
    guild_name: Option<String>,
    channel_name: Option<String>,
}

fn collect_projects_from_dir(dir: &Path) -> Result<Vec<ProjectMeta>, String> {
    let mut projects = Vec::new();
    if !dir.exists() {
        return Ok(projects);
    }
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() && path.extension().map_or(false, |e| e == "json" || e == "dscribe") {
            let path_str = path.to_string_lossy().into_owned();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let meta = std::fs::read_to_string(&path)
                .ok()
                .and_then(|json| serde_json::from_str::<ProjectMetaPartial>(&json).ok())
                .unwrap_or(ProjectMetaPartial {
                    created_at: None,
                    guild_name: None,
                    channel_name: None,
                });
            projects.push(ProjectMeta {
                name,
                path: path_str,
                guild_name: meta.guild_name,
                channel_name: meta.channel_name,
                created_at: meta.created_at.unwrap_or(0),
            });
        }
    }
    Ok(projects)
}

pub fn list_projects_with_meta(app: &tauri::AppHandle) -> Result<Vec<ProjectMeta>, String> {
    let projects_dir = paths::projects_dir(app)?;
    let recent_dir = paths::recent_projects_dir(app)?;
    let mut projects = collect_projects_from_dir(&projects_dir)?;
    if recent_dir != projects_dir {
        projects.extend(collect_projects_from_dir(&recent_dir)?);
    }
    projects.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(projects)
}

/// Generate project name from template.
/// Placeholders: {guild}, {channel}, {timestamp}, {date}, {time}
pub fn format_project_name(
    template: &str,
    guild: Option<&str>,
    channel: Option<&str>,
) -> String {
    let now = chrono::Utc::now();
    let timestamp = now.timestamp().to_string();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H-%M-%S").to_string();

    let mut s = template.to_string();
    s = s.replace("{guild}", guild.unwrap_or("Unknown"));
    s = s.replace("{channel}", channel.unwrap_or("Unknown"));
    s = s.replace("{timestamp}", &timestamp);
    s = s.replace("{date}", &date);
    s = s.replace("{time}", &time);
    s
}

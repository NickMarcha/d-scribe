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
    pub channel_name: Option<String>,
    pub channel_id: Option<String>,
    #[serde(default)]
    pub self_user_id: Option<String>,
    pub segments: Vec<SessionSegment>,
    pub transcript_texts: Vec<String>,
    pub audio_paths: SessionAudioPaths,
}

impl From<SessionState> for ProjectFile {
    fn from(s: SessionState) -> Self {
        Self {
            session_id: s.session_id,
            created_at: s.created_at,
            guild_name: s.guild_name,
            channel_name: s.channel_name,
            channel_id: s.channel_id,
            self_user_id: s.self_user_id,
            segments: s.segments,
            transcript_texts: s.transcript_texts,
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
            channel_name: p.channel_name,
            channel_id: p.channel_id,
            self_user_id: p.self_user_id,
            segments: p.segments,
            transcript_texts: p.transcript_texts,
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

pub fn load_project(path: &Path) -> Result<SessionState, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: ProjectFile = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(file.into())
}

pub fn list_projects(app: &tauri::AppHandle) -> Result<Vec<String>, String> {
    let dir = paths::projects_dir(app)?;
    let mut names = Vec::new();
    if dir.exists() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "json" || e == "dscribe") {
                if let Some(name) = path.file_stem() {
                    names.push(name.to_string_lossy().into_owned());
                }
            }
        }
    }
    names.sort();
    Ok(names)
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

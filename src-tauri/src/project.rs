//! Project file save/load.

use crate::paths;
use crate::session::{AiOutput, SessionAudioPaths, SessionSegment, SessionState};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Project file format (same as SessionState, for compatibility).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    pub session_id: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    #[serde(default)]
    pub ai_outputs: Vec<AiOutput>,
}

impl From<SessionState> for ProjectFile {
    fn from(s: SessionState) -> Self {
        Self {
            session_id: s.session_id,
            created_at: s.created_at,
            title: s.title,
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
            ai_outputs: s.ai_outputs,
        }
    }
}

impl From<ProjectFile> for SessionState {
    fn from(p: ProjectFile) -> Self {
        Self {
            session_id: p.session_id,
            created_at: p.created_at,
            title: p.title,
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
            ai_outputs: p.ai_outputs,
        }
    }
}

pub fn save_project(
    _app: &tauri::AppHandle,
    path: &Path,
    state: &SessionState,
) -> Result<(), String> {
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

/// Total size of a project file and its audio files, in bytes.
fn project_size(path: &Path, file: Option<&ProjectFile>) -> u64 {
    let mut size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if let Some(file) = file {
        for p in [&file.audio_paths.loopback, &file.audio_paths.microphone]
            .into_iter()
            .flatten()
        {
            size += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        }
    }
    size
}

/// Delete unsaved recordings (the recent folder): first those older than `retention_days`,
/// then the oldest until the rest fit in `max_bytes` (0 = no size limit). The project at
/// `keep_path` (the one open in the app) is never deleted. Deletes JSON and audio.
pub fn purge_old_recent(
    app: &tauri::AppHandle,
    retention_days: u64,
    max_bytes: u64,
    keep_path: Option<&str>,
) -> Result<u32, String> {
    let dir = paths::recent_projects_dir(app)?;
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(retention_days as i64)).timestamp() as u64;
    // (created_at, size, path), newest first
    let mut entries: Vec<(u64, u64, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !is_project_file(&path) || keep_path.is_some_and(|k| Path::new(k) == path) {
            continue;
        }
        let file = read_project_file(&path);
        let created = file.as_ref().map(|f| f.created_at).unwrap_or(0);
        entries.push((created, project_size(&path, file.as_ref()), path));
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0));

    let mut purged = 0u32;
    let mut total = 0u64;
    for (created, size, path) in entries {
        let over_size = max_bytes > 0 && total + size > max_bytes;
        if created < cutoff || over_size {
            if delete_project(&path, true).is_ok() {
                purged += 1;
            }
        } else {
            total += size;
        }
    }
    Ok(purged)
}

/// Move a project between the recent folder (unsaved, deleted by `purge_old_recent`) and the
/// projects folder (saved, kept until deleted). Audio files stay where they are. Returns the new path.
pub fn set_project_saved(app: &tauri::AppHandle, path: &Path, saved: bool) -> Result<String, String> {
    let dir = if saved {
        paths::projects_dir(app)?
    } else {
        paths::recent_projects_dir(app)?
    };
    let name = path.file_name().ok_or("Invalid project path")?;
    let target = dir.join(name);
    if target != path {
        std::fs::rename(path, &target).map_err(|e| e.to_string())?;
    }
    Ok(target.to_string_lossy().into_owned())
}

pub fn load_project(path: &Path) -> Result<SessionState, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: ProjectFile = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(file.into())
}

pub fn list_projects(app: &tauri::AppHandle) -> Result<Vec<String>, String> {
    list_projects_with_meta(app).map(|v| v.into_iter().map(|p| p.name).collect())
}

/// Metadata for the recordings overview.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectMeta {
    pub name: String,
    pub path: String,
    pub title: Option<String>,
    pub guild_name: Option<String>,
    pub channel_name: Option<String>,
    /// Discord channel type: 1=dm, 2=guild_voice, 3=group_dm
    pub channel_type: Option<u8>,
    pub created_at: u64,
    /// Saved projects are kept until deleted; unsaved ones are in the recent folder.
    pub saved: bool,
    pub duration_ms: u64,
    /// Everyone who spoke, most talk time first.
    pub speakers: Vec<String>,
    pub transcribed: bool,
    pub size_bytes: u64,
}

fn is_project_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .map_or(false, |e| e == "json" || e == "dscribe")
}

fn read_project_file(path: &Path) -> Option<ProjectFile> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

/// Recording length: from the loopback WAV (16 kHz mono 16-bit = 32 bytes/ms), else the last segment end.
fn duration_ms(file: &ProjectFile) -> u64 {
    let from_audio = file
        .audio_paths
        .loopback
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len().saturating_sub(44) / 32);
    from_audio.unwrap_or_else(|| file.segments.iter().map(|s| s.end_ms).max().unwrap_or(0))
}

fn speakers(file: &ProjectFile) -> Vec<String> {
    let mut talk: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for seg in &file.segments {
        let name = file
            .user_labels
            .get(&seg.user_id)
            .or(seg.speaker_name.as_ref())
            .cloned()
            .unwrap_or_else(|| seg.user_id.clone());
        *talk.entry(name).or_default() += seg.end_ms.saturating_sub(seg.start_ms);
    }
    let mut names: Vec<(String, u64)> = talk.into_iter().collect();
    names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    names.into_iter().map(|(n, _)| n).collect()
}

fn collect_projects_from_dir(dir: &Path, saved: bool) -> Result<Vec<ProjectMeta>, String> {
    let mut projects = Vec::new();
    if !dir.exists() {
        return Ok(projects);
    }
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !is_project_file(&path) {
            continue;
        }
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file = read_project_file(&path);
        let size_bytes = project_size(&path, file.as_ref());
        let path_str = path.to_string_lossy().into_owned();
        projects.push(match file {
            Some(f) => ProjectMeta {
                name,
                path: path_str,
                duration_ms: duration_ms(&f),
                speakers: speakers(&f),
                transcribed: f.transcript_texts.iter().any(|t| !t.trim().is_empty()),
                title: f.title,
                guild_name: f.guild_name,
                channel_name: f.channel_name,
                channel_type: f.channel_type,
                created_at: f.created_at,
                saved,
                size_bytes,
            },
            None => ProjectMeta {
                name,
                path: path_str,
                title: None,
                guild_name: None,
                channel_name: None,
                channel_type: None,
                created_at: 0,
                saved,
                duration_ms: 0,
                speakers: Vec::new(),
                transcribed: false,
                size_bytes,
            },
        });
    }
    Ok(projects)
}

pub fn list_projects_with_meta(app: &tauri::AppHandle) -> Result<Vec<ProjectMeta>, String> {
    let projects_dir = paths::projects_dir(app)?;
    let recent_dir = paths::recent_projects_dir(app)?;
    let mut projects = collect_projects_from_dir(&projects_dir, true)?;
    projects.extend(collect_projects_from_dir(&recent_dir, false)?);
    projects.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(projects)
}

/// Generate project name from template.
/// Placeholders: {guild}, {channel}, {timestamp}, {date}, {time}
pub fn format_project_name(template: &str, guild: Option<&str>, channel: Option<&str>) -> String {
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

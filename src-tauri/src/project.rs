//! Recordings on disk: one folder per call under `recordings/`, holding `recording.json`
//! (segments, transcript, title, saved flag, AI results) and the audio files.

use crate::paths;
use crate::session::SessionState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const PROJECT_FILE: &str = "recording.json";
pub const OTHERS_AUDIO: &str = "others.wav";
pub const YOU_AUDIO: &str = "you.wav";

/// `recording.json`. Audio paths are stored relative to the folder, so a folder can be moved.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectFile {
    #[serde(flatten)]
    state: SessionState,
    /// Saved recordings are kept until deleted; unsaved ones are removed by `purge_old_recent`.
    #[serde(default)]
    saved: bool,
}

fn read_project_file(path: &Path) -> Option<ProjectFile> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

fn write_project_file(path: &Path, file: &ProjectFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Audio path as stored: just the file name when it sits in the recording's folder.
fn relative_to(dir: &Path, audio: &Option<String>) -> Option<String> {
    audio.as_ref().map(|a| {
        let p = Path::new(a);
        match (p.parent(), p.file_name()) {
            (Some(parent), Some(name)) if parent == dir => name.to_string_lossy().into_owned(),
            _ => a.clone(),
        }
    })
}

fn resolve_in(dir: &Path, audio: &Option<String>) -> Option<String> {
    audio.as_ref().map(|a| dir.join(a).to_string_lossy().into_owned())
}

fn folder_of(path: &Path) -> Result<&Path, String> {
    path.parent().ok_or_else(|| "Invalid recording path".to_string())
}

/// Writes `state` to `path`, keeping the saved flag already on disk.
pub fn save_project(path: &Path, state: &SessionState) -> Result<(), String> {
    let dir = folder_of(path)?;
    let saved = read_project_file(path).is_some_and(|f| f.saved);
    let mut state = state.clone();
    state.audio_paths.loopback = relative_to(dir, &state.audio_paths.loopback);
    state.audio_paths.microphone = relative_to(dir, &state.audio_paths.microphone);
    write_project_file(path, &ProjectFile { state, saved })
}

pub fn load_project(path: &Path) -> Result<SessionState, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: ProjectFile = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let dir = folder_of(path)?;
    let mut state = file.state;
    state.audio_paths.loopback = resolve_in(dir, &state.audio_paths.loopback);
    state.audio_paths.microphone = resolve_in(dir, &state.audio_paths.microphone);
    Ok(state)
}

/// Saves a just-stopped recording next to its audio. Returns the `recording.json` path.
pub fn auto_save_project(state: &SessionState) -> Result<String, String> {
    let audio = state
        .audio_paths
        .loopback
        .as_ref()
        .or(state.audio_paths.microphone.as_ref())
        .ok_or("Recording has no audio files")?;
    let path = folder_of(Path::new(audio))?.join(PROJECT_FILE);
    save_project(&path, state)?;
    Ok(path.to_string_lossy().into_owned())
}

/// The recording's folder, if `path` is a `recording.json` directly inside `recordings/`.
fn recording_folder(app: &tauri::AppHandle, path: &Path) -> Result<PathBuf, String> {
    let root = paths::recordings_dir(app)?;
    let dir = folder_of(path)?;
    if path.file_name().is_some_and(|n| n == PROJECT_FILE) && dir.parent() == Some(root.as_path()) {
        Ok(dir.to_path_buf())
    } else {
        Err(format!("Only recordings in {} can be deleted here", root.display()))
    }
}

/// Deletes a recording's folder (transcript and audio).
pub fn delete_project(app: &tauri::AppHandle, path: &Path) -> Result<(), String> {
    let dir = recording_folder(app, path)?;
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

pub fn set_project_saved(path: &Path, saved: bool) -> Result<(), String> {
    let mut file = read_project_file(path).ok_or("Could not read the recording")?;
    file.saved = saved;
    write_project_file(path, &file)
}

/// Total size of the files in a folder, in bytes.
fn folder_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// `recording.json` of every recording folder.
fn project_files(app: &tauri::AppHandle) -> Result<Vec<PathBuf>, String> {
    let root = paths::recordings_dir(app)?;
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let file = entry.map_err(|e| e.to_string())?.path().join(PROJECT_FILE);
        if file.is_file() {
            files.push(file);
        }
    }
    Ok(files)
}

/// Delete unsaved recordings: first those older than `retention_days`, then the oldest until
/// the rest fit in `max_bytes` (0 = no size limit). The recording at `keep_path` (the one open
/// in the app) is never deleted.
pub fn purge_old_recent(
    app: &tauri::AppHandle,
    retention_days: u64,
    max_bytes: u64,
    keep_path: Option<&str>,
) -> Result<u32, String> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(retention_days as i64)).timestamp() as u64;
    // (created_at, size, path), newest first
    let mut entries: Vec<(u64, u64, PathBuf)> = Vec::new();
    for path in project_files(app)? {
        if keep_path.is_some_and(|k| Path::new(k) == path) {
            continue;
        }
        let Some(file) = read_project_file(&path) else { continue };
        if file.saved {
            continue;
        }
        entries.push((file.state.created_at, folder_size(folder_of(&path)?), path));
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0));

    let mut purged = 0u32;
    let mut total = 0u64;
    for (created, size, path) in entries {
        let over_size = max_bytes > 0 && total + size > max_bytes;
        if created < cutoff || over_size {
            if delete_project(app, &path).is_ok() {
                purged += 1;
            }
        } else {
            total += size;
        }
    }
    Ok(purged)
}

/// Metadata for the recordings overview.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectMeta {
    /// Folder name.
    pub name: String,
    /// Path of `recording.json`.
    pub path: String,
    pub title: Option<String>,
    pub guild_name: Option<String>,
    pub channel_name: Option<String>,
    /// Discord channel type: 1=dm, 2=guild_voice, 3=group_dm
    pub channel_type: Option<u8>,
    pub created_at: u64,
    pub saved: bool,
    pub duration_ms: u64,
    /// Everyone who spoke, most talk time first.
    pub speakers: Vec<String>,
    /// Everyone else in the call when it started.
    pub others: Vec<String>,
    pub transcribed: bool,
    pub size_bytes: u64,
}

/// Recording length: from the others WAV (16 kHz mono 16-bit = 32 bytes/ms), else the last segment end.
fn duration_ms(dir: &Path, state: &SessionState) -> u64 {
    let from_audio = resolve_in(dir, &state.audio_paths.loopback)
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len().saturating_sub(44) / 32);
    from_audio.unwrap_or_else(|| state.segments.iter().map(|s| s.end_ms).max().unwrap_or(0))
}

fn speakers(state: &SessionState) -> Vec<String> {
    let mut talk: HashMap<String, u64> = HashMap::new();
    for seg in &state.segments {
        let name = state
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

pub fn list_projects_with_meta(app: &tauri::AppHandle) -> Result<Vec<ProjectMeta>, String> {
    let mut projects = Vec::new();
    for path in project_files(app)? {
        let Some(file) = read_project_file(&path) else { continue };
        let dir = folder_of(&path)?;
        let s = file.state;
        projects.push(ProjectMeta {
            name: dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            duration_ms: duration_ms(dir, &s),
            speakers: speakers(&s),
            others: others(&s.user_labels, s.self_user_id.as_deref()),
            transcribed: s.transcript_texts.iter().any(|t| !t.trim().is_empty()),
            size_bytes: folder_size(dir),
            title: s.title,
            guild_name: s.guild_name,
            channel_name: s.channel_name,
            channel_type: s.channel_type,
            created_at: s.created_at,
            saved: file.saved,
        });
    }
    projects.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(projects)
}

fn others(user_labels: &HashMap<String, String>, self_user_id: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = user_labels
        .iter()
        .filter(|(id, _)| Some(id.as_str()) != self_user_id)
        .map(|(_, name)| name.clone())
        .collect();
    names.sort();
    names
}

/// Where a call happened, for its folder name: "Server - channel", "Call with Name", ...
pub fn place_name(
    guild: Option<&str>,
    channel: Option<&str>,
    channel_type: Option<u8>,
    user_labels: &HashMap<String, String>,
    self_user_id: Option<&str>,
) -> String {
    let channel = channel.filter(|c| !c.trim().is_empty());
    let others = others(user_labels, self_user_id);
    let with_others = (!others.is_empty()).then(|| format!("with {}", others.join(", ")));
    match (guild, channel_type) {
        (Some(g), _) => [Some(g), channel].into_iter().flatten().collect::<Vec<_>>().join(" - "),
        (None, Some(1)) => with_others.map_or("Direct call".into(), |w| format!("Call {w}")),
        (None, Some(3)) => channel
            .map(String::from)
            .or(with_others.map(|w| format!("Group call {w}")))
            .unwrap_or("Group call".into()),
        _ => channel.unwrap_or("Call").to_string(),
    }
}

/// Makes a text safe as a Windows folder name.
fn safe_folder_name(name: &str, max_chars: usize) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .take(max_chars)
        .collect();
    cleaned.trim().trim_end_matches('.').trim().to_string()
}

/// Creates the folder for a new recording: "2026-10-12 14.32 Call with Name" (local start time).
pub fn create_recording_dir(app: &tauri::AppHandle, place: &str) -> Result<PathBuf, String> {
    let root = paths::recordings_dir(app)?;
    let base = format!(
        "{} {}",
        chrono::Local::now().format("%Y-%m-%d %H.%M"),
        safe_folder_name(place, 80)
    );
    let base = base.trim().to_string();
    let mut dir = root.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{base} ({n})"));
        n += 1;
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_names() {
        let labels: HashMap<String, String> =
            [("1".to_string(), "Me".to_string()), ("2".to_string(), "Haaroon".to_string())].into();
        assert_eq!(place_name(None, None, Some(1), &labels, Some("1")), "Call with Haaroon");
        assert_eq!(place_name(Some("INF-252"), Some("General"), Some(2), &labels, Some("1")), "INF-252 - General");
        assert_eq!(place_name(None, Some(""), Some(3), &labels, Some("1")), "Group call with Haaroon");
        assert_eq!(place_name(None, None, Some(1), &HashMap::new(), None), "Direct call");
        assert_eq!(safe_folder_name("a/b: c. ", 80), "a_b_ c");
    }
}

//! Path utilities for app data, projects, and models directories.

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Get the app data directory (e.g. %APPDATA%/d-scribe on Windows).
pub fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

/// Get the projects directory, creating it if necessary.
pub fn projects_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?.join("projects");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Get the recent (auto-saved) projects directory.
pub fn recent_projects_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = projects_dir(app)?.join("recent");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Get the models directory, creating it if necessary.
pub fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?.join("models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Get the path to the settings file.
#[allow(dead_code)]
pub fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("settings.json"))
}

/// Get the path to the Discord tokens file (for refresh token persistence).
pub fn discord_tokens_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("discord_tokens.json"))
}

/// Get the log file path (e.g. %APPDATA%/d-scribe/logs/d-scribe.log on Windows).
pub fn log_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?.join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("d-scribe.log"))
}

/// Ensure all app directories exist.
pub fn ensure_directories(app: &AppHandle) -> Result<(), String> {
    app_data_dir(app)?;
    projects_dir(app)?;
    models_dir(app)?;
    let _ = log_file_path(app);
    Ok(())
}

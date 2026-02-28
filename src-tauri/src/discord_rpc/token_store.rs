//! Persist Discord tokens for auto-reconnect.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordTokens {
    pub client_id: String,
    pub client_secret: String,
    pub rpc_origin: String,
    pub refresh_token: String,
}

pub fn save_tokens(path: &Path, tokens: &DiscordTokens) -> Result<(), String> {
    let json = serde_json::to_string_pretty(tokens).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_tokens(path: &Path) -> Result<Option<DiscordTokens>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let tokens: DiscordTokens = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(Some(tokens))
}

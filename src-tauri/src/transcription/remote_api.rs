//! OpenAI-compatible transcription API backend (Voxtral, open-asr-server, etc.)

use std::path::Path;

/// Configuration for remote transcription API.
#[derive(Debug, Clone)]
pub struct RemoteTranscriptionConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

impl RemoteTranscriptionConfig {
    pub fn new(base_url: String, model: String, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.trim().to_string(),
            model,
            api_key,
        }
    }
}

/// Transcribe audio via OpenAI-compatible API.
/// POSTs to base_url (user provides full endpoint, e.g. http://localhost:8000/v1/audio/transcriptions).
pub async fn transcribe_via_api(
    config: &RemoteTranscriptionConfig,
    audio_path: &Path,
) -> Result<String, String> {
    let bytes = std::fs::read(audio_path).map_err(|e| e.to_string())?;
    let file_name = audio_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav");

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name.to_string())
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", config.model.clone());

    let client = reqwest::Client::new();
    let mut req = client
        .post(&config.base_url)
        .multipart(form);

    if let Some(ref key) = config.api_key {
        req = req.bearer_auth(key);
    }

    let response = req.send().await.map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API error {}: {}", status, body));
    }

    let json: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let text = json
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(text)
}

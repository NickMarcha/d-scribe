//! Whisper.cpp CLI sidecar backend.

use super::backend::{TranscriptSegment, TranscriptionBackend};
use std::path::Path;
use std::process::Command;

pub struct WhisperCliBackend {
    pub model_path: Option<String>,
    pub binary_path: Option<String>,
    pub language_code: Option<String>,
}

impl WhisperCliBackend {
    pub fn new(
        model_path: Option<String>,
        binary_path: Option<String>,
        language_code: Option<String>,
    ) -> Self {
        Self {
            model_path,
            binary_path,
            language_code,
        }
    }

    /// Transcribe using system binary (e.g. from PATH or custom path).
    /// Returns the raw transcribed text.
    pub fn transcribe_file(&self, audio_path: &Path) -> Result<String, String> {
        let model = self
            .model_path
            .as_ref()
            .ok_or("No model path configured")?;
        let model_path = Path::new(model);
        if !model_path.exists() {
            return Err(format!("Model not found: {}", model));
        }

        let binary = self
            .binary_path
            .as_deref()
            .unwrap_or("main");
        let mut args: Vec<&str> = vec![
            "-m",
            model_path.to_str().unwrap(),
            "-f",
            audio_path.to_str().unwrap(),
        ];
        if let Some(ref code) = self.language_code {
            args.push("-l");
            args.push(code);
        }
        let output = Command::new(binary)
            .args(args)
            .output()
            .map_err(|e| format!("Failed to run whisper: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Whisper failed: {}", stderr));
        }

        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.trim().to_string())
    }
}

impl TranscriptionBackend for WhisperCliBackend {
    fn id(&self) -> &'static str {
        "whisper-cli"
    }

    fn name(&self) -> &'static str {
        "Whisper (CLI)"
    }

    fn is_available(&self) -> bool {
        self.model_path.as_ref().map_or(false, |p| Path::new(p).exists())
    }

    fn transcribe(&self, audio_path: &Path) -> Result<Vec<TranscriptSegment>, String> {
        let text = self.transcribe_file(audio_path)?;
        if text.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![TranscriptSegment {
            start_ms: 0,
            end_ms: 0,
            speaker_id: String::new(),
            speaker_name: None,
            text,
        }])
    }
}

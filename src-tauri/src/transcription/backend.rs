//! Transcription backend trait and types.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single transcribed segment with speaker and timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker_id: String,
    pub speaker_name: Option<String>,
    pub text: String,
}

/// Trait for transcription backends.
#[allow(dead_code)]
pub trait TranscriptionBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn is_available(&self) -> bool;
    fn transcribe(&self, audio_path: &Path) -> Result<Vec<TranscriptSegment>, String>;
}

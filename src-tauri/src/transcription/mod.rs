//! Pluggable transcription backends.

mod backend;
mod model_download;
mod wav_extract;
mod whisper_cli;

pub use model_download::download_model;
pub use wav_extract::extract_segment;
pub use whisper_cli::WhisperCliBackend;

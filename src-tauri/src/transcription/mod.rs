//! Pluggable transcription backends.

mod backend;
mod model_download;
mod remote_api;
mod wav_extract;
mod whisper_cli;

pub use model_download::download_model;
pub use remote_api::{transcribe_via_api, RemoteTranscriptionConfig};
pub use wav_extract::{extract_segment, write_wav_from_samples};
pub use whisper_cli::WhisperCliBackend;

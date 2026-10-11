//! Pluggable transcription backends.

mod backend;
mod download;
mod engine;
mod model_download;
mod remote_api;
mod wav_extract;

pub use model_download::{
    download_model_with_progress, list_installed_model_names, resolve_model_path,
};
pub use remote_api::{list_models, transcribe_via_api, RemoteTranscriptionConfig};
pub use wav_extract::{extract_segment, rms, write_wav_from_samples};
pub use engine::{
    download_engine, engine_dir, installed_engine, is_installed, EngineKind, WhisperServer,
};

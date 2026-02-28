//! Audio capture for loopback (system output) and microphone.

#[cfg(windows)]
mod capture;

#[cfg(windows)]
pub use capture::{start_audio_capture, stop_audio_capture, AudioCaptureHandle};

#[cfg(not(windows))]
pub fn start_audio_capture(
    _output_path: &std::path::Path,
    _mic_path: &std::path::Path,
) -> Result<AudioCaptureHandle, String> {
    Err("Audio capture is only supported on Windows".into())
}

#[cfg(not(windows))]
pub fn stop_audio_capture(_handle: AudioCaptureHandle) -> Result<(), String> {
    Err("Audio capture is only supported on Windows".into())
}

#[cfg(not(windows))]
pub struct AudioCaptureHandle;

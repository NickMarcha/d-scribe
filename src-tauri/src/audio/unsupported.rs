//! Audio backend for systems without one yet. A Linux (PipeWire) or macOS backend goes in its
//! own file with the same two functions, selected in `mod.rs`.

use super::{AudioDevices, Source, Track};
use std::sync::atomic::AtomicBool;

const UNSUPPORTED: &str = "Recording isn't supported on this system yet.";

pub fn list_devices() -> Result<AudioDevices, String> {
    Err(UNSUPPORTED.into())
}

pub fn capture(_source: &Source, _stop: &AtomicBool, _track: &mut dyn Track) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

//! Export transcript to SRT and VTT formats.

mod srt;
mod vtt;

use crate::session::SessionSegment;
use std::path::Path;

/// Export transcript segments to SRT format.
pub fn export_srt(
    path: &Path,
    segments: &[SessionSegment],
    texts: &[String],
) -> Result<(), String> {
    srt::write_srt(path, segments, texts)
}

/// Export transcript segments to VTT format.
pub fn export_vtt(
    path: &Path,
    segments: &[SessionSegment],
    texts: &[String],
) -> Result<(), String> {
    vtt::write_vtt(path, segments, texts)
}

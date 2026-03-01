//! SRT (SubRip) subtitle format writer.

use crate::session::SessionSegment;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn ms_to_srt_time(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let mins = (ms % 3_600_000) / 60_000;
    let secs = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{:02}:{:02}:{:02},{:03}", hours, mins, secs, millis)
}

pub fn write_srt(path: &Path, segments: &[SessionSegment], texts: &[String]) -> Result<(), String> {
    let mut file = File::create(path).map_err(|e| e.to_string())?;

    for (i, (seg, text)) in segments.iter().zip(texts.iter()).enumerate() {
        let speaker = seg.speaker_name.as_deref().unwrap_or(&seg.user_id);
        let line = format!("[{}]: {}", speaker, text);
        writeln!(file, "{}", i + 1).map_err(|e| e.to_string())?;
        writeln!(
            file,
            "{} --> {}",
            ms_to_srt_time(seg.start_ms),
            ms_to_srt_time(seg.end_ms)
        )
        .map_err(|e| e.to_string())?;
        writeln!(file, "{}", line).map_err(|e| e.to_string())?;
        writeln!(file).map_err(|e| e.to_string())?;
    }

    Ok(())
}

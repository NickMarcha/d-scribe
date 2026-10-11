//! Extract a time range from a WAV file.

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::path::Path;

/// Write raw samples to a WAV file. 16 kHz mono 16-bit.
pub fn write_wav_from_samples(path: &Path, samples: &[i16]) -> Result<(), String> {
    let mut writer = WavWriter::create(
        path,
        WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;
    for &s in samples {
        writer.write_sample(s).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(())
}

/// Root mean square level of 16-bit samples (0 for none). Silence is well under 100,
/// speech usually in the thousands.
pub fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Extract samples from start_ms to end_ms (inclusive of start, exclusive of end)
/// and write to output_path. Returns the extracted samples' RMS level and count; the count is
/// short or 0 when the range runs past the end of the file.
/// Assumes 16 kHz mono 16-bit PCM input.
pub fn extract_segment(
    input_path: &Path,
    output_path: &Path,
    start_ms: u64,
    end_ms: u64,
) -> Result<(f32, usize), String> {
    let mut reader = WavReader::open(input_path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.sample_rate != 16000 || spec.channels != 1 || spec.bits_per_sample != 16 {
        return Err(format!(
            "Expected 16kHz mono 16-bit, got {}Hz {}ch {}bit",
            spec.sample_rate, spec.channels, spec.bits_per_sample
        ));
    }

    // At 16 kHz: 1 ms = 16 samples
    let start_sample = start_ms * 16;
    let end_sample = end_ms * 16;
    let count = end_sample.saturating_sub(start_sample) as usize;

    if start_sample >= reader.duration() as u64 {
        return Ok((0.0, 0));
    }
    reader.seek(start_sample as u32).map_err(|e| e.to_string())?;
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .take(count)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut writer = WavWriter::create(
        output_path,
        WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;
    for &s in &samples {
        writer.write_sample(s).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok((rms(&samples), samples.len()))
}

//! Writes one track to a WAV file (16 kHz mono 16-bit, what whisper.cpp wants) and keeps it
//! aligned with wall-clock time: loopback streams deliver nothing while the source is silent,
//! and segment timestamps index into the file.

use super::buffer::AudioBuffer;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const SAMPLE_RATE: u32 = 16000;
pub const CHANNELS: u16 = 1;
const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;

pub struct WavRecorder {
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    written: u64,
    started: Instant,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
}

impl WavRecorder {
    /// Creates the file. The clock that `pad_to_now` follows starts here.
    pub fn create(path: &Path, buffer: Option<Arc<Mutex<AudioBuffer>>>) -> Result<Self, String> {
        let writer = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels: CHANNELS,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { writer, written: 0, started: Instant::now(), buffer })
    }

    /// Appends samples, and copies them to the live transcription buffer if there is one.
    pub fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        for &s in samples {
            self.writer.write_sample(s).map_err(|e| e.to_string())?;
        }
        self.written += samples.len() as u64;
        if let Some(ref buf) = self.buffer {
            if let Ok(mut guard) = buf.lock() {
                for &s in samples {
                    guard.push(s);
                }
            }
        }
        Ok(())
    }

    /// Fills with silence up to `elapsed_ms` after the start.
    pub fn pad_to(&mut self, elapsed_ms: u64) -> Result<(), String> {
        let expected = elapsed_ms * SAMPLES_PER_MS;
        if self.written < expected {
            let gap = vec![0i16; (expected - self.written) as usize];
            self.write(&gap)?;
        }
        Ok(())
    }

    /// Fills with silence up to now.
    pub fn pad_to_now(&mut self) -> Result<(), String> {
        self.pad_to(self.started.elapsed().as_millis() as u64)
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.pad_to_now()?;
        self.writer.finalize().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_wav(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("d-scribe-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn read(path: &Path) -> (hound::WavSpec, Vec<i16>) {
        let mut reader = hound::WavReader::open(path).unwrap();
        let samples = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        (reader.spec(), samples)
    }

    #[test]
    fn writes_16khz_mono_16bit() {
        let path = temp_wav("a.wav");
        let mut rec = WavRecorder::create(&path, None).unwrap();
        rec.write(&[1, 2, 3]).unwrap();
        rec.writer.finalize().unwrap();
        let (spec, samples) = read(&path);
        assert_eq!((spec.sample_rate, spec.channels, spec.bits_per_sample), (16000, 1, 16));
        assert_eq!(samples, [1, 2, 3]);
    }

    #[test]
    fn pads_silence_so_time_indexes_the_file() {
        let path = temp_wav("b.wav");
        let mut rec = WavRecorder::create(&path, None).unwrap();
        rec.write(&[7; 16]).unwrap(); // 1 ms of audio
        rec.pad_to(10).unwrap(); // nothing arrived until 10 ms
        rec.write(&[9; 16]).unwrap();
        rec.pad_to(5).unwrap(); // already past 5 ms: no-op
        rec.writer.finalize().unwrap();
        let (_, samples) = read(&path);
        assert_eq!(samples.len(), 11 * 16);
        assert!(samples[..16].iter().all(|&s| s == 7));
        assert!(samples[16..160].iter().all(|&s| s == 0));
        // The sample at 10 ms is the audio that arrived at 10 ms: segment math is ms * 16.
        assert!(samples[10 * 16..].iter().all(|&s| s == 9));
    }

    #[test]
    fn finish_pads_to_wall_clock() {
        let path = temp_wav("c.wav");
        let rec = WavRecorder::create(&path, None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        rec.finish().unwrap();
        let (_, samples) = read(&path);
        assert!(samples.len() >= 50 * 16, "got {} samples", samples.len());
    }

    #[test]
    fn copies_to_live_buffer() {
        let path = temp_wav("d.wav");
        let buffer = Arc::new(Mutex::new(AudioBuffer::new()));
        let mut rec = WavRecorder::create(&path, Some(buffer.clone())).unwrap();
        rec.write(&[5; 32]).unwrap();
        rec.pad_to(4).unwrap();
        assert_eq!(buffer.lock().unwrap().len(), 64);
        assert_eq!(buffer.lock().unwrap().extract(0, 2), vec![5; 32]);
    }
}

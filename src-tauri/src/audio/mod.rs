//! Audio capture for the other participants (Discord's audio or an output device) and the
//! microphone.
//!
//! What's shared lives here: the two recording threads, the start handshake, WAV writing and
//! the level test. Each system has a backend module with the same two functions:
//! `list_devices` and `capture`, which opens a `Source` and feeds a `Track` until `stop` is set.

// The stub backend for systems without one never feeds a track.
#![cfg_attr(not(windows), allow(dead_code))]

mod buffer;
mod wav;

#[cfg(windows)]
#[path = "windows.rs"]
mod backend;

#[cfg(not(windows))]
#[path = "unsupported.rs"]
mod backend;

pub use buffer::AudioBuffer;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use wav::WavRecorder;

/// Where each track comes from. `None` (or empty) means the default.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSources {
    /// Input device id; default: the system's default input.
    pub mic_device_id: Option<String>,
    /// Output device id to record everything it plays; default: only the Discord app's audio.
    pub others_device_id: Option<String>,
}

#[derive(Serialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct AudioDevices {
    pub inputs: Vec<AudioDevice>,
    pub outputs: Vec<AudioDevice>,
    pub default_input: Option<String>,
    pub default_output: Option<String>,
}

/// What a backend records. Device ids are the ones `list_devices` returns.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// The microphone: a chosen input device, or the default one.
    Mic(Option<String>),
    /// The other participants: everything a chosen output device plays, or only Discord's audio.
    /// Without a device, the backend records silence while Discord isn't running and attaches
    /// when it starts.
    Others(Option<String>),
}

impl Source {
    fn new(mic: bool, device_id: Option<String>) -> Self {
        let id = device_id.filter(|s| !s.is_empty());
        if mic {
            Source::Mic(id)
        } else {
            Source::Others(id)
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Source::Mic(_) => "the microphone",
            Source::Others(_) => "the other participants",
        }
    }
}

/// Receives what a backend captures.
pub trait Track {
    /// The source is open. A recording starts its file and clock here and the start succeeds;
    /// an error from the backend before this call fails the start.
    fn opened(&mut self) -> Result<(), String>;
    /// 16 kHz mono 16-bit samples, or `None` when nothing arrived for a while (`GAP_MS`).
    fn audio(&mut self, samples: Option<&[i16]>) -> Result<(), String>;
    /// Discord isn't running (only for `Source::Others(None)`). The backend keeps waiting for it;
    /// a recording fills the time with silence.
    fn discord_missing(&mut self) -> Result<(), String> {
        self.audio(None)
    }
}

/// How long a backend waits for audio before reporting a gap.
#[allow(dead_code)]
pub const GAP_MS: u32 = 200;

/// Active input and output devices, for the Settings pickers.
pub fn list_audio_devices() -> Result<AudioDevices, String> {
    backend::list_devices()
}

/// Handle to control an active audio capture session.
pub struct AudioCaptureHandle {
    stop_flag: Arc<AtomicBool>,
}

type CaptureFn = fn(&Source, &AtomicBool, &mut dyn Track) -> Result<(), String>;

/// Start recording the other participants and the microphone into two WAV files. Returns an
/// error if a chosen device can't be opened. With live transcription, samples also go to the
/// given buffers.
pub fn start_audio_capture(
    output_path: &Path,
    mic_path: &Path,
    sources: &AudioSources,
    loopback_buffer: Option<Arc<Mutex<AudioBuffer>>>,
    mic_buffer: Option<Arc<Mutex<AudioBuffer>>>,
) -> Result<AudioCaptureHandle, String> {
    start_with(
        backend::capture,
        [
            (Source::new(false, sources.others_device_id.clone()), output_path.to_path_buf(), loopback_buffer),
            (Source::new(true, sources.mic_device_id.clone()), mic_path.to_path_buf(), mic_buffer),
        ],
        Duration::from_secs(10),
    )
}

fn start_with(
    capture: CaptureFn,
    tracks: [(Source, PathBuf, Option<Arc<Mutex<AudioBuffer>>>); 2],
    open_timeout: Duration,
) -> Result<AudioCaptureHandle, String> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    for (source, path, buffer) in tracks {
        let stop = stop_flag.clone();
        let ready = ready_tx.clone();
        thread::spawn(move || {
            let mut track = FileTrack { path, buffer, ready: Some(ready), recorder: None };
            let captured = capture(&source, &stop, &mut track);
            let finished = track.recorder.take().map_or(Ok(()), WavRecorder::finish);
            if let Err(e) = captured.and(finished) {
                log::error!("[audio] Recording {} failed: {}", source.label(), e);
                if let Some(ready) = track.ready.take() {
                    let _ = ready.send(Err(e));
                }
            }
        });
    }
    drop(ready_tx);

    // Both threads report once their source is open, so a missing device fails the start.
    for _ in 0..2 {
        let outcome = ready_rx
            .recv_timeout(open_timeout)
            .unwrap_or_else(|_| Err("Audio device did not respond".into()));
        if let Err(e) = outcome {
            stop_flag.store(true, Ordering::SeqCst);
            return Err(e);
        }
    }
    Ok(AudioCaptureHandle { stop_flag })
}

/// Stop an active audio capture session.
pub fn stop_audio_capture(handle: AudioCaptureHandle) -> Result<(), String> {
    handle.stop_flag.store(true, Ordering::SeqCst);
    Ok(())
}

/// A track recorded to a WAV file.
struct FileTrack {
    path: PathBuf,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
    /// Reports the outcome of opening; taken once used.
    ready: Option<mpsc::Sender<Result<(), String>>>,
    recorder: Option<WavRecorder>,
}

impl Track for FileTrack {
    fn opened(&mut self) -> Result<(), String> {
        self.recorder = Some(WavRecorder::create(&self.path, self.buffer.take())?);
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(Ok(()));
        }
        Ok(())
    }

    fn audio(&mut self, samples: Option<&[i16]>) -> Result<(), String> {
        let rec = self.recorder.as_mut().ok_or("Audio arrived before the source was open")?;
        match samples {
            Some(s) => rec.write(s),
            None => rec.pad_to_now(),
        }
    }
}

/// Records a source for `duration` and returns its peak level (0..1), so the user can check in
/// Settings that the right device hears them. `others` uses the same rules as recording.
pub fn measure_level(mic: bool, device_id: Option<String>, duration: Duration) -> Result<f32, String> {
    measure_with(backend::capture, Source::new(mic, device_id), duration)
}

fn measure_with(capture: CaptureFn, source: Source, duration: Duration) -> Result<f32, String> {
    let stop = AtomicBool::new(false);
    let mut meter = PeakMeter { stop: &stop, duration, started: None, peak: 0 };
    capture(&source, &stop, &mut meter)?;
    Ok(meter.peak as f32 / i16::MAX as f32)
}

struct PeakMeter<'a> {
    stop: &'a AtomicBool,
    duration: Duration,
    started: Option<Instant>,
    peak: i32,
}

impl Track for PeakMeter<'_> {
    fn opened(&mut self) -> Result<(), String> {
        self.started = Some(Instant::now());
        Ok(())
    }

    fn audio(&mut self, samples: Option<&[i16]>) -> Result<(), String> {
        if let Some(s) = samples {
            self.peak = s.iter().fold(self.peak, |m, &x| m.max((x as i32).abs()));
        }
        if self.started.is_some_and(|t| t.elapsed() >= self.duration) {
            self.stop.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn discord_missing(&mut self) -> Result<(), String> {
        Err("Discord is not running".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("d-scribe-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn samples(path: &Path) -> Vec<i16> {
        hound::WavReader::open(path).unwrap().samples::<i16>().map(|s| s.unwrap()).collect()
    }

    fn tracks(dir: &Path, sources: &AudioSources) -> [(Source, PathBuf, Option<Arc<Mutex<AudioBuffer>>>); 2] {
        [
            (Source::new(false, sources.others_device_id.clone()), dir.join("others.wav"), None),
            (Source::new(true, sources.mic_device_id.clone()), dir.join("you.wav"), None),
        ]
    }

    /// Plays a short beep on each track until stopped, like a backend with audio arriving.
    fn beeping(_: &Source, stop: &AtomicBool, track: &mut dyn Track) -> Result<(), String> {
        track.opened()?;
        track.audio(Some(&[1000; 160]))?;
        while !stop.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
            track.audio(None)?;
        }
        Ok(())
    }

    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while hound::WavReader::open(path).is_err() {
            assert!(Instant::now() < deadline, "{} was not finished", path.display());
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn empty_device_ids_mean_the_defaults() {
        let sources = AudioSources { mic_device_id: Some(String::new()), others_device_id: Some("spk".into()) };
        let [others, mic] = tracks(Path::new("."), &sources);
        assert_eq!(others.0, Source::Others(Some("spk".into())));
        assert_eq!(mic.0, Source::Mic(None));
    }

    #[test]
    fn records_both_tracks_until_stopped() {
        let dir = temp_dir();
        let handle = start_with(beeping, tracks(&dir, &AudioSources::default()), Duration::from_secs(5)).unwrap();
        thread::sleep(Duration::from_millis(100));
        stop_audio_capture(handle).unwrap();
        for name in ["others.wav", "you.wav"] {
            let path = dir.join(name);
            wait_for_file(&path);
            let s = samples(&path);
            assert!(s[..160].iter().all(|&x| x == 1000), "{name} starts with the beep");
            assert!(s.len() >= 100 * 16, "{name} is padded to wall-clock time ({} samples)", s.len());
        }
    }

    #[test]
    fn a_device_that_cannot_open_fails_the_start() {
        fn mic_missing(source: &Source, stop: &AtomicBool, track: &mut dyn Track) -> Result<(), String> {
            match source {
                Source::Mic(Some(_)) => Err("The microphone chosen in Settings was not found.".into()),
                _ => beeping(source, stop, track),
            }
        }
        let dir = temp_dir();
        let sources = AudioSources { mic_device_id: Some("gone".into()), others_device_id: None };
        let err = start_with(mic_missing, tracks(&dir, &sources), Duration::from_secs(5)).err().unwrap();
        assert_eq!(err, "The microphone chosen in Settings was not found.");
        assert!(!dir.join("you.wav").exists());
        // The other track was stopped and its file finished.
        wait_for_file(&dir.join("others.wav"));
    }

    #[test]
    fn a_backend_that_never_opens_times_out() {
        fn hangs(_: &Source, stop: &AtomicBool, _: &mut dyn Track) -> Result<(), String> {
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        }
        let err = start_with(hangs, tracks(&temp_dir(), &AudioSources::default()), Duration::from_millis(100))
            .err()
            .unwrap();
        assert_eq!(err, "Audio device did not respond");
    }

    #[test]
    fn level_test_reports_the_peak() {
        fn loud(_: &Source, stop: &AtomicBool, track: &mut dyn Track) -> Result<(), String> {
            track.opened()?;
            while !stop.load(Ordering::SeqCst) {
                track.audio(Some(&[100, -16384, 200]))?;
                thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        }
        let level = measure_with(loud, Source::Mic(None), Duration::from_millis(30)).unwrap();
        assert!((level - 0.5).abs() < 0.01, "level {level}");
    }

    #[test]
    fn level_test_of_discord_fails_when_it_is_not_running() {
        fn no_discord(_: &Source, _: &AtomicBool, track: &mut dyn Track) -> Result<(), String> {
            track.opened()?;
            track.discord_missing()
        }
        let err = measure_with(no_discord, Source::Others(None), Duration::from_secs(3)).err().unwrap();
        assert_eq!(err, "Discord is not running");
    }

    #[test]
    fn recording_keeps_going_while_discord_is_missing() {
        fn discord_late(source: &Source, stop: &AtomicBool, track: &mut dyn Track) -> Result<(), String> {
            track.opened()?;
            if *source == Source::Others(None) {
                thread::sleep(Duration::from_millis(50));
                track.discord_missing()?;
            }
            track.audio(Some(&[1000; 16]))?;
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        }
        let dir = temp_dir();
        let handle = start_with(discord_late, tracks(&dir, &AudioSources::default()), Duration::from_secs(5)).unwrap();
        thread::sleep(Duration::from_millis(100));
        stop_audio_capture(handle).unwrap();
        let path = dir.join("others.wav");
        wait_for_file(&path);
        // Silence while Discord was missing, then its audio at the right time.
        let s = samples(&path);
        let first = s.iter().position(|&x| x == 1000).unwrap();
        assert!(first >= 50 * 16, "audio starts at sample {first}");
    }
}

//! Windows WASAPI capture for the other participants (Discord's audio or an output device)
//! and the microphone.

use super::buffer::AudioBuffer;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const SAMPLE_RATE: u32 = 16000;
const CHANNELS: u16 = 1;
const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;
/// How long to wait for audio before filling the gap with silence.
const GAP_MS: u32 = 200;
/// How often to check that the Discord process we capture is still running.
const PROCESS_CHECK: Duration = Duration::from_secs(2);

const DISCORD_EXES: &[&str] = &["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe"];

/// Where each track comes from. `None` (or empty) means the default.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSources {
    /// Input device id; default: the Windows default input.
    pub mic_device_id: Option<String>,
    /// Output device id to record everything it plays; default: only the Discord app's audio.
    pub others_device_id: Option<String>,
}

fn non_empty(id: &Option<String>) -> Option<&str> {
    id.as_deref().filter(|s| !s.is_empty())
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

/// Active input and output devices, for the Settings pickers.
pub fn list_audio_devices() -> Result<AudioDevices, String> {
    let _ = wasapi::initialize_mta().ok();
    let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let list = |dir: &wasapi::Direction| -> Result<Vec<AudioDevice>, String> {
        let coll = enumerator.get_device_collection(dir).map_err(|e| e.to_string())?;
        let n = coll.get_nbr_devices().map_err(|e| e.to_string())?;
        Ok((0..n)
            .filter_map(|i| coll.get_device_at_index(i).ok())
            .filter_map(|d| Some(AudioDevice { id: d.get_id().ok()?, name: d.get_friendlyname().ok()? }))
            .collect())
    };
    let default_name = |dir: &wasapi::Direction| enumerator.get_default_device(dir).ok().and_then(|d| d.get_friendlyname().ok());
    Ok(AudioDevices {
        inputs: list(&wasapi::Direction::Capture)?,
        outputs: list(&wasapi::Direction::Render)?,
        default_input: default_name(&wasapi::Direction::Capture),
        default_output: default_name(&wasapi::Direction::Render),
    })
}

/// Handle to control an active audio capture session.
pub struct AudioCaptureHandle {
    stop_flag: Arc<AtomicBool>,
}

/// Start recording the other participants and the microphone into two WAV files
/// (16 kHz mono 16-bit, what whisper.cpp wants). Returns an error if a chosen device can't be
/// opened. With live transcription, samples also go to the given buffers.
pub fn start_audio_capture(
    output_path: &Path,
    mic_path: &Path,
    sources: &AudioSources,
    loopback_buffer: Option<Arc<Mutex<AudioBuffer>>>,
    mic_buffer: Option<Arc<Mutex<AudioBuffer>>>,
) -> Result<AudioCaptureHandle, String> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    {
        let path = output_path.to_path_buf();
        let stop = stop_flag.clone();
        let ready = ready_tx.clone();
        let device = non_empty(&sources.others_device_id).map(str::to_string);
        thread::spawn(move || {
            let result = match device {
                Some(id) => record_device(&path, &id, &wasapi::Direction::Render, &stop, loopback_buffer, ready),
                None => record_discord(&path, &stop, loopback_buffer, ready),
            };
            if let Err(e) = result {
                log::error!("[audio] Recording the other participants failed: {}", e);
            }
        });
    }
    {
        let path = mic_path.to_path_buf();
        let stop = stop_flag.clone();
        let device = non_empty(&sources.mic_device_id).map(str::to_string);
        thread::spawn(move || {
            let result = match device {
                Some(id) => record_device(&path, &id, &wasapi::Direction::Capture, &stop, mic_buffer, ready_tx),
                None => record_default_mic(&path, &stop, mic_buffer, ready_tx),
            };
            if let Err(e) = result {
                log::error!("[audio] Recording the microphone failed: {}", e);
            }
        });
    }

    // Both threads report once their source is open, so a missing device fails the start.
    for _ in 0..2 {
        let outcome = ready_rx
            .recv_timeout(Duration::from_secs(10))
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

/// Writes samples to the WAV and keeps the file aligned with wall-clock time: loopback streams
/// deliver nothing while the source is silent, and segment timestamps index into the file.
struct Recorder {
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    written: u64,
    started: Instant,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
}

impl Recorder {
    fn create(path: &Path, buffer: Option<Arc<Mutex<AudioBuffer>>>) -> Result<Self, String> {
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

    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
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

    /// Fill with silence up to now.
    fn pad_to_now(&mut self) -> Result<(), String> {
        let expected = self.started.elapsed().as_millis() as u64 * SAMPLES_PER_MS;
        if self.written < expected {
            let gap = vec![0i16; (expected - self.written) as usize];
            self.write(&gap)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(), String> {
        self.pad_to_now()?;
        self.writer.finalize().map_err(|e| e.to_string())
    }
}

/// Reads one WASAPI stream until `stop` is set or `keep_going` returns false (checked every
/// `PROCESS_CHECK`). `on_audio` gets samples, or `None` after `GAP_MS` without any.
fn run_stream(
    mut client: wasapi::AudioClient,
    stop: &AtomicBool,
    mut keep_going: impl FnMut() -> bool,
    mut on_audio: impl FnMut(Option<&[i16]>) -> Result<(), String>,
) -> Result<(), String> {
    let format = wasapi::WaveFormat::new(16, 16, &wasapi::SampleType::Int, SAMPLE_RATE as usize, CHANNELS as usize, None);
    let mode = wasapi::StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client
        .initialize_client(&format, &wasapi::Direction::Capture, &mode)
        .map_err(|e| e.to_string())?;
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    let mut queue: VecDeque<u8> = VecDeque::new();
    let mut samples: Vec<i16> = Vec::new();
    let mut last_check = Instant::now();
    client.start_stream().map_err(|e| e.to_string())?;

    let result = (|| {
        while !stop.load(Ordering::SeqCst) {
            if last_check.elapsed() >= PROCESS_CHECK {
                last_check = Instant::now();
                if !keep_going() {
                    break;
                }
            }
            if event.wait_for_event(GAP_MS).is_ok() {
                capture.read_from_device_to_deque(&mut queue).map_err(|e| e.to_string())?;
                samples.clear();
                while queue.len() >= 2 {
                    let lo = queue.pop_front().unwrap();
                    let hi = queue.pop_front().unwrap();
                    samples.push(i16::from_le_bytes([lo, hi]));
                }
                on_audio(Some(&samples))?;
            } else {
                on_audio(None)?;
            }
        }
        Ok(())
    })();
    let _ = client.stop_stream();
    result
}

/// PID of the top Discord process. Process loopback with the tree flag must target the parent,
/// since voice is played by a child process.
fn discord_root_pid() -> Option<u32> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    for exe in DISCORD_EXES {
        for p in sys.processes_by_exact_name(OsStr::new(exe)) {
            let parent_is_discord = p
                .parent()
                .and_then(|pp| sys.process(pp))
                .map(|pp| pp.name() == p.name())
                .unwrap_or(false);
            if !parent_is_discord {
                return Some(p.pid().as_u32());
            }
        }
    }
    None
}

fn process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
    sys.process(pid).is_some()
}

/// Records only Discord's audio, whatever device it plays to. If Discord isn't running or
/// closes (e.g. restarts for an update), records silence and attaches again when it is back.
fn record_discord(
    path: &PathBuf,
    stop: &AtomicBool,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
    ready: mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let _ = wasapi::initialize_mta().ok();
    let mut rec = Recorder::create(path, buffer)?;
    let _ = ready.send(Ok(()));
    let mut waiting_logged = false;

    while !stop.load(Ordering::SeqCst) {
        let Some(pid) = discord_root_pid() else {
            if !waiting_logged {
                log::warn!("[audio] Discord is not running; recording silence until it starts");
                waiting_logged = true;
            }
            rec.pad_to_now()?;
            thread::sleep(Duration::from_millis(500));
            continue;
        };
        waiting_logged = false;
        log::info!("[audio] Capturing Discord audio (pid {})", pid);
        let client = match wasapi::AudioClient::new_application_loopback_client(pid, true) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[audio] Could not attach to Discord (pid {}): {}", pid, e);
                rec.pad_to_now()?;
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let result = run_stream(client, stop, || process_alive(pid), |audio| match audio {
            Some(s) => rec.write(s),
            None => rec.pad_to_now(),
        });
        if let Err(e) = result {
            log::warn!("[audio] Discord audio stream ended: {}", e);
        } else if !stop.load(Ordering::SeqCst) {
            log::info!("[audio] Discord closed; waiting for it to start again");
        }
    }
    wasapi::deinitialize();
    rec.finish()
}

fn record_default_mic(
    path: &PathBuf,
    stop: &AtomicBool,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
    ready: mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let _ = wasapi::initialize_mta().ok();
    let opened = (|| {
        let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
        let device = enumerator.get_default_device(&wasapi::Direction::Capture).map_err(|e| e.to_string())?;
        log::info!("[audio] Capturing default mic: {}", device.get_friendlyname().unwrap_or_default());
        device.get_iaudioclient().map_err(|e| e.to_string())
    })();
    record_client(path, opened, stop, buffer, ready)
}

/// Records a chosen device: an input (microphone) or everything an output device plays.
fn record_device(
    path: &PathBuf,
    device_id: &str,
    direction: &wasapi::Direction,
    stop: &AtomicBool,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
    ready: mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let _ = wasapi::initialize_mta().ok();
    let opened = open_device(device_id, direction);
    if let Ok(_) = &opened {
        log::info!("[audio] Capturing {:?} device {}", direction, device_id);
    }
    record_client(path, opened, stop, buffer, ready)
}

fn open_device(device_id: &str, direction: &wasapi::Direction) -> Result<wasapi::AudioClient, String> {
    let kind = if *direction == wasapi::Direction::Capture { "microphone" } else { "output device" };
    // Looked up by walking the active devices: wasapi's get_device(id) can pass Windows a
    // pointer to a string that is already freed, and then fails at random.
    let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let coll = enumerator.get_device_collection(direction).map_err(|e| e.to_string())?;
    let n = coll.get_nbr_devices().map_err(|e| e.to_string())?;
    let device = (0..n)
        .filter_map(|i| coll.get_device_at_index(i).ok())
        .find(|d| d.get_id().map(|id| id == device_id).unwrap_or(false))
        .ok_or_else(|| format!("The {} chosen in Settings was not found. Is it plugged in?", kind))?;
    device
        .get_iaudioclient()
        .map_err(|e| format!("The {} chosen in Settings is not available ({}). Is it plugged in?", kind, e))
}

fn record_client(
    path: &PathBuf,
    opened: Result<wasapi::AudioClient, String>,
    stop: &AtomicBool,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
    ready: mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let client = match opened {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            wasapi::deinitialize();
            return Err(e);
        }
    };
    let mut rec = Recorder::create(path, buffer)?;
    let _ = ready.send(Ok(()));
    let result = run_stream(client, stop, || true, |audio| match audio {
        Some(s) => rec.write(s),
        None => rec.pad_to_now(),
    });
    wasapi::deinitialize();
    let finished = rec.finish();
    result.and(finished)
}

/// Records a source for `duration` and returns its peak level (0..1), so the user can check in
/// Settings that the right device hears them. `others` uses the same rules as recording.
pub fn measure_level(mic: bool, device_id: Option<String>, duration: Duration) -> Result<f32, String> {
    let _ = wasapi::initialize_mta().ok();
    let id = device_id.filter(|s| !s.is_empty());
    let client = match (mic, id) {
        (_, Some(id)) => open_device(&id, if mic { &wasapi::Direction::Capture } else { &wasapi::Direction::Render })?,
        (true, None) => {
            let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
            let device = enumerator.get_default_device(&wasapi::Direction::Capture).map_err(|e| e.to_string())?;
            device.get_iaudioclient().map_err(|e| e.to_string())?
        }
        (false, None) => {
            let pid = discord_root_pid().ok_or("Discord is not running")?;
            wasapi::AudioClient::new_application_loopback_client(pid, true).map_err(|e| e.to_string())?
        }
    };
    let stop = AtomicBool::new(false);
    let started = Instant::now();
    let mut peak: i32 = 0;
    run_stream(client, &stop, || true, |audio| {
        if let Some(s) = audio {
            peak = s.iter().fold(peak, |m, &x| m.max((x as i32).abs()));
        }
        if started.elapsed() >= duration {
            stop.store(true, Ordering::SeqCst);
        }
        Ok(())
    })?;
    wasapi::deinitialize();
    Ok(peak as f32 / i16::MAX as f32)
}

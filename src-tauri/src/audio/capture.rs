//! Windows WASAPI audio capture for Discord's output and the microphone.

use super::buffer::AudioBuffer;
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

const SAMPLE_RATE: u32 = 16000;
const CHANNELS: u16 = 1;
const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;
/// How long to wait for audio before filling the gap with silence.
const GAP_MS: u32 = 200;

const DISCORD_EXES: &[&str] = &["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe"];

/// Handle to control an active audio capture session.
pub struct AudioCaptureHandle {
    stop_flag: Arc<AtomicBool>,
}

/// Start capturing the other participants (Discord's audio output) and the microphone.
/// Writes to two WAV files: output_path (others) and mic_path (microphone).
/// Format: 16 kHz, mono, 16-bit PCM (whisper.cpp requirement).
/// When live_realtime is true, also pushes samples to the provided buffers for real-time transcription.
pub fn start_audio_capture(
    output_path: &Path,
    mic_path: &Path,
    loopback_buffer: Option<Arc<Mutex<AudioBuffer>>>,
    mic_buffer: Option<Arc<Mutex<AudioBuffer>>>,
) -> Result<AudioCaptureHandle, String> {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let out_path = output_path.to_path_buf();
    let mic_path_buf = mic_path.to_path_buf();
    let stop_loopback = stop_flag.clone();
    let stop_mic = stop_flag.clone();

    thread::spawn(move || {
        if let Err(e) = run_loopback_capture(&out_path, &stop_loopback, loopback_buffer) {
            log::error!("[audio] Discord audio capture failed: {}", e);
        }
    });

    thread::spawn(move || {
        if let Err(e) = run_mic_capture(&mic_path_buf, &stop_mic, mic_buffer) {
            log::error!("[audio] Mic capture failed: {}", e);
        }
    });

    Ok(AudioCaptureHandle { stop_flag })
}

/// Stop an active audio capture session.
pub fn stop_audio_capture(handle: AudioCaptureHandle) -> Result<(), String> {
    handle.stop_flag.store(true, Ordering::SeqCst);
    Ok(())
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

/// Captures only Discord's audio, whatever device it plays to. Falls back to the default
/// output device when Discord isn't running.
fn run_loopback_capture(
    output_path: &Path,
    stop_flag: &AtomicBool,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
) -> Result<(), String> {
    let _ = wasapi::initialize_mta().ok();

    let audio_client = match discord_root_pid() {
        Some(pid) => {
            log::info!("[audio] Capturing Discord audio (pid {})", pid);
            wasapi::AudioClient::new_application_loopback_client(pid, true).map_err(|e| e.to_string())?
        }
        None => {
            log::warn!("[audio] Discord is not running; capturing the default output device instead");
            let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
            let device = enumerator
                .get_default_device(&wasapi::Direction::Render)
                .map_err(|e| e.to_string())?;
            device.get_iaudioclient().map_err(|e| e.to_string())?
        }
    };

    let result = capture_to_wav(audio_client, output_path, stop_flag, buffer);
    wasapi::deinitialize();
    result
}

fn run_mic_capture(
    mic_path: &Path,
    stop_flag: &AtomicBool,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
) -> Result<(), String> {
    let _ = wasapi::initialize_mta().ok();

    let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let device = enumerator
        .get_default_device(&wasapi::Direction::Capture)
        .map_err(|e| e.to_string())?;
    log::info!("[audio] Capturing mic: {}", device.get_friendlyname().unwrap_or_default());
    let audio_client = device.get_iaudioclient().map_err(|e| e.to_string())?;

    let result = capture_to_wav(audio_client, mic_path, stop_flag, buffer);
    wasapi::deinitialize();
    result
}

/// Records until stop_flag is set. Loopback streams deliver nothing while the source is silent,
/// so gaps are filled with silence to keep the file aligned with wall-clock time (segment
/// timestamps index into it).
fn capture_to_wav(
    mut audio_client: wasapi::AudioClient,
    path: &Path,
    stop_flag: &AtomicBool,
    buffer: Option<Arc<Mutex<AudioBuffer>>>,
) -> Result<(), String> {
    // 16 kHz mono 16-bit for whisper
    let desired_format = wasapi::WaveFormat::new(
        16,
        16,
        &wasapi::SampleType::Int,
        SAMPLE_RATE as usize,
        CHANNELS as usize,
        None,
    );

    let mode = wasapi::StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 0,
    };

    audio_client
        .initialize_client(&desired_format, &wasapi::Direction::Capture, &mode)
        .map_err(|e| e.to_string())?;

    let h_event = audio_client
        .set_get_eventhandle()
        .map_err(|e| e.to_string())?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| e.to_string())?;

    let mut sample_queue: VecDeque<u8> = VecDeque::new();

    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;

    let mut written: u64 = 0;
    let mut write = |sample: i16, written: &mut u64| -> Result<(), String> {
        writer.write_sample(sample).map_err(|e| e.to_string())?;
        *written += 1;
        if let Some(ref buf) = buffer {
            if let Ok(mut guard) = buf.lock() {
                guard.push(sample);
            }
        }
        Ok(())
    };

    let started = Instant::now();
    audio_client.start_stream().map_err(|e| e.to_string())?;

    while !stop_flag.load(Ordering::SeqCst) {
        let got_event = h_event.wait_for_event(GAP_MS).is_ok();
        if got_event {
            capture_client
                .read_from_device_to_deque(&mut sample_queue)
                .map_err(|e| e.to_string())?;
            while sample_queue.len() >= 2 {
                let low = sample_queue.pop_front().unwrap();
                let high = sample_queue.pop_front().unwrap();
                write(i16::from_le_bytes([low, high]), &mut written)?;
            }
        } else {
            // No audio for GAP_MS: catch up to wall-clock time with silence.
            let expected = started.elapsed().as_millis() as u64 * SAMPLES_PER_MS;
            while written < expected {
                write(0, &mut written)?;
            }
        }
    }

    let expected = started.elapsed().as_millis() as u64 * SAMPLES_PER_MS;
    while written < expected {
        write(0, &mut written)?;
    }

    audio_client.stop_stream().map_err(|e| e.to_string())?;
    drop(write);
    writer.finalize().map_err(|e| e.to_string())?;

    Ok(())
}

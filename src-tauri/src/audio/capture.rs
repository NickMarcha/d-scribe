//! Windows WASAPI audio capture for loopback and microphone.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

const SAMPLE_RATE: u32 = 16000;
const CHANNELS: u16 = 1;

/// Handle to control an active audio capture session.
pub struct AudioCaptureHandle {
    stop_flag: Arc<AtomicBool>,
}

/// Start capturing audio from loopback (system output) and microphone.
/// Writes to two WAV files: output_path (loopback) and mic_path (microphone).
/// Format: 16 kHz, mono, 16-bit PCM (whisper.cpp requirement).
pub fn start_audio_capture(
    output_path: &Path,
    mic_path: &Path,
) -> Result<AudioCaptureHandle, String> {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let out_path = output_path.to_path_buf();
    let mic_path_buf = mic_path.to_path_buf();
    let stop_loopback = stop_flag.clone();
    let stop_mic = stop_flag.clone();

    // Loopback: capture from render device with Direction::Capture = system output
    // (WASAPI uses loopback when capturing from a render endpoint)
    thread::spawn(move || {
        if let Err(e) = run_loopback_capture(&out_path, &stop_loopback) {
            eprintln!("Loopback capture error: {}", e);
        }
    });

    // Microphone: capture from default capture device
    thread::spawn(move || {
        if let Err(e) = run_mic_capture(&mic_path_buf, &stop_mic) {
            eprintln!("Mic capture error: {}", e);
        }
    });

    Ok(AudioCaptureHandle { stop_flag })
}

/// Stop an active audio capture session.
pub fn stop_audio_capture(handle: AudioCaptureHandle) -> Result<(), String> {
    handle.stop_flag.store(true, Ordering::SeqCst);
    Ok(())
}

fn run_loopback_capture(output_path: &Path, stop_flag: &AtomicBool) -> Result<(), String> {
    let _ = wasapi::initialize_mta().ok();

    let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
    // Direction::Render = playback device, Capture on it = loopback
    let device = enumerator
        .get_default_device(&wasapi::Direction::Render)
        .map_err(|e| e.to_string())?;

    capture_to_wav(device, output_path, stop_flag)?;

    wasapi::deinitialize();
    Ok(())
}

fn run_mic_capture(mic_path: &Path, stop_flag: &AtomicBool) -> Result<(), String> {
    let _ = wasapi::initialize_mta().ok();

    let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let device = enumerator
        .get_default_device(&wasapi::Direction::Capture)
        .map_err(|e| e.to_string())?;

    capture_to_wav(device, mic_path, stop_flag)?;

    wasapi::deinitialize();
    Ok(())
}

fn capture_to_wav(
    device: wasapi::Device,
    path: &Path,
    stop_flag: &AtomicBool,
) -> Result<(), String> {
    let mut audio_client = device.get_iaudioclient().map_err(|e| e.to_string())?;

    // 16 kHz mono 16-bit for whisper
    let desired_format = wasapi::WaveFormat::new(
        16,
        16,
        &wasapi::SampleType::Int,
        SAMPLE_RATE as usize,
        CHANNELS as usize,
        None,
    );

    let (_def_time, min_time) = audio_client.get_device_period().map_err(|e| e.to_string())?;

    let mode = wasapi::StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };

    audio_client
        .initialize_client(&desired_format, &wasapi::Direction::Capture, &mode)
        .map_err(|e| e.to_string())?;

    let h_event = audio_client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture_client = audio_client.get_audiocaptureclient().map_err(|e| e.to_string())?;

    let blockalign = desired_format.get_blockalign();
    let buffer_frame_count = audio_client.get_buffer_size().map_err(|e| e.to_string())?;
    let mut sample_queue: VecDeque<u8> = VecDeque::with_capacity(
        blockalign as usize * (1024 + 2 * buffer_frame_count as usize),
    );

    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: CHANNELS as u16,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;

    audio_client.start_stream().map_err(|e| e.to_string())?;

    while !stop_flag.load(Ordering::SeqCst) {
        capture_client
            .read_from_device_to_deque(&mut sample_queue)
            .map_err(|e| e.to_string())?;

        // Write 16-bit samples (2 bytes each, little-endian)
        while sample_queue.len() >= 2 {
            let low = sample_queue.pop_front().unwrap();
            let high = sample_queue.pop_front().unwrap();
            let sample = i16::from_le_bytes([low, high]);
            writer.write_sample(sample).map_err(|e| e.to_string())?;
        }

        if h_event.wait_for_event(1000).is_err() {
            break;
        }
    }

    audio_client.stop_stream().map_err(|e| e.to_string())?;
    writer.finalize().map_err(|e| e.to_string())?;

    Ok(())
}

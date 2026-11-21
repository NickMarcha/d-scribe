//! Windows audio backend (WASAPI): Discord's own audio through process loopback, a chosen
//! output device through device loopback, and the microphone.

use super::wav::{CHANNELS, SAMPLE_RATE};
use super::{AudioDevice, AudioDevices, Source, Track, GAP_MS};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// How often to check that the Discord process we capture is still running.
const PROCESS_CHECK: Duration = Duration::from_secs(2);

const DISCORD_EXES: &[&str] = &["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe"];

pub fn list_devices() -> Result<AudioDevices, String> {
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

pub fn capture(source: &Source, stop: &AtomicBool, track: &mut dyn Track) -> Result<(), String> {
    let _ = wasapi::initialize_mta().ok();
    let result = match source {
        Source::Others(None) => capture_discord(stop, track),
        Source::Others(Some(id)) => open_device(id, &wasapi::Direction::Render).and_then(|client| {
            log::info!("[audio] Capturing output device {}", id);
            capture_client(client, stop, track)
        }),
        Source::Mic(Some(id)) => open_device(id, &wasapi::Direction::Capture).and_then(|client| {
            log::info!("[audio] Capturing microphone {}", id);
            capture_client(client, stop, track)
        }),
        Source::Mic(None) => open_default_mic().and_then(|client| capture_client(client, stop, track)),
    };
    wasapi::deinitialize();
    result
}

fn capture_client(client: wasapi::AudioClient, stop: &AtomicBool, track: &mut dyn Track) -> Result<(), String> {
    track.opened()?;
    run_stream(client, stop, || true, |audio| track.audio(audio))
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
/// closes (e.g. restarts for an update), waits and attaches again when it is back.
fn capture_discord(stop: &AtomicBool, track: &mut dyn Track) -> Result<(), String> {
    track.opened()?;
    let mut waiting_logged = false;

    while !stop.load(Ordering::SeqCst) {
        let Some(pid) = discord_root_pid() else {
            if !waiting_logged {
                log::warn!("[audio] Discord is not running; recording silence until it starts");
                waiting_logged = true;
            }
            track.discord_missing()?;
            thread::sleep(Duration::from_millis(500));
            continue;
        };
        waiting_logged = false;
        log::info!("[audio] Capturing Discord audio (pid {})", pid);
        let client = match wasapi::AudioClient::new_application_loopback_client(pid, true) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[audio] Could not attach to Discord (pid {}): {}", pid, e);
                track.audio(None)?;
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let result = run_stream(client, stop, || process_alive(pid), |audio| track.audio(audio));
        if let Err(e) = result {
            log::warn!("[audio] Discord audio stream ended: {}", e);
        } else if !stop.load(Ordering::SeqCst) {
            log::info!("[audio] Discord closed; waiting for it to start again");
        }
    }
    Ok(())
}

fn open_default_mic() -> Result<wasapi::AudioClient, String> {
    let enumerator = wasapi::DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let device = enumerator.get_default_device(&wasapi::Direction::Capture).map_err(|e| e.to_string())?;
    log::info!("[audio] Capturing default mic: {}", device.get_friendlyname().unwrap_or_default());
    device.get_iaudioclient().map_err(|e| e.to_string())
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

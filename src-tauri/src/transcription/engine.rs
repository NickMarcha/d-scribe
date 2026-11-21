//! whisper.cpp engine: downloaded on demand into app data, run as a local server so the model
//! is loaded once per transcription instead of once per segment.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Pinned whisper.cpp build. Every zip contains a `Release/` folder (or the files directly)
/// with whisper-server.exe and the ggml DLLs. On a bump, the `engine` workflow builds the
/// Vulkan zip for the new build and prints the SHA256 values for `asset_sha256`.
const RELEASE: &str = "b5454";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineKind {
    Cpu,
    /// NVIDIA GPU (CUDA 12.4 runtime included in the zip).
    Cuda,
    /// Any GPU with a Vulkan driver (AMD, Intel, NVIDIA). Built by this repo's `engine`
    /// workflow, since whisper.cpp doesn't publish a Windows Vulkan build. Falls back to CPU.
    Vulkan,
}

impl EngineKind {
    /// Order used when the engine setting is "auto": fastest first.
    pub const ALL: [EngineKind; 3] = [EngineKind::Cuda, EngineKind::Vulkan, EngineKind::Cpu];

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "cpu" => Ok(Self::Cpu),
            "cuda" => Ok(Self::Cuda),
            "vulkan" => Ok(Self::Vulkan),
            _ => Err(format!("Unknown engine: {}", s)),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
            Self::Vulkan => "vulkan",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Cuda => "NVIDIA GPU",
            Self::Vulkan => "GPU (Vulkan)",
        }
    }

    fn url(self) -> String {
        match self {
            Self::Cpu => format!("https://github.com/ggml-org/whisper.cpp/releases/download/{RELEASE}/whisper-bin-x64.zip"),
            Self::Cuda => format!(
                "https://github.com/ggml-org/whisper.cpp/releases/download/{RELEASE}/whisper-bin-win-cuda-12.4.0-x64.zip"
            ),
            Self::Vulkan => format!(
                "https://github.com/NickMarcha/d-scribe/releases/download/engine-{RELEASE}/whisper-bin-win-vulkan-x64.zip"
            ),
        }
    }

    /// SHA256 of the zip for `RELEASE`, checked after download.
    fn asset_sha256(self) -> &'static str {
        match self {
            Self::Cpu => "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
            Self::Cuda => "afef0b881c500958921c3f5523b50e59ee2ec9b6f5cbd25b324c51ed308a957a",
            Self::Vulkan => "97a688c17a574a5b24234064e8b1b9a5c9b33dca4a48977b37005ab29c1741d4",
        }
    }
}

pub fn engine_dir(engines_root: &Path, kind: EngineKind) -> PathBuf {
    engines_root.join(format!("{}-{}", kind.name(), RELEASE))
}

pub fn is_installed(engines_root: &Path, kind: EngineKind) -> bool {
    engine_dir(engines_root, kind).join("whisper-server.exe").exists()
}

/// The engine to run. `choice` is the "engine" setting: "auto" (or empty) picks the fastest
/// installed engine and returns None when none is installed; a specific engine that isn't
/// installed is an error, so a broken choice isn't silently replaced by another engine.
pub fn chosen_engine(engines_root: &Path, choice: &str) -> Result<Option<(EngineKind, PathBuf)>, String> {
    if choice.is_empty() || choice == "auto" {
        return Ok(EngineKind::ALL
            .into_iter()
            .find(|k| is_installed(engines_root, *k))
            .map(|k| (k, engine_dir(engines_root, k))));
    }
    let kind = EngineKind::parse(choice)?;
    if !is_installed(engines_root, kind) {
        return Err(format!(
            "The {} engine is not installed. Download it in Settings > Manage models, or set the engine to Automatic.",
            kind.label()
        ));
    }
    Ok(Some((kind, engine_dir(engines_root, kind))))
}

/// An engine ready to start, with the GPU to run on (whisper-server's `-dev`; None = its default, the first GPU).
pub struct Engine {
    pub kind: EngineKind,
    pub dir: PathBuf,
    pub device: Option<usize>,
}

#[derive(serde::Serialize)]
pub struct GpuDevice {
    /// What the setting stores: the name, plus " #2", " #3"... when several cards share a name
    /// (numbered in the engine's order), so two identical cards can be told apart.
    pub id: String,
    pub name: String,
    /// Shares memory with the CPU (Vulkan's `uma`), i.e. integrated graphics.
    pub integrated: bool,
}

fn hidden_command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// The GPUs an engine can use, in its own order, so the index is the `-dev` value. Read from
/// what `whisper-server --help` prints while loading its backends. Empty for the CPU engine.
pub fn gpu_devices(engine_dir: &Path) -> Result<Vec<GpuDevice>, String> {
    let out = hidden_command(&engine_dir.join("whisper-server.exe"))
        .current_dir(engine_dir)
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not start whisper-server: {}", e))?;
    let text = String::from_utf8_lossy(&out.stderr) + String::from_utf8_lossy(&out.stdout);
    Ok(parse_gpu_devices(&text))
}

fn parse_gpu_devices(text: &str) -> Vec<GpuDevice> {
    let mut devices = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        // Vulkan: "ggml_vulkan: 0 = AMD Radeon(TM) Graphics (AMD proprietary driver) | uma: 1 | ..."
        if let Some((_, rest)) = line.strip_prefix("ggml_vulkan: ").and_then(|l| l.split_once(" = ")) {
            let name = rest.split(" | ").next().unwrap_or(rest);
            let name = match name.rsplit_once(" (") {
                Some((n, _)) if name.ends_with(')') => n,
                _ => name,
            };
            devices.push((name.trim().to_string(), rest.contains("| uma: 1")));
        // CUDA: "Device 0: NVIDIA GeForce RTX 3080, compute capability 8.6, ..."
        } else if let Some((_, rest)) = line.strip_prefix("Device ").and_then(|l| l.split_once(": ")) {
            let name = rest.split(',').next().unwrap_or(rest);
            devices.push((name.trim().to_string(), false));
        }
    }
    devices
        .iter()
        .enumerate()
        .map(|(i, (name, integrated))| {
            let same = devices.iter().filter(|(n, _)| n == name).count();
            let nth = devices[..=i].iter().filter(|(n, _)| n == name).count();
            let id = if same > 1 { format!("{} #{}", name, nth) } else { name.clone() };
            GpuDevice { id, name: name.clone(), integrated: *integrated }
        })
        .collect()
}

/// Index of the GPU with id `wanted` (the setting for this engine; empty = the default GPU).
/// A saved GPU that's no longer there is an error rather than a silent switch to another card.
pub fn device_index(kind: EngineKind, engine_dir: &Path, wanted: &str) -> Result<Option<usize>, String> {
    if wanted.is_empty() || kind == EngineKind::Cpu {
        return Ok(None);
    }
    gpu_devices(engine_dir)?.iter().position(|d| d.id == wanted).map(Some).ok_or_else(|| {
        format!(
            "The graphics card \"{}\" chosen for the {} engine was not found. Pick another one in Settings > Manage models.",
            wanted,
            kind.label()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(text: &str) -> Vec<(String, bool)> {
        parse_gpu_devices(text).into_iter().map(|d| (d.id, d.integrated)).collect()
    }

    #[test]
    fn parses_vulkan_devices() {
        let text = "ggml_vulkan: Found 2 Vulkan devices:
            ggml_vulkan: 0 = NVIDIA GeForce RTX 3080 (NVIDIA) | uma: 0 | fp16: 1
            ggml_vulkan: 1 = AMD Radeon(TM) Graphics (AMD proprietary driver) | uma: 1 | fp16: 1
            load_backend: loaded Vulkan backend from x.dll
";
        assert_eq!(
            ids(text),
            [("NVIDIA GeForce RTX 3080".to_string(), false), ("AMD Radeon(TM) Graphics".to_string(), true)]
        );
    }

    #[test]
    fn numbers_identical_cuda_devices() {
        let text = "ggml_cuda_init: found 3 CUDA devices:
              Device 0: NVIDIA GeForce RTX 3080, compute capability 8.6, VMM: yes
              Device 1: NVIDIA GeForce RTX 4090, compute capability 8.9, VMM: yes
              Device 2: NVIDIA GeForce RTX 3080, compute capability 8.6, VMM: yes
";
        let got: Vec<String> = ids(text).into_iter().map(|(id, _)| id).collect();
        assert_eq!(got, ["NVIDIA GeForce RTX 3080 #1", "NVIDIA GeForce RTX 4090", "NVIDIA GeForce RTX 3080 #2"]);
    }
}

/// Deletes engines from older pinned builds and leftover downloads. Run at startup, before
/// anything can be using an engine.
pub fn remove_old_engines(engines_root: &Path) {
    let Ok(entries) = std::fs::read_dir(engines_root) else { return };
    let current: Vec<PathBuf> = EngineKind::ALL.iter().map(|k| engine_dir(engines_root, *k)).collect();
    for entry in entries.flatten() {
        let path = entry.path();
        if current.contains(&path) {
            continue;
        }
        let result = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match result {
            Ok(()) => log::info!("[engine] Removed old engine file {}", path.display()),
            Err(e) => log::warn!("[engine] Could not remove {}: {}", path.display(), e),
        }
    }
}

fn file_sha256(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}

/// Downloads and unpacks an engine. Returns its directory.
pub async fn download_engine<F>(engines_root: &Path, kind: EngineKind, on_progress: F) -> Result<PathBuf, String>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    let dir = engine_dir(engines_root, kind);
    // One install per engine at a time; a second caller waits and then finds it installed.
    let _lock = super::download::lock_path(&dir).await;
    if is_installed(engines_root, kind) {
        return Ok(dir);
    }
    std::fs::create_dir_all(engines_root).map_err(|e| e.to_string())?;
    let zip_path = engines_root.join(format!("{}.zip", kind.name()));
    super::download::download_to_file(&kind.url(), &zip_path, on_progress).await?;
    let hash = file_sha256(&zip_path);
    if hash.as_deref() != Ok(kind.asset_sha256()) {
        let _ = std::fs::remove_file(&zip_path);
        return Err(format!(
            "The downloaded {} engine doesn't match its checksum ({}). Try again.",
            kind.label(),
            hash.unwrap_or_else(|e| e)
        ));
    }

    let tmp_dir = dir.with_extension("part");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let unpack = || -> Result<(), String> {
        let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            let Some(rel) = entry.enclosed_name() else { continue };
            // Flatten the zip's Release/ folder; keep only the exe and DLLs we run.
            let Some(name) = rel.file_name() else { continue };
            let name_s = name.to_string_lossy().to_lowercase();
            if entry.is_dir() || !(name_s.ends_with(".dll") || name_s == "whisper-server.exe" || name_s == "whisper-cli.exe") {
                continue;
            }
            std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
            let mut out = std::fs::File::create(tmp_dir.join(name)).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    let result = unpack();
    let _ = std::fs::remove_file(&zip_path);
    result?;
    if !tmp_dir.join("whisper-server.exe").exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err("The downloaded engine has no whisper-server.exe".into());
    }
    std::fs::rename(&tmp_dir, &dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub struct Inference {
    pub text: String,
    /// Probability that the audio is English, from whisper's language detection.
    /// Only meaningful when transcribed with language "auto".
    pub english_prob: f32,
}

/// A running whisper-server with one model loaded. Killed on drop.
pub struct WhisperServer {
    child: Child,
    base_url: String,
    client: reqwest::Client,
}

impl WhisperServer {
    pub async fn start(engine: &Engine, model: &Path, log_path: &Path) -> Result<Self, String> {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| e.to_string())?
            .port();
        let log = std::fs::File::create(log_path).map_err(|e| e.to_string())?;
        let mut cmd = hidden_command(&engine.dir.join("whisper-server.exe"));
        cmd.current_dir(&engine.dir).args(["--host", "127.0.0.1", "--port", &port.to_string(), "-bs", "5"]);
        if let Some(device) = engine.device {
            cmd.args(["-dev", &device.to_string()]);
        }
        cmd.arg("-m")
            .arg(model)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        let child = cmd.spawn().map_err(|e| format!("Could not start whisper-server: {}", e))?;
        let mut server = Self {
            child,
            base_url: format!("http://127.0.0.1:{}", port),
            client: reqwest::Client::new(),
        };

        // Loading a large model can take a while on CPU.
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if let Ok(Some(status)) = server.child.try_wait() {
                let tail = std::fs::read_to_string(log_path).unwrap_or_default();
                let tail: String = tail.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                return Err(format!("whisper-server exited ({}) while loading {}:\n{}", status, model.display(), tail));
            }
            if server.client.get(&server.base_url).send().await.is_ok() {
                break;
            }
            if Instant::now() > deadline {
                return Err("whisper-server did not start within 5 minutes".into());
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        log::info!(
            "[transcribe] whisper-server ready on {} with {} ({} engine, GPU {})",
            server.base_url,
            model.display(),
            engine.kind.label(),
            engine.device.map_or("default".to_string(), |d| d.to_string())
        );
        Ok(server)
    }

    pub async fn transcribe(&self, wav: &Path, language: &str) -> Result<Inference, String> {
        let bytes = tokio::fs::read(wav).await.map_err(|e| e.to_string())?;
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name("segment.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("language", language.to_string())
            .text("response_format", "verbose_json");
        let resp = self
            .client
            .post(format!("{}/inference", self.base_url))
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if !status.is_success() || body.get("error").is_some() {
            return Err(format!("whisper-server: {}", body));
        }
        let text = speech_text(&body);
        let english_prob = body["language_probabilities"]["en"].as_f64().unwrap_or(0.0) as f32;
        Ok(Inference { text, english_prob })
    }
}

/// Text of the parts whisper thinks are speech. Uses whisper's own rule for dropping a
/// segment (no-speech probability > 0.6 with average log probability < -1) and removes special
/// tokens such as NB-Whisper's `<|nocaptions|>`.
fn speech_text(body: &serde_json::Value) -> String {
    let parts: Vec<&str> = match body["segments"].as_array() {
        Some(segs) => segs
            .iter()
            .filter(|s| {
                let no_speech = s["no_speech_prob"].as_f64().unwrap_or(0.0);
                let logprob = s["avg_logprob"].as_f64().unwrap_or(0.0);
                !(no_speech > 0.6 && logprob < -1.0)
            })
            .filter_map(|s| s["text"].as_str())
            .collect(),
        None => vec![body["text"].as_str().unwrap_or_default()],
    };
    let mut text = parts.join(" ");
    while let Some(start) = text.find("<|") {
        match text[start..].find("|>") {
            Some(end) => text.replace_range(start..start + end + 2, " "),
            None => break,
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Drop for WhisperServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

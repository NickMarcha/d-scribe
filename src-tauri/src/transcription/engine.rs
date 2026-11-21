//! whisper.cpp engine: downloaded on demand into app data, run as a local server so the model
//! is loaded once per transcription instead of once per segment.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Pinned whisper.cpp build. Every zip contains a `Release/` folder (or the files directly)
/// with whisper-server and the ggml libraries. On a bump, the `engine` workflow builds the
/// Vulkan zip for the new build and prints the SHA256 values for `asset`.
const RELEASE: &str = "b5454";

/// A downloadable engine build for this system.
struct Asset {
    url: String,
    /// SHA256 of the zip, checked after download.
    sha256: &'static str,
}

/// The engine builds for this system. A kind without one isn't offered.
#[cfg(windows)]
fn asset(kind: EngineKind) -> Option<Asset> {
    let (url, sha256) = match kind {
        EngineKind::Cpu => (
            format!("https://github.com/ggml-org/whisper.cpp/releases/download/{RELEASE}/whisper-bin-x64.zip"),
            "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
        ),
        EngineKind::Cuda => (
            format!("https://github.com/ggml-org/whisper.cpp/releases/download/{RELEASE}/whisper-bin-win-cuda-12.4.0-x64.zip"),
            "afef0b881c500958921c3f5523b50e59ee2ec9b6f5cbd25b324c51ed308a957a",
        ),
        EngineKind::Vulkan => (
            format!("https://github.com/NickMarcha/d-scribe/releases/download/engine-{RELEASE}/whisper-bin-win-vulkan-x64.zip"),
            "97a688c17a574a5b24234064e8b1b9a5c9b33dca4a48977b37005ab29c1741d4",
        ),
    };
    Some(Asset { url, sha256 })
}

/// No builds yet. Linux builds would come from the `engine` workflow like the Windows Vulkan one.
#[cfg(not(windows))]
fn asset(_kind: EngineKind) -> Option<Asset> {
    None
}

/// Whether the drivers each GPU engine needs are installed: (NVIDIA/CUDA, Vulkan).
#[cfg(windows)]
pub fn gpu_drivers() -> (bool, bool) {
    let system32 = Path::new(r"C:\Windows\System32");
    (system32.join("nvcuda.dll").exists(), system32.join("vulkan-1.dll").exists())
}

#[cfg(not(windows))]
pub fn gpu_drivers() -> (bool, bool) {
    (false, false)
}

/// The server executable inside an engine folder.
fn server_exe() -> String {
    format!("whisper-server{}", std::env::consts::EXE_SUFFIX)
}

/// Files kept when unpacking an engine zip: the executables and their shared libraries.
fn is_engine_file(name: &str) -> bool {
    let lib = std::env::consts::DLL_SUFFIX;
    name == server_exe()
        || name == format!("whisper-cli{}", std::env::consts::EXE_SUFFIX)
        || name.ends_with(lib)
        // Versioned Linux libraries: libggml.so.1
        || (lib == ".so" && name.contains(".so."))
}

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

    /// The engines that can be downloaded on this system, fastest first.
    pub fn available() -> Vec<EngineKind> {
        Self::ALL.into_iter().filter(|k| asset(*k).is_some()).collect()
    }
}

pub fn engine_dir(engines_root: &Path, kind: EngineKind) -> PathBuf {
    engines_root.join(format!("{}-{}", kind.name(), RELEASE))
}

pub fn is_installed(engines_root: &Path, kind: EngineKind) -> bool {
    engine_dir(engines_root, kind).join(server_exe()).exists()
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
    #[cfg_attr(not(windows), allow(unused_mut))]
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
    let out = hidden_command(&engine_dir.join(server_exe()))
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

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("d-scribe-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_with(path: &Path, files: &[&str]) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for name in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn install(root: &Path, kind: EngineKind) {
        let dir = engine_dir(root, kind);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(server_exe()), "").unwrap();
    }

    #[test]
    fn unpacks_only_the_engine_files_flat() {
        let root = temp_dir();
        let exe = server_exe();
        let lib = format!("ggml{}", std::env::consts::DLL_SUFFIX);
        zip_with(
            &root.join("e.zip"),
            &[&format!("Release/{exe}"), &format!("Release/{lib}"), "Release/README.md", "bench.txt"],
        );
        let dir = root.join("cpu.part");
        unpack_engine(&root.join("e.zip"), &dir).unwrap();
        let mut names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        let mut want = vec![exe.clone(), lib];
        want.sort();
        assert_eq!(names, want);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(&exe)).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "whisper-server is executable");
        }
    }

    #[test]
    fn a_zip_without_the_server_leaves_nothing() {
        let root = temp_dir();
        zip_with(&root.join("e.zip"), &["Release/ggml.dll", "Release/libggml.so"]);
        let dir = root.join("cpu.part");
        let err = unpack_engine(&root.join("e.zip"), &dir).unwrap_err();
        assert!(err.contains("has no whisper-server"), "{}", err);
        assert!(!dir.exists());
    }

    #[test]
    fn engine_files_for_this_system() {
        assert!(is_engine_file(&server_exe()));
        assert!(!is_engine_file("readme.md"));
        #[cfg(windows)]
        {
            assert_eq!(server_exe(), "whisper-server.exe");
            assert!(is_engine_file("ggml-vulkan.dll"));
            assert!(!is_engine_file("whisper-server"));
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(server_exe(), "whisper-server");
            assert!(is_engine_file("libggml.so"));
            assert!(is_engine_file("libggml.so.1"));
            assert!(!is_engine_file("ggml.dll"));
        }
    }

    #[test]
    fn engines_offered_on_this_system() {
        let available = EngineKind::available();
        #[cfg(windows)]
        assert_eq!(available, [EngineKind::Cuda, EngineKind::Vulkan, EngineKind::Cpu]);
        // Every offered engine has a pinned checksum.
        for kind in available {
            let a = asset(kind).unwrap();
            assert_eq!(a.sha256.len(), 64, "{:?}", kind);
            assert!(a.url.contains(RELEASE), "{:?} url names the pinned build", kind);
        }
    }

    #[test]
    fn automatic_picks_the_fastest_installed_engine() {
        let root = temp_dir();
        assert!(chosen_engine(&root, "auto").unwrap().is_none());
        install(&root, EngineKind::Cpu);
        assert_eq!(chosen_engine(&root, "").unwrap().unwrap().0, EngineKind::Cpu);
        install(&root, EngineKind::Vulkan);
        assert_eq!(chosen_engine(&root, "auto").unwrap().unwrap().0, EngineKind::Vulkan);
        install(&root, EngineKind::Cuda);
        assert_eq!(chosen_engine(&root, "auto").unwrap().unwrap().0, EngineKind::Cuda);
        assert_eq!(chosen_engine(&root, "cpu").unwrap().unwrap().0, EngineKind::Cpu);
    }

    #[test]
    fn a_chosen_engine_that_is_missing_is_an_error() {
        let root = temp_dir();
        install(&root, EngineKind::Cpu);
        let err = chosen_engine(&root, "cuda").unwrap_err();
        assert!(err.starts_with("The NVIDIA GPU engine is not installed."), "{}", err);
        assert!(chosen_engine(&root, "metal").is_err());
    }

    #[test]
    fn cpu_engine_needs_no_gpu_lookup() {
        assert_eq!(device_index(EngineKind::Cpu, Path::new("nowhere"), "Some GPU").unwrap(), None);
        assert_eq!(device_index(EngineKind::Vulkan, Path::new("nowhere"), "").unwrap(), None);
    }

    #[test]
    fn removes_engines_from_older_builds() {
        let root = temp_dir();
        install(&root, EngineKind::Cpu);
        std::fs::create_dir_all(root.join("cpu-b1234")).unwrap();
        std::fs::write(root.join("cuda.zip"), "partial").unwrap();
        remove_old_engines(&root);
        let names: Vec<String> =
            std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, [format!("cpu-{RELEASE}")]);
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
    let asset = asset(kind).ok_or_else(|| format!("The {} engine isn't available on this system.", kind.label()))?;
    std::fs::create_dir_all(engines_root).map_err(|e| e.to_string())?;
    let zip_path = engines_root.join(format!("{}.zip", kind.name()));
    super::download::download_to_file(&asset.url, &zip_path, on_progress).await?;
    let hash = file_sha256(&zip_path);
    if hash.as_deref() != Ok(asset.sha256) {
        let _ = std::fs::remove_file(&zip_path);
        return Err(format!(
            "The downloaded {} engine doesn't match its checksum ({}). Try again.",
            kind.label(),
            hash.unwrap_or_else(|e| e)
        ));
    }

    let tmp_dir = dir.with_extension("part");
    let result = unpack_engine(&zip_path, &tmp_dir);
    let _ = std::fs::remove_file(&zip_path);
    result?;
    std::fs::rename(&tmp_dir, &dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Unpacks the executables and libraries of an engine zip into `dir`, flattening the zip's
/// `Release/` folder. Fails (leaving no `dir`) if there's no whisper-server in it.
fn unpack_engine(zip_path: &Path, dir: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dir);
    let unpack = || -> Result<(), String> {
        let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            let Some(rel) = entry.enclosed_name() else { continue };
            let Some(name) = rel.file_name() else { continue };
            if entry.is_dir() || !is_engine_file(&name.to_string_lossy().to_lowercase()) {
                continue;
            }
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let path = dir.join(name);
            let mut out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = entry.unix_mode().unwrap_or(0o644) | 0o755;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
            }
        }
        if !dir.join(server_exe()).exists() {
            return Err(format!("The downloaded engine has no {}", server_exe()));
        }
        Ok(())
    };
    let result = unpack();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(dir);
    }
    result
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
        let mut cmd = hidden_command(&engine.dir.join(server_exe()));
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

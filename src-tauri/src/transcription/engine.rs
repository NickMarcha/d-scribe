//! whisper.cpp engine: downloaded on demand into app data, run as a local server so the model
//! is loaded once per transcription instead of once per segment.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Pinned whisper.cpp build. Both zips contain a `Release/` folder with whisper-cli.exe,
/// whisper-server.exe and the ggml DLLs.
const RELEASE: &str = "b5454";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineKind {
    Cpu,
    /// NVIDIA GPU (CUDA 12.4 runtime included in the zip).
    Cuda,
}

impl EngineKind {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "cpu" => Ok(Self::Cpu),
            "cuda" => Ok(Self::Cuda),
            _ => Err(format!("Unknown engine: {}", s)),
        }
    }

    fn dir_name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
        }
    }

    fn asset(self) -> &'static str {
        match self {
            Self::Cpu => "whisper-bin-x64.zip",
            Self::Cuda => "whisper-bin-win-cuda-12.4.0-x64.zip",
        }
    }
}

pub fn engine_dir(engines_root: &Path, kind: EngineKind) -> PathBuf {
    engines_root.join(format!("{}-{}", kind.dir_name(), RELEASE))
}

pub fn is_installed(engines_root: &Path, kind: EngineKind) -> bool {
    engine_dir(engines_root, kind).join("whisper-server.exe").exists()
}

/// The GPU engine when installed, otherwise the CPU engine (if installed).
pub fn installed_engine(engines_root: &Path) -> Option<(EngineKind, PathBuf)> {
    [EngineKind::Cuda, EngineKind::Cpu]
        .into_iter()
        .find(|k| is_installed(engines_root, *k))
        .map(|k| (k, engine_dir(engines_root, k)))
}

/// Downloads and unpacks an engine. Returns its directory.
pub async fn download_engine<F>(engines_root: &Path, kind: EngineKind, mut on_progress: F) -> Result<PathBuf, String>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    use futures_util::StreamExt;

    let dir = engine_dir(engines_root, kind);
    if is_installed(engines_root, kind) {
        return Ok(dir);
    }
    std::fs::create_dir_all(engines_root).map_err(|e| e.to_string())?;
    let url = format!(
        "https://github.com/ggml-org/whisper.cpp/releases/download/{}/{}",
        RELEASE,
        kind.asset()
    );
    let response = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Download failed ({}): {}", response.status(), url));
    }
    let total = response.content_length();
    let zip_path = engines_root.join(format!("{}.zip.part", kind.dir_name()));
    {
        let mut file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            downloaded += bytes.len() as u64;
            on_progress(downloaded, total);
        }
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
    pub async fn start(engine_dir: &Path, model: &Path, log_path: &Path) -> Result<Self, String> {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| e.to_string())?
            .port();
        let log = std::fs::File::create(log_path).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(engine_dir.join("whisper-server.exe"));
        cmd.current_dir(engine_dir)
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "-bs", "5", "-m"])
            .arg(model)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
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
        log::info!("[transcribe] whisper-server ready on {} with {}", server.base_url, model.display());
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
        let text = body["text"]
            .as_str()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let english_prob = body["language_probabilities"]["en"].as_f64().unwrap_or(0.0) as f32;
        Ok(Inference { text, english_prob })
    }
}

impl Drop for WhisperServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

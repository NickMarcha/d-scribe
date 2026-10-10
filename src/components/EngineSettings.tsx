import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface EngineStatus {
  cpu: boolean;
  cuda: boolean;
  nvidia: boolean;
}

/** Downloads the whisper.cpp engine. The GPU build is used whenever it is installed. */
export function EngineSettings() {
  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [downloading, setDownloading] = useState<"cpu" | "cuda" | null>(null);
  const [progress, setProgress] = useState<{ bytes: number; total: number | null } | null>(null);
  const [error, setError] = useState("");

  function refresh() {
    invoke<EngineStatus>("engine_status_command").then(setEngine).catch((e) => setError(String(e)));
  }

  useEffect(refresh, []);

  useEffect(() => {
    const unlisten = listen<{ bytesDownloaded: number; totalBytes: number | null }>("engine-download-progress", (e) =>
      setProgress({ bytes: e.payload.bytesDownloaded, total: e.payload.totalBytes }),
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  async function download(kind: "cpu" | "cuda") {
    setDownloading(kind);
    setProgress(null);
    setError("");
    try {
      await invoke("download_engine_command", { kind });
    } catch (e) {
      setError(`Download failed: ${e}`);
    }
    setDownloading(null);
    refresh();
  }

  async function remove(kind: "cpu" | "cuda") {
    try {
      await invoke("remove_engine_command", { kind });
    } catch (e) {
      setError(`Could not remove: ${e}`);
    }
    refresh();
  }

  if (!engine) return null;
  const active = engine.cuda ? "NVIDIA GPU" : engine.cpu ? "CPU" : "none yet";

  return (
    <div className="form-group">
      <label>Transcription engine</label>
      <span className="field-hint">
        In use: {active}. The CPU engine (8 MB) downloads by itself the first time you transcribe. On an NVIDIA
        graphics card, the GPU engine is about 10 times faster with large models. It is a 650 MB download and is
        used whenever it is installed.
      </span>
      <div className="button-row" style={{ marginTop: "0.5rem" }}>
        {!engine.cuda ? (
          <button type="button" onClick={() => download("cuda")} disabled={downloading != null || !engine.nvidia}>
            {downloading === "cuda" ? "Downloading GPU engine..." : "Download GPU engine (NVIDIA)"}
          </button>
        ) : (
          <button type="button" onClick={() => remove("cuda")} disabled={downloading != null}>
            Remove GPU engine
          </button>
        )}
        {!engine.cpu && (
          <button type="button" onClick={() => download("cpu")} disabled={downloading != null}>
            {downloading === "cpu" ? "Downloading CPU engine..." : "Download CPU engine"}
          </button>
        )}
      </div>
      {!engine.nvidia && !engine.cuda && (
        <span className="field-hint">No NVIDIA driver found, so only the CPU engine is available.</span>
      )}
      {downloading && progress && (
        <div className="download-progress-wrap">
          <div className="download-progress-bar">
            <div
              className="download-progress-fill"
              style={{ width: progress.total ? `${(progress.bytes / progress.total) * 100}%` : "0%" }}
            />
          </div>
          <span className="download-progress-text">
            {Math.round(progress.bytes / 1024 / 1024)} MB
            {progress.total ? ` of ${Math.round(progress.total / 1024 / 1024)} MB` : ""}
          </span>
        </div>
      )}
      {error && <span className="field-hint" style={{ color: "var(--danger)" }}>{error}</span>}
    </div>
  );
}

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { runTask, bytesProgress, useTasks } from "../tasks";

interface EngineStatus {
  cpu: boolean;
  cuda: boolean;
  nvidia: boolean;
}

/** Downloads the whisper.cpp engine. The GPU build is used whenever it is installed. */
export function EngineSettings() {
  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const tasks = useTasks();
  const downloading = (["cpu", "cuda"] as const).find((k) => tasks.some((t) => t.id === `engine:${k}`)) ?? null;
  const [error, setError] = useState("");

  function refresh() {
    invoke<EngineStatus>("engine_status_command").then(setEngine).catch((e) => setError(String(e)));
  }

  useEffect(refresh, []);

  async function download(kind: "cpu" | "cuda") {
    setError("");
    try {
      await runTask(
        `engine:${kind}`,
        kind === "cuda" ? "Downloading the GPU engine" : "Downloading the CPU engine",
        [{ event: "engine-download-progress", map: (p) => (p.kind === kind ? bytesProgress(p) : null) }],
        () => invoke("download_engine_command", { kind }),
      );
    } catch (e) {
      setError(`Download failed: ${e}`);
    }
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
      {error && <span className="field-hint" style={{ color: "var(--danger)" }}>{error}</span>}
    </div>
  );
}

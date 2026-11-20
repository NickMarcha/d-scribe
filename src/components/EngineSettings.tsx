import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { load } from "@tauri-apps/plugin-store";
import { openPath } from "@tauri-apps/plugin-opener";
import { runTask, bytesProgress, useTasks } from "../tasks";

type Kind = "cuda" | "vulkan" | "cpu";

interface EngineStatus {
  cpu: boolean;
  cuda: boolean;
  vulkan: boolean;
  nvidia: boolean;
  vulkanDriver: boolean;
  /** The engine transcription will use, or null if the chosen one isn't installed. */
  active: Kind | null;
}

const ENGINES: { kind: Kind; label: string; size: string }[] = [
  { kind: "cuda", label: "NVIDIA GPU", size: "650 MB" },
  { kind: "vulkan", label: "GPU (Vulkan)", size: "19 MB" },
  { kind: "cpu", label: "CPU", size: "8 MB" },
];

const labelOf = (kind: Kind) => ENGINES.find((e) => e.kind === kind)!.label;

/** Downloads whisper.cpp engines and picks which one transcribes. Saved as "engine" in settings.json. */
export function EngineSettings() {
  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [choice, setChoice] = useState("auto");
  const tasks = useTasks();
  const downloading = ENGINES.find((e) => tasks.some((t) => t.id === `engine:${e.kind}`))?.kind ?? null;
  const [error, setError] = useState("");

  function refresh() {
    invoke<EngineStatus>("engine_status_command").then(setEngine).catch((e) => setError(String(e)));
  }

  useEffect(() => {
    refresh();
    load("settings.json", { defaults: {}, autoSave: true }).then(async (store) => {
      setChoice((await store.get<string>("engine")) || "auto");
    });
  }, []);

  async function choose(value: string) {
    setChoice(value);
    const store = await load("settings.json", { defaults: {}, autoSave: true });
    await store.set("engine", value);
    await store.save();
    refresh();
  }

  async function download(kind: Kind) {
    setError("");
    try {
      await runTask(
        `engine:${kind}`,
        `Downloading the ${labelOf(kind)} engine`,
        [{ event: "engine-download-progress", map: (p) => (p.kind === kind ? bytesProgress(p) : null) }],
        () => invoke("download_engine_command", { kind }),
      );
    } catch (e) {
      setError(`Download failed: ${e}`);
    }
    refresh();
  }

  async function remove(kind: Kind) {
    try {
      await invoke("remove_engine_command", { kind });
      if (choice === kind) await choose("auto");
    } catch (e) {
      setError(`Could not remove: ${e}`);
    }
    refresh();
  }

  if (!engine) return null;
  const hasDriver = (kind: Kind) => (kind === "cuda" ? engine.nvidia : kind === "vulkan" ? engine.vulkanDriver : true);

  return (
    <div className="form-group">
      <label htmlFor="engine-choice">Transcription engine</label>
      <select id="engine-choice" value={choice} onChange={(e) => choose(e.target.value)}>
        <option value="auto">Automatic (fastest installed)</option>
        {ENGINES.map((e) => (
          <option key={e.kind} value={e.kind} disabled={!engine[e.kind] && choice !== e.kind}>
            {e.label}
            {engine[e.kind] ? "" : " (not installed)"}
          </option>
        ))}
      </select>
      <span className="field-hint">
        {engine.active
          ? `In use: ${labelOf(engine.active)}.`
          : choice === "auto"
            ? "In use: none yet. The CPU engine downloads by itself the first time you transcribe."
            : `The ${labelOf(choice as Kind)} engine is not installed. Download it below or pick another engine.`}{" "}
        On a graphics card, transcribing is about 10 times faster with large models. Use the NVIDIA engine on an
        NVIDIA card and the Vulkan engine on AMD or Intel graphics. If an engine gives errors or wrong text, pick
        CPU here.
      </span>
      <div className="button-row" style={{ marginTop: "0.5rem" }}>
        {ENGINES.map((e) =>
          engine[e.kind] ? (
            <button key={e.kind} type="button" onClick={() => remove(e.kind)} disabled={downloading != null}>
              Remove {e.label} engine
            </button>
          ) : (
            <button
              key={e.kind}
              type="button"
              onClick={() => download(e.kind)}
              disabled={downloading != null || !hasDriver(e.kind)}
            >
              {downloading === e.kind ? `Downloading ${e.label} engine...` : `Download ${e.label} engine (${e.size})`}
            </button>
          ),
        )}
        <button
          type="button"
          onClick={() =>
            invoke<string>("get_engines_dir")
              .then((dir) => openPath(dir))
              .catch((e) => setError(`Could not open the folder: ${e}`))
          }
        >
          Open engines folder
        </button>
      </div>
      {(!engine.nvidia || !engine.vulkanDriver) && (
        <span className="field-hint">
          {!engine.nvidia && !engine.vulkanDriver
            ? "No graphics driver with GPU support found, so only the CPU engine is available."
            : !engine.nvidia
              ? "No NVIDIA driver found, so the NVIDIA engine is not available."
              : "No Vulkan driver found, so the Vulkan engine is not available."}
        </span>
      )}
      {error && <span className="field-hint" style={{ color: "var(--danger)" }}>{error}</span>}
    </div>
  );
}

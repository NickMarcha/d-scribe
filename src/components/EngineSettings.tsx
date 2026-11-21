import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { load } from "@tauri-apps/plugin-store";
import { openPath } from "@tauri-apps/plugin-opener";
import { useTasks } from "../tasks";
import {
  clearDownloadError,
  downloadEngine,
  refreshEngineStatus,
  useDownloadErrors,
  useEngineStatus,
  type EngineKind as Kind,
} from "../installed";

const ENGINES: { kind: Kind; label: string; size: string }[] = [
  { kind: "cuda", label: "NVIDIA GPU", size: "650 MB" },
  { kind: "vulkan", label: "GPU (Vulkan)", size: "19 MB" },
  { kind: "cpu", label: "CPU", size: "8 MB" },
];

const labelOf = (kind: Kind) => ENGINES.find((e) => e.kind === kind)!.label;

interface GpuDevice {
  /** The name, plus " #2" etc. when several cards share it. This is what gets saved. */
  id: string;
  name: string;
  integrated: boolean;
}

/** Which graphics card a GPU engine runs on. Saved by id as "gpu_device_<kind>"; empty means the engine's default (the first one). */
function GpuChoice({ kind }: { kind: "cuda" | "vulkan" }) {
  const [devices, setDevices] = useState<GpuDevice[] | null>(null);
  const [choice, setChoice] = useState("");
  const [error, setError] = useState("");
  const key = `gpu_device_${kind}`;

  useEffect(() => {
    invoke<GpuDevice[]>("gpu_devices_command", { kind }).then(setDevices).catch((e) => setError(String(e)));
    load("settings.json", { defaults: {}, autoSave: true }).then(async (store) => {
      setChoice((await store.get<string>(key)) ?? "");
    });
  }, [kind]);

  async function choose(value: string) {
    setChoice(value);
    const store = await load("settings.json", { defaults: {}, autoSave: true });
    await store.set(key, value);
    await store.save();
  }

  if (error) return <span className="field-hint" style={{ color: "var(--danger)" }}>Could not list graphics cards: {error}</span>;
  // Nothing to choose with one card (or none, when the engine falls back to CPU).
  if (!devices || (devices.length < 2 && choice === "")) return null;
  const label = (d: GpuDevice) => (d.integrated ? `${d.id} (built-in graphics)` : d.id);

  return (
    <div style={{ marginTop: "0.5rem" }}>
      <label htmlFor="gpu-choice">Graphics card</label>
      <select id="gpu-choice" value={choice} onChange={(e) => choose(e.target.value)}>
        <option value="">Default ({devices[0] ? label(devices[0]) : "none found"})</option>
        {choice !== "" && !devices.some((d) => d.id === choice) && <option value={choice}>{choice} (not found)</option>}
        {devices.map((d) => (
          <option key={d.id} value={d.id}>
            {label(d)}
          </option>
        ))}
      </select>
      <span className="field-hint">
        Built-in graphics is slower than a separate card but leaves it free for games and uses less power. It
        applies to the next transcription or recording.
      </span>
    </div>
  );
}

/** Downloads whisper.cpp engines and picks which one transcribes. Saved as "engine" in settings.json. */
export function EngineSettings() {
  const engine = useEngineStatus();
  const [choice, setChoice] = useState("auto");
  const tasks = useTasks();
  const downloading = ENGINES.find((e) => tasks.some((t) => t.id === `engine:${e.kind}`))?.kind ?? null;
  const downloadErrors = useDownloadErrors();
  const [error, setError] = useState("");

  function refresh() {
    refreshEngineStatus().catch((e) => setError(String(e)));
  }

  useEffect(() => {
    // Engines can also change outside this screen (the CPU engine downloads on first transcribe).
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
    // A failure is kept in useDownloadErrors and shown below, even if you left meanwhile.
    await downloadEngine(kind, labelOf(kind)).catch(() => {});
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
  const offered = ENGINES.filter((e) => engine.available.includes(e.kind));
  const gpuEngines = offered.filter((e) => e.kind !== "cpu");
  const noDriver = gpuEngines.filter((e) => !hasDriver(e.kind));

  return (
    <div className="form-group">
      <label htmlFor="engine-choice">Transcription engine</label>
      <select id="engine-choice" value={choice} onChange={(e) => choose(e.target.value)}>
        <option value="auto">Automatic (fastest installed)</option>
        {offered.map((e) => (
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
      {(engine.active === "cuda" || engine.active === "vulkan") && <GpuChoice key={engine.active} kind={engine.active} />}
      <div className="button-row" style={{ marginTop: "0.5rem" }}>
        {offered.map((e) =>
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
      {noDriver.length > 0 && (
        <span className="field-hint">
          {noDriver.length === gpuEngines.length && gpuEngines.length > 1
            ? "No graphics driver with GPU support found, so only the CPU engine is available."
            : noDriver
                .map((e) => (e.kind === "cuda" ? "NVIDIA" : "Vulkan"))
                .map((name) => `No ${name} driver found, so the ${name} engine is not available.`)
                .join(" ")}
        </span>
      )}
      {ENGINES.filter((e) => downloadErrors[`engine:${e.kind}`]).map((e) => (
        <span key={e.kind} className="field-hint" style={{ color: "var(--danger)" }}>
          Downloading the {e.label} engine failed: {downloadErrors[`engine:${e.kind}`]}{" "}
          <button type="button" className="link-button" onClick={() => clearDownloadError(`engine:${e.kind}`)}>
            Dismiss
          </button>
        </span>
      ))}
      {error && <span className="field-hint" style={{ color: "var(--danger)" }}>{error}</span>}
    </div>
  );
}

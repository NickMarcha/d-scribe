import { useEffect, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { runTask, bytesProgress } from "./tasks";

/**
 * What's installed (Whisper models, engines) and the downloads that change it. Kept outside
 * React like tasks.ts: a download outlives the screen that started it, so the lists refresh
 * here when it ends, and its error is kept until the next try.
 */

export type EngineKind = "cuda" | "vulkan" | "cpu";

export interface EngineStatus {
  cpu: boolean;
  cuda: boolean;
  vulkan: boolean;
  nvidia: boolean;
  vulkanDriver: boolean;
  /** The engine transcription will use, or null if the chosen one isn't installed. */
  active: EngineKind | null;
}

let models: string[] | null = null;
let engine: EngineStatus | null = null;
/** Failed downloads by task id ("model:<name>", "engine:<kind>"). */
let errors: Record<string, string> = {};
const running = new Map<string, Promise<unknown>>();
const subscribers = new Set<() => void>();

function emit() {
  subscribers.forEach((fn) => fn());
}

function subscribe(fn: () => void) {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

export async function refreshInstalledModels() {
  models = await invoke<string[]>("list_installed_model_names_command");
  emit();
}

export async function refreshEngineStatus() {
  engine = await invoke<EngineStatus>("engine_status_command");
  emit();
}

/** Installed model names; null until the first read. */
export function useInstalledModels(): string[] | null {
  useEffect(() => {
    if (models === null) refreshInstalledModels().catch((e) => console.error("Could not list models:", e));
  }, []);
  return useSyncExternalStore(subscribe, () => models);
}

export function useEngineStatus(): EngineStatus | null {
  useEffect(() => {
    if (engine === null) refreshEngineStatus().catch((e) => console.error("Could not read engines:", e));
  }, []);
  return useSyncExternalStore(subscribe, () => engine);
}

/** Download errors by task id, e.g. errors["model:small"]. */
export function useDownloadErrors(): Record<string, string> {
  return useSyncExternalStore(subscribe, () => errors);
}

export function clearDownloadError(id: string) {
  if (!(id in errors)) return;
  const { [id]: _, ...rest } = errors;
  errors = rest;
  emit();
}

/** Runs one download per id: a second call while it runs gets the same promise. */
function download<T>(id: string, start: () => Promise<T>, refresh: () => Promise<void>): Promise<T> {
  const existing = running.get(id);
  if (existing) return existing as Promise<T>;
  clearDownloadError(id);
  const job = start()
    .catch((e) => {
      errors = { ...errors, [id]: String(e) };
      emit();
      throw e;
    })
    .finally(() => {
      running.delete(id);
      return refresh().catch((e) => console.error("Could not refresh after download:", e));
    });
  running.set(id, job);
  return job;
}

/** Downloads a Whisper model. Resolves with its path. */
export function downloadModel(name: string): Promise<string> {
  const id = `model:${name}`;
  return download(
    id,
    () =>
      runTask(id, `Downloading ${name}`, [{ event: "download-progress", map: (p) => (p.modelName === name ? bytesProgress(p) : null) }], () =>
        invoke<string>("download_model_command", { modelName: name }),
      ),
    refreshInstalledModels,
  );
}

export function downloadEngine(kind: EngineKind, label: string): Promise<void> {
  const id = `engine:${kind}`;
  return download(
    id,
    () =>
      runTask(id, `Downloading the ${label} engine`, [{ event: "engine-download-progress", map: (p) => (p.kind === kind ? bytesProgress(p) : null) }], () =>
        invoke<void>("download_engine_command", { kind }),
      ),
    refreshEngineStatus,
  );
}

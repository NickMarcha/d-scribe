// In-memory stand-in for the Rust backend, wired up through mockIPC.
//
// - settings.json lives in `backend.settings` (plugin:store|* commands).
// - Recordings live in `backend.projects`, keyed by path. load/save/auto_save read and write it.
// - Long jobs (downloads, transcription, AI) are "controlled": each call returns a promise the
//   test settles through `backend.nextCall(cmd)` / `backend.pendingCalls(cmd)`.
// - Every invoke is recorded in `backend.calls`.
// - `backend.emit(event, payload)` delivers a backend event to `listen` handlers.

import { mockConvertFileSrc, mockIPC } from "@tauri-apps/api/mocks";

export interface Segment {
  start_ms: number;
  end_ms: number;
  user_id: string;
  speaker_name: string | null;
}

export interface SessionState {
  session_id: string;
  created_at: number;
  title?: string | null;
  guild_name: string | null;
  guild_id: string | null;
  channel_name: string | null;
  channel_id: string | null;
  channel_type?: number;
  live_mode_enabled?: boolean;
  self_user_id: string | null;
  user_labels?: Record<string, string>;
  segments: Segment[];
  transcript_texts: string[];
  live_transcript_texts?: string[];
  audio_paths: { loopback: string | null; microphone: string | null };
  ai_outputs?: { prompt_name: string; output: string; created_at: number }[];
}

export interface EngineStatus {
  cpu: boolean;
  cuda: boolean;
  vulkan: boolean;
  nvidia: boolean;
  vulkanDriver: boolean;
  active: "cuda" | "vulkan" | "cpu" | null;
}

export type Args = Record<string, any>;

export interface Call {
  cmd: string;
  args: Args;
}

export interface PendingCall<T = unknown> extends Call {
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
  settled: boolean;
}

/** Commands that return a promise the test settles. */
export const DEFAULT_CONTROLLED = [
  "download_model_command",
  "transcribe_session_command",
  "run_ai_command",
  "download_engine_command",
];

const clone = <T,>(v: T): T => (v === undefined ? v : JSON.parse(JSON.stringify(v)));

export const RECORDINGS_DIR = "C:\\fake\\d-scribe\\recordings";
export const MODELS_DIR = "C:\\fake\\d-scribe\\models";

export function recordingPath(name: string) {
  return `${RECORDINGS_DIR}\\${name}`;
}

export function modelPath(name: string) {
  return `${MODELS_DIR}\\ggml-${name}.bin`;
}

export interface FakeBackendOptions {
  settings?: Record<string, unknown>;
  /** path -> state. Use `recordingPath(name)` for paths. */
  projects?: Record<string, SessionState>;
  /** path -> saved flag. */
  saved?: Record<string, boolean>;
  installedModels?: string[];
  engine?: Partial<EngineStatus>;
  /** Extra commands to control (e.g. "list_installed_model_names_command"). */
  control?: string[];
}

export class FakeBackend {
  settings: Record<string, unknown>;
  projects = new Map<string, SessionState>();
  saved = new Map<string, boolean>();
  installedModels: string[];
  engine: EngineStatus;
  calls: Call[] = [];
  controlled: Set<string>;
  /** Returned by `stop_recording`. */
  nextRecording: SessionState | null = null;
  /** Path `auto_save_project_command` stores a state under. */
  autoSavePath: (state: SessionState) => string = (state) => recordingPath(state.session_id);
  /** Per-command overrides; take precedence over the built-in handlers. */
  handlers: Record<string, (args: Args) => unknown> = {};

  private pending: PendingCall<any>[] = [];
  private waiters: { cmd: string; fn: () => boolean }[] = [];
  private listeners = new Map<number, { event: string; handler: number }>();
  private nextEventId = 1;

  constructor(opts: FakeBackendOptions = {}) {
    this.settings = clone(opts.settings ?? {});
    for (const [path, state] of Object.entries(opts.projects ?? {})) {
      this.projects.set(path, clone(state));
      this.saved.set(path, opts.saved?.[path] ?? false);
    }
    this.installedModels = [...(opts.installedModels ?? [])];
    this.engine = { cpu: true, cuda: false, vulkan: false, nvidia: false, vulkanDriver: true, active: "cpu", ...opts.engine };
    this.controlled = new Set([...DEFAULT_CONTROLLED, ...(opts.control ?? [])]);
  }

  /** Installs this backend as the Tauri IPC. Call before rendering. */
  install() {
    mockIPC((cmd, args) => this.invoke(cmd, (args ?? {}) as Args));
    mockConvertFileSrc("windows");
    return this;
  }

  // ---- inspection -------------------------------------------------------------------------

  callsTo(cmd: string): Call[] {
    return this.calls.filter((c) => c.cmd === cmd);
  }

  /** Current state on disk for a recording. */
  project(path: string): SessionState | undefined {
    return clone(this.projects.get(path));
  }

  /** Every state written to `path` by save_project_command, in order. */
  savesTo(path: string): SessionState[] {
    return this.callsTo("save_project_command")
      .filter((c) => c.args.path === path)
      .map((c) => c.args.state as SessionState);
  }

  pendingCalls<T = unknown>(cmd: string): PendingCall<T>[] {
    return this.pending.filter((p) => p.cmd === cmd && !p.settled);
  }

  /** Resolves with the first unsettled call to `cmd`, waiting for one if needed. */
  nextCall<T = unknown>(cmd: string, timeoutMs = 4000): Promise<PendingCall<T>> {
    const find = () => this.pendingCalls<T>(cmd)[0];
    const found = find();
    if (found) return Promise.resolve(found);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`No call to ${cmd} within ${timeoutMs} ms`)), timeoutMs);
      this.waiters.push({
        cmd,
        fn: () => {
          const p = find();
          if (p) {
            clearTimeout(timer);
            resolve(p);
            return true;
          }
          return false;
        },
      });
    });
  }

  resolveAll(cmd: string, value: unknown) {
    for (const p of this.pendingCalls(cmd)) p.resolve(clone(value));
  }

  // ---- events -------------------------------------------------------------------------------

  /** Sends a backend event to every `listen` handler for it. */
  emit(event: string, payload: unknown) {
    const internals = (window as any).__TAURI_INTERNALS__;
    for (const [id, l] of this.listeners) {
      if (l.event === event) internals.runCallback(l.handler, { event, id, payload });
    }
  }

  listenerCount(event: string) {
    return [...this.listeners.values()].filter((l) => l.event === event).length;
  }

  // ---- IPC ----------------------------------------------------------------------------------

  private invoke(cmd: string, args: Args): unknown {
    this.calls.push({ cmd, args: clone(args) });
    if (this.handlers[cmd]) return this.handlers[cmd](args);
    if (this.controlled.has(cmd)) return this.defer(cmd, args);
    return this.builtin(cmd, args);
  }

  private defer(cmd: string, args: Args) {
    return new Promise((resolve, reject) => {
      const entry: PendingCall<any> = {
        cmd,
        args: clone(args),
        settled: false,
        resolve: (v) => {
          entry.settled = true;
          resolve(v);
        },
        reject: (e) => {
          entry.settled = true;
          reject(e);
        },
      };
      this.pending.push(entry);
      this.waiters = this.waiters.filter((w) => w.cmd !== cmd || !w.fn());
    });
  }

  private meta(path: string, s: SessionState) {
    const speakers = [...new Set(s.segments.map((seg) => seg.speaker_name ?? seg.user_id))];
    const duration = s.segments.reduce((m, seg) => Math.max(m, seg.end_ms), 0);
    return {
      name: s.session_id,
      path,
      title: s.title ?? null,
      guild_name: s.guild_name,
      channel_name: s.channel_name,
      channel_type: s.channel_type ?? null,
      created_at: s.created_at,
      saved: this.saved.get(path) ?? false,
      duration_ms: duration,
      speakers,
      others: Object.entries(s.user_labels ?? {})
        .filter(([id]) => id !== s.self_user_id)
        .map(([, n]) => n),
      transcribed: s.transcript_texts.some((t) => t?.trim()),
      size_bytes: 1024 * 1024,
    };
  }

  private builtin(cmd: string, args: Args): unknown {
    if (cmd.startsWith("plugin:store|")) return this.store(cmd.slice("plugin:store|".length), args);
    switch (cmd) {
      // events
      case "plugin:event|listen": {
        const id = this.nextEventId++;
        this.listeners.set(id, { event: args.event, handler: args.handler });
        return id;
      }
      case "plugin:event|unlisten":
        this.listeners.delete(args.eventId);
        return null;
      case "plugin:event|emit":
        this.emit(args.event, args.payload);
        return null;

      // plugins
      case "plugin:app|version":
        return "0.1.21";
      case "plugin:updater|check":
        return null;
      case "plugin:autostart|is_enabled":
        return false;
      case "plugin:autostart|enable":
      case "plugin:autostart|disable":
        return null;
      case "plugin:resources|close":
        return null;

      // app
      case "discord_rpc_connection_state":
        return { state: "Disconnected", discord_running: false };
      case "get_channel_info_command":
        return null;
      case "get_app_data_dir":
        return "C:\\fake\\d-scribe";
      case "get_log_file_path":
        return "C:\\fake\\d-scribe\\logs\\d-scribe.log";
      case "get_recordings_dir":
        return RECORDINGS_DIR;
      case "get_models_dir":
        return MODELS_DIR;
      case "get_engines_dir":
        return "C:\\fake\\d-scribe\\engines";
      case "storage_usage_command":
        return { recordingCount: this.projects.size, recordings: 0, unsaved: 0, models: 0, engines: 0 };

      // recordings
      case "purge_recent_command":
        return null;
      case "list_projects_with_meta_command":
        return [...this.projects].map(([path, s]) => this.meta(path, s));
      case "load_project_command": {
        const s = this.projects.get(args.path);
        if (!s) throw `No recording at ${args.path}`;
        return clone(s);
      }
      case "save_project_command":
        this.projects.set(args.path, clone(args.state));
        return null;
      case "auto_save_project_command": {
        const path = this.autoSavePath(args.state);
        this.projects.set(path, clone(args.state));
        if (!this.saved.has(path)) this.saved.set(path, false);
        return path;
      }
      case "set_project_saved_command":
        this.saved.set(args.path, args.saved);
        return null;
      case "delete_project_command":
        this.projects.delete(args.path);
        this.saved.delete(args.path);
        return null;
      case "start_recording":
        return null;
      case "stop_recording":
        return clone(this.nextRecording);
      case "export_transcript":
        return null;

      // models and engines
      case "list_installed_model_names_command":
        return [...this.installedModels];
      case "resolve_model_path_command":
        return this.installedModels.includes(args.modelName) ? modelPath(args.modelName) : null;
      case "list_remote_models_command":
        return [];
      case "engine_status_command":
        return { ...this.engine };
      case "gpu_devices_command":
        return [];
      case "remove_engine_command":
        return null;

      // audio
      case "list_audio_devices_command":
        return { inputs: [], outputs: [], default_input: "Mic", default_output: "Speakers" };
      case "measure_audio_level_command":
        return 0.5;

      default:
        return null;
    }
  }

  private store(op: string, args: Args): unknown {
    switch (op) {
      case "load":
      case "get_store":
        return 1;
      case "get": {
        const exists = Object.prototype.hasOwnProperty.call(this.settings, args.key);
        return [exists ? clone(this.settings[args.key]) : null, exists];
      }
      case "set":
        this.settings[args.key] = clone(args.value);
        return null;
      case "has":
        return Object.prototype.hasOwnProperty.call(this.settings, args.key);
      case "delete": {
        const had = Object.prototype.hasOwnProperty.call(this.settings, args.key);
        delete this.settings[args.key];
        return had;
      }
      case "clear":
      case "reset":
        this.settings = {};
        return null;
      case "keys":
        return Object.keys(this.settings);
      case "values":
        return Object.values(this.settings).map(clone);
      case "entries":
        return Object.entries(this.settings).map(([k, v]) => [k, clone(v)]);
      case "length":
        return Object.keys(this.settings).length;
      default:
        // save, reload, close: nothing to do in memory.
        return null;
    }
  }
}

// ---- sample data --------------------------------------------------------------------------

const NOW_S = Math.floor(new Date("2026-11-20T10:00:00").getTime() / 1000);

export const PATH_A = recordingPath("2026-11-20 10.00 Alpha server - general");
export const PATH_B = recordingPath("2026-11-20 11.00 Beta server - lounge");

/** Three turns: Alice, Bob, Alice. Not transcribed. */
export function recordingA(overrides: Partial<SessionState> = {}): SessionState {
  return {
    session_id: "2026-11-20 10.00 Alpha server - general",
    created_at: NOW_S,
    title: null,
    guild_name: "Alpha server",
    guild_id: "g1",
    channel_name: "general",
    channel_id: "c1",
    channel_type: 2,
    self_user_id: "u1",
    user_labels: { u1: "Alice", u2: "Bob" },
    segments: [
      { start_ms: 0, end_ms: 5000, user_id: "u1", speaker_name: "Alice" },
      { start_ms: 6000, end_ms: 10000, user_id: "u2", speaker_name: "Bob" },
      { start_ms: 30000, end_ms: 35000, user_id: "u1", speaker_name: "Alice" },
    ],
    transcript_texts: ["", "", ""],
    audio_paths: { loopback: `${PATH_A}\\others.wav`, microphone: `${PATH_A}\\you.wav` },
    ai_outputs: [],
    ...overrides,
  };
}

/** Two turns: Carol, Dan. Already transcribed. */
export function recordingB(overrides: Partial<SessionState> = {}): SessionState {
  return {
    session_id: "2026-11-20 11.00 Beta server - lounge",
    created_at: NOW_S + 3600,
    title: null,
    guild_name: "Beta server",
    guild_id: "g2",
    channel_name: "lounge",
    channel_id: "c2",
    channel_type: 2,
    self_user_id: "u3",
    user_labels: { u3: "Carol", u4: "Dan" },
    segments: [
      { start_ms: 0, end_ms: 4000, user_id: "u3", speaker_name: "Carol" },
      { start_ms: 5000, end_ms: 9000, user_id: "u4", speaker_name: "Dan" },
    ],
    transcript_texts: ["Carol said this before.", "Dan said this before."],
    audio_paths: { loopback: `${PATH_B}\\others.wav`, microphone: `${PATH_B}\\you.wav` },
    ai_outputs: [],
    ...overrides,
  };
}

/** A recording made during a test (returned by stop_recording). */
export function recordingC(overrides: Partial<SessionState> = {}): SessionState {
  return {
    session_id: "2026-11-20 12.00 Gamma server - voice",
    created_at: NOW_S + 7200,
    title: null,
    guild_name: "Gamma server",
    guild_id: "g3",
    channel_name: "voice",
    channel_id: "c3",
    channel_type: 2,
    self_user_id: "u5",
    user_labels: { u5: "Erin" },
    segments: [{ start_ms: 0, end_ms: 3000, user_id: "u5", speaker_name: "Erin" }],
    transcript_texts: [""],
    audio_paths: { loopback: null, microphone: null },
    ai_outputs: [],
    ...overrides,
  };
}

export const REGISTRY = [
  { id: "base.en", type: "integrated" },
  { id: "medium.en", type: "integrated" },
  { id: "small.en", type: "integrated" },
  { id: "large-v3", type: "integrated" },
];

/** Settings where English uses medium.en (installed by default in `defaultBackend`). */
export function baseSettings(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    model_registry: REGISTRY,
    language_slots: [
      { id: "en", label: "English", languageCode: "en", liveModel: "base.en", regularModel: "medium.en" },
    ],
    selected_language_id: "en",
    ai_command: "fake-ai --print",
    ai_prompts: [{ id: "summary", name: "Summary", instructions: "Summarize the call." }],
    ...overrides,
  };
}

/** Recordings A and B, base.en / medium.en / small.en installed. */
export function defaultBackend(opts: FakeBackendOptions = {}) {
  return new FakeBackend({
    settings: baseSettings(),
    projects: { [PATH_A]: recordingA(), [PATH_B]: recordingB() },
    installedModels: ["base.en", "medium.en", "small.en"],
    ...opts,
  }).install();
}

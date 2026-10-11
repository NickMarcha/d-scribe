import { useState, useEffect, useCallback, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { load } from "@tauri-apps/plugin-store";
import { StatsPanel } from "./StatsPanel";
import { PlaybackBar } from "./PlaybackBar";
import { Transcript, type Speaker } from "./Transcript";
import { setText, type TranscriptData } from "./transcriptOps";
import { AiPanel } from "./AiPanel";
import { Menu } from "./Menu";
import { runTask, bytesProgress } from "../tasks";
import { RecordingsOverview, placeLabel, type ProjectMeta } from "./RecordingsOverview";
import type { AiOutput } from "./aiConfig";
import "./Session.css";

interface SessionSegment {
  start_ms: number;
  end_ms: number;
  user_id: string;
  speaker_name: string | null;
}

interface SessionState {
  session_id: string;
  created_at: number;
  guild_name: string | null;
  guild_id: string | null;
  channel_name: string | null;
  channel_id: string | null;
  channel_type?: number; // 1=dm, 2=guild_voice, 3=group_dm
  live_mode_enabled?: boolean;
  self_user_id: string | null;
  user_labels?: Record<string, string>;
  segments: SessionSegment[];
  transcript_texts: string[];
  live_transcript_texts?: string[];
  audio_paths: { loopback: string | null; microphone: string | null };
  ai_outputs?: AiOutput[];
}

const DEFAULT_TEMPLATE = "{guild}_{channel}_{timestamp}";

interface LanguageSlot {
  id: string;
  label: string;
  languageCode: string;
  liveModel: string;
  regularModel: string;
  /** Optional: turns this model detects as English are kept as English; the rest use regularModel. */
  englishModel?: string;
}

const DEFAULT_LANGUAGE_SLOTS: LanguageSlot[] = [
  { id: "en", label: "English", languageCode: "en", liveModel: "base.en", regularModel: "medium.en" },
  { id: "multilingual", label: "Multilingual", languageCode: "auto", liveModel: "tiny", regularModel: "large-v3" },
];

/** How long you can be alone in the channel before an automatic recording stops. */
const AUTO_STOP_GRACE_MS = 60_000;

export function Session() {
  const [recording, setRecording] = useState(false);
  const [session, setSession] = useState<SessionState | null>(null);
  const [projects, setProjects] = useState<ProjectMeta[]>([]);
  const [retention, setRetention] = useState({ days: 10, maxGb: 20 });
  const [currentProjectPath, setCurrentProjectPath] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [deleteAudio, setDeleteAudio] = useState(false);
  const [status, setStatus] = useState("");
  const [playbackMode, setPlaybackMode] = useState<"remote" | "local" | "both">("both");
  const [localVolume, setLocalVolume] = useState(1);
  const [remoteVolume, setRemoteVolume] = useState(1);
  const [isPlaying, setIsPlaying] = useState(false);
  const [activeSegmentIndex, setActiveSegmentIndex] = useState<number | null>(null);
  const [liveSegments, setLiveSegments] = useState<SessionSegment[]>([]);
  const [liveTexts, setLiveTexts] = useState<string[]>([]);
  const headRef = useRef<HTMLDivElement>(null);
  const [headHeight, setHeadHeight] = useState(0);
  const [playbackCurrentTime, setPlaybackCurrentTime] = useState(0);
  const [playbackDuration, setPlaybackDuration] = useState(0);
  const [languageSlots, setLanguageSlots] = useState<LanguageSlot[]>(DEFAULT_LANGUAGE_SLOTS);
  const [selectedLanguageId, setSelectedLanguageId] = useState("en");
  const [remoteSources, setRemoteSources] = useState<{ id: string; name: string; host: string; transcriptionPath?: string; apiKey?: string }[]>([]);
  const [modelRegistry, setModelRegistry] = useState<{ id: string; type: "integrated" | "remote"; sourceId?: string; modelName?: string }[]>([]);
  const [installedModels, setInstalledModels] = useState<string[] | null>(null);
  const audioRemoteRef = useRef<HTMLAudioElement | null>(null);
  const audioLocalRef = useRef<HTMLAudioElement | null>(null);
  const [retranscribeModelId, setRetranscribeModelId] = useState("");
  const [retranscribingIndex, setRetranscribingIndex] = useState<number | null>(null);
  const history = useRef<{ past: TranscriptData[]; future: TranscriptData[]; lastKey?: string }>({ past: [], future: [] });
  const [historySize, setHistorySize] = useState({ past: 0, future: 0 });
  // Set by edits and transcription so only real changes get auto-saved.
  const dirty = useRef(false);
  const sessionRef = useRef(session);
  sessionRef.current = session;

  const currentPathRef = useRef(currentProjectPath);
  currentPathRef.current = currentProjectPath;

  const loadProjects = useCallback(async () => {
    try {
      const store = await load("settings.json", { defaults: {}, autoSave: true });
      const days = (await store.get<number>("recent_retention_days")) ?? 10;
      const maxGb = (await store.get<number>("recent_max_gb")) ?? 20;
      setRetention({ days, maxGb });
      await invoke("purge_recent_command", {
        retentionDays: days,
        maxBytes: Math.round(maxGb * 1024 ** 3),
        keepPath: currentPathRef.current,
      });
      setProjects(await invoke<ProjectMeta[]>("list_projects_with_meta_command"));
    } catch (e) {
      console.error("Failed to list projects:", e);
    }
  }, []);

  useEffect(() => {
    loadProjects();
  }, [loadProjects]);

  useEffect(() => {
    (async () => {
      try {
        const store = await load("settings.json", { defaults: {}, autoSave: true });
        const pm = await store.get<string>("playback_mode");
        if (pm === "remote" || pm === "local" || pm === "both") setPlaybackMode(pm);
        const slots = (await store.get<LanguageSlot[]>("language_slots")) || DEFAULT_LANGUAGE_SLOTS;
        const selId = (await store.get<string>("selected_language_id")) || "en";
        setLanguageSlots(slots.length > 0 ? slots : DEFAULT_LANGUAGE_SLOTS);
        setSelectedLanguageId(slots.some((s) => s.id === selId) ? selId : slots[0]?.id || "en");
        const sources = (await store.get<{ id: string; name: string; host: string; transcriptionPath?: string; apiKey?: string }[]>("remote_sources")) || [];
        setRemoteSources(sources);
        setModelRegistry((await store.get<{ id: string; type: "integrated" | "remote"; sourceId?: string; modelName?: string }[]>("model_registry")) || []);
      } catch {
        /* ignore */
      }
    })();
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const store = await load("settings.json", { defaults: {}, autoSave: true });
        await store.set("playback_mode", playbackMode);
        await store.save();
      } catch {
        /* ignore */
      }
    })();
  }, [playbackMode]);

  useEffect(() => {
    (async () => {
      try {
        const store = await load("settings.json", { defaults: {}, autoSave: true });
        await store.set("selected_language_id", selectedLanguageId);
        await store.save();
      } catch {
        /* ignore */
      }
    })();
  }, [selectedLanguageId]);

  const selectedSlot = languageSlots.find((s) => s.id === selectedLanguageId) ?? languageSlots[0];

  function getLiveModelId() {
    return selectedSlot?.liveModel ?? "";
  }

  function getRegularModelId() {
    return selectedSlot?.regularModel ?? "";
  }

  // Re-read on language change, since models can be downloaded in Settings meanwhile.
  useEffect(() => {
    invoke<string[]>("list_installed_model_names_command").then(setInstalledModels).catch(() => setInstalledModels(null));
  }, [selectedLanguageId]);

  /** First local model the selected language needs that isn't downloaded yet. */
  const missingModel =
    installedModels == null
      ? null
      : ([getRegularModelId(), selectedSlot?.englishModel].find(
          (id) => id && getModelType(id) === "integrated" && !installedModels.includes(id),
        ) ?? null);

  function getLanguageCode(): string | null {
    return selectedSlot?.languageCode ?? null;
  }

  function getModelType(modelId: string): "integrated" | "remote" | null {
    return modelRegistry.find((m) => m.id === modelId)?.type ?? null;
  }

  function resolveRemoteConfig(modelId: string): { baseUrl: string; apiKey: string | null; modelName: string } | null {
    const entry = modelRegistry.find((m) => m.id === modelId);
    if (!entry || entry.type !== "remote" || !entry.sourceId || !entry.modelName) return null;
    const source = remoteSources.find((s) => s.id === entry.sourceId);
    if (!source?.host.trim()) return null;
    const host = source.host.trim().replace(/\/+$/, "");
    const path = (source.transcriptionPath?.trim() || "/v1/audio/transcriptions").replace(/^\/?/, "/");
    return {
      baseUrl: `${host}${path}`,
      apiKey: source.apiKey?.trim() || null,
      modelName: entry.modelName,
    };
  }

  // Save pending transcript edits now, before a new recording replaces the open session.
  async function flushPendingSave() {
    const latest = sessionRef.current;
    if (!dirty.current || !latest || !currentProjectPath) return;
    dirty.current = false;
    await invoke("save_project_command", { path: currentProjectPath, state: latest });
  }

  /** Returns whether recording started. */
  async function startRecording(liveRealtime = false): Promise<boolean> {
    if (liveRealtime) {
      const modelId = getLiveModelId();
      const modelType = getModelType(modelId);
      if (!modelType) {
        setStatus("Select a model in Settings for live transcription.");
        return false;
      }
      const remoteConfig = modelType === "remote" ? resolveRemoteConfig(modelId) : null;
      const useRemote = !!remoteConfig;
      if (!useRemote) {
        const path = await invoke<string | null>("resolve_model_path_command", { modelName: modelId });
        if (!path) {
          setStatus(`Download the ${modelId} model in Settings to use live transcription.`);
          return false;
        }
      }
      if (modelType === "remote" && !remoteConfig) {
        setStatus("Set up a remote API source in Settings to use live transcription.");
        return false;
      }
    }
    setStatus("Starting...");
    try {
      const [projectsPath, channelInfo] = await Promise.all([
        invoke<string>("get_projects_dir"),
        invoke<{ guild_name?: string; channel_name?: string } | null>("get_channel_info_command"),
      ]);
      const store = await load("settings.json", { defaults: {}, autoSave: true });
      const projectNameTemplate = (await store.get<string>("project_name_template")) || DEFAULT_TEMPLATE;
      const guild = channelInfo?.guild_name ?? null;
      const channel = channelInfo?.channel_name ?? null;
      const name = await invoke<string>("format_project_name_command", {
        template: projectNameTemplate,
        guild,
        channel,
      });
      const timestamp = Date.now();
      const safeName = name.replace(/[<>:"/\\|?*]/g, "_");
      const loopbackPath = `${projectsPath}/${safeName}_${timestamp}_loopback.wav`;
      const micPath = `${projectsPath}/${safeName}_${timestamp}_mic.wav`;

      const bufferMs = (await store.get<number>("segment_merge_buffer_ms")) ?? 1000;

      let liveModelPath: string | null = null;
      let liveTranscriptionMode: string = "integrated";
      let liveRemoteBaseUrl: string | null = null;
      let liveRemoteModel: string | null = null;
      let liveRemoteApiKey: string | null = null;

      if (liveRealtime) {
        const modelId = getLiveModelId();
        const modelType = getModelType(modelId);
        const remoteConfig = modelType === "remote" ? resolveRemoteConfig(modelId) : null;
        if (remoteConfig) {
          liveTranscriptionMode = "remote";
          liveRemoteBaseUrl = remoteConfig.baseUrl;
          liveRemoteModel = remoteConfig.modelName;
          liveRemoteApiKey = remoteConfig.apiKey;
        } else if (modelType === "integrated") {
          const path = await invoke<string | null>("resolve_model_path_command", { modelName: modelId });
          liveModelPath = path;
        }
      }

      const args: Record<string, unknown> = {
        outputPath: loopbackPath,
        micPath,
        segmentMergeBufferMs: bufferMs,
        projectNameTemplate: projectNameTemplate,
        liveRealtime,
        liveModelPath,
        liveTranscriptionMode,
        liveRemoteBaseUrl,
        liveRemoteModel,
        liveRemoteApiKey,
        liveLanguageCode: liveRealtime ? getLanguageCode() : null,
      };
      await flushPendingSave();
      await invoke("start_recording", args);
      setRecording(true);
      setSession(null);
      setCurrentProjectPath(null);
      setLiveSegments([]);
      setLiveTexts([]);
      setStatus(liveRealtime ? "Recording with live transcription. Text appears as people speak." : "Recording. Join a voice channel and speak.");
      return true;
    } catch (e) {
      setStatus(`Failed to start: ${e}`);
      return false;
    }
  }

  async function stopRecording() {
    setStatus("Stopping...");
    try {
      const state = await invoke<SessionState | null>("stop_recording");
      setRecording(false);
      setSession(state);
      if (state) {
        try {
          const path = await invoke<string>("auto_save_project_command", { state });
          setCurrentProjectPath(path);
          currentPathRef.current = path; // so loadProjects below never purges it
        } catch (e) {
          setStatus(`Auto-save failed: ${e}`);
        }
      }
      setStatus(state ? "Recording stopped. You can edit the transcript and export it." : "");
      loadProjects();
    } catch (e) {
      setStatus(`Failed to stop: ${e}`);
    }
  }

  // Auto-record: start when you are in a voice channel with someone else; stop when you leave
  // it or have been alone for AUTO_STOP_GRACE_MS. Only stops recordings it started. After a
  // manual Stop it waits until you leave that channel before starting again.
  const auto = useRef({ channelId: null as string | null, aloneSince: null as number | null, skipChannelId: null as string | null, busy: false });
  const recordingRef = useRef(recording);
  recordingRef.current = recording;
  const recordingActions = useRef({ startRecording, stopRecording });
  recordingActions.current = { startRecording, stopRecording };

  useEffect(() => {
    async function tick() {
      const a = auto.current;
      if (a.busy) return;
      a.busy = true;
      try {
        const info = await invoke<{ channel_id: string; channel_name?: string; member_count?: number } | null>(
          "get_channel_info_command"
        );
        const channelId = info?.channel_id ?? null;
        const withOthers = (info?.member_count ?? 0) >= 2;
        if (a.skipChannelId && a.skipChannelId !== channelId) a.skipChannelId = null;

        if (a.channelId) {
          if (!recordingRef.current) {
            a.skipChannelId = a.channelId;
            a.channelId = null;
          } else if (channelId !== a.channelId) {
            a.channelId = null;
            await recordingActions.current.stopRecording();
          } else if (withOthers) {
            a.aloneSince = null;
          } else if (a.aloneSince === null) {
            a.aloneSince = Date.now();
          } else if (Date.now() - a.aloneSince >= AUTO_STOP_GRACE_MS) {
            a.channelId = null;
            await recordingActions.current.stopRecording();
          }
          return;
        }

        if (recordingRef.current || !channelId || !withOthers || a.skipChannelId === channelId) return;
        const store = await load("settings.json", { defaults: {}, autoSave: true });
        if (!(await store.get<boolean>("auto_record"))) return;
        if (await recordingActions.current.startRecording(false)) {
          a.channelId = channelId;
          a.aloneSince = null;
          setStatus(`Recording started automatically in ${info?.channel_name || "the call"}.`);
        }
      } catch (e) {
        console.error("Auto-record check failed:", e);
      } finally {
        a.busy = false;
      }
    }
    const interval = setInterval(tick, 3000);
    return () => clearInterval(interval);
  }, []);

  async function backToRecordings() {
    try {
      await flushPendingSave();
    } catch (e) {
      setStatus(`Auto-save failed: ${e}`);
      return;
    }
    setSession(null);
    loadProjects();
  }

  /** Save keeps a recording until you delete it; unsaved ones are cleaned up after a while. */
  async function setSaved(path: string, saved: boolean) {
    try {
      if (path === currentProjectPath) await flushPendingSave();
      const newPath = await invoke<string>("set_project_saved_command", { path, saved });
      if (path === currentProjectPath) setCurrentProjectPath(newPath);
      loadProjects();
    } catch (e) {
      setStatus(`Could not ${saved ? "save" : "unsave"}: ${e}`);
    }
  }

  async function loadProject() {
    try {
      const path = await open({
        multiple: false,
        filters: [{ name: "Discord Scribe", extensions: ["json"] }],
      });
      if (path && typeof path === "string") {
        await loadProjectByPath(path);
      }
    } catch (e) {
      setStatus(`Load failed: ${e}`);
    }
  }

  async function confirmDelete() {
    if (!deleteTarget) return;
    try {
      await invoke("delete_project_command", { path: deleteTarget, deleteAudio });
      if (currentProjectPath === deleteTarget) {
        setSession(null);
        setCurrentProjectPath(null);
      }
      setDeleteTarget(null);
      setDeleteAudio(false);
      loadProjects();
      setStatus("Recording deleted.");
    } catch (e) {
      setStatus(`Delete failed: ${e}`);
    }
  }

  async function loadProjectByPath(path: string) {
    try {
      const state = await invoke<SessionState>("load_project_command", { path });
      setSession(state);
      setCurrentProjectPath(path);
      setStatus("");
    } catch (e) {
      setStatus(`Load failed: ${e}`);
    }
  }

  function transcriptData(state: SessionState): TranscriptData {
    return { segments: state.segments, texts: state.transcript_texts, liveTexts: state.live_transcript_texts };
  }

  function withTranscript(state: SessionState, data: TranscriptData): SessionState {
    return { ...state, segments: data.segments, transcript_texts: data.texts, live_transcript_texts: data.liveTexts };
  }

  function syncHistorySize() {
    const h = history.current;
    setHistorySize({ past: h.past.length, future: h.future.length });
  }

  function applyEdit(next: TranscriptData, coalesceKey?: string) {
    const session = sessionRef.current;
    if (!session) return;
    const h = history.current;
    if (!coalesceKey || coalesceKey !== h.lastKey) {
      h.past.push(transcriptData(session));
      if (h.past.length > 200) h.past.shift();
    }
    h.future = [];
    h.lastKey = coalesceKey;
    dirty.current = true;
    setSession(withTranscript(session, next));
    syncHistorySize();
  }

  function updateAiOutputs(change: (list: AiOutput[]) => AiOutput[]) {
    const latest = sessionRef.current;
    if (!latest) return;
    dirty.current = true;
    setSession({ ...latest, ai_outputs: change(latest.ai_outputs ?? []) });
  }

  function undo() {
    const h = history.current;
    const prev = h.past.pop();
    if (!session || !prev) return;
    h.future.push(transcriptData(session));
    h.lastKey = undefined;
    dirty.current = true;
    setSession(withTranscript(session, prev));
    syncHistorySize();
  }

  function redo() {
    const h = history.current;
    const next = h.future.pop();
    if (!session || !next) return;
    h.past.push(transcriptData(session));
    h.lastKey = undefined;
    dirty.current = true;
    setSession(withTranscript(session, next));
    syncHistorySize();
  }

  useEffect(() => {
    history.current = { past: [], future: [] };
    syncHistorySize();
    dirty.current = false;
  }, [session?.session_id]);

  // Edits are saved to the open project file shortly after they happen.
  useEffect(() => {
    if (!session || !currentProjectPath || !dirty.current) return;
    const timer = setTimeout(async () => {
      dirty.current = false;
      try {
        await invoke("save_project_command", { path: currentProjectPath, state: session });
      } catch (e) {
        setStatus(`Auto-save failed: ${e}`);
      }
    }, 800);
    return () => clearTimeout(timer);
  }, [session, currentProjectPath]);

  const [transcribing, setTranscribing] = useState(false);

  /** Runs transcription with `modelId`. Returns the new texts, or null if it couldn't start or failed. */
  async function runTranscription(
    modelId: string,
    onlyIndices?: number[],
    englishModelId?: string,
  ): Promise<string[] | null> {
    if (!session) return null;
    const modelType = getModelType(modelId);
    if (!modelType) {
      setStatus("Select a model in Settings for transcription.");
      return null;
    }
    const remoteConfig = modelType === "remote" ? resolveRemoteConfig(modelId) : null;
    if (modelType === "remote" && !remoteConfig) {
      setStatus("Set up a remote API source in Settings.");
      return null;
    }
    let modelPath: string | null = null;
    if (!remoteConfig) {
      modelPath = await invoke<string | null>("resolve_model_path_command", { modelName: modelId });
      if (!modelPath) {
        setStatus(`Download the ${modelId} model in Settings, or pick a remote model for this language.`);
        return null;
      }
    }
    let englishModelPath: string | null = null;
    if (!remoteConfig && englishModelId && getModelType(englishModelId) === "integrated") {
      englishModelPath = await invoke<string | null>("resolve_model_path_command", { modelName: englishModelId });
      if (!englishModelPath) {
        setStatus(`Download the ${englishModelId} model in Settings, or turn off English turns for this language.`);
        return null;
      }
    }
    try {
      const newState = await invoke<SessionState>("transcribe_session_command", {
        state: session,
        modelPath,
        englishModelPath,
        transcriptionMode: remoteConfig ? "remote" : "integrated",
        remoteBaseUrl: remoteConfig?.baseUrl ?? null,
        remoteModel: remoteConfig?.modelName ?? null,
        remoteApiKey: remoteConfig?.apiKey ?? null,
        languageCode: getLanguageCode(),
        onlyIndices: onlyIndices ?? null,
      });
      return newState.transcript_texts;
    } catch (e) {
      setStatus(`Transcription failed: ${e}`);
      return null;
    }
  }

  async function transcribeSession() {
    if (!session) return;
    setTranscribing(true);
    setStatus("");
    const texts = await runTask(
      "transcribe",
      "Loading the model",
      [
        {
          event: "transcribe-progress",
          map: (p) => ({ done: p.done, total: p.total, unit: "items", label: "Transcribing segments" }),
        },
        {
      event: "engine-download-progress",
      map: (p) => ({ ...bytesProgress(p), label: "Downloading the Whisper engine" }),
    },
      ],
      () => runTranscription(getRegularModelId(), undefined, selectedSlot?.englishModel),
    );
    setTranscribing(false);
    if (texts) {
      applyEdit({ ...transcriptData(session), texts });
      setStatus("Transcription complete.");
    }
  }

  async function retranscribeSegment(index: number) {
    if (!session) return;
    setRetranscribingIndex(index);
    const texts = await runTranscription(retranscribeModelId || getRegularModelId(), [index]);
    setRetranscribingIndex(null);
    const latest = sessionRef.current;
    if (texts && latest) applyEdit(setText(transcriptData(latest), index, texts[index] ?? ""));
  }

  async function downloadModel(modelName: string) {
    setStatus("");
    try {
      const path = await runTask(
        `model:${modelName}`,
        `Downloading ${modelName}`,
        [{ event: "download-progress", map: (p) => (p.modelName === modelName ? bytesProgress(p) : null) }],
        () => invoke<string>("download_model_command", { modelName }),
      );
      setStatus(`Model downloaded to ${path}`);
      setInstalledModels(await invoke<string[]>("list_installed_model_names_command"));
    } catch (e) {
      const msg = String(e);
      setStatus(msg.includes("coming soon") ? "large-v3-turbo: coming soon" : `Download failed: ${e}`);
    }
  }

  async function exportTranscript(format: "srt" | "vtt") {
    if (!session) return;
    try {
      const path = await save({
        defaultPath: `${session.session_id}.${format}`,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (path) {
        await invoke("export_transcript", {
          path,
          format,
          segments: session.segments,
          texts: session.transcript_texts.length >= session.segments.length
            ? session.transcript_texts
            : session.segments.map((_, i) => session.transcript_texts[i] ?? ""),
        });
        setStatus(`Exported to ${format.toUpperCase()}.`);
      }
    } catch (e) {
      setStatus(`Export failed: ${e}`);
    }
  }

  const segments = recording ? liveSegments : (session?.segments ?? []);
  const texts = recording ? liveTexts : (session?.transcript_texts ?? []);

  const playAudio = useCallback(
    (modeOverride?: "remote" | "local" | "both") => {
      const mode = modeOverride ?? playbackMode;
      const remotePath = session?.audio_paths?.loopback;
      const localPath = session?.audio_paths?.microphone;

      const playOne = (audio: HTMLAudioElement | null, path: string | null | undefined, vol: number) => {
        if (!audio || !path) return;
        loadSource(audio, path);
        audio.volume = vol;
        audio.play().catch(() => {});
      };

      try {
        if (mode === "both") {
          playOne(audioRemoteRef.current, remotePath, remoteVolume);
          playOne(audioLocalRef.current, localPath, localVolume);
          if (remotePath || localPath) setIsPlaying(true);
        } else {
          const path = mode === "remote" ? remotePath : localPath;
          const audio = mode === "remote" ? audioRemoteRef.current : audioLocalRef.current;
          const vol = mode === "remote" ? remoteVolume : localVolume;
          if (!path || !audio) return;
          loadSource(audio, path);
          audio.volume = vol;
          audio.play().then(() => setIsPlaying(true)).catch(() => setIsPlaying(false));
        }
      } catch {
        setIsPlaying(false);
      }
    },
    [session?.audio_paths, playbackMode, remoteVolume, localVolume]
  );

  const seekTo = useCallback(
    (index: number) => {
      const seg = segments[index];
      if (!seg) return;
      setActiveSegmentIndex(index);
      const seconds = seg.start_ms / 1000;
      const sources: [HTMLAudioElement | null, string | null | undefined][] = [
        [audioRemoteRef.current, session?.audio_paths?.loopback],
        [audioLocalRef.current, session?.audio_paths?.microphone],
      ];
      for (const [audio, path] of sources) {
        if (audio && path) {
          loadSource(audio, path);
          audio.currentTime = seconds;
        }
      }
      playAudio();
    },
    [segments, session?.audio_paths, playAudio]
  );

  const seekToTime = useCallback(
    (seconds: number) => {
      const sources: [HTMLAudioElement | null, string | null | undefined][] = [
        [audioRemoteRef.current, session?.audio_paths?.loopback],
        [audioLocalRef.current, session?.audio_paths?.microphone],
      ];
      for (const [audio, path] of sources) {
        if (audio && path) {
          loadSource(audio, path);
          audio.currentTime = seconds;
        }
      }
      setPlaybackCurrentTime(seconds);
      const ms = seconds * 1000;
      const index = segments.findIndex((seg) => seg.start_ms <= ms && ms < seg.end_ms);
      if (index >= 0) setActiveSegmentIndex(index);
    },
    [segments, session?.audio_paths],
  );

  // Load the audio up front so the seek slider knows the length before the first Play.
  useEffect(() => {
    if (audioRemoteRef.current && session?.audio_paths?.loopback) loadSource(audioRemoteRef.current, session.audio_paths.loopback);
    if (audioLocalRef.current && session?.audio_paths?.microphone) loadSource(audioLocalRef.current, session.audio_paths.microphone);
  }, [session?.audio_paths?.loopback, session?.audio_paths?.microphone]);

  // The transcript toolbar sticks just under the session header, so it needs its height.
  useEffect(() => {
    const el = headRef.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setHeadHeight(el.offsetHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, [session != null, recording]);

  const pauseAudio = useCallback(() => {
    audioRemoteRef.current?.pause();
    audioLocalRef.current?.pause();
    setIsPlaying(false);
  }, []);

  const handleTimeUpdate = useCallback(() => {
    const audio = playbackMode === "local" ? audioLocalRef.current : audioRemoteRef.current;
    if (!audio) return;
    const ms = audio.currentTime * 1000;
    setPlaybackCurrentTime(audio.currentTime);
    setPlaybackDuration(audio.duration);
    if (segments.length > 0) {
      const idx = segments.findIndex((s) => s.start_ms <= ms && ms < s.end_ms);
      if (idx !== -1) {
        setActiveSegmentIndex(idx);
      }
    }
  }, [segments, playbackMode]);

  const handleLoadedMetadata = useCallback(() => {
    const audio = playbackMode === "local" ? audioLocalRef.current : audioRemoteRef.current;
    if (audio) setPlaybackDuration(audio.duration);
  }, [playbackMode]);

  const setPlaybackModeAndPlay = useCallback(
    (mode: "remote" | "local" | "both") => {
      if (mode === playbackMode) return;
      if (isPlaying) {
        pauseAudio();
        setPlaybackMode(mode);
        requestAnimationFrame(() => playAudio(mode));
      } else {
        setPlaybackMode(mode);
      }
    },
    [playbackMode, isPlaying, pauseAudio, playAudio]
  );

  useEffect(() => {
    setActiveSegmentIndex(null);
    setIsPlaying(false);
    setPlaybackCurrentTime(0);
    setPlaybackDuration(0);
  }, [session?.session_id]);

  useEffect(() => {
    if (!isPlaying) return;
    if (audioRemoteRef.current && (playbackMode === "remote" || playbackMode === "both")) {
      audioRemoteRef.current.volume = remoteVolume;
    }
    if (audioLocalRef.current && (playbackMode === "local" || playbackMode === "both")) {
      audioLocalRef.current.volume = localVolume;
    }
  }, [remoteVolume, localVolume, playbackMode, isPlaying]);

  useEffect(() => {
    if (!recording) return;
    const unlisten = listen<{ segment: SessionSegment; text: string; index: number }>(
      "transcript-segment",
      (evt) => {
        const { segment, text, index } = evt.payload;
        setLiveSegments((prev) => {
          const next = [...prev];
          while (next.length <= index) next.push(segment);
          next[index] = segment;
          return next;
        });
        setLiveTexts((prev) => {
          const next = [...prev];
          while (next.length <= index) next.push("");
          next[index] = text;
          return next;
        });
      }
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [recording]);

  const isSaved = !!currentProjectPath && projects.some((p) => p.path === currentProjectPath && p.saved);

  const speakers: Speaker[] = (() => {
    const map = new Map<string, string>();
    for (const [id, name] of Object.entries(session?.user_labels ?? {})) map.set(id, name);
    for (const seg of segments) if (!map.has(seg.user_id)) map.set(seg.user_id, seg.speaker_name ?? seg.user_id);
    return [...map].map(([id, name]) => ({ id, name }));
  })();

  const canTranscribe = !(() => {
    const modelType = getModelType(getRegularModelId());
    return modelType === "remote" && !resolveRemoteConfig(getRegularModelId());
  })();

  return (
    <div className="session" style={{ ["--session-head-h" as string]: `${headHeight}px` }}>
      {!recording && !session && <h2>Recordings</h2>}

      {!recording && !session && (
        <div className="session-idle">
          <p>Connect to Discord in Settings, join a voice channel, then start recording.</p>
          <div className="form-group">
            <label htmlFor="language-select">Language</label>
            <select
              id="language-select"
              value={selectedLanguageId}
              onChange={(e) => setSelectedLanguageId(e.target.value)}
            >
              {languageSlots.map((slot) => (
                <option key={slot.id} value={slot.id}>
                  {slot.label}
                </option>
              ))}
            </select>
          </div>
          <div className="button-row">
            <button type="button" onClick={() => startRecording(false)}>
              Start recording
            </button>
            <button
              type="button"
              onClick={() => startRecording(true)}
              disabled={
                (() => {
                  const modelId = getLiveModelId();
                  const modelType = getModelType(modelId);
                  return modelType === "remote" && !resolveRemoteConfig(modelId);
                })()
              }
              title="Transcribe while recording. Pick a live model in Settings first."
            >
              Start live recording
            </button>
            <button type="button" onClick={loadProject}>
              Open file
            </button>
          </div>
          <p className="field-hint" style={{ marginTop: "0.5rem" }}>
            Live recording transcribes each segment as it ends. Set up a live model or remote API in Settings first.
          </p>
          <RecordingsOverview
            projects={projects}
            retentionDays={retention.days}
            maxGb={retention.maxGb}
            onOpen={loadProjectByPath}
            onSetSaved={setSaved}
            onDelete={(path) => {
              setDeleteTarget(path);
              setDeleteAudio(false);
            }}
          />
        </div>
      )}

      {(session || recording) && (
        <div className="session-head" ref={headRef}>
          <div className="session-head-row">
            {recording ? (
              <>
                <span className="recording-indicator">● Recording</span>
                <span className="session-title">Live transcript</span>
                <div className="session-head-actions">
                  <button type="button" className="stop-btn" onClick={stopRecording}>
                    Stop recording
                  </button>
                </div>
              </>
            ) : (
              session && (
                <>
                  <button type="button" className="link-button" onClick={backToRecordings}>
                    Recordings
                  </button>
                  <span className="session-title" title={session.session_id}>
                    {placeLabel({ guild_name: session.guild_name, channel_name: session.channel_name, channel_type: session.channel_type ?? null })}
                    <span className="session-date">{new Date(session.created_at * 1000).toLocaleString()}</span>
                    {session.live_mode_enabled && <span className="meta-badge">Live</span>}
                  </span>
                  <div className="session-head-actions">
                    <select
                      value={selectedLanguageId}
                      onChange={(e) => setSelectedLanguageId(e.target.value)}
                      aria-label="Language"
                      title="Language to transcribe in"
                    >
                      {languageSlots.map((slot) => (
                        <option key={slot.id} value={slot.id}>
                          {slot.label}
                        </option>
                      ))}
                    </select>
                    {missingModel ? (
                      <button type="button" onClick={() => downloadModel(missingModel)} disabled={transcribing}>
                        Download {missingModel}
                      </button>
                    ) : (
                      <button type="button" onClick={transcribeSession} disabled={transcribing || !canTranscribe}>
                        {transcribing ? "Transcribing..." : "Transcribe"}
                      </button>
                    )}
                    {currentProjectPath && (
                      <button
                        type="button"
                        className={isSaved ? "active" : ""}
                        onClick={() => setSaved(currentProjectPath, !isSaved)}
                        title={isSaved ? "Saved. Click to unsave, so it gets deleted like other unsaved recordings." : "Keep this recording until you delete it"}
                      >
                        {isSaved ? "Saved" : "Save"}
                      </button>
                    )}
                    <Menu label="More" title="Export and delete">
                      {(close) => (
                        <>
                          <button type="button" onClick={() => { close(); exportTranscript("srt"); }}>
                            Export SRT subtitles
                          </button>
                          <button type="button" onClick={() => { close(); exportTranscript("vtt"); }}>
                            Export VTT subtitles
                          </button>
                          {currentProjectPath && (
                            <>
                              <div className="menu-separator" />
                              <button
                                type="button"
                                className="menu-danger"
                                onClick={() => {
                                  close();
                                  setDeleteTarget(currentProjectPath);
                                  setDeleteAudio(false);
                                }}
                              >
                                Delete recording
                              </button>
                            </>
                          )}
                        </>
                      )}
                    </Menu>
                  </div>
                </>
              )
            )}
          </div>
          {session && !recording && (
            <PlaybackBar
              isPlaying={isPlaying}
              onPlay={() => playAudio()}
              onPause={pauseAudio}
              onSeekTime={seekToTime}
              playbackMode={playbackMode}
              setPlaybackModeAndPlay={setPlaybackModeAndPlay}
              remoteVolume={remoteVolume}
              setRemoteVolume={setRemoteVolume}
              localVolume={localVolume}
              setLocalVolume={setLocalVolume}
              currentTime={playbackCurrentTime}
              duration={playbackDuration}
              hasAudio={!!(session.audio_paths?.loopback || session.audio_paths?.microphone)}
            />
          )}
          {status && <p className="session-status">{status}</p>}
        </div>
      )}

      {session && (
        <>
          <audio
            ref={audioRemoteRef}
            preload="metadata"
            onTimeUpdate={handleTimeUpdate}
            onLoadedMetadata={handleLoadedMetadata}
            onEnded={() => {
              if (playbackMode === "both") pauseAudio();
              else setIsPlaying(false);
            }}
          />
          <audio
            ref={audioLocalRef}
            preload="metadata"
            onTimeUpdate={handleTimeUpdate}
            onLoadedMetadata={handleLoadedMetadata}
            onEnded={() => {
              if (playbackMode === "both") pauseAudio();
              else setIsPlaying(false);
            }}
          />
        </>
      )}

      {(session || recording) && (
        <Transcript
          data={recording || !session ? { segments: liveSegments, texts: liveTexts } : transcriptData(session)}
          activeIndex={activeSegmentIndex}
          editable={!recording && !!session}
          speakers={speakers}
          onSeek={seekTo}
          onEdit={applyEdit}
          canUndo={historySize.past > 0}
          canRedo={historySize.future > 0}
          onUndo={undo}
          onRedo={redo}
          models={modelRegistry.map((m) => ({ id: m.id, label: m.id }))}
          retranscribeModelId={retranscribeModelId || getRegularModelId()}
          onRetranscribeModelChange={setRetranscribeModelId}
          onRetranscribe={retranscribeSegment}
          retranscribingIndex={retranscribingIndex}
          extraViews={
            session && !recording
              ? [
                  { id: "stats", label: "Stats", render: () => <StatsPanel segments={segments} texts={texts} collapsed={false} /> },
                  {
                    id: "ai",
                    label: "AI",
                    render: () => (
                      <AiPanel
                        key={session.session_id}
                        data={transcriptData(session)}
                        outputs={session.ai_outputs ?? []}
                        onAdd={(output) => updateAiOutputs((list) => [...list, output])}
                        onDelete={(index) => updateAiOutputs((list) => list.filter((_, i) => i !== index))}
                      />
                    ),
                  },
                ]
              : undefined
          }
        />
      )}

      {deleteTarget && (
        <div className="delete-modal-overlay" onClick={() => setDeleteTarget(null)}>
          <div className="delete-modal" onClick={(e) => e.stopPropagation()}>
            <h4>Delete recording?</h4>
            <label className="delete-audio-checkbox">
              <input
                type="checkbox"
                checked={deleteAudio}
                onChange={(e) => setDeleteAudio(e.target.checked)}
              />
              Also delete the audio files (loopback and microphone WAV)
            </label>
            <div className="delete-modal-actions">
              <button type="button" onClick={() => setDeleteTarget(null)}>
                Cancel
              </button>
              <button type="button" className="delete-confirm-btn" onClick={confirmDelete}>
                Delete
              </button>
            </div>
          </div>
        </div>
      )}

      {status && !session && !recording && <p className="status">{status}</p>}
    </div>
  );
}


/** Point an audio element at a file, keeping its position if it already plays that file. */
function loadSource(audio: HTMLAudioElement, path: string) {
  if (audio.dataset.path === path) return;
  audio.src = convertFileSrc(path);
  audio.dataset.path = path;
}

import { useState, useEffect, useCallback, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { load } from "@tauri-apps/plugin-store";
import { StatsPanel } from "./StatsPanel";
import { PlaybackBar } from "./PlaybackBar";
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
}

interface ProjectMeta {
  name: string;
  path: string;
  guild_name: string | null;
  channel_name: string | null;
  created_at: number;
}

const DEFAULT_TEMPLATE = "{guild}_{channel}_{timestamp}";

interface LanguageSlot {
  id: string;
  label: string;
  languageCode: string;
  liveModel: string;
  regularModel: string;
}

const DEFAULT_LANGUAGE_SLOTS: LanguageSlot[] = [
  { id: "en", label: "English", languageCode: "en", liveModel: "base.en", regularModel: "medium.en" },
  { id: "multilingual", label: "Multilingual", languageCode: "auto", liveModel: "tiny", regularModel: "large-v3" },
];

export function Session() {
  const [recording, setRecording] = useState(false);
  const [session, setSession] = useState<SessionState | null>(null);
  const [projects, setProjects] = useState<ProjectMeta[]>([]);
  const [projectsDir, setProjectsDir] = useState("");
  const [currentProjectPath, setCurrentProjectPath] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [deleteAudio, setDeleteAudio] = useState(false);
  const [projectNameTemplate, setProjectNameTemplate] = useState(DEFAULT_TEMPLATE);
  const [status, setStatus] = useState("");
  const [playbackMode, setPlaybackMode] = useState<"remote" | "local" | "both">("both");
  const [localVolume, setLocalVolume] = useState(1);
  const [remoteVolume, setRemoteVolume] = useState(1);
  const [isPlaying, setIsPlaying] = useState(false);
  const [activeSegmentIndex, setActiveSegmentIndex] = useState<number | null>(null);
  const [liveSegments, setLiveSegments] = useState<SessionSegment[]>([]);
  const [liveTexts, setLiveTexts] = useState<string[]>([]);
  const [statsCollapsed, setStatsCollapsed] = useState(false);
  const [playbackCurrentTime, setPlaybackCurrentTime] = useState(0);
  const [playbackDuration, setPlaybackDuration] = useState(0);
  const [languageSlots, setLanguageSlots] = useState<LanguageSlot[]>(DEFAULT_LANGUAGE_SLOTS);
  const [selectedLanguageId, setSelectedLanguageId] = useState("en");
  const [remoteSources, setRemoteSources] = useState<{ id: string; name: string; host: string; transcriptionPath?: string; apiKey?: string }[]>([]);
  const [modelRegistry, setModelRegistry] = useState<{ id: string; type: "integrated" | "remote"; sourceId?: string; modelName?: string }[]>([]);
  const audioRemoteRef = useRef<HTMLAudioElement | null>(null);
  const audioLocalRef = useRef<HTMLAudioElement | null>(null);
  const segmentRefs = useRef<(HTMLDivElement | null)[]>([]);

  const loadProjects = useCallback(async () => {
    try {
      const store = await load("settings.json", { defaults: {}, autoSave: true });
      const retention = (await store.get<number>("recent_retention_days")) ?? 10;
      await invoke("purge_recent_command", { retentionDays: retention });
      const [list, dir] = await Promise.all([
        invoke<ProjectMeta[]>("list_projects_with_meta_command"),
        invoke<string>("get_projects_dir"),
      ]);
      setProjects(list);
      setProjectsDir(dir);
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
        const tpl = await store.get<string>("project_name_template");
        if (tpl) setProjectNameTemplate(tpl);
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

  async function startRecording(liveRealtime = false) {
    if (liveRealtime) {
      const modelId = getLiveModelId();
      const modelType = getModelType(modelId);
      if (!modelType) {
        setStatus("Select a model in Settings for live transcription.");
        return;
      }
      const remoteConfig = modelType === "remote" ? resolveRemoteConfig(modelId) : null;
      const useRemote = !!remoteConfig;
      if (!useRemote) {
        const path = await invoke<string | null>("resolve_model_path_command", { modelName: modelId });
        if (!path) {
          setStatus(`Download ${modelId} model in Settings for live transcription.`);
          return;
        }
      }
      if (modelType === "remote" && !remoteConfig) {
        setStatus("Configure remote API source in Settings for live transcription.");
        return;
      }
    }
    setStatus("Starting...");
    try {
      const [projectsPath, channelInfo] = await Promise.all([
        invoke<string>("get_projects_dir"),
        invoke<{ guild_name?: string; channel_name?: string } | null>("get_channel_info_command"),
      ]);
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

      const store = await load("settings.json", { defaults: {}, autoSave: true });
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
      await invoke("start_recording", args);
      setRecording(true);
      setSession(null);
      setCurrentProjectPath(null);
      setLiveSegments([]);
      setLiveTexts([]);
      setStatus(liveRealtime ? "Recording (live)... Transcriptions will appear as you speak." : "Recording... Join a voice channel and speak.");
    } catch (e) {
      setStatus(`Failed to start: ${e}`);
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
          await invoke("auto_save_project_command", { state });
        } catch {
          /* ignore auto-save failure */
        }
      }
      setStatus(state ? "Recording stopped. Edit transcript and export." : "");
      loadProjects();
    } catch (e) {
      setStatus(`Failed to stop: ${e}`);
    }
  }

  async function saveProject() {
    if (!session) return;
    try {
      const path = await save({
        defaultPath: `${projectsDir}/${session.session_id}.json`,
        filters: [{ name: "Discord Scribe", extensions: ["json"] }],
      });
      if (path) {
        await invoke("save_project_command", { path, state: session });
        const prevPath = currentProjectPath;
        setCurrentProjectPath(path);
        if (prevPath && prevPath.includes("recent")) {
          try {
            await invoke("delete_project_command", { path: prevPath, deleteAudio: false });
          } catch {
            /* ignore */
          }
        }
        setStatus("Project saved.");
        loadProjects();
      }
    } catch (e) {
      setStatus(`Save failed: ${e}`);
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
      setStatus("Session deleted.");
    } catch (e) {
      setStatus(`Delete failed: ${e}`);
    }
  }

  async function loadProjectByPath(path: string) {
    try {
      const state = await invoke<SessionState>("load_project_command", { path });
      setSession(state);
      setCurrentProjectPath(path);
      setStatus("Project loaded.");
    } catch (e) {
      setStatus(`Load failed: ${e}`);
    }
  }

  function updateSegmentText(index: number, text: string) {
    if (!session) return;
    const texts = [...session.transcript_texts];
    while (texts.length <= index) texts.push("");
    texts[index] = text;
    setSession({ ...session, transcript_texts: texts });
  }

  const [transcribing, setTranscribing] = useState(false);

  async function transcribeSession() {
    if (!session) return;
    const modelId = getRegularModelId();
    const modelType = getModelType(modelId);
    if (!modelType) {
      setStatus("Select a model in Settings for transcription.");
      return;
    }
    const remoteConfig = modelType === "remote" ? resolveRemoteConfig(modelId) : null;
    const useRemote = !!remoteConfig;
    let modelPath: string | null = null;
    if (!useRemote) {
      modelPath = await invoke<string | null>("resolve_model_path_command", { modelName: modelId });
      if (!modelPath) {
        setStatus(`Download ${modelId} model in Settings, or use a Remote model for this slot.`);
        return;
      }
    }
    if (modelType === "remote" && !remoteConfig) {
      setStatus("Configure remote API source in Settings.");
      return;
    }
    setTranscribing(true);
    setStatus("Transcribing...");
    try {
      const newState = await invoke<SessionState>("transcribe_session_command", {
        state: session,
        modelPath: useRemote ? null : modelPath,
        transcriptionMode: useRemote ? "remote" : "integrated",
        remoteBaseUrl: useRemote ? remoteConfig!.baseUrl : null,
        remoteModel: useRemote ? remoteConfig!.modelName : null,
        remoteApiKey: useRemote ? remoteConfig!.apiKey : null,
        languageCode: getLanguageCode(),
      });
      setSession(newState);
      setStatus("Transcription complete.");
    } catch (e) {
      setStatus(`Transcription failed: ${e}`);
    } finally {
      setTranscribing(false);
    }
  }

  async function downloadModel(modelName: string) {
    setStatus("Downloading model...");
    try {
      const path = await invoke<string>("download_model_command", { modelName });
      setStatus(`Model downloaded to ${path}`);
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
        audio.src = convertFileSrc(path);
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
          audio.src = convertFileSrc(path);
          audio.volume = vol;
          audio.play().then(() => setIsPlaying(true)).catch(() => setIsPlaying(false));
        }
      } catch {
        setIsPlaying(false);
      }
    },
    [session?.audio_paths, playbackMode, remoteVolume, localVolume]
  );

  const pauseAudio = useCallback(() => {
    audioRemoteRef.current?.pause();
    audioLocalRef.current?.pause();
    setIsPlaying(false);
  }, []);

  const stopAudio = useCallback(() => {
    audioRemoteRef.current?.pause();
    audioLocalRef.current?.pause();
    if (audioRemoteRef.current) audioRemoteRef.current.currentTime = 0;
    if (audioLocalRef.current) audioLocalRef.current.currentTime = 0;
    setIsPlaying(false);
    setActiveSegmentIndex(null);
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
        setActiveSegmentIndex((prev) => {
          if (prev !== idx) {
            segmentRefs.current[idx]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
            return idx;
          }
          return prev;
        });
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
    segmentRefs.current = segmentRefs.current.slice(0, segments.length);
  }, [segments.length]);

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

  return (
    <div className="session">
      <h2>Session</h2>

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
          <div className="form-group">
            <label htmlFor="project-template">Project name template</label>
            <input
              id="project-template"
              type="text"
              value={projectNameTemplate}
              onChange={(e) => setProjectNameTemplate(e.target.value)}
              placeholder={DEFAULT_TEMPLATE}
            />
            <span className="field-hint">
              Placeholders: {"{guild}"}, {"{channel}"}, {"{timestamp}"}, {"{date}"}, {"{time}"}
            </span>
          </div>
          {projects.length > 0 && (
            <div className="project-list">
              <h4>Projects</h4>
              <ul className="project-list-items">
                {projects.map((p) => (
                  <li key={p.path} className="project-list-item">
                    <button
                      type="button"
                      className="project-item"
                      onClick={() => loadProjectByPath(p.path)}
                    >
                      <span className="project-name">{p.name}</span>
                      <span className="project-meta">
                        {[p.guild_name, p.channel_name].filter(Boolean).join(" / ") || "—"}
                        {p.created_at > 0 && (
                          <> · {new Date(p.created_at * 1000).toLocaleDateString()}</>
                        )}
                      </span>
                    </button>
                    <button
                      type="button"
                      className="project-delete-btn"
                      onClick={(e) => {
                        e.stopPropagation();
                        setDeleteTarget(p.path);
                        setDeleteAudio(false);
                      }}
                      title="Delete session"
                    >
                      🗑
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          <div className="button-row">
            <button type="button" onClick={() => startRecording(false)}>
              Start Recording
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
              title="Transcribe in real time as you speak. Configure model in Settings first."
            >
              Start Recording (Live)
            </button>
            <button type="button" onClick={loadProject}>
              Open Project
            </button>
          </div>
          <p className="field-hint" style={{ marginTop: "0.5rem" }}>
            Live = real-time transcription as you speak. Configure model or remote API in Settings first.
          </p>
        </div>
      )}

      {recording && (
        <div className="session-recording">
          <p className="recording-indicator">● Recording</p>
          <button type="button" className="stop-btn" onClick={stopRecording}>
            Stop Recording
          </button>
        </div>
      )}

      {(session || recording) && (
        <div className="session-transcript">
          <div className="session-two-column">
            {session && (
              <StatsPanel
                segments={segments}
                texts={texts}
                collapsed={statsCollapsed}
                onToggleCollapsed={() => setStatsCollapsed(!statsCollapsed)}
              />
            )}
            <div className="transcript-scroll-column">
          <div className="transcript-meta">
            <span>
              {recording
                ? "Live recording"
                : `${session?.guild_name ?? "Unknown"} / ${session?.channel_name ?? "Unknown"}`}
              {session?.channel_type != null && (
                <span className="meta-badge">
                  {session.channel_type === 1 ? "DM" : session.channel_type === 2 ? "Server" : session.channel_type === 3 ? "Group DM" : ""}
                </span>
              )}
              {session?.live_mode_enabled && (
                <span className="meta-badge">Live</span>
              )}
            </span>
          </div>

          {session && (
          <PlaybackBar
            isPlaying={isPlaying}
            onPlay={() => playAudio()}
            onPause={pauseAudio}
            onStop={stopAudio}
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

          {session && (
            <>
              <audio
                ref={audioRemoteRef}
                onTimeUpdate={handleTimeUpdate}
                onLoadedMetadata={handleLoadedMetadata}
                onEnded={() => {
                  if (playbackMode === "both") pauseAudio();
                  else setIsPlaying(false);
                }}
              />
              <audio
                ref={audioLocalRef}
                onTimeUpdate={handleTimeUpdate}
                onLoadedMetadata={handleLoadedMetadata}
                onEnded={() => {
                  if (playbackMode === "both") pauseAudio();
                  else setIsPlaying(false);
                }}
              />
            </>
          )}
          <div className="segments-list scrollable">
            {segments.map((seg, i) => (
              <div
                key={i}
                ref={(el) => {
                  segmentRefs.current[i] = el;
                }}
                className={`segment ${activeSegmentIndex === i ? "active" : ""}`}
              >
                <div className="segment-header">
                  <span className="speaker">{seg.speaker_name ?? seg.user_id}</span>
                  <span className="time">
                    {formatMs(seg.start_ms)} → {formatMs(seg.end_ms)}
                  </span>
                </div>
                <input
                  type="text"
                  className="segment-text"
                  value={texts[i] ?? ""}
                  onChange={(e) => updateSegmentText(i, e.target.value)}
                  placeholder="Transcription..."
                  readOnly={recording}
                />
              </div>
            ))}
          </div>
            </div>
          </div>

          {session && (
            <>
          <div className="transcribe-section">
            <select
              value={selectedLanguageId}
              onChange={(e) => setSelectedLanguageId(e.target.value)}
              className="language-select"
            >
              {languageSlots.map((slot) => (
                <option key={slot.id} value={slot.id}>
                  {slot.label}
                </option>
              ))}
            </select>
            {getModelType(getRegularModelId()) === "integrated" && (
              <button
                type="button"
                onClick={() => downloadModel(getRegularModelId())}
                disabled={transcribing}
              >
                Download {getRegularModelId()}
              </button>
            )}
            <button
              type="button"
              onClick={transcribeSession}
              disabled={
                transcribing ||
                (() => {
                  const modelId = getRegularModelId();
                  const modelType = getModelType(modelId);
                  return modelType === "remote" && !resolveRemoteConfig(modelId);
                })()
              }
            >
              {transcribing ? "Transcribing..." : "Transcribe"}
            </button>
          </div>

          <div className="session-actions">
            <button type="button" onClick={saveProject}>
              Save Project
            </button>
            <button type="button" onClick={() => setSession(null)}>
              New Session
            </button>
            {currentProjectPath && (
              <button
                type="button"
                className="delete-btn"
                onClick={() => {
                  setDeleteTarget(currentProjectPath);
                  setDeleteAudio(false);
                }}
              >
                Delete
              </button>
            )}
            <button type="button" onClick={() => exportTranscript("srt")}>
              Export SRT
            </button>
            <button type="button" onClick={() => exportTranscript("vtt")}>
              Export VTT
            </button>
          </div>
            </>
          )}
        </div>
      )}

      {deleteTarget && (
        <div className="delete-modal-overlay" onClick={() => setDeleteTarget(null)}>
          <div className="delete-modal" onClick={(e) => e.stopPropagation()}>
            <h4>Delete session?</h4>
            <label className="delete-audio-checkbox">
              <input
                type="checkbox"
                checked={deleteAudio}
                onChange={(e) => setDeleteAudio(e.target.checked)}
              />
              Also delete associated audio files (loopback + microphone WAV)
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

      {status && <p className="status">{status}</p>}
    </div>
  );
}

function formatMs(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  const ss = s % 60;
  const mm = m % 60;
  if (h > 0) return `${h}:${mm.toString().padStart(2, "0")}:${ss.toString().padStart(2, "0")}`;
  return `${mm}:${ss.toString().padStart(2, "0")}`;
}

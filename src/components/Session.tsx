import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { load } from "@tauri-apps/plugin-store";
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
  channel_name: string | null;
  channel_id: string | null;
  segments: SessionSegment[];
  transcript_texts: string[];
  audio_paths: { loopback: string | null; microphone: string | null };
}

const DEFAULT_TEMPLATE = "{guild}_{channel}_{timestamp}";

export function Session() {
  const [recording, setRecording] = useState(false);
  const [session, setSession] = useState<SessionState | null>(null);
  const [projects, setProjects] = useState<string[]>([]);
  const [projectsDir, setProjectsDir] = useState("");
  const [projectNameTemplate, setProjectNameTemplate] = useState(DEFAULT_TEMPLATE);
  const [status, setStatus] = useState("");
  const [playbackMode, setPlaybackMode] = useState<"toggle" | "stereo">("toggle");
  const [localVolume, setLocalVolume] = useState(1);
  const [remoteVolume, setRemoteVolume] = useState(1);
  const [playingChannel, setPlayingChannel] = useState<"local" | "remote">("remote");

  const loadProjects = useCallback(async () => {
    try {
      const [list, dir] = await Promise.all([
        invoke<string[]>("list_projects_command"),
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
        const models = await invoke<string[]>("list_models_command");
        if (models.length > 0) {
          setModelPath(models[0]);
        }
      } catch {
        /* ignore */
      }
    })();
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const store = await load("settings.json", { defaults: {}, autoSave: true });
        const tpl = await store.get<string>("project_name_template");
        if (tpl) setProjectNameTemplate(tpl);
      } catch {
        /* ignore */
      }
    })();
  }, []);

  async function startRecording() {
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

      await invoke("start_recording", {
        outputPath: loopbackPath,
        micPath,
        segmentMergeBufferMs: bufferMs,
      });
      setRecording(true);
      setSession(null);
      setStatus("Recording... Join a voice channel and speak.");
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
        const state = await invoke<SessionState>("load_project_command", { path });
        setSession(state);
        setStatus("Project loaded.");
      }
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
  const [modelPath, setModelPath] = useState("");
  const [modelName, setModelName] = useState("base.en");

  async function transcribeSession() {
    if (!session) return;
    if (!modelPath) {
      setStatus("Download a model first (Settings) or set model path.");
      return;
    }
    setTranscribing(true);
    setStatus("Transcribing...");
    try {
      const newState = await invoke<SessionState>("transcribe_session_command", {
        state: session,
        modelPath,
      });
      setSession(newState);
      setStatus("Transcription complete.");
    } catch (e) {
      setStatus(`Transcription failed: ${e}`);
    } finally {
      setTranscribing(false);
    }
  }

  async function downloadModel() {
    setStatus("Downloading model...");
    try {
      const path = await invoke<string>("download_model_command", { modelName });
      setModelPath(path);
      setStatus(`Model downloaded to ${path}`);
    } catch (e) {
      setStatus(`Download failed: ${e}`);
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

  const texts = session?.transcript_texts ?? [];
  const segments = session?.segments ?? [];

  return (
    <div className="session">
      <h2>Session</h2>

      {!recording && !session && (
        <div className="session-idle">
          <p>Connect to Discord in Settings, join a voice channel, then start recording.</p>
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
            <p className="recent-hint">Recent: {projects.slice(-5).reverse().join(", ")}</p>
          )}
          <div className="button-row">
            <button type="button" onClick={startRecording}>
              Start Recording
            </button>
            <button type="button" onClick={loadProject}>
              Open Project
            </button>
          </div>
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

      {session && !recording && (
        <div className="session-transcript">
          <div className="transcript-meta">
            <span>
              {session.guild_name ?? "Unknown"} / {session.channel_name ?? "Unknown"}
            </span>
          </div>

          <div className="playback-controls">
            <div className="playback-mode">
              <label>Playback:</label>
              <button
                type="button"
                className={playbackMode === "toggle" ? "active" : ""}
                onClick={() => setPlaybackMode("toggle")}
              >
                Toggle (Local/Remote)
              </button>
              <button
                type="button"
                className={playbackMode === "stereo" ? "active" : ""}
                onClick={() => setPlaybackMode("stereo")}
              >
                Stereo (L=Remote, R=Local)
              </button>
            </div>
            {playbackMode === "toggle" && (
              <div className="channel-toggle">
                <button
                  type="button"
                  className={playingChannel === "remote" ? "active" : ""}
                  onClick={() => setPlayingChannel("remote")}
                >
                  Remote
                </button>
                <button
                  type="button"
                  className={playingChannel === "local" ? "active" : ""}
                  onClick={() => setPlayingChannel("local")}
                >
                  Local
                </button>
              </div>
            )}
            <div className="volume-sliders">
              <label>
                Remote: <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.1"
                  value={remoteVolume}
                  onChange={(e) => setRemoteVolume(parseFloat(e.target.value))}
                />
              </label>
              <label>
                Local: <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.1"
                  value={localVolume}
                  onChange={(e) => setLocalVolume(parseFloat(e.target.value))}
                />
              </label>
            </div>
          </div>

          <div className="segments-list">
            {segments.map((seg, i) => (
              <div key={i} className="segment">
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
                />
              </div>
            ))}
          </div>

          <div className="transcribe-section">
            <label>Transcription: </label>
            <select
              value={modelName}
              onChange={(e) => setModelName(e.target.value)}
              disabled={transcribing}
            >
              <option value="tiny.en">tiny.en</option>
              <option value="base.en">base.en</option>
              <option value="small.en">small.en</option>
            </select>
            <button
              type="button"
              onClick={downloadModel}
              disabled={transcribing}
            >
              Download Model
            </button>
            <button
              type="button"
              onClick={transcribeSession}
              disabled={transcribing || !modelPath}
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
            <button type="button" onClick={() => exportTranscript("srt")}>
              Export SRT
            </button>
            <button type="button" onClick={() => exportTranscript("vtt")}>
              Export VTT
            </button>
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

import { useState, useEffect } from "react";
import { load } from "@tauri-apps/plugin-store";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./Settings.css";

const STORE_PATH = "settings.json";


export function Settings() {
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [rpcOrigin, setRpcOrigin] = useState("https://localhost");
  const [segmentMergeBufferMs, setSegmentMergeBufferMs] = useState(1000);
  const [recentRetentionDays, setRecentRetentionDays] = useState(10);
  const [transcriptionMode, setTranscriptionMode] = useState<"integrated" | "remote">("integrated");
  const [remoteBaseUrl, setRemoteBaseUrl] = useState("");
  const [remoteModel, setRemoteModel] = useState("");
  const [remoteApiKey, setRemoteApiKey] = useState("");
  const [showInstructions, setShowInstructions] = useState(false);
  const [status, setStatus] = useState("");
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    loadSettings();
  }, []);

  async function loadSettings() {
    try {
      const store = await load(STORE_PATH, { defaults: {}, autoSave: true });
      const cid = await store.get<string>("client_id");
      const secret = await store.get<string>("client_secret");
      const origin = await store.get<string>("rpc_origin");
      const buffer = await store.get<number>("segment_merge_buffer_ms");
      const retention = await store.get<number>("recent_retention_days");
      setClientId(cid || "");
      setClientSecret(secret || "");
      setRpcOrigin(origin || "https://localhost");
      setSegmentMergeBufferMs(buffer ?? 1000);
      setRecentRetentionDays(retention ?? 10);
      const mode = await store.get<string>("transcription_mode");
      setTranscriptionMode(mode === "remote" ? "remote" : "integrated");
      setRemoteBaseUrl((await store.get<string>("remote_base_url")) || "");
      setRemoteModel((await store.get<string>("remote_model")) || "");
      setRemoteApiKey((await store.get<string>("remote_api_key")) || "");
    } catch (e) {
      console.error("Failed to load settings:", e);
    }
  }

  async function saveSettings() {
    try {
      const store = await load(STORE_PATH, { defaults: {}, autoSave: true });
      await store.set("client_id", clientId);
      await store.set("client_secret", clientSecret);
      await store.set("rpc_origin", rpcOrigin);
      await store.set("segment_merge_buffer_ms", segmentMergeBufferMs);
      await store.set("recent_retention_days", recentRetentionDays);
      await store.set("transcription_mode", transcriptionMode);
      await store.set("remote_base_url", remoteBaseUrl);
      await store.set("remote_model", remoteModel);
      await store.set("remote_api_key", remoteApiKey);
      await store.save();
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    }
  }

  async function connectDiscord() {
    setStatus("Connecting...");
    try {
      await invoke("discord_rpc_connect", {
        clientId,
        clientSecret,
        rpcOrigin,
      });
      setStatus("Connected! Join a voice channel to start.");
    } catch (e) {
      setStatus(`Connection failed: ${e}`);
    }
  }

  return (
    <div className="settings">
      <h2>Settings</h2>

      <section className="settings-section">
        <h3>Discord RPC Credentials</h3>
        <p className="settings-hint">
          Enter your Discord application credentials. Each user needs their own
          Discord app.
        </p>

        <button
          type="button"
          className="instructions-toggle"
          onClick={() => setShowInstructions(!showInstructions)}
        >
          {showInstructions ? "Hide" : "Show"} instructions for getting Client ID
        </button>

        {showInstructions && (
          <div className="instructions">
            <ol>
              <li>
                Go to{" "}
                <button
                  type="button"
                  className="link-button"
                  onClick={() => openUrl("https://discord.com/developers/applications")}
                >
                  Discord Developer Portal
                </button>
              </li>
              <li>Click &quot;New Application&quot; and name it (e.g. &quot;d-scribe&quot; — avoid &quot;Discord&quot; in the name)</li>
              <li>
                In the app, open &quot;OAuth2&quot; → copy the <strong>Client ID</strong>
              </li>
              <li>
                In &quot;OAuth2&quot; → &quot;Client Secret&quot; → &quot;Reset Secret&quot; if needed, then copy{" "}
                <strong>Client Secret</strong>
              </li>
              <li>
                <strong>OAuth2 Redirects</strong> (separate from RPC Origin): In &quot;OAuth2&quot; → &quot;Redirects&quot;, add <code>https://localhost</code> (must match exactly). This is used for token exchange.
              </li>
              <li>
                <strong>RPC Origin</strong> (Windows uses IPC, no Origin needed): On Windows, this app uses Discord&apos;s IPC (named pipes) which is officially supported and does not require RPC Origin. On other platforms, add <code>https://localhost</code> to RPC Origins if you get &quot;Invalid Origin&quot;.
              </li>
              <li>
                If you get &quot;Invalid Origin&quot;, your app needs <code>https://localhost</code> in RPC Origins. If you don&apos;t see an RPC Origin field, your app may not have RPC access.
              </li>
              <li>
                You do not need to open the OAuth URL in a browser — authorization happens in a Discord popup when you click Connect.
              </li>
            </ol>
          </div>
        )}

        <div className="form-group">
          <label htmlFor="client-id">Client ID</label>
          <input
            id="client-id"
            type="text"
            value={clientId}
            onChange={(e) => setClientId(e.target.value)}
            placeholder="Your Discord app Client ID"
          />
        </div>
        <div className="form-group">
          <label htmlFor="client-secret">Client Secret</label>
          <input
            id="client-secret"
            type="password"
            value={clientSecret}
            onChange={(e) => setClientSecret(e.target.value)}
            placeholder="Your Discord app Client Secret"
          />
        </div>
        <div className="form-group">
          <label htmlFor="rpc-origin">RPC Origin</label>
          <input
            id="rpc-origin"
            type="text"
            value={rpcOrigin}
            onChange={(e) => setRpcOrigin(e.target.value)}
            placeholder="https://localhost"
          />
          <span className="field-hint">
            Must match the origin configured in your Discord app
          </span>
        </div>
        <div className="form-group">
          <label htmlFor="segment-buffer">Segment merge buffer (ms)</label>
          <input
            id="segment-buffer"
            type="number"
            min="100"
            max="5000"
            step="100"
            value={segmentMergeBufferMs}
            onChange={(e) => setSegmentMergeBufferMs(parseInt(e.target.value, 10) || 1000)}
          />
          <span className="field-hint">
            Min silence before splitting segments (default 1000ms). Brief pauses are merged.
          </span>
        </div>
        <div className="form-group">
          <label htmlFor="recent-retention">Recent sessions retention (days)</label>
          <input
            id="recent-retention"
            type="number"
            min="1"
            max="365"
            value={recentRetentionDays}
            onChange={(e) => setRecentRetentionDays(parseInt(e.target.value, 10) || 10)}
          />
          <span className="field-hint">
            Auto-saved sessions older than this are purged (default 10).
          </span>
        </div>

        <div className="button-row">
          <button type="button" onClick={saveSettings}>
            {saved ? "Saved!" : "Save"}
          </button>
          <button type="button" onClick={connectDiscord}>
            Connect to Discord
          </button>
        </div>
        {status && <p className="status">{status}</p>}
      </section>

      <section className="settings-section">
        <h3>Transcription</h3>
        <div className="form-group">
          <label>Transcription mode</label>
          <div className="radio-group">
            <label>
              <input
                type="radio"
                name="transcription-mode"
                checked={transcriptionMode === "integrated"}
                onChange={() => setTranscriptionMode("integrated")}
              />
              Integrated (Whisper)
            </label>
            <label>
              <input
                type="radio"
                name="transcription-mode"
                checked={transcriptionMode === "remote"}
                onChange={() => setTranscriptionMode("remote")}
              />
              Remote (OpenAI-compatible API)
            </label>
          </div>
        </div>
        {transcriptionMode === "remote" && (
          <>
            <div className="form-group">
              <label htmlFor="remote-base-url">API endpoint URL</label>
              <input
                id="remote-base-url"
                type="text"
                value={remoteBaseUrl}
                onChange={(e) => setRemoteBaseUrl(e.target.value)}
                placeholder="http://localhost:8000/v1/audio/transcriptions"
              />
              <span className="field-hint">
                Full endpoint URL, e.g. http://localhost:8000/v1/audio/transcriptions
              </span>
            </div>
            <div className="form-group">
              <label htmlFor="remote-model">Model</label>
              <input
                id="remote-model"
                type="text"
                value={remoteModel}
                onChange={(e) => setRemoteModel(e.target.value)}
                placeholder="whisper-1 or mistralai/Voxtral-Small-24B-2507"
              />
            </div>
            <div className="form-group">
              <label htmlFor="remote-api-key">API Key (optional)</label>
              <input
                id="remote-api-key"
                type="password"
                value={remoteApiKey}
                onChange={(e) => setRemoteApiKey(e.target.value)}
                placeholder="For authenticated endpoints"
              />
            </div>
          </>
        )}
      </section>
    </div>
  );
}

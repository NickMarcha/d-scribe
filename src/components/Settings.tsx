import { useState, useEffect } from "react";
import { load } from "@tauri-apps/plugin-store";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import "./Settings.css";

const STORE_PATH = "settings.json";

const WHISPER_MODELS = [
  "tiny.en", "base.en", "small.en", "medium.en",
  "tiny", "base", "small", "medium", "large-v3", "large-v3-turbo",
  "nb-whisper-tiny", "nb-whisper-base", "nb-whisper-small", "nb-whisper-medium", "nb-whisper-large",
];

interface LanguageSlot {
  id: string;
  label: string;
  languageCode: string;
  liveModel: string;
  regularModel: string;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

const DEFAULT_LANGUAGE_SLOTS: LanguageSlot[] = [
  { id: "en", label: "English", languageCode: "en", liveModel: "base.en", regularModel: "medium.en" },
  { id: "multilingual", label: "Multilingual", languageCode: "auto", liveModel: "tiny", regularModel: "large-v3" },
];

const DEFAULT_REGISTRY = [
  { id: "base.en", type: "integrated" as const },
  { id: "medium.en", type: "integrated" as const },
  { id: "tiny", type: "integrated" as const },
  { id: "large-v3", type: "integrated" as const },
];

interface RemoteSource {
  id: string;
  name: string;
  host: string;
  transcriptionPath?: string;
  modelsPath?: string;
  apiKey?: string;
}

interface RegistryModel {
  id: string;
  type: "integrated" | "remote";
  sourceId?: string;
  modelName?: string;
}

export function Settings() {
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [rpcOrigin, setRpcOrigin] = useState("https://localhost");
  const [segmentMergeBufferMs, setSegmentMergeBufferMs] = useState(1000);
  const [recentRetentionDays, setRecentRetentionDays] = useState(10);
  const [remoteSources, setRemoteSources] = useState<RemoteSource[]>([]);
  const [showInstructions, setShowInstructions] = useState(false);
  const [status, setStatus] = useState("");
  const [saved, setSaved] = useState(false);

  const [modelRegistry, setModelRegistry] = useState<RegistryModel[]>([]);
  const [installedModels, setInstalledModels] = useState<string[]>([]);
  const [languageSlots, setLanguageSlots] = useState<LanguageSlot[]>(DEFAULT_LANGUAGE_SLOTS);

  const [addIntegratedModel, setAddIntegratedModel] = useState("base.en");
  const [addRemoteSourceId, setAddRemoteSourceId] = useState("");
  const [addRemoteModel, setAddRemoteModel] = useState("");
  const [fetchedModels, setFetchedModels] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [downloadingModel, setDownloadingModel] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<{ bytes: number; total: number | null } | null>(null);
  const [modelsDir, setModelsDir] = useState<string | null>(null);

  const [recordingExpanded, setRecordingExpanded] = useState(false);
  const [discordExpanded, setDiscordExpanded] = useState(false);
  const [manageModelsExpanded, setManageModelsExpanded] = useState(false);
  const [selectedModelsExpanded, setSelectedModelsExpanded] = useState(false);
  const [addLanguagePreset, setAddLanguagePreset] = useState("");

  useEffect(() => {
    loadSettings();
  }, []);

  useEffect(() => {
    invoke<string>("get_models_dir").then(setModelsDir).catch(() => setModelsDir(null));
  }, []);

  useEffect(() => {
    const unlisten = listen<{ modelName: string; bytesDownloaded: number; totalBytes: number | null }>(
      "download-progress",
      (evt) => {
        if (downloadingModel === evt.payload.modelName) {
          setDownloadProgress({
            bytes: evt.payload.bytesDownloaded,
            total: evt.payload.totalBytes ?? null,
          });
        }
      }
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [downloadingModel]);

  async function refreshInstalledModels() {
    try {
      const names = await invoke<string[]>("list_installed_model_names_command");
      setInstalledModels(names);
    } catch {
      setInstalledModels([]);
    }
  }

  function isModelReady(m: RegistryModel): boolean {
    if (m.type === "integrated") {
      return installedModels.includes(m.id);
    }
    if (m.type === "remote" && m.sourceId && m.modelName) {
      return remoteSources.some((s) => s.id === m.sourceId && s.host.trim());
    }
    return false;
  }

  const readyModels = modelRegistry.filter(isModelReady);

  useEffect(() => {
    const ready = modelRegistry.filter((m) => isModelReady(m));
    const first = ready[0]?.id;
    if (!first) return;
    const valid = (id: string) => ready.some((m) => m.id === id);
    setLanguageSlots((prev) =>
      prev.map((slot) => ({
        ...slot,
        liveModel: slot.liveModel && valid(slot.liveModel) ? slot.liveModel : first,
        regularModel: slot.regularModel && valid(slot.regularModel) ? slot.regularModel : first,
      }))
    );
  }, [modelRegistry, installedModels, remoteSources]);

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

      const sources = (await store.get<RemoteSource[]>("remote_sources")) || [];
      setRemoteSources(sources);

      let registry = (await store.get<RegistryModel[]>("model_registry")) || [];
      let slots = (await store.get<LanguageSlot[]>("language_slots")) || null;

      if (registry.length === 0) {
        registry = [...DEFAULT_REGISTRY];
        await store.set("model_registry", registry);
        await store.save();
      }
      if (!slots || slots.length === 0) {
        slots = [...DEFAULT_LANGUAGE_SLOTS];
        await store.set("language_slots", slots);
        await store.save();
      }

      setModelRegistry(registry);
      setLanguageSlots(slots);

      if (addRemoteSourceId === "" && sources.length > 0) {
        setAddRemoteSourceId(sources[0].id);
      }

      await refreshInstalledModels();
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
      await store.set("remote_sources", remoteSources);
      await store.set("model_registry", modelRegistry);
      await store.set("language_slots", languageSlots);
      await store.save();
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    }
  }

  function addModelToRegistry(model: RegistryModel) {
    if (modelRegistry.some((m) => m.id === model.id)) return;
    setModelRegistry((prev) => [...prev, model]);
  }

  function removeModelFromRegistry(id: string) {
    const remaining = modelRegistry.filter((m) => m.id !== id);
    setModelRegistry(remaining);
    const firstReady = remaining.find(isModelReady)?.id;
    const fallback = firstReady || remaining[0]?.id || "";
    setLanguageSlots((prev) =>
      prev.map((slot) => ({
        ...slot,
        liveModel: slot.liveModel === id ? fallback : slot.liveModel,
        regularModel: slot.regularModel === id ? fallback : slot.regularModel,
      }))
    );
  }

  async function downloadIntegratedModel(modelName: string) {
    setDownloadingModel(modelName);
    setDownloadProgress({ bytes: 0, total: null });
    setStatus(`Downloading ${modelName}...`);
    try {
      await invoke<string>("download_model_command", { modelName });
      await refreshInstalledModels();
      setStatus(`Downloaded ${modelName}`);
    } catch (e) {
      const msg = String(e);
      setStatus(msg.includes("coming soon") ? "large-v3-turbo: coming soon" : `Download failed: ${msg}`);
    } finally {
      setDownloadingModel(null);
      setDownloadProgress(null);
    }
  }

  async function openModelsDir() {
    try {
      const dir = await invoke<string>("get_models_dir");
      await revealItemInDir(dir);
    } catch (e) {
      setStatus(`Failed to open folder: ${e}`);
    }
  }

  async function fetchRemoteModels() {
    const source = remoteSources.find((s) => s.id === addRemoteSourceId);
    if (!source?.host.trim()) {
      setStatus("Select a source with a host URL first.");
      return;
    }
    setFetchingModels(true);
    setStatus("Fetching models...");
    try {
      const models = await invoke<string[]>("list_remote_models_command", {
        host: source.host.trim(),
        modelsPath: source.modelsPath?.trim() || null,
        apiKey: source.apiKey?.trim() || null,
      });
      setFetchedModels(models);
      setStatus(models.length > 0 ? `Found ${models.length} models.` : "No models in response.");
    } catch (e) {
      setFetchedModels([]);
      setStatus(`Could not fetch models. Enter model name manually.`);
    } finally {
      setFetchingModels(false);
    }
  }

  function addRemoteModelToRegistry() {
    const modelName = addRemoteModel.trim();
    if (!modelName || !addRemoteSourceId) return;
    const sourceId = addRemoteSourceId;
    const id = `${sourceId}:${modelName}`;
    if (modelRegistry.some((m) => m.id === id)) return;
    addModelToRegistry({ id, type: "remote", sourceId, modelName });
    setAddRemoteModel("");
  }

  function addRemoteSource() {
    const id = `src_${Date.now()}`;
    setRemoteSources((prev) => [...prev, { id, name: "New source", host: "", apiKey: "" }]);
    setAddRemoteSourceId(id);
  }

  function updateRemoteSource(id: string, updates: Partial<RemoteSource>) {
    setRemoteSources((prev) =>
      prev.map((s) => (s.id === id ? { ...s, ...updates } : s))
    );
  }

  function removeRemoteSource(id: string) {
    const remainingSources = remoteSources.filter((s) => s.id !== id);
    setRemoteSources(remainingSources);
    const remaining = modelRegistry.filter((m) => m.sourceId !== id);
    const firstReady = remaining.find((r) =>
      r.type === "integrated" ? installedModels.includes(r.id) : remainingSources.some((s) => s.id === r.sourceId)
    )?.id;
    const fallback = firstReady || remaining[0]?.id || "";
    setModelRegistry(remaining);
    setLanguageSlots((prev) =>
      prev.map((slot) => ({
        ...slot,
        liveModel: slot.liveModel.startsWith(`${id}:`) ? fallback : slot.liveModel,
        regularModel: slot.regularModel.startsWith(`${id}:`) ? fallback : slot.regularModel,
      }))
    );
    if (addRemoteSourceId === id) {
      setAddRemoteSourceId(remainingSources[0]?.id || "");
    }
  }

  function updateLanguageSlot(slotId: string, updates: Partial<LanguageSlot>) {
    setLanguageSlots((prev) =>
      prev.map((s) => (s.id === slotId ? { ...s, ...updates } : s))
    );
  }

  function removeLanguageSlot(slotId: string) {
    setLanguageSlots((prev) => prev.filter((s) => s.id !== slotId));
  }

  function addLanguageSlot(slot: LanguageSlot) {
    if (languageSlots.some((s) => s.id === slot.id)) {
      setLanguageSlots((prev) => [...prev, { ...slot, id: `${slot.id}_${Date.now()}` }]);
    } else {
      setLanguageSlots((prev) => [...prev, slot]);
    }
  }

  function restoreDefaultLanguageSlots() {
    setLanguageSlots([...DEFAULT_LANGUAGE_SLOTS]);
  }

  function displayModelLabel(m: RegistryModel): string {
    if (m.type === "integrated") return m.id;
    const sourceName = remoteSources.find((s) => s.id === m.sourceId)?.name || m.sourceId || "?";
    return `${m.modelName || m.id} (${sourceName})`;
  }

  const integratedNotInRegistry = WHISPER_MODELS.filter((m) => !modelRegistry.some((r) => r.id === m));
  const allIntegratedAdded = integratedNotInRegistry.length === 0;

  useEffect(() => {
    if (allIntegratedAdded) {
      setAddIntegratedModel("");
    } else if (!integratedNotInRegistry.includes(addIntegratedModel)) {
      setAddIntegratedModel(integratedNotInRegistry[0] || "");
    }
  }, [allIntegratedAdded, integratedNotInRegistry, addIntegratedModel]);

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

  function CollapsibleHeader({
    title,
    expanded,
    onToggle,
  }: {
    title: string;
    expanded: boolean;
    onToggle: () => void;
  }) {
    return (
      <button
        type="button"
        className="collapsible-header"
        onClick={onToggle}
      >
        <span className="collapsible-chevron">{expanded ? "▼" : "▶"}</span>
        <span>{title}</span>
      </button>
    );
  }

  return (
    <div className="settings">
      <h2>Settings</h2>

      <div className="settings-sticky-bar">
        <button type="button" className="save-all-btn" onClick={saveSettings}>
          {saved ? "Saved!" : "Save all settings"}
        </button>
      </div>

      <div className="settings-sections">
        <section className="settings-section collapsible">
          <CollapsibleHeader
            title="Recording"
            expanded={recordingExpanded}
            onToggle={() => setRecordingExpanded(!recordingExpanded)}
          />
          {recordingExpanded && (
            <div className="collapsible-content">
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
            </div>
          )}
        </section>

        <section className="settings-section collapsible">
          <CollapsibleHeader
            title="Discord"
            expanded={discordExpanded}
            onToggle={() => setDiscordExpanded(!discordExpanded)}
          />
          {discordExpanded && (
            <div className="collapsible-content">
              <p className="settings-hint">
                Enter your Discord application credentials. Each user needs their own Discord app.
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
                    <li>Click &quot;New Application&quot; and name it (e.g. &quot;d-scribe&quot;)</li>
                    <li>In the app, open &quot;OAuth2&quot; → copy the <strong>Client ID</strong></li>
                    <li>In &quot;OAuth2&quot; → &quot;Client Secret&quot; → copy <strong>Client Secret</strong></li>
                    <li>Add <code>https://localhost</code> to OAuth2 Redirects and RPC Origins if needed.</li>
                    <li>Authorization happens in a Discord popup when you click Connect.</li>
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
              </div>
              <div className="button-row">
                <button type="button" onClick={connectDiscord}>
                  Connect to Discord
                </button>
              </div>
            </div>
          )}
        </section>

        <section className="settings-section collapsible">
          <CollapsibleHeader
            title="Manage Models"
            expanded={manageModelsExpanded}
            onToggle={() => setManageModelsExpanded(!manageModelsExpanded)}
          />
          {manageModelsExpanded && (
            <div className="collapsible-content">
              <h4 className="settings-subsection">Remote Sources</h4>
              <p className="field-hint" style={{ marginBottom: "0.75rem" }}>
                Add API sources for remote transcription. Each source has host, optional path overrides, and API key.
              </p>
              {remoteSources.map((src) => (
                <div key={src.id} className="remote-source-card">
                  <div className="form-group">
                    <label>Name</label>
                    <input
                      type="text"
                      value={src.name}
                      onChange={(e) => updateRemoteSource(src.id, { name: e.target.value })}
                      placeholder="e.g. OpenAI"
                    />
                  </div>
                  <div className="form-group">
                    <label>Host</label>
                    <input
                      type="text"
                      value={src.host}
                      onChange={(e) => updateRemoteSource(src.id, { host: e.target.value })}
                      placeholder="http://localhost:8000"
                    />
                  </div>
                  <div className="form-group">
                    <label>Transcription path (optional)</label>
                    <input
                      type="text"
                      value={src.transcriptionPath || ""}
                      onChange={(e) => updateRemoteSource(src.id, { transcriptionPath: e.target.value || undefined })}
                      placeholder="/v1/audio/transcriptions"
                    />
                  </div>
                  <div className="form-group">
                    <label>Models path (optional, for Fetch)</label>
                    <input
                      type="text"
                      value={src.modelsPath || ""}
                      onChange={(e) => updateRemoteSource(src.id, { modelsPath: e.target.value || undefined })}
                      placeholder="/v1/models"
                    />
                  </div>
                  <div className="form-group">
                    <label>API Key (optional)</label>
                    <input
                      type="password"
                      value={src.apiKey || ""}
                      onChange={(e) => updateRemoteSource(src.id, { apiKey: e.target.value || undefined })}
                      placeholder="Bearer token"
                    />
                  </div>
                  <button
                    type="button"
                    className="model-remove-btn"
                    onClick={() => removeRemoteSource(src.id)}
                  >
                    Remove source
                  </button>
                </div>
              ))}
              <button type="button" className="add-source-btn" onClick={addRemoteSource}>
                Add source
              </button>

              <h4 className="settings-subsection">Installed Models</h4>
              <p className="field-hint" style={{ marginBottom: "0.75rem" }}>
                Integrated = Whisper (local). Remote = API (uses a source above).
              </p>
              {modelsDir && (
                <div className="models-dir-row">
                  <button type="button" className="open-folder-btn" onClick={openModelsDir}>
                    Open models folder
                  </button>
                </div>
              )}
              <div className="model-registry-list">
                {modelRegistry.map((m) => (
                  <div key={m.id} className="model-registry-item">
                    <span className="model-registry-id">{displayModelLabel(m)}</span>
                    <span className={`model-registry-badge model-registry-badge--${m.type}`}>
                      {m.type === "integrated" ? "Integrated" : "Remote"}
                    </span>
                    {m.type === "integrated" && (
                      <span className="model-registry-status">
                        {installedModels.includes(m.id) ? (
                          <span className="model-status-installed">Installed</span>
                        ) : (
                          <>
                            <span className="model-status-missing">Not installed</span>
                            <button
                              type="button"
                              className="model-download-btn"
                              onClick={() => downloadIntegratedModel(m.id)}
                              disabled={downloadingModel !== null}
                            >
                              {downloadingModel === m.id ? "Downloading..." : "Download"}
                            </button>
                            {downloadingModel === m.id && downloadProgress && (
                              <div className="download-progress-wrap">
                                <div className="download-progress-bar">
                                  <div
                                    className="download-progress-fill"
                                    style={{
                                      width: downloadProgress.total
                                        ? `${Math.min(100, (downloadProgress.bytes / downloadProgress.total) * 100)}%`
                                        : "50%",
                                      animation: downloadProgress.total ? undefined : "download-indeterminate 1.2s ease-in-out infinite",
                                    }}
                                  />
                                </div>
                                <span className="download-progress-text">
                                  {downloadProgress.total
                                    ? `${formatBytes(downloadProgress.bytes)} / ${formatBytes(downloadProgress.total)}`
                                    : `${formatBytes(downloadProgress.bytes)}`}
                                </span>
                              </div>
                            )}
                          </>
                        )}
                      </span>
                    )}
                    {m.type === "remote" && (
                      <span className="model-registry-status model-status-remote">Available</span>
                    )}
                    <button
                      type="button"
                      className="model-remove-btn"
                      onClick={() => removeModelFromRegistry(m.id)}
                      title="Remove from list"
                    >
                      Remove
                    </button>
                  </div>
                ))}
              </div>

              <div className="model-add-area">
                <div className="model-add-row">
                  <label>Add Integrated (Whisper):</label>
                  <select
                    value={addIntegratedModel}
                    onChange={(e) => setAddIntegratedModel(e.target.value)}
                  >
                    {integratedNotInRegistry.map((m) => (
                      <option key={m} value={m}>{m}</option>
                    ))}
                    {allIntegratedAdded && <option value="">All added</option>}
                  </select>
                  <button
                    type="button"
                    onClick={() => {
                      if (!modelRegistry.some((r) => r.id === addIntegratedModel)) {
                        addModelToRegistry({ id: addIntegratedModel, type: "integrated" });
                      }
                      downloadIntegratedModel(addIntegratedModel);
                    }}
                    disabled={downloadingModel !== null || allIntegratedAdded}
                  >
                    {modelRegistry.some((r) => r.id === addIntegratedModel)
                      ? "Download"
                      : "Download & Add"}
                  </button>
                  {allIntegratedAdded && (
                    <span className="field-hint">All Whisper models are already in your list.</span>
                  )}
                </div>
                <div className="model-add-row">
                  <label>Add Remote (API):</label>
                  <select
                    value={addRemoteSourceId}
                    onChange={(e) => setAddRemoteSourceId(e.target.value)}
                    disabled={remoteSources.length === 0}
                  >
                    {remoteSources.map((s) => (
                      <option key={s.id} value={s.id}>{s.name || s.id}</option>
                    ))}
                    {remoteSources.length === 0 && <option value="">Add a source first</option>}
                  </select>
                  <button
                    type="button"
                    onClick={fetchRemoteModels}
                    disabled={fetchingModels || remoteSources.length === 0}
                  >
                    {fetchingModels ? "Fetching..." : "Fetch models"}
                  </button>
                  <input
                    type="text"
                    list="models-datalist"
                    value={addRemoteModel}
                    onChange={(e) => setAddRemoteModel(e.target.value)}
                    placeholder="whisper-1 or type custom"
                    className="model-input"
                  />
                  <datalist id="models-datalist">
                    {fetchedModels.map((mid) => (
                      <option key={mid} value={mid} />
                    ))}
                  </datalist>
                  <button
                    type="button"
                    onClick={addRemoteModelToRegistry}
                    disabled={
                      !addRemoteModel.trim() ||
                      !addRemoteSourceId ||
                      modelRegistry.some((r) => r.id === `${addRemoteSourceId}:${addRemoteModel.trim()}`)
                    }
                  >
                    Add
                  </button>
                </div>
              </div>
            </div>
          )}
        </section>

        <section className="settings-section collapsible">
          <CollapsibleHeader
            title="Selected Models"
            expanded={selectedModelsExpanded}
            onToggle={() => setSelectedModelsExpanded(!selectedModelsExpanded)}
          />
          {selectedModelsExpanded && (
            <div className="collapsible-content">
              <p className="field-hint" style={{ marginBottom: "0.75rem" }}>
                Select which model to use for each language. Only ready models (installed or with valid source) are listed.
              </p>
              {readyModels.length === 0 && (
                <p className="field-hint" style={{ color: "#c00" }}>
                  No models ready. Add and install models above.
                </p>
              )}
              <div className="language-slots-table-wrap">
                <table className="language-slots-table">
                  <thead>
                    <tr>
                      <th>Language</th>
                      <th>Live</th>
                      <th>Regular</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {languageSlots.map((slot) => (
                      <tr key={slot.id}>
                        <td>{slot.label}</td>
                        <td>
                          <select
                            value={slot.liveModel}
                            onChange={(e) => updateLanguageSlot(slot.id, { liveModel: e.target.value })}
                          >
                            {readyModels.map((m) => (
                              <option key={m.id} value={m.id}>{displayModelLabel(m)}</option>
                            ))}
                            {readyModels.length === 0 && <option value="">Select model</option>}
                          </select>
                        </td>
                        <td>
                          <select
                            value={slot.regularModel}
                            onChange={(e) => updateLanguageSlot(slot.id, { regularModel: e.target.value })}
                          >
                            {readyModels.map((m) => (
                              <option key={m.id} value={m.id}>{displayModelLabel(m)}</option>
                            ))}
                            {readyModels.length === 0 && <option value="">Select model</option>}
                          </select>
                        </td>
                        <td>
                          <button
                            type="button"
                            className="remove-slot-btn"
                            onClick={() => removeLanguageSlot(slot.id)}
                            title="Remove language"
                          >
                            Remove
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <div className="language-slots-actions">
                <select
                  value={addLanguagePreset}
                  onChange={(e) => setAddLanguagePreset(e.target.value)}
                >
                  <option value="">Add language...</option>
                  <option value="no">Norwegian</option>
                </select>
                <button
                  type="button"
                  onClick={() => {
                    if (addLanguagePreset === "no") {
                      const nbModel = readyModels.find((m) => m.id.startsWith("nb-whisper-"));
                      const fallback = nbModel?.id || readyModels[0]?.id || "nb-whisper-base";
                      addLanguageSlot({
                        id: "no",
                        label: "Norwegian",
                        languageCode: "no",
                        liveModel: fallback,
                        regularModel: fallback,
                      });
                      setAddLanguagePreset("");
                    }
                  }}
                  disabled={!addLanguagePreset || readyModels.length === 0}
                >
                  Add
                </button>
                <button type="button" className="restore-defaults-btn" onClick={restoreDefaultLanguageSlots}>
                  Restore defaults
                </button>
              </div>
            </div>
          )}
        </section>
      </div>

      {status && <p className="status">{status}</p>}
    </div>
  );
}

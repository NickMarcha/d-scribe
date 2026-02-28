import { useState, useEffect } from "react";
import { Settings } from "./components/Settings";
import { Session } from "./components/Session";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Tab = "session" | "settings";

function App() {
  const [activeTab, setActiveTab] = useState<Tab>("session");
  const [appDataDir, setAppDataDir] = useState<string>("");
  const [logFilePath, setLogFilePath] = useState<string>("");
  const [discordConnected, setDiscordConnected] = useState<boolean | null>(null);
  const [channelInfo, setChannelInfo] = useState<{
    guild_name?: string;
    channel_name?: string;
    self_username?: string;
  } | null>(null);

  useEffect(() => {
    async function checkConnection() {
      try {
        const state = await invoke<string>("discord_rpc_connection_state");
        const connected = state === "Connected";
        setDiscordConnected(connected);
        if (connected) {
          const info = await invoke<{
            guild_name?: string;
            channel_name?: string;
            self_username?: string;
          } | null>("get_channel_info_command");
          setChannelInfo(info ?? null);
        } else {
          setChannelInfo(null);
        }
      } catch {
        setDiscordConnected(false);
        setChannelInfo(null);
      }
    }
    checkConnection();
    const interval = setInterval(checkConnection, 3000);
    return () => clearInterval(interval);
  }, []);

  async function loadPaths() {
    try {
      const dir = await invoke<string>("get_app_data_dir");
      setAppDataDir(dir);
      const logPath = await invoke<string>("get_log_file_path");
      setLogFilePath(logPath);
    } catch (e) {
      console.error("Failed to get paths:", e);
    }
  }

  return (
    <main className="container">
      <h1>Discord Scribe</h1>
      <p className="subtitle">Transcribe Discord voice calls with speaker labels</p>

      <nav className="tabs">
        <button
          className={activeTab === "session" ? "active" : ""}
          onClick={() => setActiveTab("session")}
        >
          Session
        </button>
        <button
          className={activeTab === "settings" ? "active" : ""}
          onClick={() => {
            setActiveTab("settings");
            loadPaths();
          }}
        >
          Settings
        </button>
      </nav>

      {activeTab === "session" && (
        <div className="tab-content">
          <Session />
        </div>
      )}

      {activeTab === "settings" && (
        <div className="tab-content">
          <Settings />
          {appDataDir && (
            <p className="paths-hint">
              Data stored in: <code>{appDataDir}</code>
              {logFilePath && (
                <>
                  <br />
                  Log file: <code>{logFilePath}</code>
                </>
              )}
            </p>
          )}
        </div>
      )}

      <div
        className={`discord-status discord-status--${discordConnected === true ? "connected" : "disconnected"} discord-status--expandable`}
        title={discordConnected === true ? "Discord RPC connected" : "Discord RPC not connected"}
      >
        <span className="discord-status-label">
          Discord: {discordConnected === true ? "Connected" : "Disconnected"}
        </span>
        {discordConnected === true && channelInfo && (
          <div className="discord-status-details">
            {channelInfo.self_username && (
              <div className="discord-status-row">
                <span className="discord-status-key">User</span>
                <span>{channelInfo.self_username}</span>
              </div>
            )}
            {channelInfo.guild_name && (
              <div className="discord-status-row">
                <span className="discord-status-key">Server</span>
                <span>{channelInfo.guild_name}</span>
              </div>
            )}
            {channelInfo.channel_name && (
              <div className="discord-status-row">
                <span className="discord-status-key">Channel</span>
                <span>{channelInfo.channel_name}</span>
              </div>
            )}
            {!channelInfo.guild_name && channelInfo.channel_name && (
              <div className="discord-status-row discord-status-hint">
                DM / Group call
              </div>
            )}
          </div>
        )}
      </div>
    </main>
  );
}

export default App;

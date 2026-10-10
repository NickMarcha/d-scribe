import { useState, useEffect } from "react";
import { Settings } from "./components/Settings";
import { Session } from "./components/Session";
import { UpdateNotice } from "./components/UpdateNotice";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import "./App.css";

type Tab = "session" | "settings";
type DiscordState = "InVoice" | "Idle" | "Disconnected";

function App() {
  const [activeTab, setActiveTab] = useState<Tab>("session");
  const [appDataDir, setAppDataDir] = useState<string>("");
  const [logFilePath, setLogFilePath] = useState<string>("");
  const [version, setVersion] = useState<string>("");
  const [discordState, setDiscordState] = useState<DiscordState | null>(null);
  const [discordRunning, setDiscordRunning] = useState<boolean>(false);
  const [channelInfo, setChannelInfo] = useState<{
    guild_name?: string;
    channel_name?: string;
    self_username?: string;
  } | null>(null);

  useEffect(() => {
    async function checkConnection() {
      try {
        const result = await invoke<{
          state: DiscordState;
          discord_running: boolean;
        }>("discord_rpc_connection_state");
        setDiscordState(result.state);
        setDiscordRunning(result.discord_running);
        if (result.state === "InVoice") {
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
        setDiscordState("Disconnected");
        setDiscordRunning(false);
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
      setVersion(await getVersion());
    } catch (e) {
      console.error("Failed to get paths:", e);
    }
  }

  const statusClass =
    discordState === "InVoice" ? "in-voice" : discordState === "Idle" ? "idle" : "disconnected";

  return (
    <div className="app">
      <header className="app-toolbar">
        <span className="app-name">d-scribe</span>
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
        <div className="toolbar-right">
          <UpdateNotice />
          <div
            className={`discord-status discord-status--${statusClass}`}
            title={
              discordState === "InVoice"
                ? undefined
                : discordState === "Idle"
                  ? "Connected to Discord, not in a voice channel"
                  : discordRunning
                    ? "Not connected. Connect in Settings."
                    : "Start Discord to connect."
            }
          >
            Discord:{" "}
            {discordState === "InVoice"
              ? "in voice"
              : discordState === "Idle"
                ? "idle"
                : discordRunning
                  ? "disconnected"
                  : "not running"}
            {discordState === "InVoice" && channelInfo && (
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
                  <div className="discord-status-hint">DM or group call</div>
                )}
              </div>
            )}
          </div>
        </div>
      </header>

      <div className="tab-content">
        {activeTab === "session" && <Session />}
        {activeTab === "settings" && (
          <>
            <Settings />
            {appDataDir && (
              <p className="paths-hint">
                {version && (
                  <>
                    Version {version}
                    <br />
                  </>
                )}
                Data folder: <code>{appDataDir}</code>
                {logFilePath && (
                  <>
                    <br />
                    Log file: <code>{logFilePath}</code>
                  </>
                )}
              </p>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export default App;

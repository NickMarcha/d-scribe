import { useState } from "react";
import { Settings } from "./components/Settings";
import { Session } from "./components/Session";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Tab = "session" | "settings";

function App() {
  const [activeTab, setActiveTab] = useState<Tab>("session");
  const [appDataDir, setAppDataDir] = useState<string>("");

  async function loadPaths() {
    try {
      const dir = await invoke<string>("get_app_data_dir");
      setAppDataDir(dir);
    } catch (e) {
      console.error("Failed to get app data dir:", e);
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
            </p>
          )}
        </div>
      )}
    </main>
  );
}

export default App;

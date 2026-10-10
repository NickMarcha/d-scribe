import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";

type State =
  | { kind: "idle" }
  | { kind: "available"; update: Update }
  | { kind: "installing"; percent: number | null }
  | { kind: "error"; message: string };

export function UpdateNotice() {
  const [state, setState] = useState<State>({ kind: "idle" });

  useEffect(() => {
    check()
      .then((update) => {
        if (update) setState({ kind: "available", update });
      })
      .catch((e) => console.error("Update check failed:", e));
  }, []);

  async function install(update: Update) {
    let total = 0;
    let received = 0;
    setState({ kind: "installing", percent: null });
    try {
      // On Windows the installer exits the app and restarts it when done.
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          if (total) setState({ kind: "installing", percent: Math.round((received / total) * 100) });
        }
      });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  }

  switch (state.kind) {
    case "idle":
      return null;
    case "available":
      return (
        <div className="update-notice">
          <span>Version {state.update.version} is available</span>
          <button onClick={() => install(state.update)}>Install and restart</button>
        </div>
      );
    case "installing":
      return (
        <div className="update-notice">
          Downloading update{state.percent !== null ? ` ${state.percent}%` : ""}
        </div>
      );
    case "error":
      return (
        <div className="update-notice update-notice--error" title={state.message}>
          Update failed
        </div>
      );
  }
}

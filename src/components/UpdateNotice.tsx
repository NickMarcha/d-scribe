import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";

type State =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "latest" }
  | { kind: "available"; update: Update }
  | { kind: "installing"; percent: number | null }
  | { kind: "error"; message: string };

export interface Updater {
  state: State;
  checkNow: () => Promise<void>;
  install: (update: Update) => Promise<void>;
}

/** Update state shared by the toolbar notice and the check button in Settings. Checks once on startup. */
export function useUpdater(): Updater {
  const [state, setState] = useState<State>({ kind: "idle" });

  async function checkNow() {
    setState({ kind: "checking" });
    try {
      const update = await check();
      setState(update ? { kind: "available", update } : { kind: "latest" });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  }

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

  return { state, checkNow, install };
}

/** Toolbar notice: shown only when an update is available or installing. */
export function UpdateNotice({ updater }: { updater: Updater }) {
  const { state, install } = updater;
  switch (state.kind) {
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
    default:
      return null;
  }
}

/** "Check for updates" button and its result, for Settings. */
export function UpdateCheck({ updater }: { updater: Updater }) {
  const { state, checkNow } = updater;
  const busy = state.kind === "checking" || state.kind === "installing";
  return (
    <span className="update-check">
      <button type="button" className="link-button" onClick={checkNow} disabled={busy}>
        Check for updates
      </button>
      {state.kind === "checking" && <span>Checking...</span>}
      {state.kind === "latest" && <span>You have the latest version.</span>}
      {state.kind === "available" && <span>Version {state.update.version} is available. Install it from the toolbar.</span>}
      {state.kind === "error" && <span className="update-check--error">Update check failed: {state.message}</span>}
    </span>
  );
}

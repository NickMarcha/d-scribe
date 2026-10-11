import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { load } from "@tauri-apps/plugin-store";

interface AudioDevice {
  id: string;
  name: string;
}

interface AudioDevices {
  inputs: AudioDevice[];
  outputs: AudioDevice[];
  default_input: string | null;
  default_output: string | null;
}

type TestState = { running: boolean; level: number | null; error: string };

/** Picks the microphone and the source for the other participants. Saves on change. */
export function AudioSettings() {
  const [devices, setDevices] = useState<AudioDevices | null>(null);
  const [micId, setMicId] = useState("");
  const [othersId, setOthersId] = useState("");
  const [error, setError] = useState("");

  async function refresh() {
    try {
      setDevices(await invoke<AudioDevices>("list_audio_devices_command"));
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    refresh();
    load("settings.json", { defaults: {}, autoSave: true }).then(async (store) => {
      setMicId((await store.get<string>("mic_device_id")) ?? "");
      setOthersId((await store.get<string>("others_device_id")) ?? "");
    });
  }, []);

  async function persist(key: string, value: string) {
    const store = await load("settings.json", { defaults: {}, autoSave: true });
    await store.set(key, value);
    await store.save();
  }

  if (!devices) return error ? <p className="field-hint">{error}</p> : null;
  const missing = (id: string, list: AudioDevice[]) => id !== "" && !list.some((d) => d.id === id);

  return (
    <>
      <div className="form-group">
        <label htmlFor="mic-device">Your microphone</label>
        <div className="device-row">
          <select
            id="mic-device"
            value={micId}
            onChange={(e) => {
              setMicId(e.target.value);
              persist("mic_device_id", e.target.value);
            }}
          >
            <option value="">Windows default ({devices.default_input ?? "none"})</option>
            {missing(micId, devices.inputs) && <option value={micId}>Not connected</option>}
            {devices.inputs.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </select>
          <LevelTest mic deviceId={micId} />
        </div>
        <span className="field-hint">
          Pick the same microphone you use in Discord. Click Test and talk: the bar should move.
        </span>
      </div>
      <div className="form-group">
        <label htmlFor="others-device">Other people in the call</label>
        <div className="device-row">
          <select
            id="others-device"
            value={othersId}
            onChange={(e) => {
              setOthersId(e.target.value);
              persist("others_device_id", e.target.value);
            }}
          >
            <option value="">Discord app (recommended)</option>
            {missing(othersId, devices.outputs) && <option value={othersId}>Not connected</option>}
            {devices.outputs.map((d) => (
              <option key={d.id} value={d.id}>
                Everything played on {d.name}
              </option>
            ))}
          </select>
          <LevelTest mic={false} deviceId={othersId} />
        </div>
        <span className="field-hint">
          Discord app records only Discord, whatever speakers or headset it plays to, and picks it up again if
          Discord restarts during a call. Choose an output device only if you use Discord in a browser: then
          everything played on that device is recorded, including music and games.
        </span>
      </div>
      <button type="button" className="link-button" onClick={refresh}>
        Refresh device list
      </button>
    </>
  );
}

function LevelTest({ mic, deviceId }: { mic: boolean; deviceId: string }) {
  const [state, setState] = useState<TestState>({ running: false, level: null, error: "" });

  async function run() {
    setState({ running: true, level: null, error: "" });
    try {
      const level = await invoke<number>("measure_audio_level_command", { mic, deviceId: deviceId || null });
      setState({ running: false, level, error: "" });
    } catch (e) {
      setState({ running: false, level: null, error: String(e) });
    }
  }

  const pct = state.level == null ? 0 : Math.min(100, Math.round(state.level * 100));
  return (
    <div className="level-test">
      <button type="button" onClick={run} disabled={state.running}>
        {state.running ? (mic ? "Talk now..." : "Listening...") : "Test"}
      </button>
      {state.level != null && (
        <span className="level-result">
          <span className="level-track">
            <span className="level-fill" style={{ width: `${pct}%` }} />
          </span>
          {pct >= 1 ? `Peak ${pct}%` : mic ? "Nothing heard" : "Nothing heard. Test while someone talks in Discord."}
        </span>
      )}
      {state.error && <span className="level-error">{state.error}</span>}
    </div>
  );
}

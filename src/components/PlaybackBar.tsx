import { Menu } from "./Menu";
import "./PlaybackBar.css";

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60).toString().padStart(2, "0");
  return h > 0 ? `${h}:${m.toString().padStart(2, "0")}:${s}` : `${m}:${s}`;
}

type Mode = "remote" | "local" | "both";

interface PlaybackBarProps {
  isPlaying: boolean;
  onPlay: () => void;
  onPause: () => void;
  onSeekTime: (seconds: number) => void;
  playbackMode: Mode;
  setPlaybackModeAndPlay: (mode: Mode) => void;
  remoteVolume: number;
  setRemoteVolume: (v: number) => void;
  localVolume: number;
  setLocalVolume: (v: number) => void;
  currentTime: number;
  duration: number;
  hasAudio: boolean;
}

/** One-row player: play/pause, seek slider, time, and a menu for what to hear. */
export function PlaybackBar(p: PlaybackBarProps) {
  return (
    <div className="playback-bar">
      <button
        type="button"
        className="playback-toggle"
        onClick={p.isPlaying ? p.onPause : p.onPlay}
        disabled={!p.hasAudio}
      >
        {p.isPlaying ? "Pause" : "Play"}
      </button>
      <input
        type="range"
        className="playback-seek"
        min={0}
        max={p.duration || 0}
        step={0.1}
        value={Math.min(p.currentTime, p.duration || 0)}
        onChange={(e) => p.onSeekTime(parseFloat(e.target.value))}
        disabled={!p.hasAudio || !p.duration}
        aria-label="Position"
      />
      <span className="playback-time">
        {formatTime(p.currentTime)} / {formatTime(p.duration)}
      </span>
      <Menu label="Audio" title="Choose who you hear and set volumes">
        {() => (
          <>
            <label className="menu-field">
              Play
              <select value={p.playbackMode} onChange={(e) => p.setPlaybackModeAndPlay(e.target.value as Mode)}>
                <option value="both">Everyone</option>
                <option value="remote">Others only</option>
                <option value="local">You only</option>
              </select>
            </label>
            <label className="menu-field">
              Others
              <input
                type="range"
                min="0"
                max="1"
                step="0.05"
                value={p.remoteVolume}
                onChange={(e) => p.setRemoteVolume(parseFloat(e.target.value))}
              />
            </label>
            <label className="menu-field">
              You
              <input
                type="range"
                min="0"
                max="1"
                step="0.05"
                value={p.localVolume}
                onChange={(e) => p.setLocalVolume(parseFloat(e.target.value))}
              />
            </label>
          </>
        )}
      </Menu>
    </div>
  );
}

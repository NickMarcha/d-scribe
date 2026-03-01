import { useState } from "react";
import "./PlaybackBar.css";

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

interface PlaybackBarProps {
  isPlaying: boolean;
  onPlay: () => void;
  onPause: () => void;
  onStop: () => void;
  playbackMode: "remote" | "local" | "both";
  setPlaybackModeAndPlay: (mode: "remote" | "local" | "both") => void;
  remoteVolume: number;
  setRemoteVolume: (v: number) => void;
  localVolume: number;
  setLocalVolume: (v: number) => void;
  currentTime: number;
  duration: number;
  hasAudio: boolean;
}

export function PlaybackBar({
  isPlaying,
  onPlay,
  onPause,
  onStop,
  playbackMode,
  setPlaybackModeAndPlay,
  remoteVolume,
  setRemoteVolume,
  localVolume,
  setLocalVolume,
  currentTime,
  duration,
  hasAudio,
}: PlaybackBarProps) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className={`playback-bar ${expanded ? "playback-bar-expanded" : ""}`}>
      <div className="playback-bar-header">
        <div className="playback-bar-buttons">
          <button type="button" onClick={onPlay} disabled={!hasAudio}>
            Play
          </button>
          <button type="button" onClick={onPause} disabled={!isPlaying}>
            Pause
          </button>
          <button type="button" onClick={onStop} disabled={!isPlaying}>
            Stop
          </button>
        </div>
        <span className="playback-bar-time">
          {formatTime(currentTime)} / {formatTime(duration)}
        </span>
        <button
          type="button"
          className="playback-bar-expand"
          onClick={() => setExpanded(!expanded)}
          title={expanded ? "Collapse" : "Expand"}
        >
          {expanded ? "▼" : "▲"}
        </button>
      </div>
      {expanded && (
        <div className="playback-bar-details">
          <div className="playback-mode">
            <label>Playback:</label>
            <button
              type="button"
              className={playbackMode === "remote" ? "active" : ""}
              onClick={() => setPlaybackModeAndPlay("remote")}
            >
              Remote
            </button>
            <button
              type="button"
              className={playbackMode === "local" ? "active" : ""}
              onClick={() => setPlaybackModeAndPlay("local")}
            >
              Local
            </button>
            <button
              type="button"
              className={playbackMode === "both" ? "active" : ""}
              onClick={() => setPlaybackModeAndPlay("both")}
            >
              Both
            </button>
          </div>
          <div className="volume-sliders">
            <label>
              Remote:{" "}
              <input
                type="range"
                min="0"
                max="1"
                step="0.1"
                value={remoteVolume}
                onChange={(e) => setRemoteVolume(parseFloat(e.target.value))}
              />
            </label>
            <label>
              Local:{" "}
              <input
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
      )}
    </div>
  );
}

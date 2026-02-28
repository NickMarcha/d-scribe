# d-scribe – Agent Instructions

## Pre-release

App is not released yet. Do not prioritize backwards compatibility or repair logic for existing projects. Prefer simplicity over migration/repair code. Do not add fallbacks that hide broken behavior (e.g. creating fake segments when Discord RPC fails); fix the root cause instead.

## Overview

Tauri 2 app: React/TypeScript frontend, Rust backend. Records Discord voice (WASAPI loopback + mic), tracks speakers via Discord RPC, transcribes with whisper.cpp CLI. Sessions are auto-saved to a recent folder; playback supports remote/local/both with transcript auto-scroll. Discord auth is persisted via refresh token.

## Build & Run

- Dev: `npm run tauri dev` (runs Vite + cargo with `transcription-whisper-cli` feature)
- Build: `npm run tauri build`
- Whisper binary required: run `.\src-tauri\binaries\download-whisper.ps1` first

## Logging

Logs go to the terminal (WASAPI trace filtered out) and to a file. Log file: `%APPDATA%/d-scribe/logs/d-scribe.log` (Windows Roaming, same folder as projects). Path shown in Settings.

## Architecture

- **Frontend** (`src/`): React, Vite. Calls Tauri commands for recording, transcription, export, project CRUD, playback.
- **Backend** (`src-tauri/src/`):
  - `lib.rs`: Tauri commands, transcription orchestration (prefers `std::process::Command` over sidecar)
  - `audio/`: WASAPI capture (loopback + mic), 16 kHz mono 16-bit WAV
  - `discord_rpc/`: Discord RPC client, OAuth, token persistence (refresh token), speaking events
  - `session/`: Segment tracking, merge buffer (configurable ms), session state
  - `project.rs`: Save/load, `auto_save_project`, `delete_project`, `purge_old_recent`, `list_projects_with_meta`
  - `paths.rs`: `projects_dir`, `recent_projects_dir`
  - `transcription/`: `extract_segment` (WAV time-range), Whisper CLI invocation, `remote_api` (OpenAI-compatible API)

## Conventions

- **Audio**: 16 kHz mono 16-bit PCM. Segment extraction uses `start_sample = start_ms * 16` (16 samples/ms at 16 kHz).
- **Transcription**: Uses `whisper-cli.exe` via `std::process::Command` when found next to main exe; falls back to sidecar. Output via `-otxt -of` (file), not stdout.
- **Remote transcription**: Uses OpenAI-compatible API (`POST /v1/audio/transcriptions`). Compatible with open-asr-server, vLLM (Voxtral), LocalAI, and OpenAI.
- **Discord RPC voice**: Works with server voice channels, group DM calls, and 1:1 DM calls. Connect in Settings while in a voice channel.
- **Paths**: App data under `%APPDATA%/d-scribe/` (projects, projects/recent, models, transcribe_temp).
- **Session IDs**: Use `format_project_name` with template `{guild}_{channel}_{timestamp}` (placeholders: `{guild}`, `{channel}`, `{timestamp}`, `{date}`, `{time}`).
- **Playback**: Default "both" (remote + local mixed); persisted in settings. Volume sliders apply in real time.

## Gotchas

1. **Whisper binary**: Use `whisper-cli.exe`, not `main.exe` (deprecated). DLLs must be in same dir as exe.
2. **Segment extraction**: Sample math is `ms * 16` for 16 kHz. Do not divide by 1000.
3. **Direct vs sidecar**: Prefer direct `Command` for file access; sidecar can have sandbox issues on Windows.
4. **Segment merge buffer**: Short pauses (< buffer ms) are merged; configurable in Settings.
5. **Recent purge**: `purge_recent_command` runs when listing projects; uses `recent_retention_days` from settings.

## Planned (roadmap)

- AI summary of transcripts
- Participant stats (word count, speaking time per speaker)
- Meeting notes workflow
- Debate/discussion analysis (arguments, positions, rebuttals)
- Custom workflows with configurable LLM instructions
- Flexible AI/LLM: local (Ollama, llama.cpp) and remote (OpenAI, Anthropic, etc.)
- Privacy: mute-aware recording – Discord mute events disable local audio stream for muted participants

## Key Files

| File | Purpose |
|------|---------|
| `src-tauri/src/lib.rs` | Tauri commands: transcribe, auto_save, delete, purge, etc. |
| `src-tauri/src/project.rs` | `auto_save_project`, `delete_project`, `purge_old_recent`, `list_projects_with_meta` |
| `src-tauri/src/paths.rs` | `projects_dir`, `recent_projects_dir` |
| `src-tauri/src/transcription/wav_extract.rs` | `extract_segment` |
| `src-tauri/src/session/recorder.rs` | Segments, merge buffer, speaking events |
| `src/components/Session.tsx` | Main UI, project list, playback, delete modal |
| `src-tauri/tauri.conf.json` | `externalBin: ["binaries/whisper-cli"]` |

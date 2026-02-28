# d-scribe – Agent Instructions

## Overview

Tauri 2 app: React/TypeScript frontend, Rust backend. Records Discord voice (WASAPI loopback + mic), tracks speakers via Discord RPC, transcribes with whisper.cpp CLI.

## Build & Run

- Dev: `npm run tauri dev` (runs Vite + cargo with `transcription-whisper-cli` feature)
- Build: `npm run tauri build`
- Whisper binary required: run `.\src-tauri\binaries\download-whisper.ps1` first

## Architecture

- **Frontend** (`src/`): React, Vite. Calls Tauri commands for recording, transcription, export.
- **Backend** (`src-tauri/src/`):
  - `lib.rs`: Tauri commands, transcription orchestration (prefers `std::process::Command` over sidecar)
  - `audio/`: WASAPI capture (loopback + mic), 16 kHz mono 16-bit WAV
  - `discord_rpc/`: Discord RPC client, speaking start/stop events
  - `session/`: Segment tracking, merge buffer (configurable ms), session state
  - `transcription/`: `extract_segment` (WAV time-range), Whisper CLI invocation

## Conventions

- **Audio**: 16 kHz mono 16-bit PCM. Segment extraction uses `start_sample = start_ms * 16` (16 samples/ms at 16 kHz).
- **Transcription**: Uses `whisper-cli.exe` via `std::process::Command` when found next to main exe; falls back to sidecar. Output via `-otxt -of` (file), not stdout.
- **Paths**: App data under `%APPDATA%/d-scribe/` (projects, models, transcribe_temp).

## Gotchas

1. **Whisper binary**: Use `whisper-cli.exe`, not `main.exe` (deprecated). DLLs must be in same dir as exe.
2. **Segment extraction**: Sample math is `ms * 16` for 16 kHz. Do not divide by 1000.
3. **Direct vs sidecar**: Prefer direct `Command` for file access; sidecar can have sandbox issues on Windows.
4. **Segment merge buffer**: Short pauses (< buffer ms) are merged; configurable in Settings.

## Key Files

| File | Purpose |
|------|---------|
| `src-tauri/src/lib.rs` | `transcribe_session_command`, Whisper invocation |
| `src-tauri/src/transcription/wav_extract.rs` | `extract_segment` |
| `src-tauri/src/session/recorder.rs` | Segments, merge buffer, speaking events |
| `src-tauri/tauri.conf.json` | `externalBin: ["binaries/whisper-cli"]` |

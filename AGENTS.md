# d-scribe – Agent Instructions

## Pre-release

App is not released yet. Do not prioritize backwards compatibility or repair logic for existing projects. Prefer simplicity over migration/repair code. Do not add fallbacks that hide broken behavior (e.g. creating fake segments when Discord RPC fails); fix the root cause instead.

**No migration until 1.0.0:** Do not implement migration logic for settings or projects (e.g. converting `remote_base_url` to `remote_sources`, or old registry formats). Add migrations only when preparing a first major release.

## Overview

Tauri 2 app: React/TypeScript frontend, Rust backend. Records Discord voice (WASAPI loopback + mic), tracks speakers via Discord RPC, transcribes with whisper.cpp CLI. Each recording is a folder under `recordings/` (auto-saved, unsaved until you click Save) and listed by day in `RecordingsOverview.tsx`; playback supports remote/local/both with transcript auto-scroll. Discord auth is persisted via refresh token. Auto-updates via tauri-plugin-updater (checked on startup, `src/components/UpdateNotice.tsx`).

## Website

`site/` is a static page (index, privacy, `download.js` picks the latest `-setup.exe` from the GitHub API and shows the SmartScreen note on click). `.github/workflows/pages.yml` deploys it to https://nickmarcha.github.io/d-scribe/ on pushes to `release` (the only branch) that touch `site/`. Test access: the d-scribe Discord server (https://discord.gg/WPvhz8x7SF, #request-access; owner adds testers by hand in the Developer Portal and gives the Tester role), linked from the site and the app (`ACCESS_REQUEST_URL`). Screenshots in `site/img/` are taken from `npx vite` with a temporary `src/devMock.ts` (mockIPC from `@tauri-apps/api/mocks` returning sample recordings, imported first in `main.tsx`) and Playwright at 2x scale; delete the mock afterwards, never commit it. Keep the tone friendly: meeting notes, not legal warnings.

## License

PolyForm Noncommercial 1.0.0 (`LICENSE.md`).

## Build & Run

- Dev: `npm run tauri dev` (runs Vite + cargo with `transcription-whisper-cli` feature)
- Build: `npm run tauri build`
- No bundled Whisper binary: the app downloads whisper.cpp into `%APPDATA%/d-scribe/engines/` (see `transcription/engine.rs`)

## Logging

Logs go to the terminal (WASAPI trace filtered out) and to a file. Log file: `%APPDATA%/d-scribe/logs/d-scribe.log` (Windows Roaming, next to `recordings/`). Path shown in Settings.

## Architecture

- **Frontend** (`src/`): React, Vite. Calls Tauri commands for recording, transcription, export, project CRUD, playback.
- **Backend** (`src-tauri/src/`):
  - `lib.rs`: Tauri commands, transcription orchestration (one `WhisperServer` per model per job, English-turn routing)
  - `audio/`: WASAPI capture, 16 kHz mono 16-bit WAV, gaps padded with silence. Others: Discord process loopback by default (`others_device_id` empty), or loopback of a chosen output device. Mic: Windows default or `mic_device_id`. If Discord isn't running or exits mid-recording, records silence and reattaches when a Discord process appears (checked every `PROCESS_CHECK`). Chosen devices that can't be opened fail `start_recording`. `measure_level` backs the Test buttons
  - `discord_rpc/`: Discord RPC client, OAuth, token persistence (refresh token), speaking events
  - `session/`: Segment tracking, merge buffer (configurable ms), session state
  - `project.rs`: one folder per recording, `recordings/<YYYY-MM-DD HH.MM place>/` with `recording.json` (the `SessionState` plus a `saved` flag; audio paths stored relative to the folder, resolved on load), `others.wav`, `you.wav`. `create_recording_dir` + `place_name` name the folder at record start ("Call with Name" for DMs, "Server - channel"). `set_project_saved` flips the flag, `save_project` keeps it, `delete_project` removes the whole folder (only inside `recordings/`), `purge_old_recent`, `list_projects_with_meta`
  - `paths.rs`: `recordings_dir`
  - `tray.rs`: tray icon and menu (status, Open, Quit). Closing the main window hides it (`on_window_event` prevents close). Quit while recording emits `quit-requested`; `Session.tsx` stops, saves and calls `exit_app`. The window is `visible: false` in `tauri.conf.json` and shown in setup unless started with `--minimized` (passed by tauri-plugin-autostart, "Start with Windows" in Settings > Recording). tauri-plugin-single-instance focuses the running copy on a second launch
  - `transcription/`: `extract_segment` (WAV time-range), `engine` (whisper.cpp download + `whisper-server` process), `remote_api` (OpenAI-compatible API)

## Conventions

- **Audio**: 16 kHz mono 16-bit PCM. Segment extraction uses `start_sample = start_ms * 16` (16 samples/ms at 16 kHz).
- **Transcription**: whisper.cpp release `RELEASE` in `engine.rs`, downloaded on demand: CPU build (auto, on first transcribe) or CUDA 12.4 build (Settings button). CUDA is used when installed. A `whisper-server.exe` is started per model for a transcription job (and once per live recording) and segments are POSTed to `/inference`; never spawn one process per segment (each reloads the model). Segments under `MIN_SEGMENT_MS`, past the end of the WAV, or quieter than `SILENCE_RMS` are left empty without calling whisper (it invents "Thank you." / "(c) transcript Emily Beynon" on silence; measured: junk segments sit under ~100 RMS, real short words above ~500). `speech_text` in `engine.rs` drops whisper segments with no_speech_prob > 0.6 and avg_logprob < -1 and strips `<|...|>` tokens (NB-Whisper emits `<|nocaptions|>`).
- **English turns**: a language slot can set `englishModel`. Each segment is first transcribed by that model with language `auto`; if `language_probabilities.en >= ENGLISH_THRESHOLD` its text is kept, otherwise the slot's regular model runs with the slot language. Needed because NB-Whisper writes English speech as Norwegian, and stock large-v3 on Norwegian drifts into Swedish/German. Live mode doesn't route.
- **Remote transcription**: Uses `remote_sources` (array of sources with `host`, `transcriptionPath`, `modelsPath`, `apiKey`). Each source can have optional path overrides; transcription URL = `host + (transcriptionPath || "/v1/audio/transcriptions")`. Compatible with open-asr-server, vLLM (Voxtral), LocalAI, and OpenAI.
- **Model registry**: `model_registry` lists integrated (Whisper) and remote (API) models. Remote models use `sourceId:modelName` as id; `list_remote_models_command` fetches models from a source via `GET /v1/models` (optional override).
- **Discord RPC voice**: Works with server voice channels, group DM calls, and 1:1 DM calls. Connect in Settings; joining a voice channel before or after connecting both work (the selected channel is polled). Only one connection is active: a new connect aborts the previous one. Auto-reconnect saves the rotated refresh token before connecting.
- **Discord setup**: The Client ID field is optional; empty means `SHARED_CLIENT_ID` in `Settings.tsx` (the maintainer's app, testers only; `ACCESS_REQUEST_URL` links to the d-scribe Discord server, where people ask in #request-access). Users can paste their own Client ID instead. The Discord app must have "Public Client" on and `https://localhost` under OAuth2 Redirects (`OAUTH_REDIRECT_URI`). Auth is OAuth2 PKCE (no client secret). Transport is IPC named pipes only (no WebSocket, no RPC Origin).
- **Discord scopes**: `OAUTH_SCOPES` in `src-tauri/src/discord_rpc/client.rs` (`rpc`, `identify`). The narrower `identify`, `rpc.voice.read`, `guilds` was tried and Discord rejects it with an empty OAuth2 error after approval. Channel changes are detected by polling `GET_SELECTED_VOICE_CHANNEL` every 2 s. Guild names come from RPC `GET_GUILD`.
- **Paths**: App data under `%APPDATA%/d-scribe/` (recordings, models, engines, transcribe_temp). whisper-server logs next to the app log.
- **Session IDs**: `session_id` is the recording's folder name. There is no name template; the backend picks the folder in `start_recording`.
- **Auto-record**: `auto_record` setting. `Session.tsx` checks `get_channel_info_command` every 3 s and starts a regular recording when `member_count` (everyone in the voice channel, including you) is 2 or more. It stops only recordings it started: when you leave or switch channel, or after `AUTO_STOP_GRACE_MS` (60 s) alone. After a manual Stop it waits until you leave that channel.
- **Playback**: Default "both" (remote + local mixed); persisted in settings. Volume sliders apply in real time.

## Settings

Settings stored in `settings.json` (tauri-plugin-store). Sections: **Recording** (segment merge buffer, unsaved retention days and size limit, recordings folder, auto-record (turning it on asks for confirmation with a short note about recording rules), start with Windows via the autostart plugin, saved immediately), **Audio** (`AudioSettings.tsx`: mic device, source for others, Test buttons; saves on change), **Discord** (optional Client ID), **Manage models** (remote sources CRUD, model registry, add Whisper/remote model), **Selected models** (4 slots: English/Multilingual × Live/Regular). Collapsible headers (collapsed by default). Sticky "Save all settings" button.

## Gotchas

1. **Whisper engine**: the CUDA 11.8 build silently falls back to CPU on current drivers; use the 12.4 build. Engine DLLs must sit next to `whisper-server.exe` (the zip's `Release/` folder is flattened on unpack).
2. **Segment extraction**: Sample math is `ms * 16` for 16 kHz. Do not divide by 1000.
3. **wasapi `get_device(id)`**: can pass Windows a freed string and fail at random (0x80070002). Look devices up by walking `get_device_collection` and comparing `get_id()` (`open_device`).
4. **Loopback timing**: loopback streams deliver nothing while the source is silent. `Recorder::pad_to_now` pads with silence to wall-clock time so segment timestamps still index the WAV.
5. **Segment merge buffer**: Short pauses (< buffer ms) are merged; configurable in Settings.
6. **Unsaved vs saved**: the `saved` flag in `recording.json`; nothing moves. `purge_recent_command` runs when listing; deletes unsaved recording folders older than `recent_retention_days`, then the oldest until they fit in `recent_max_gb` (0 = no limit). The open recording is never purged.
7. **No migration**: Do not add migration logic for old settings keys or registry formats before 1.0.0.

## UI and writing

- UI follows Uncodixify: plain toolbar + underline tabs, 6px radius, 1px borders, no gradients/glows/pills, shadows max `0 2px 8px rgba(0,0,0,0.1)`. Colors come from the CSS variables in `src/App.css` (stone/zinc neutrals, muted teal accent); don't add new hex colors in components. Font: IBM Plex Sans/Mono (bundled via fontsource).
- User-facing text follows Unslop: sentence case, plain words, no em dashes, say what a thing does.

## Planned (roadmap)

- Debate/discussion analysis (arguments, positions, rebuttals); possible today as a custom AI prompt
- Privacy: mute-aware recording – Discord mute events disable local audio stream for muted participants

## Key Files

| File | Purpose |
|------|---------|
| `src-tauri/src/lib.rs` | Tauri commands: transcribe, auto_save, delete, purge, list_remote_models, etc. |
| `src-tauri/src/project.rs` | Recording folders: `create_recording_dir`, `place_name`, `auto_save_project`, `set_project_saved`, `delete_project`, `purge_old_recent`, `list_projects_with_meta` (duration, speakers, others, size) |
| `src/components/RecordingsOverview.tsx` | Recordings list grouped by day with search and server/channel/person filters |
| `src-tauri/src/paths.rs` | `recordings_dir`, `models_dir`, `engines_dir` |
| `src-tauri/src/transcription/wav_extract.rs` | `extract_segment` |
| `src-tauri/src/transcription/remote_api.rs` | `transcribe_via_api`, `list_models` (OpenAI-compatible API) |
| `src-tauri/src/session/recorder.rs` | Segments, merge buffer, speaking events |
| `src/components/Session.tsx` | Main UI. An open recording has a sticky `.session-head` (back link, title (click to rename; stored as `title` on the project, shown and searched in the overview), language, Transcribe, Save, More menu with rename, Show in folder (reveals the audio file), export/delete, and `PlaybackBar` with seek slider). Its height goes in `--session-head-h` so the transcript toolbar sticks right under it. The transcript scrolls with the page (no inner scroll) |
| `src/components/Menu.tsx` | Small dropdown (More, Audio) |
| `src/components/Transcript.tsx` | Tabs: Read, Edit, plus `extraViews` (Stats, AI). Reader hides empty segments once anything has text and starts a new paragraph after `PARAGRAPH_GAP_MS`; search with match count and Previous/Next (Enter, Shift+Enter), current match outlined; click to seek. Editor (speaker, split, merge, delete, re-transcribe one segment). Edit ops live in `transcriptOps.ts`; undo history and auto-save in `Session.tsx` |
| `src/components/AiPanel.tsx`, `aiConfig.ts`, `src-tauri/src/ai.rs` | AI prompts: runs the user's CLI command (default `claude -p --tools ""`) with prompt + transcript on stdin; results saved in `ai_outputs` on the project. No provider-specific code |
| `src/components/Settings.tsx` | Settings UI, remote sources, model registry, slot dropdowns |
| `src-tauri/src/transcription/engine.rs` | whisper.cpp engine download (`download_engine`) and `WhisperServer` |
| `src/tasks.ts`, `src/components/TaskBar.tsx` | Long jobs (transcribe, model/engine downloads) go through `runTask`, which maps backend progress events to the bar fixed at the bottom of the window. State lives outside React so it survives tab switches |
| `src/components/EngineSettings.tsx` | CPU/GPU engine download in Settings > Manage models |

# d-scribe

Record Discord voice channel audio and transcribe it with speaker attribution using Whisper.

## Features

- **Capture audio** from Discord (loopback + microphone) via WASAPI on Windows
- **Track speakers** using Discord RPC speaking events
- **Transcribe** segments with [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
- **Export** to SRT or VTT
- **Auto-save** sessions to a recent folder (configurable retention, default 10 days)
- **Playback** with remote/local/both modes, auto-scroll transcript during playback
- **Project list** from default location; click to open, delete with optional audio cleanup
- **Discord auth** persisted via refresh token; auto-reconnect on startup

## Prerequisites

- Node.js & npm
- Rust (for Tauri)
- Discord running (for RPC connection)
- **Whisper binary** – required for transcription (see below)

## Quick Start

### 1. Install dependencies

```bash
npm install
```

### 2. Set up Whisper (required for transcription)

**Windows (recommended):**

```powershell
.\src-tauri\binaries\download-whisper.ps1
```

Or see [src-tauri/binaries/README.md](src-tauri/binaries/README.md) for manual setup.

### 3. Download a Whisper model

Run the app, open Settings, and download a model (e.g. `ggml-base.en.bin`). Models are stored in `%APPDATA%/d-scribe/models/`.

### 4. Run the app

```bash
npm run tauri dev
```

### 5. Connect & record

1. Open **Settings** → connect to Discord (Client ID, Client Secret, RPC Origin). Auth is saved; you typically only need to authorize once.
2. Join a voice channel
3. Click **Start Recording**
4. When done, click **Stop Recording** – the session is auto-saved to recent
5. Use **Play** to listen; transcript auto-scrolls with playback
6. Click **Transcribe** to run Whisper on each segment
7. **Save Project** to move to permanent storage, or **Export** to SRT/VTT

## Project Structure

```
d-scribe/
├── src/                 # React frontend (Vite + TypeScript)
├── src-tauri/
│   ├── src/             # Rust backend
│   │   ├── lib.rs       # Tauri commands, transcription orchestration
│   │   ├── audio/       # WASAPI capture
│   │   ├── discord_rpc/ # Discord RPC, OAuth, token persistence
│   │   ├── session/     # Recording, segments, merge buffer
│   │   ├── project.rs   # Save/load, auto-save, purge, delete
│   │   └── transcription/ # WAV extraction, Whisper CLI
│   └── binaries/        # whisper-cli.exe + DLLs (run download script)
└── docs/
```

## Data Locations

- **Projects**: `%APPDATA%/d-scribe/projects/` (permanent saves)
- **Recent sessions**: `%APPDATA%/d-scribe/projects/recent/` (auto-saved; purged by retention)
- **Models**: `%APPDATA%/d-scribe/models/`
- **Transcription temp**: `%APPDATA%/d-scribe/transcribe_temp/`

## Settings

- **Project name template**: Placeholders `{guild}`, `{channel}`, `{timestamp}`, `{date}`, `{time}` for session IDs and filenames
- **Recent sessions retention (days)**: How long auto-saved sessions are kept (default 10)
- **Segment merge buffer (ms)**: Min silence before splitting segments
- **Playback mode**: Remote, Local, or Both (default Both; persisted)

## Build

```bash
npm run tauri build
```

## Troubleshooting

**Enable debug logging** (to diagnose Discord RPC, transcription, etc.):

- **PowerShell:** `$env:RUST_LOG="d_scribe=debug,wasapi=warn"; npm run tauri dev`
- **Cmd:** `set RUST_LOG=d_scribe=debug,wasapi=warn && npm run tauri dev`

(Plain `RUST_LOG=debug` floods the terminal with WASAPI trace logs.)

**Zero segments after recording:** Segmentation comes from Discord RPC speaking events. Ensure you're connected in Settings and in the voice channel before recording. If you get 0 segments, the RPC subscription or connection may need debugging.

## Planned Features

- **AI summary** – Generate summaries of transcripts using AI/LLM
- **Participant stats** – Word counts and speaking time per participant
- **Meeting notes workflow** – Supported flow for creating structured meeting notes from transcripts
- **Debate/discussion analysis** – Workflow for analyzing debates and discussions (arguments, positions, rebuttals)
- **Custom workflows** – User-defined workflows with configurable LLM instructions (e.g. custom prompts, output formats)
- **Flexible AI/LLM support** – Use both local models (e.g. Ollama, llama.cpp) and remote APIs (OpenAI, Anthropic, etc.) for summarization and other tasks
- **Privacy: mute-aware recording** – Listen for Discord mute events; when the current user or others mute, disable the corresponding local audio stream so muted participants are not recorded

## License

MIT

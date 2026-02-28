# d-scribe

Record Discord voice channel audio and transcribe it with speaker attribution using Whisper.

## Features

- **Capture audio** from Discord (loopback + microphone) via WASAPI on Windows
- **Track speakers** using Discord RPC speaking events
- **Transcribe** segments with [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
- **Export** to SRT or VTT

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

1. Open **Settings** → connect to Discord (Client ID, Client Secret, RPC Origin)
2. Join a voice channel
3. Click **Start Recording**
4. When done, click **Stop Recording**
5. Click **Transcribe** to run Whisper on each segment
6. Export to SRT/VTT if needed

## Project Structure

```
d-scribe/
├── src/                 # React frontend (Vite + TypeScript)
├── src-tauri/
│   ├── src/             # Rust backend
│   │   ├── lib.rs       # Tauri commands, transcription orchestration
│   │   ├── audio/       # WASAPI capture
│   │   ├── discord_rpc/ # Discord RPC + speaking events
│   │   ├── session/     # Recording, segments, merge buffer
│   │   └── transcription/ # WAV extraction, Whisper CLI
│   └── binaries/        # whisper-cli.exe + DLLs (run download script)
└── docs/
```

## Data Locations

- **Projects/sessions**: `%APPDATA%/d-scribe/projects/`
- **Models**: `%APPDATA%/d-scribe/models/`
- **Transcription temp**: `%APPDATA%/d-scribe/transcribe_temp/`

## Build

```bash
npm run tauri build
```

## License

MIT

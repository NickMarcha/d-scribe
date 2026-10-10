# d-scribe

![d-scribe logo](src/assets/d-scribe-logo.svg)

d-scribe records a Discord voice call and turns it into a transcript with a speaker name on every line. It uses Discord RPC to see who is talking and [whisper.cpp](https://github.com/ggml-org/whisper.cpp) to transcribe.

It's a personal project, so expect rough edges. It only runs on Windows for now, because audio capture uses WASAPI. Grab the installer from [Releases](https://github.com/NickMarcha/d-scribe/releases). The app checks for updates when it starts.

## What it does

- Records Discord audio (loopback) and your microphone as two separate WAV files.
- Splits the recording into segments using Discord RPC speaking events, so each segment has a speaker.
- Transcribes each segment with Whisper, either after the call or live while recording.
- Shows word count, speaking time and words per minute for each speaker.
- Exports transcripts to SRT or VTT.
- Auto-saves every session to a recent folder. Recent sessions are deleted after 10 days by default.
- Plays back the remote audio, your local audio, or both, and scrolls the transcript along with playback.
- Lists your projects on the start screen. Click one to open it. When you delete one, you can also delete its audio files.
- Saves your Discord login with a refresh token and reconnects on startup.

## Prerequisites

- Node.js and npm
- Rust (for Tauri)
- Discord desktop app, running (for the RPC connection)
- The Whisper binary, which transcription needs (see below)

## Quick start

### 1. Install dependencies

```bash
npm install
```

### 2. Set up Whisper

On Windows, run the download script:

```powershell
.\src-tauri\binaries\download-whisper.ps1
```

For manual setup, see [src-tauri/binaries/README.md](src-tauri/binaries/README.md).

### 3. Run the app

```bash
npm run tauri dev
```

### 4. Download a Whisper model

Open Settings, expand **Manage models** and download a model such as `base.en`. Models go in `%APPDATA%/d-scribe/models/`. Under **Selected models**, pick which model each language uses for live and regular transcription.

### 5. Connect and record

1. In Settings, expand **Discord**, enter your client ID, client secret and RPC origin, then click **Connect to Discord**. The login is saved, so you usually only authorize once.
2. Join a voice channel.
3. Click **Start recording**, or **Start live recording** to see text appear during the call.
4. Click **Stop recording** when you're done. The session is saved to the recent folder.
5. Click **Play** to listen back. The transcript scrolls with the audio.
6. Click **Transcribe** to run Whisper on each segment.
7. Click **Save project** to move the session out of the recent folder, or **Export SRT** / **Export VTT** to get subtitle files.

## Project structure

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

## Data locations

| What | Where |
|------|-------|
| Saved projects | `%APPDATA%/d-scribe/projects/` |
| Recent sessions (auto-saved, deleted after the retention period) | `%APPDATA%/d-scribe/projects/recent/` |
| Models | `%APPDATA%/d-scribe/models/` |
| Temporary transcription files | `%APPDATA%/d-scribe/transcribe_temp/` |

## Options

- **Project name template.** Set on the start screen. Controls session IDs and filenames. Placeholders are `{guild}`, `{channel}`, `{timestamp}`, `{date}` and `{time}`.
- **Recent sessions retention (days).** How long auto-saved sessions are kept. Default is 10.
- **Segment merge buffer (ms).** Pauses shorter than this stay in the same segment. Default is 1000.
- **Playback mode.** Remote, Local or Both, set in the playback bar. Default is Both, and the app remembers your choice.

## Build

```bash
npm run tauri build
```

## Troubleshooting

To turn on debug logging (for Discord RPC, transcription and so on):

- PowerShell: `$env:RUST_LOG="d_scribe=debug,wasapi=warn"; npm run tauri dev`
- Cmd: `set RUST_LOG=d_scribe=debug,wasapi=warn && npm run tauri dev`

Plain `RUST_LOG=debug` floods the terminal with WASAPI trace logs.

**No segments after recording.** Segments come from Discord RPC speaking events. Connect in Settings and join the voice channel before you start recording. If you still get 0 segments, check the debug log for the RPC connection and subscription.

## Planned features

- Summaries of transcripts using an LLM.
- A meeting notes workflow that turns a transcript into structured notes.
- Debate analysis that pulls out arguments, positions and rebuttals.
- Custom workflows with your own LLM prompts and output formats.
- Support for local models (Ollama, llama.cpp) and remote APIs (OpenAI, Anthropic and others).
- Mute-aware recording. When someone mutes in Discord, the app stops recording their audio stream.

## License

MIT

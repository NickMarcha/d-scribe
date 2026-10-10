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
- Lists every recording by day, with who spoke, where and for how long. Filter by server, channel or person, or search.
- Unsaved recordings are deleted after 10 days or when they use more than 20 GB (both adjustable). Click **Save** on a recording to keep it until you delete it.
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

### 5. Get access to Discord

d-scribe reads who is speaking from your Discord client. Discord only allows this for accounts on an app's tester list, so pick one of these:

- **Use the d-scribe app.** Ask to be added as a tester (Settings > **Discord** has a **Request access** link). Once you're added, accept the invite from Discord and leave the Client ID field empty.
- **Use your own Discord app.** No need to ask anyone:
  1. Open the [Discord Developer Portal](https://discord.com/developers/applications) and click **New Application**.
  2. On the **OAuth2** page, turn on **Public Client**.
  3. On the same page, add `https://localhost` under **Redirects**.
  4. Copy the **Client ID** and paste it in Settings > **Discord**.

  You own the app, so you can connect right away. To let friends use your app too, add them under **App Testers** (up to 50).

d-scribe asks Discord for the `rpc` and `identify` scopes.

### 6. Connect and record

1. In Settings, expand **Discord** and click **Connect to Discord**. Approve the popup in Discord. The login is saved, so you usually only do this once.
2. Join a voice channel. You can do this before or after connecting.
3. Click **Start recording**, or **Start live recording** to see text appear during the call.
4. Click **Stop recording** when you're done. The recording shows up in the list on the Session tab.

   To skip steps 3 and 4, turn on **Record automatically** in Settings > **Recording**. d-scribe then starts recording when you're in a voice channel or call with at least one other person, and stops when you leave or have been alone for a minute.
5. Click **Transcribe** to run Whisper on each segment.
6. Read the transcript in the **Read** view. Click a line to play from there. Fix speakers, split or merge segments and re-transcribe single lines in the **Edit** view. Edits save automatically.
7. Click **Save** to keep the recording until you delete it, or **Export SRT** / **Export VTT** to get subtitle files.

### 7. Summaries and notes (optional)

Settings > **AI** runs a command-line AI tool you already have on the transcript. The default is [Claude Code](https://claude.com/claude-code) (`claude -p` with all tools turned off). Swap in `ollama run llama3.1`, `llm` or `gemini` if you prefer. d-scribe sends the prompt and transcript on stdin and saves what the tool prints with the project. The prompts (summary, meeting notes, action items) are editable, and you can add your own. **Copy transcript** puts the whole transcript on the clipboard for pasting into a chat app.

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
| Unsaved recordings (deleted after the retention period or size limit) | `%APPDATA%/d-scribe/projects/recent/` |
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

- Debate analysis that pulls out arguments, positions and rebuttals. A custom AI prompt gets you most of the way today.
- Mute-aware recording. When someone mutes in Discord, the app stops recording their audio stream.

## License

MIT

# d-scribe

![d-scribe logo](src/assets/d-scribe-logo.svg)

d-scribe records a Discord voice call and turns it into a transcript with a speaker name on every line. It uses Discord RPC to see who is talking and [whisper.cpp](https://github.com/ggml-org/whisper.cpp) to transcribe.

It's a personal project, so expect rough edges. It only runs on Windows for now, because audio capture uses WASAPI. Grab the installer from the [website](https://nickmarcha.github.io/d-scribe/) or [Releases](https://github.com/NickMarcha/d-scribe/releases). The app checks for updates when it starts.

## What it does

- Records Discord's audio and your microphone as two separate WAV files. Only Discord is captured, whatever output device it uses, so music or game sound doesn't end up in the recording.
- Splits the recording into segments using Discord RPC speaking events, so each segment has a speaker.
- Transcribes each segment with Whisper, either after the call or live while recording. Runs on an NVIDIA GPU if you download the GPU engine.
- Handles groups that switch between Norwegian (or another language) and English: each turn is checked for English first, then transcribed with the right model.
- Shows word count, speaking time and words per minute for each speaker.
- Exports transcripts to SRT or VTT.
- Lists every recording by day, with who spoke, where and for how long. Filter by server, channel or person, or search.
- Unsaved recordings are deleted after 10 days or when they use more than 20 GB (both adjustable). Click **Save** on a recording to keep it until you delete it.
- Plays back the remote audio, your local audio, or both, and scrolls the transcript along with playback.
- Lists your projects on the start screen. Click one to open it. Deleting one deletes its folder.
- Saves your Discord login with a refresh token and reconnects on startup.
- Lives in the tray: closing the window keeps it running (and recording automatically). Quit from the tray menu. It can start with Windows.

## Prerequisites

- Node.js and npm
- Rust (for Tauri)
- Discord desktop app, running (for the RPC connection)

## Quick start

### 1. Install dependencies

```bash
npm install
```

### 2. Run the app

```bash
npm run tauri dev
```

### 3. Download a Whisper model

Open Settings, expand **Manage models** and download a model such as `base.en`. Models go in `%APPDATA%/d-scribe/models/`. Under **Selected models**, pick which model each language uses for live and regular transcription.

The Whisper engine itself (whisper.cpp) downloads the first time you transcribe. If you have an NVIDIA graphics card, click **Download GPU engine** under **Manage models**. It's a 650 MB download and makes large models about 10 times faster.

**Norwegian and English.** Add the Norwegian language, download `nb-whisper-large` and `large-v3`, and set the Norwegian row to Regular `nb-whisper-large` and English turns `large-v3`. NB-Whisper is the most accurate model for Norwegian, but it writes English speech as Norwegian. With English turns set, large-v3 checks each turn first and keeps the English ones.

### 4. Get access to Discord

d-scribe reads who is speaking from your Discord client. Discord only allows this for accounts on an app's tester list, so pick one of these:

- **Use the d-scribe app.** Ask to be added as a tester (Settings > **Discord** has a **Request access** link). Once you're added, accept the invite from Discord and leave the Client ID field empty.
- **Use your own Discord app.** No need to ask anyone:
  1. Open the [Discord Developer Portal](https://discord.com/developers/applications) and click **New Application**.
  2. On the **OAuth2** page, turn on **Public Client**.
  3. On the same page, add `https://localhost` under **Redirects**.
  4. Copy the **Client ID** and paste it in Settings > **Discord**.

  You own the app, so you can connect right away. To let friends use your app too, add them under **App Testers** (up to 50).

d-scribe asks Discord for the `rpc` and `identify` scopes.

### 5. Check your audio

Open Settings > **Audio**. Pick the microphone you use in Discord (by default d-scribe takes the Windows default, which may be a different one) and click **Test** while you talk. Leave **Other people in the call** on **Discord app** unless you use Discord in a browser.

### 6. Connect and record

1. In Settings, expand **Discord** and click **Connect to Discord**. Approve the popup in Discord. The login is saved, so you usually only do this once.
2. Join a voice channel. You can do this before or after connecting.
3. Click **Start recording**, or **Start live recording** to see text appear during the call.
4. Click **Stop recording** when you're done. The recording shows up in the list on the Session tab.

   To skip steps 3 and 4, turn on **Record automatically** in Settings > **Recording**. d-scribe then starts recording when you're in a voice channel or call with at least one other person, and stops when you leave or have been alone for a minute.
5. Pick the language next to **Transcribe** and click it. Progress shows in a bar at the bottom of the window, so you can keep using the app meanwhile. Downloads show there too.
6. Read the transcript in the **Read** tab. Click a line to play from there, or drag the slider in the player at the top. Fix speakers, split or merge segments and re-transcribe single lines in the **Edit** tab. Edits save automatically. **Stats** and **AI** are tabs next to them.
7. Click **Save** to keep the recording until you delete it. **More** has subtitle export (SRT, VTT) and Delete.

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
│   │   └── transcription/ # WAV extraction, whisper.cpp engine download and server
└── docs/
```

## Data locations

| What | Where |
|------|-------|
| Recordings, one folder per call (`recording.json`, `others.wav`, `you.wav`). Unsaved ones are deleted after the retention period or size limit | `%APPDATA%/d-scribe/recordings/` |
| Models | `%APPDATA%/d-scribe/models/` |
| Whisper engines (CPU and GPU) | `%APPDATA%/d-scribe/engines/` |
| Temporary transcription files | `%APPDATA%/d-scribe/transcribe_temp/` |

## Options

- **Recording folders** are named by start time and place, for example `2026-10-12 14.32 Call with Haaroon` or `2026-10-12 20.00 Server - General`.
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

[PolyForm Noncommercial 1.0.0](LICENSE.md): free to use, change and share for any noncommercial purpose, keeping the copyright notice. For commercial use, get in touch. Versions before 0.1.17 were published with "MIT" in this README.

Third-party parts keep their own licenses: whisper.cpp and the Whisper models (MIT), NB-Whisper (Apache-2.0).

# d-scribe

![d-scribe logo](src/assets/d-scribe-logo.svg)

d-scribe is a Windows app that records your Discord calls and turns them into a transcript with a speaker name on every line. Transcription runs on your own PC with [whisper.cpp](https://github.com/ggml-org/whisper.cpp).

- **Just want to use it?** Download it from the [website](https://nickmarcha.github.io/d-scribe/) and follow [Using d-scribe](#using-d-scribe) below.
- **Need help, or tester access?** Join the [d-scribe Discord](https://discord.gg/WPvhz8x7SF).
- **Want to build it or change the code?** See [Developing d-scribe](#developing-d-scribe).

It's a personal project, so expect rough edges.

## What it does

- Records the people in your call and your microphone as two separate tracks. Only Discord's sound is captured, so music or game audio stays out.
- Knows who is talking (Discord tells it), so every line of the transcript has a name.
- Transcribes after the call, or live while you talk. Much faster with an NVIDIA graphics card.
- Handles calls that switch between Norwegian (or another language) and English.
- Read, search, fix and re-transcribe the transcript. Click a line to hear it.
- Talk time and words per minute per person, subtitle export (SRT, VTT), and summaries with an AI tool you already use.
- Can record automatically when you join a call with someone, from the tray, and start with Windows.
- Each call is a plain folder on your PC. Unsaved calls are cleaned up after a while; saved ones are kept.

## Using d-scribe

### 1. Install

Download the installer from the [website](https://nickmarcha.github.io/d-scribe/) (or [Releases](https://github.com/NickMarcha/d-scribe/releases)) and run it. You need Windows 10 or 11 and the Discord desktop app.

Windows may say "Windows protected your PC", because the installer isn't signed with a paid certificate. Click **More info**, then **Run anyway**. After that, d-scribe updates itself.

### 2. Get access to Discord

d-scribe reads who is speaking from your Discord app. Discord only allows that for accounts on an app's tester list, so pick one:

- **Use the d-scribe app.** Join the [d-scribe Discord](https://discord.gg/WPvhz8x7SF) and ask in #request-access. When you're added, accept the invite from Discord. Leave the Client ID field in Settings empty.
- **Use your own Discord app.** No waiting, about five minutes:
  1. Open the [Discord Developer Portal](https://discord.com/developers/applications) and click **New Application**.
  2. On the **OAuth2** page, turn on **Public Client**, and add `https://localhost` under **Redirects**.
  3. Copy the **Client ID** and paste it in d-scribe under Settings > **Discord**.

  To let friends use your app too, add them under **App Testers** (up to 50).

d-scribe asks Discord for the `rpc` and `identify` scopes.

### 3. Set up

1. **Connect.** Settings > **Discord** > **Connect to Discord**, then approve in Discord. You only do this once.
2. **Audio.** Settings > **Audio**: pick the microphone you use in Discord and click **Test** while you talk. Leave **Other people in the call** on **Discord app**.
3. **Models.** Settings > **Manage models**: download a model for your language, for example `medium.en` for English or `large-v3` for other languages. On an NVIDIA graphics card, also click **Download GPU engine** (650 MB, about 10 times faster). Under **Selected models**, choose which model each language uses.

**Norwegian and English.** Download `nb-whisper-large` and `large-v3`, and set the Norwegian row to Regular `nb-whisper-large` and English turns `large-v3`. NB-Whisper is the best model for Norwegian but writes English speech as Norwegian; with English turns set, the English parts are kept in English.

### 4. Record and read

1. Join a voice channel or call, and click **Start recording** (or **Start live recording** to see text during the call). Or turn on **Record automatically** in Settings > **Recording**.
2. Click **Stop recording** when you're done. The call appears in the list.
3. Open it, pick the language and click **Transcribe**. Progress shows at the bottom of the window.
4. **Read** shows the transcript: click a line to play from there, and search with Previous/Next. **Edit** fixes words and speakers. **Stats** and **AI** are next to them.
5. Click **Save** to keep the call until you delete it. **More** has rename, show in folder, subtitle export and delete.

**Summaries (optional).** Settings > **AI** runs a command-line AI tool you already have on the transcript, by default [Claude Code](https://claude.com/claude-code) (`claude -p` with tools turned off). `ollama run llama3.1`, `llm` or `gemini` work too. The prompts are editable.

### Where your files are

| What | Where |
|------|-------|
| Recordings, one folder per call (`recording.json`, `others.wav`, `you.wav`) | `%APPDATA%\d-scribe\recordings\` |
| Models and transcription engines | `%APPDATA%\d-scribe\models\`, `%APPDATA%\d-scribe\engines\` |
| Log file | `%APPDATA%\d-scribe\logs\d-scribe.log` |

Settings has buttons to open these folders. Nothing is uploaded unless you set up a remote transcription service or an AI tool. See the [privacy page](https://nickmarcha.github.io/d-scribe/privacy.html).

### Problems

- **No speaker names, or no lines after recording.** Lines come from Discord's "who is speaking" events. Check that Settings > **Discord** says connected before the call starts.
- **Other people are silent in the recording.** Check Settings > **Audio** > **Other people in the call** and use **Test** while someone talks.
- **Anything else.** Ask in #help on the [d-scribe Discord](https://discord.gg/WPvhz8x7SF) or open an [issue](https://github.com/NickMarcha/d-scribe/issues). Mention your d-scribe version (bottom of Settings) and attach the log file if you can.

## Developing d-scribe

d-scribe is a [Tauri 2](https://tauri.app) app: React and TypeScript for the window, Rust for recording, Discord and transcription. It only builds for Windows, because audio capture uses WASAPI.

### Run from source

You need Node.js, Rust and the Discord desktop app.

```bash
npm install
npm run tauri dev
```

The dev build uses the same data folder (`%APPDATA%\d-scribe`) and Discord login as an installed copy, so quit the installed one first.

To build an installer locally: `npm run tauri build`. Releases are built by GitHub Actions (`.github/workflows/publish.yml`) when the `release` branch is pushed, and the website in `site/` deploys from the same branch.

### Debug logging

- PowerShell: `$env:RUST_LOG="d_scribe=debug,wasapi=warn"; npm run tauri dev`
- Cmd: `set RUST_LOG=d_scribe=debug,wasapi=warn && npm run tauri dev`

Plain `RUST_LOG=debug` floods the terminal with WASAPI trace logs.

### Project layout

```
d-scribe/
├── src/                   # React frontend (Vite + TypeScript)
├── src-tauri/src/         # Rust backend
│   ├── lib.rs             # Tauri commands, transcription orchestration
│   ├── audio/             # WASAPI capture
│   ├── discord_rpc/       # Discord RPC, OAuth, token persistence
│   ├── session/           # Recording, segments, merge buffer
│   ├── project.rs         # Recording folders: save, load, purge, delete
│   └── transcription/     # Segment extraction, whisper.cpp engine and server
└── site/                  # Website (GitHub Pages)
```

[AGENTS.md](AGENTS.md) has the conventions, design notes and known gotchas.

### Contributing

Bug reports and ideas are welcome in [issues](https://github.com/NickMarcha/d-scribe/issues) or #feedback on the [Discord](https://discord.gg/WPvhz8x7SF). For bigger code changes, open an issue first so we can agree on the approach.

### Planned

- Debate analysis that pulls out arguments, positions and rebuttals. A custom AI prompt gets you most of the way today.
- Mute-aware recording: when someone mutes in Discord, stop recording their audio.

## License

[PolyForm Noncommercial 1.0.0](LICENSE.md): free to use, change and share for any noncommercial purpose, keeping the copyright notice. For commercial use, get in touch. Versions before 0.1.17 were published with "MIT" in this README.

Third-party parts keep their own licenses: whisper.cpp and the Whisper models (MIT), NB-Whisper (Apache-2.0).

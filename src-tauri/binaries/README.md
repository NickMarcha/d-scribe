# Whisper.cpp Binary (required for transcription)

Transcription uses whisper-cli from whisper.cpp. You must place the binary here before building.

## Quick setup (Windows)

From the project root, run:

```powershell
.\src-tauri\binaries\download-whisper.ps1
```

This downloads whisper-cli and required DLLs. Then run `npm run tauri dev` or build the app.

## Manual setup

### Windows (x64)

1. Go to https://github.com/ggml-org/whisper.cpp/releases
2. Download `whisper-bin-x64.zip`
3. Extract the `Release` folder
4. Copy `whisper-cli.exe` → rename to `whisper-cli-x86_64-pc-windows-msvc.exe`
5. Copy `ggml-base.dll`, `ggml-cpu.dll`, `ggml.dll`, `whisper.dll` to this folder
6. Place all in this `binaries/` folder

Note: `main.exe` is deprecated (exits immediately). Use `whisper-cli.exe` instead.

### Other platforms

Download the appropriate build and rename to match your target triple (`rustc --print host-tuple`).

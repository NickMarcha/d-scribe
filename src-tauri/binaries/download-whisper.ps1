# Download and set up whisper.cpp binary for discord-scribe
# Run from project root: .\src-tauri\binaries\download-whisper.ps1

$ErrorActionPreference = "Stop"
$binDir = $PSScriptRoot
$zipPath = Join-Path $env:TEMP "whisper-bin-x64.zip"
$extractDir = Join-Path $env:TEMP "whisper-bin-x64"
$url = "https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.3/whisper-bin-x64.zip"

Write-Host "Downloading whisper.cpp binary..."
Invoke-WebRequest -Uri $url -OutFile $zipPath -UseBasicParsing

Write-Host "Extracting..."
if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force }
Expand-Archive -Path $zipPath -DestinationPath $extractDir

# Use whisper-cli.exe (main.exe is a deprecation stub that exits immediately)
$cliExe = Get-ChildItem -Path $extractDir -Filter "whisper-cli.exe" -Recurse | Select-Object -First 1
if (-not $cliExe) {
    Write-Error "Could not find whisper-cli.exe in the zip. Contents:"
    Get-ChildItem -Path $extractDir -Recurse
    exit 1
}

$releaseDir = $cliExe.DirectoryName

# Copy whisper-cli.exe
$destExe = Join-Path $binDir "whisper-cli-x86_64-pc-windows-msvc.exe"
Copy-Item $cliExe.FullName -Destination $destExe -Force
Write-Host "Placed: $destExe"

# Copy required DLLs (whisper-cli needs these at runtime)
$dlls = @("ggml-base.dll", "ggml-cpu.dll", "ggml.dll", "whisper.dll")
foreach ($dll in $dlls) {
    $src = Join-Path $releaseDir $dll
    if (Test-Path $src) {
        Copy-Item $src -Destination (Join-Path $binDir $dll) -Force
        Write-Host "  + $dll"
    }
}

Remove-Item $zipPath -Force -ErrorAction SilentlyContinue
Remove-Item $extractDir -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "Done. You can now run 'npm run tauri dev' or build the app."

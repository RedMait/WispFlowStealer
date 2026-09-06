# SPDX-License-Identifier: MIT
# One-file install (AH-01): release build + Desktop shortcut + models.
# Run: powershell -ExecutionPolicy Bypass -File scripts/install.ps1
$ErrorActionPreference = "Stop"
$Repo = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location -LiteralPath $Repo
if ($env:CARGO_TARGET_DIR) { $TargetDir = $env:CARGO_TARGET_DIR } else { $TargetDir = Join-Path $Repo "target" }
$env:CARGO_TARGET_DIR = $TargetDir

powershell -ExecutionPolicy Bypass -NoProfile -File (Join-Path $Repo "scripts\get-native.ps1")
cargo build --release -p flowvoice --features audio,gui
if ($LASTEXITCODE -ne 0) { throw "build failed" }
powershell -ExecutionPolicy Bypass -NoProfile -File (Join-Path $Repo "scripts\stamp-icon.ps1")

$exe = Join-Path $TargetDir "release\flowvoice.exe"
$ws = New-Object -ComObject WScript.Shell
$lnk = $ws.CreateShortcut((Join-Path ([Environment]::GetFolderPath("Desktop")) "flowvoice.lnk"))
$lnk.TargetPath = $exe
$lnk.Arguments = "--gui"
$lnk.WorkingDirectory = $Repo
$lnk.IconLocation = "$exe,0"
$lnk.Description = "flowvoice - hold-to-talk dictation"
$lnk.Save()

Write-Host "installed: Desktop shortcut -> $exe --gui"
Write-Host "settings/history live in %APPDATA%\WispFlowStealer (kept across updates)"

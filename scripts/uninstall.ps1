# SPDX-License-Identifier: MIT
# One-command uninstall (AH-06): stops the app, removes the shortcut.
# Settings/history in %APPDATA%\WispFlowStealer are KEPT by default;
# pass -WipeData to erase them too (AH-10: asks via explicit flag).
# A removal report is printed at the end (AH-11).
param([switch]$WipeData)
$ErrorActionPreference = "Stop"

$removed = @()
Get-Process flowvoice -ErrorAction SilentlyContinue | Stop-Process -Force
$removed += "process: stopped running instances"

$desk = Join-Path ([Environment]::GetFolderPath("Desktop")) "flowvoice.lnk"
if (Test-Path -LiteralPath $desk) {
    Remove-Item -LiteralPath $desk -Force
    $removed += "shortcut: Desktop\flowvoice.lnk"
}
$repoLnk = Join-Path (Get-Location) "flowvoice-GUI.lnk"
if (Test-Path -LiteralPath $repoLnk) {
    Write-Host "note: repo-local flowvoice-GUI.lnk left in place (gitignored)"
}

if ($WipeData) {
    $data = Join-Path $env:APPDATA "WispFlowStealer"
    if (Test-Path -LiteralPath $data) {
        Remove-Item -LiteralPath $data -Recurse -Force
        $removed += "data: %APPDATA%\WispFlowStealer (history, journal, settings)"
    }
} else {
    $removed += "data: kept %APPDATA%\WispFlowStealer (re-run with -WipeData to erase)"
}

Write-Host "== removal report =="
$removed | ForEach-Object { Write-Host ("  " + $_) }
Write-Host "hotkey hook and tray icon die with the process; no autostart or services are installed"

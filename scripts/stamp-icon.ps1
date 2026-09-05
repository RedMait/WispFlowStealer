# Stamps assets/flowvoice.ico into the release exe (repeat after every
# release rebuild, since linking drops the icon resource).
# Refuses to run while the app is alive (Windows locks the running exe).
# rcedit is fetched on demand into native/tools (gitignored).

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Repo = Split-Path -Parent $ScriptDir
$Exe = Join-Path "W:\opencode\target" "release\flowvoice.exe"
$Icon = Join-Path $Repo "assets\flowvoice.ico"
$Tools = Join-Path $Repo "native\tools"
$Rcedit = Join-Path $Tools "rcedit-x64.exe"
$RceditUrl = "https://github.com/electron/rcedit/releases/latest/download/rcedit-x64.exe"

if (Get-Process flowvoice -ErrorAction SilentlyContinue) {
    throw "flowvoice is running - quit it first (tray menu), then re-run"
}
foreach ($f in @($Exe, $Icon)) {
    if (-not (Test-Path -LiteralPath $f)) { throw "missing: $f" }
}
if (-not (Test-Path -LiteralPath $Rcedit)) {
    New-Item -ItemType Directory -Force -Path $Tools | Out-Null
    & curl.exe -fSL --retry 3 -o $Rcedit -- $RceditUrl
    if ($LASTEXITCODE -ne 0) { throw "rcedit download failed" }
}

# Stamp a copy, then move over the original: rcedit cannot always commit
# changes in place (locked/network paths), but copy+move is reliable.
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) "flowvoice-icon.exe"
Copy-Item -LiteralPath $Exe -Destination $tmp -Force
& $Rcedit $tmp --set-icon $Icon
if ($LASTEXITCODE -ne 0) { throw "rcedit failed" }
Move-Item -LiteralPath $tmp -Destination $Exe -Force
Write-Host "icon stamped: $Exe"

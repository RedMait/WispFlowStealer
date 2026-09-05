# Double-click launcher for the flowvoice GUI (no terminal typing needed).
# A companion start-gui.bat runs this with a single double-click.
# Uses the repo-relative release binary when present, otherwise falls back
# to `cargo run` (first launch after a fresh build).
# Any failure is logged next to this script and the window pauses,
# so an error is never hidden behind a flashing console.

$ErrorActionPreference = "Stop"

$Repo = Split-Path -Parent $MyInvocation.MyCommand.Path
$Log = Join-Path $Repo "start-gui.log"

try {
    Set-Location -LiteralPath $Repo
    $env:CARGO_TARGET_DIR = "W:\opencode\target"

    $exe = Join-Path "W:\opencode\target" "release\flowvoice.exe"
    if (Test-Path -LiteralPath $exe) {
        # Detached: no console window lingers; quit via the tray menu.
        Start-Process -FilePath $exe -ArgumentList "--gui" -WorkingDirectory $Repo
    } else {
        cargo run -p flowvoice --features audio,gui -- --gui
    }
} catch {
    $msg = "[{0}] {1}" -f (Get-Date -Format "yyyy-MM-dd HH:mm:ss"), $_.Exception.Message
    Add-Content -LiteralPath $Log -Value $msg
    Write-Host $msg -ForegroundColor Red
    Write-Host "See start-gui.log next to this script."
    pause
    exit 1
}

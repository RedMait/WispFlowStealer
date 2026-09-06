# SPDX-License-Identifier: MIT
# Reproducibility check (A-05): two clean release builds into different
# directories must produce bit-identical flowvoice.exe files.
# Run: powershell -ExecutionPolicy Bypass -File scripts/verify-repro.ps1
$ErrorActionPreference = "Stop"
$Repo = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location -LiteralPath $Repo
$base = Join-Path ([System.IO.Path]::GetTempPath()) "flowvoice-repro"
$first = Join-Path $base "a"
$second = Join-Path $base "b"
foreach ($d in @($first, $second)) {
    if (Test-Path -LiteralPath $d) { Remove-Item -LiteralPath $d -Recurse -Force }
}
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_TARGET_DIR = $first
cargo build --locked --release -p flowvoice --features audio,gui
if ($LASTEXITCODE -ne 0) { throw "first build failed" }
$env:CARGO_TARGET_DIR = $second
cargo build --locked --release -p flowvoice --features audio,gui
if ($LASTEXITCODE -ne 0) { throw "second build failed" }
$h1 = (Get-FileHash -LiteralPath (Join-Path $first "release\flowvoice.exe") -Algorithm SHA256).Hash
$h2 = (Get-FileHash -LiteralPath (Join-Path $second "release\flowvoice.exe") -Algorithm SHA256).Hash
Remove-Item -LiteralPath $base -Recurse -Force
if ($h1 -ne $h2) { throw "REPRODUCIBILITY FAIL: $h1 vs $h2" }
Write-Host "reproducible: $h1"

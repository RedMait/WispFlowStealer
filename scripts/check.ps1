# SPDX-License-Identifier: MIT
# One-command local check (T-16): fmt + clippy (default and audio) + tests.
# Run: powershell -ExecutionPolicy Bypass -File scripts/check.ps1
$ErrorActionPreference = "Stop"
$Repo = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location -LiteralPath $Repo
if ($env:CARGO_TARGET_DIR) { $TargetDir = $env:CARGO_TARGET_DIR } else { $TargetDir = Join-Path $Repo "target" }
$env:CARGO_TARGET_DIR = $TargetDir
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw "fmt failed" }
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw "clippy failed" }
cargo test --workspace
if ($LASTEXITCODE -ne 0) { throw "test failed" }
cargo clippy -p flowvoice --all-targets --features audio -- -D warnings
if ($LASTEXITCODE -ne 0) { throw "clippy(audio) failed" }
cargo test -p flowvoice --features audio
if ($LASTEXITCODE -ne 0) { throw "test(audio) failed" }
Write-Host "check: all green"

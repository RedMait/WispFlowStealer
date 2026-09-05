# Downloads the runtime assets needed by the audio (mic dictation) mode:
#   * native/vosk.dll  - Vosk speech recognition library (Windows x64)
#   * models/ru        - Russian small model
#   * models/en        - English small model
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts/get-native.ps1

$ErrorActionPreference = "Stop"

# Windows PowerShell 5.1 defaults to TLS 1.0; force a modern protocol.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo      = Split-Path -Parent $PSScriptRoot
$NativeDir = Join-Path $Repo "native"
$ModelsDir = Join-Path $Repo "models"
$VoskDll   = Join-Path $NativeDir "vosk.dll"
$Base      = "https://alphacephei.com/vosk/models"
$VoskUrl   = "https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-win64-0.3.45.zip"

New-Item -ItemType Directory -Force -Path $NativeDir, $ModelsDir | Out-Null

function Save-Zip([string]$Url, [string]$Zip) {
    if (-not (Test-Path $Zip)) {
        Write-Host "  downloading $([System.IO.Path]::GetFileName($Zip)) ..."
        try {
            Invoke-WebRequest -Uri $Url -OutFile $Zip
        } catch {
            Remove-Item -LiteralPath $Zip -ErrorAction SilentlyContinue
            throw
        }
    }
}

function Add-Model([string]$ZipName, [string]$Dest) {
    if (Test-Path $Dest) {
        Write-Host "  already present: $Dest"
        return
    }
    $zip     = Join-Path $ModelsDir $ZipName
    $extract = Join-Path $ModelsDir ([System.IO.Path]::GetFileNameWithoutExtension($ZipName))
    Save-Zip "$Base/$ZipName" $zip
    Write-Host "  extracting $ZipName ..."
    Expand-Archive -Path $zip -DestinationPath $ModelsDir
    if (Test-Path $extract) { Move-Item -LiteralPath $extract -Destination $Dest }
    Write-Host "  model ready at $Dest"
}

Write-Host "== vosk.dll =="
if (-not (Test-Path $VoskDll)) {
    $zip    = Join-Path $NativeDir "vosk-win64-0.3.45.zip"
    $srcDir = Join-Path $NativeDir "vosk-win64-0.3.45"
    Save-Zip $VoskUrl $zip
    Write-Host "  extracting $([System.IO.Path]::GetFileName($zip))"
    if (Test-Path $srcDir) { Remove-Item -LiteralPath $srcDir -Recurse -Force }
    Expand-Archive -Path $zip -DestinationPath $NativeDir
    $libdll = Get-ChildItem -LiteralPath $srcDir -Filter "libvosk.dll" | Select-Object -First 1
    if (-not $libdll) { throw "libvosk.dll not found in the archive" }
    Copy-Item -LiteralPath $libdll.FullName -Destination $VoskDll
    foreach ($dep in "libgcc_s_seh-1.dll", "libstdc++-6.dll", "libwinpthread-1.dll") {
        $runc = Join-Path $srcDir $dep
        if (Test-Path $runc) { Copy-Item -LiteralPath $runc -Destination (Join-Path $NativeDir $dep) }
    }
    Write-Host "  vosk.dll ready (with mingw runtime deps)"
    Remove-Item -LiteralPath $srcDir -Recurse -Force
    Remove-Item -LiteralPath $zip -ErrorAction SilentlyContinue
} else {
    Write-Host "  already present: $VoskDll"
}

Write-Host "== models =="
Add-Model "vosk-model-small-ru-0.22.zip"  (Join-Path $ModelsDir "ru")
Add-Model "vosk-model-small-en-us-0.15.zip" (Join-Path $ModelsDir "en")

Write-Host ""
Write-Host "Done."
Write-Host "  native = $VoskDll"
Write-Host "  models = $ModelsDir"
Write-Host ""
Write-Host "Run:  cargo run --release -p flowvoice --features audio"
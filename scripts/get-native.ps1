# Downloads the runtime assets needed by the audio (mic dictation) mode:
#   * native/vosk.dll  - Vosk speech recognition library (Windows x64)
#   * models/ru        - Russian small model
#   * models/en        - English small model
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts/get-native.ps1

$ErrorActionPreference = "Stop"

$Repo      = Split-Path -Parent $PSScriptRoot
$NativeDir = Join-Path $Repo "native"
$ModelsDir = Join-Path $Repo "models"
$VoskDll   = Join-Path $NativeDir "vosk.dll"
$Base      = "https://alphacephei.com/vosk/models"

New-Item -ItemType Directory -Force -Path $NativeDir, $ModelsDir | Out-Null

function Save-Zip([string]$Url, [string]$Zip) {
    if (-not (Test-Path $Zip)) {
        Write-Host "  downloading $([System.IO.Path]::GetFileName($Zip)) ..."
        Invoke-WebRequest -Uri $Url -OutFile $Zip
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
    $zip = Join-Path $NativeDir "vosk-win64-0.3.45.zip"
    Save-Zip "$Base/vosk-win64-0.3.45.zip" $zip
    Write-Host "  extracting $([System.IO.Path]::GetFileName($zip))"
    Expand-Archive -Path $zip -DestinationPath $NativeDir
    $dll = Get-ChildItem -LiteralPath $NativeDir -Recurse -Filter "vosk.dll" | Select-Object -First 1
    if (-not $dll) { throw "vosk.dll not found in the archive" }
    Move-Item -LiteralPath $dll.FullName -Destination $VoskDll -Force
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
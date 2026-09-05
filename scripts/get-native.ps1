# Downloads the runtime assets needed by the audio (mic dictation) mode:
#   * native/vosk.dll  - Vosk speech recognition library (Windows x64)
#   * models/ru        - FULL Russian model vosk-model-ru-0.42 (~1.8 GB, WER 4.5).
#                        The old small model (small-ru-0.22, WER 22.71) recognized
#                        poorly, so the full model is now the default.
#   * models/en        - English small model (small-en-us-0.15, ~40 MB)
#   * models/punct/    - RUPunct_small punctuation model for Russian:
#                          rupunct_small_int8.onnx + tokenizer.json
#                        (~30 MB) from Hugging Face.
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts/get-native.ps1
#
# Notes:
#   * Downloads use curl.exe (Invoke-WebRequest TLS is broken in this env).
#   * Run the app from the repo root so the relative defaults `models/ru`
#     and `models/punct` resolve. Keep the checkout at an ASCII-only path:
#     Kaldi cannot open model paths with Cyrillic characters.

$ErrorActionPreference = "Stop"

$Repo      = Split-Path -Parent $PSScriptRoot
$NativeDir = Join-Path $Repo "native"
$ModelsDir = Join-Path $Repo "models"
$VoskDll   = Join-Path $NativeDir "vosk.dll"
$Base      = "https://alphacephei.com/vosk/models"
$VoskUrl   = "https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-win64-0.3.45.zip"
$PunctBase = "https://huggingface.co/ekhodzitsky/rupunct-small-onnx/resolve/main"

New-Item -ItemType Directory -Force -Path $NativeDir, $ModelsDir | Out-Null

function Save-Url([string]$Url, [string]$Out) {
    if (Test-Path $Out) { return }
    Write-Host "  downloading $([System.IO.Path]::GetFileName($Out)) ..."
    & curl.exe -fSL --retry 3 -o $Out -- $Url
    if ($LASTEXITCODE -ne 0) {
        Remove-Item -LiteralPath $Out -ErrorAction SilentlyContinue
        throw "curl failed for $Url (exit $LASTEXITCODE)"
    }
}

function Add-Model([string]$ZipName, [string]$Dest) {
    if (Test-Path $Dest) {
        Write-Host "  already present: $Dest"
        return
    }
    $zip     = Join-Path $ModelsDir $ZipName
    $extract = Join-Path $ModelsDir ([System.IO.Path]::GetFileNameWithoutExtension($ZipName))
    Save-Url "$Base/$ZipName" $zip
    Write-Host "  extracting $ZipName ..."
    Expand-Archive -Path $zip -DestinationPath $ModelsDir
    if (Test-Path $extract) { Move-Item -LiteralPath $extract -Destination $Dest }
    Write-Host "  model ready at $Dest"
}

function Add-PunctFile([string]$FileName, [string]$DestDir) {
    $dest = Join-Path $DestDir $FileName
    if (Test-Path $dest) {
        Write-Host "  already present: $dest"
        return
    }
    New-Item -ItemType Directory -Force -Path $DestDir | Out-Null
    Save-Url "$PunctBase/$FileName" $dest
    Write-Host "  punct file ready at $dest"
}

Write-Host "== vosk.dll =="
if (-not (Test-Path $VoskDll)) {
    $zip    = Join-Path $NativeDir "vosk-win64-0.3.45.zip"
    $srcDir = Join-Path $NativeDir "vosk-win64-0.3.45"
    Save-Url $VoskUrl $zip
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
Add-Model "vosk-model-ru-0.42.zip"  (Join-Path $ModelsDir "ru")
Add-Model "vosk-model-small-en-us-0.15.zip" (Join-Path $ModelsDir "en")

$RuDir = Join-Path $ModelsDir "ru"
if (Test-Path $RuDir) {
    Write-Host "  NOTE: if $RuDir still holds the old small-ru-0.22 model,"
    Write-Host "  delete it and re-run this script to upgrade to vosk-model-ru-0.42."
}

Write-Host "== punct =="
$PunctDir = Join-Path $ModelsDir "punct"
Add-PunctFile "rupunct_small_int8.onnx" $PunctDir
Add-PunctFile "tokenizer.json" $PunctDir

Write-Host ""
Write-Host "Done."
Write-Host "  native = $VoskDll"
Write-Host "  models = $ModelsDir"
Write-Host ""
Write-Host "Run:  cargo run --release -p flowvoice --features audio"
Write-Host "Env:  FLOWVOICE_MODEL (default models/ru), FLOWPUNCT_MODEL (default models/punct)"

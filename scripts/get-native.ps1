# Downloads the runtime assets needed by the audio (mic dictation) mode:
#   * native/vosk.dll  - Vosk speech recognition library (Windows x64, fallback)
#   * models/ru        - FULL Russian Vosk model vosk-model-ru-0.42 (~1.8 GB,
#                        WER 4.5). Fallback only; Whisper below is the default.
#   * models/en        - English small Vosk model (small-en-us-0.15, ~40 MB)
#   * models/punct/    - RUPunct_small punctuation model for Russian (~30 MB)
#   * native/whisper/  - whisper-server.exe + DLLs (prebuilt whisper.cpp
#                        v1.9.2 CPU; v1.9.3 shipped no binaries, hence the pin)
#   * models/whisper/  - ggml-large-v3-turbo.bin (~1.5 GB). Default STT engine:
#                        far bigger Russian vocabulary than Vosk.
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts/get-native.ps1
#
# Notes:
#   * Downloads use curl.exe (Invoke-WebRequest TLS is broken in this env).
#   * Run the app from the repo root so the relative defaults resolve.
#     Keep the checkout at an ASCII-only path (Kaldi/Vosk cannot open
#     Cyrillic model paths; whisper handles them, Vosk does not).

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

Write-Host "== whisper (default STT) =="
$WhisperVer   = "v1.9.2"
$WhisperUrl   = "https://github.com/ggml-org/whisper.cpp/releases/download/$WhisperVer/whisper-bin-x64.zip"
$WhisperDir   = Join-Path $NativeDir "whisper"
$WhisperExe   = Join-Path $WhisperDir "whisper-server.exe"
$WhisperModelDir = Join-Path $ModelsDir "whisper"
$WhisperModel = Join-Path $WhisperModelDir "ggml-large-v3-turbo.bin"
$WhisperModelUrl = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin"

if (-not (Test-Path $WhisperExe)) {
    $zip = Join-Path $NativeDir "whisper-bin-x64.zip"
    $tmp = Join-Path $NativeDir "whisper-tmp"
    Save-Url $WhisperUrl $zip
    Write-Host "  extracting $([System.IO.Path]::GetFileName($zip)) ..."
    if (Test-Path $tmp) { Remove-Item -LiteralPath $tmp -Recurse -Force }
    Expand-Archive -Path $zip -DestinationPath $tmp
    $exe = Get-ChildItem -LiteralPath $tmp -Filter "whisper-server.exe" -Recurse | Select-Object -First 1
    if (-not $exe) { throw "whisper-server.exe not found in the archive" }
    New-Item -ItemType Directory -Force -Path $WhisperDir | Out-Null
    Copy-Item -LiteralPath $exe.FullName -Destination $WhisperExe
    foreach ($dll in Get-ChildItem -LiteralPath $exe.DirectoryName -Filter "*.dll") {
        Copy-Item -LiteralPath $dll.FullName -Destination (Join-Path $WhisperDir $dll.Name)
    }
    $cli = Get-ChildItem -LiteralPath $tmp -Filter "whisper-cli.exe" -Recurse | Select-Object -First 1
    if ($cli) { Copy-Item -LiteralPath $cli.FullName -Destination (Join-Path $WhisperDir "whisper-cli.exe") }
    Remove-Item -LiteralPath $tmp -Recurse -Force
    Remove-Item -LiteralPath $zip -ErrorAction SilentlyContinue
    Write-Host "  whisper-server ready at $WhisperExe"
} else {
    Write-Host "  already present: $WhisperExe"
}
New-Item -ItemType Directory -Force -Path $WhisperModelDir | Out-Null
if (-not (Test-Path $WhisperModel)) {
    Write-Host "  downloading ggml-large-v3-turbo.bin (~1.5 GB, takes a while) ..."
}
Save-Url $WhisperModelUrl $WhisperModel
Write-Host "  whisper model ready at $WhisperModel"

Write-Host ""
Write-Host "Done."
Write-Host "  native = $VoskDll"
Write-Host "  models = $ModelsDir"
Write-Host ""
Write-Host "Run:  cargo run --release -p flowvoice --features audio"
Write-Host "Tip:  set GROQ_API_KEY for cloud whisper (fastest), local engines stay as fallback"
Write-Host "Env:  FLOWVOICE_MODEL (vosk dir, fallback), FLOWPUNCT_MODEL (default models/punct)"
Write-Host "      FLOWVOICE_WHISPER_MODEL (default models/whisper/ggml-large-v3-turbo.bin)"
Write-Host "      FLOWVOICE_WHISPER_BIN (default native/whisper/whisper-server.exe)"
Write-Host "      FLOWVOICE_WHISPER_PORT (default 8178), FLOWVOICE_LANG (default ru)"

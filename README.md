# WispFlowStealer

Open-source clone of [Wispr Flow](https://getflow.ai): hold a hotkey, speak,
release, and get clean, auto-formatted text inserted where your cursor is.

Zero-cost, dependency-light, **fully offline** voice dictation for Windows,
with automatic capitalization, sentence type detection (`!? .`), heuristic
commas, removal of filler words and punctuation/whitespace cleanup. Supports
Russian and English, with an optional neural punctuation model for Russian.

```
hold Right Ctrl  ->  speak  ->  release  ->  "эм я тут подумал что можно как бы..."
                                               becomes
                                               "Я тут подумал, что можно..."
                                               pasted into the active window
```

## Features

- **Hold-to-talk** dictation via a low-level keyboard hook (native WinAPI).
- **Auto-formatting engine** (`flowcore`) that works on any text — also exposed
  as a standalone CLI tool, so the text pipeline is fully testable without a mic.
- **Filler word removal** (Russian *эм, ну, вот, как бы, типа, такое* ... and
  English *uh, um, like, you know, basically* ...).
- **Sentence type detection**: statement `.`, question `?`, exclamation `!`.
- **Heuristic commas**: subordinate clauses (*что, чтобы, потому что, который*,
  *because, although, but* ...) and mid-sentence inserts (*например, конечно же,
  к сожалению* ...) get commas without any model.
- **Neural punctuation for Russian** (`flowpunct`, behind the `audio` build):
  RUPunct_small INT8 ONNX run by the pure-Rust `tract-onnx` runtime — no DLLs,
  no network, no Python. E.g. *шестьдесят тысяч тенге сколько будет стоить*
  becomes *Шестьдесят тысяч тенге, сколько будет стоить?* When the model files
  are absent, Russian falls back to the heuristic pipeline.
- **Fully offline** speech recognition with [Whisper](https://github.com/ggml-org/whisper.cpp)
  (`large-v3-turbo`, ~1.5 GB) served by a resident local `whisper-server` —
  far bigger Russian vocabulary than Vosk, with punctuation and casing out
  of the box. Vosk stays as an automatic fallback when the Whisper files
  are absent (full RU model `vosk-model-ru-0.42` / small EN model).
- **Hermetic default build**: `cargo build`/`cargo test` compile everywhere with
  no audio/native dependencies; the mic mode (+ neural punctuation) is behind
  the `audio` feature.

## Quickstart

### 1. Formatting engine (works on any OS, no mic needed)

```sh
cargo test --workspace        # flowcore: 26 tests, flowpunct: 3 tests, all green
cargo run --release -p flowfmt -- "эм ну я тут подумал"
#   Я тут подумал.
```

### 2. Pipeline demo (+ clipboard copy on Windows)

```sh
cargo run --release -p flowvoice -- --demo "эм ну то есть мы выиграем супер конкурс"
#   flowvoice demo
#     raw:       "эм ну то есть мы выиграем супер конкурс"
#     language:  ru
#     sentence:  exclamation (!)
#     formatted: То есть мы выиграем супер конкурс!
```

### 3. Real microphone dictation (Windows only)

```sh
powershell -ExecutionPolicy Bypass -File scripts/get-native.ps1
#   whisper-server + large-v3-turbo model (~1.5 GB) into native/whisper/,
#   models/whisper/; vosk.dll + vosk models (fallback) + RUPunct files (~30 MB)
cargo run --release -p flowvoice --features audio
```

Then hold **Right Ctrl** anywhere, speak, release. The formatted text is put
into the clipboard and pasted via Ctrl+V.

At startup the app preloads the models in the background (`[ready] ...`
lines); the Whisper server stays resident and is reused between restarts,
so every press records immediately.

Speech backend resolution (in order):
1. Groq Cloud — `whisper-large-v3-turbo` over HTTPS (needs `GROQ_API_KEY`).
   Fastest and most accurate; key at https://console.groq.com/keys, keep it
   in the process env only, never in the repo. Override model with
   `FLOWVOICE_GROQ_MODEL` (`whisper-large-v3` for accuracy-first).
2. Local Whisper — `native/whisper/whisper-server.exe` +
   `models/whisper/ggml-large-v3-turbo.bin` (overrides:
   `FLOWVOICE_WHISPER_BIN`, `FLOWVOICE_WHISPER_MODEL`,
   `FLOWVOICE_WHISPER_PORT` default `8178`). Fully offline fallback.
3. Vosk fallback — `models/ru` (override with `FLOWVOICE_MODEL`).
4. None present → error telling you to run the script above.

`FLOWVOICE_LANG` (default `ru`) sets the recognition language for Groq and
the local server; use `en` (or `auto` locally) for non-Russian dictation.

Punctuation model: `models/punct/` with `rupunct_small_int8.onnx` +
`tokenizer.json` (override with `FLOWPUNCT_MODEL`). If the punct files are
missing, Russian dictation uses the deterministic heuristic formatter
instead — no error, just commas from rules rather than the network.

### Path caveats (important)

- Run the app **from the repo root** (`cargo run` from the root sets the
  working directory to the root), so the relative defaults `models/ru`,
  `models/punct` and `native/vosk.dll` resolve.
- Keep the checkout at an **ASCII-only path**: the Kaldi/Vosk backend cannot
  open model paths with Cyrillic characters, so the relative `models/ru`
  default is intentional — do not point `FLOWVOICE_MODEL` at a path with `Ё`,
  `й`, etc.

### Hotkeys

`flowvoice [--key <NAME>]` where `NAME` is `RCONTROL` (default), `F7`, `F8` or
`F9`.

## Architecture

Rust workspace monorepo:

| Crate | Purpose |
|-------|---------|
| `crates/flowcore` | Text engine: tokenizer, filler detection, sentence classification, heuristic commas, `format()` / `format_raw()` / `clean()`. No dependencies. |
| `crates/punct` (`flowpunct`) | Neural punctuation: RUPunct_small token classifier on `tract-onnx` (pure Rust). Built only with the `onnx` feature; unit-tested without the model files. |
| `crates/flowcore/src/bin/flowfmt.rs` | CLI for the formatting pipeline (stdin / args / `--lang`). |
| `crates/app` (bin `flowvoice`) | Hotkey hook (WinAPI), audio capture (`cpal`), STT (resident `whisper-server` over localhost HTTP, Vosk fallback), clipboard paste (`arboard`). Whisper transcripts already carry punctuation, so they go `format()` (fillers + heuristic commas); Vosk transcripts go `clean()` → `Punctuator::punct()` for RU, fallback to `format()`. |

The audio stack (`cpal`, `vosk`, `arboard`, `flowpunct`+`tract`) lives behind
the `audio` feature, so the default build stays environment-agnostic and
CI-friendly.

`native/vosk-sys` is a small vendored fork of the `vosk-sys` bindings that loads
`vosk.dll` lazily at runtime (LoadLibraryExW with search flags + absolute path)
instead of at process start — the demo mode works even on machines without the
native library.

## Project layout

```
crates/flowcore/          text formatting engine + flowfmt CLI + tests
crates/punct/             RUPunct neural punctuation (flowpunct crate)
crates/app/               flowvoice binary (hook, audio, whisper/vosk STT, paste)
native/vosk-sys/          vendored FFI bindings, runtime-loaded vosk.dll
native/whisper/           whisper-server.exe + DLLs (downloaded, gitignored)
models/whisper/           ggml-large-v3-turbo.bin (downloaded, gitignored)
models/punct/             rupunct_small_int8.onnx + tokenizer.json (downloaded, gitignored)
scripts/get-native.ps1    downloads vosk.dll, full ru / small en models, punct files
.github/workflows/ci.yml  fmt + clippy + test + release build (Windows & Linux)
```

## License

MIT

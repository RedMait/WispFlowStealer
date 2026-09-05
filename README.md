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
- **Fully offline** speech recognition with [Vosk](https://alphacephei.com/vosk/).
  Russian uses the **full** model `vosk-model-ru-0.42` (~1.8 GB, WER 4.5) —
  the old small model (`small-ru-0.22`, WER 22.71) recognized poorly. English
  uses the small model (~40 MB).
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
#   vosk.dll + full RU model (~1.8 GB) + small EN model (~40 MB)
#   + RUPunct files (~30 MB) into models/punct/
cargo run --release -p flowvoice --features audio
```

Then hold **Right Ctrl** anywhere, speak, release. The formatted text is put
into the clipboard and pasted via Ctrl+V.

Models are resolved from `models/ru` (override with `FLOWVOICE_MODEL`) and
`models/punct/` with `rupunct_small_int8.onnx` + `tokenizer.json` (override
with `FLOWPUNCT_MODEL`). If the punct files are missing, Russian dictation
uses the deterministic heuristic formatter instead — no error, just commas
from rules rather than the network.

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
| `crates/app` (bin `flowvoice`) | Hotkey hook (WinAPI), audio capture (`cpal`), STT (`vosk`), clipboard paste (`arboard`). With `audio` it also wires `flowpunct`: RU transcripts go `clean()` → `Punctuator::punct()`, fallback to `format()`. |

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
crates/app/               flowvoice binary (hook, audio, STT, paste)
native/vosk-sys/          vendored FFI bindings, runtime-loaded vosk.dll
models/punct/             rupunct_small_int8.onnx + tokenizer.json (downloaded, gitignored)
scripts/get-native.ps1    downloads vosk.dll, full ru / small en models, punct files
.github/workflows/ci.yml  fmt + clippy + test + release build (Windows & Linux)
```

## License

MIT

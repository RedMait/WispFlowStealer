# WispFlowStealer

Open-source clone of [Wispr Flow](https://getflow.ai): hold a hotkey, speak,
release, and get clean, auto-formatted text inserted where your cursor is.

Zero-cost, dependency-light, **fully offline** voice dictation for Windows,
with automatic capitalization, sentence type detection (`!? .`), removal of
filler words and punctuation/whitespace cleanup. Supports Russian and English.

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
- **Fully offline** speech recognition with [Vosk](https://alphacephei.com/vosk/)
  (small Russian and English models, ~40 MB each).
- **Hermetic default build**: `cargo build`/`cargo test` compile everywhere with
  no audio/native dependencies; the mic mode is behind the `audio` feature.

## Quickstart

### 1. Formatting engine (works on any OS, no mic needed)

```sh
cargo test --workspace        # 23 tests, all green
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
powershell -ExecutionPolicy Bypass -File scripts/get-native.ps1   # vosk.dll + ru/en models (~120 MB)
cargo run --release -p flowvoice --features audio
```

Then hold **Right Ctrl** anywhere, speak, release. The formatted text is put
into the clipboard and pasted via Ctrl+V.

Models are resolved from `models/ru` (override with `FLOWVOICE_MODEL`). The
native `vosk.dll` is looked up in the exe directory / PATH and then in
`native/`.

### Hotkeys

`flowvoice [--key <NAME>]` where `NAME` is `RCONTROL` (default), `F7`, `F8` or
`F9`.

## Architecture

Rust workspace monorepo:

| Crate | Purpose |
|-------|---------|
| `crates/flowcore` | Text engine: tokenizer, filler detection, sentence classification, `format()` / `format_raw()`. No dependencies. |
| `crates/flowcore/src/bin/flowfmt.rs` | CLI for the formatting pipeline (stdin / args / `--lang`). |
| `crates/app` (bin `flowvoice`) | Hotkey hook (WinAPI), audio capture (`cpal`), STT (`vosk`), clipboard paste (`arboard`). |

The audio stack (`cpal`, `vosk`, `arboard`) lives behind the `audio` feature, so
the default build stays environment-agnostic and CI-friendly.

`native/vosk-sys` is a small vendored fork of the `vosk-sys` bindings that loads
`vosk.dll` lazily at runtime (LoadLibrary/GetProcAddress) instead of at process
start — the demo mode works even on machines without the native library.

## Project layout

```
crates/flowcore/          text formatting engine + flowfmt CLI + tests
crates/app/               flowvoice binary (hook, audio, STT, paste)
native/vosk-sys/          vendored FFI bindings, runtime-loaded vosk.dll
scripts/get-native.ps1    downloads vosk.dll and ru/en models
.github/workflows/ci.yml  fmt + clippy + test + release build (Windows & Linux)
```

## License

MIT
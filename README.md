# WispFlowStealer — MIT-licensed offline voice dictation for Windows

Open-source clone of [Wispr Flow](https://getflow.ai): hold a hotkey, speak,
release, and get clean, auto-formatted text inserted where your cursor is.

Hybrid speech stack: Groq Cloud whisper when a key is configured, resident
local Whisper and Vosk as fully-offline fallbacks. No telemetry, no accounts,
history and journal stay on your machine. License: MIT (see `LICENSE`;
third-party notices in `THIRD-PARTY.md`).

```
hold Right Ctrl  ->  speak  ->  release  ->  "эм я тут подумал что можно как бы..."
                                               becomes
                                               "Я тут подумал, что можно..."
                                               pasted into the active window
```

## Features

- **Hold-to-talk** dictation via a low-level keyboard hook (native WinAPI),
  plus a pause toggle (window + tray) that never closes the UI.
- **Desktop GUI** (`--gui`, eframe): status, settings, history with search,
  statistics, recording overlay pill, system-tray icon.
- **Auto-formatting engine** (`flowcore`) that works on any text — also exposed
  as a standalone CLI tool, so the text pipeline is fully testable without a mic.
- **Filler word removal** (Russian *эм, ну, вот, как бы, типа, такое* ... and
  English *uh, um, like, you know, basically* ...).
- **Sentence type detection**: statement `.`, question `?`, exclamation `!`,
  per-sentence punctuation and capitalization, glued-mark repair (`.,` → `.`).
- **Heuristic commas**: subordinate clauses (*что, чтобы, потому что, который*,
  *because, although, but* ...) and mid-sentence inserts (*например, конечно же,
  к сожалению* ...) get commas without any model.
- **Neural punctuation for Russian** (`flowpunct`): RUPunct_small INT8 ONNX on
  the pure-Rust `tract-onnx` runtime for utterances over 10 words.
- **Voice commands**: `переведи:`, `сократи:`, `замени X на Y`, `перепиши`,
  `отмени`, `транслит:` (see `flowvoice --help`).
- **Post profiles**: chat / mail / code, auto-detected from the active window.
- **History + journal**: every replica stored locally with app, timings and
  WPM; JSONL/CSV export; per-replica JSON journal lines (schema: `docs/JOURNAL.md`).
- **Fail-safe**: clipboard is restored after paste; second instance exits with
  a message; corrupt settings/history fall back to defaults; crash leftovers
  are reported on next start.
- **Hermetic default build**: `cargo build`/`cargo test` compile everywhere with
  no audio/native dependencies; mic + GUI live behind `audio`/`gui` features.

## Quickstart

### 1. Formatting engine (works on any OS, no mic needed)

```sh
cargo test --workspace
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
#   whisper-server + large-v3-turbo model (~1.5 GB), vosk.dll + vosk models
#   (fallback), RUPunct files (~30 MB)
cargo run --release -p flowvoice --features audio,gui -- --gui
```

Or one-command install (build + models + Desktop shortcut): `scripts/install.ps1`.
Uninstall: `scripts/uninstall.ps1` (keeps `%APPDATA%\WispFlowStealer` unless
`-WipeData`). Local checker: `scripts/check.ps1` (fmt + clippy + tests).

Then hold **Right Ctrl** anywhere, speak, release. The formatted text is put
into the clipboard and pasted via Ctrl+V (your previous clipboard content is
restored ~400 ms later).

At startup the app preloads the models in the background (`[ready] ...`
lines); the Whisper server stays resident and is reused between restarts,
so every press records immediately. Each paste logs `[timing] Ns keyup->paste`.

Speech backend resolution (in order):
1. Groq Cloud — `whisper-large-v3-turbo` over HTTPS (needs `GROQ_API_KEY`,
   or `flowvoice --set-key <KEY>` for the Windows Credential Manager store).
   Override model with `FLOWVOICE_GROQ_MODEL` (`whisper-large-v3` for
   accuracy-first). Domain words via `FLOWVOICE_GROQ_PROMPT`.
2. Local Whisper — `native/whisper/whisper-server.exe` +
   `models/whisper/ggml-large-v3-turbo.bin` (overrides:
   `FLOWVOICE_WHISPER_BIN`, `FLOWVOICE_WHISPER_MODEL`,
   `FLOWVOICE_WHISPER_PORT` default `8178` +2 fallback, `FLOWVOICE_THREADS`).
   Fully offline fallback.
3. Vosk fallback — `models/ru` (override with `FLOWVOICE_MODEL`).
4. None present → error telling you to run the script above.

`FLOWVOICE_LANG` (default `ru`, or `en`/`auto`) sets the recognition language
and disables auto-detection when fixed. `FLOWVOICE_BACKEND` pins one engine.
`FLOWVOICE_PROFILE` selects the post profile (`auto|chat|mail|code`).
`FLOWVOICE_RAW=1` returns the bare transcript (post-processing off).
`FLOWVOICE_NO_HISTORY=1` is the privacy mode (no history, no journal).

Punctuation model: `models/punct/` with `rupunct_small_int8.onnx` +
`tokenizer.json` (override with `FLOWPUNCT_MODEL`). Short texts (≤10 words)
always use the deterministic rules.

### Settings reference (config file + env override the file)

| Setting | Config key | Env | Default |
|---|---|---|---|
| Hotkey | `hotkey` | `--key` | `RCONTROL` |
| Language | `lang` | `FLOWVOICE_LANG` | `ru` |
| Groq model | `groq_model` | `FLOWVOICE_GROQ_MODEL` | `whisper-large-v3-turbo` |
| Chat model (commands) | – | `FLOWVOICE_CHAT_MODEL` | `openai/gpt-oss-20b` |
| Backend | `backend` | `FLOWVOICE_BACKEND` | `auto` |
| Profile | `profile` | `FLOWVOICE_PROFILE` | `auto` |
| Sounds | `sound` | `FLOWVOICE_SOUND` | on |
| Vocab hint | – | `FLOWVOICE_GROQ_PROMPT` | – |
| Vosk model dir | – | `FLOWVOICE_MODEL` | `models/ru` |
| Punct model dir | – | `FLOWPUNCT_MODEL` | `models/punct` |
| Mic substring | – | `FLOWVOICE_DEVICE` / `--device` | default device |
| Raw transcript | – | `FLOWVOICE_RAW` / `--raw` | off |
| Privacy mode | – | `FLOWVOICE_NO_HISTORY` | off |
| Paste delay, ms | – | `FLOWVOICE_PASTE_DELAY_MS` | `0` |
| Clipboard restore, ms | – | `FLOWVOICE_RESTORE_MS` | `400` |
| Groq timeout, s | – | `FLOWVOICE_TIMEOUT_S` | `180` |

Config file: `%APPDATA%\WispFlowStealer\config.json` (GUI edits it).
CLI run flags mirror the env names: `--model/--lang/--backend/--device/--raw`,
plus `--file/--dir/--stats/--set-key/--version/--json/--save/--timestamps`.

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
`F9`. GUI pause toggle (window + tray) freezes dictation without closing.

## Architecture (4 blocks)

1. **Capture** (`crates/app/src/audio.rs`, `win.rs`): low-level keyboard hook
   (pure state machine in `hotkey.rs`, unit-tested), mic stream via `cpal`
   (device selectable), 16 kHz mono resampling, WAV file/stdin input.
2. **Recognition** (`groq.rs`, `whisper.rs`, vendored `vosk`): Groq Cloud
   (curl, TLS by the OS), resident local `whisper-server` on 127.0.0.1,
   cached Vosk model; priority table in `backend.rs`, file uploads for
   wav/mp3/m4a/ogg/flac/webm, per-segment times for `--timestamps`.
3. **Post-processing** (`crates/flowcore`, `crates/punct`, `command.rs`):
   fillers, repeats, sentence kind, heuristic commas, mark repair,
   per-sentence casing, profiles (chat/mail/code), voice commands
   (translate/summarize/replace/rewrite/undo/transliterate), neural RU
   punctuation for long texts, clipboard-safe paste with restore.
4. **Delivery & UI** (`win.rs` paste, `gui.rs`, `state.rs`, `journal.rs`):
   clipboard + Ctrl+V, recording overlay, tray, settings/history/statistics
   windows, per-replica JSONL journal (`docs/JOURNAL.md`), error log with
   rotation, single-instance guard, crash-recovery marker.

Rust workspace monorepo:

| Crate | Purpose |
|-------|---------|
| `crates/flowcore` | Text engine: tokenizer, filler detection, sentence classification, heuristic commas, mark repair, profiles, transliteration; `format()` / `format_raw()` / `format_code()` / `clean()`. No dependencies. |
| `crates/punct` (`flowpunct`) | Neural punctuation: RUPunct_small token classifier on `tract-onnx` (pure Rust). Built only with the `onnx` feature; unit-tested without the model files. |
| `crates/flowcore/src/bin/flowfmt.rs` | CLI for the formatting pipeline (stdin / args / `--lang`). |
| `crates/app` (bin `flowvoice`) | Hook, capture, STT backends, commands, paste, GUI, journal. Audio stack (`cpal`, `vosk`, `arboard`, `flowpunct`+`tract`) behind `audio`; GUI (`eframe`, `tray-icon`, `chrono`) behind `gui`. |

`native/vosk-sys` is a small vendored fork of the `vosk-sys` bindings that loads
`vosk.dll` lazily at runtime (LoadLibraryExW with search flags + absolute path)
instead of at process start — the demo mode works even on machines without the
native library.

## Offline mode, privacy, network

- Fully offline chain: local Whisper or Vosk + heuristic/ML punctuation —
  see “Speech backend resolution”; force it with `FLOWVOICE_BACKEND=local`
  (or `vosk`) and no `GROQ_API_KEY`.
- Telemetry: none exists in the codebase. No accounts, no analytics.
- Outgoing network (only when you enable it): `api.groq.com` (STT/chat, key
  required), plus one-time downloads `github.com` (binaries),
  `huggingface.co` (models), `alphacephei.com` (Vosk) via
  `scripts/get-native.ps1` (curl, SHA256-recorded, resumable).
- Secrets: `GROQ_API_KEY` lives in process env or Windows Credential Manager
  (`flowvoice/GROQ_API_KEY`), never in the repo, configs, logs or journal
  (verified: `git log -S gsk_` is empty).
- Local files (`%APPDATA%\WispFlowStealer`, owner-only ACL best-effort):
  `config.json`, `history.json`, `journal.jsonl`, `errors.log`.

## Model & data licenses (code is MIT)

- whisper.cpp + `ggml-large-v3-turbo.bin` (via Hugging Face `ggerganov/whisper.cpp`): MIT / Apache-2.0 family — see the model repo.
- Vosk models (`alphacephei.com`): Apache-2.0.
- RUPunct weights: see the source repo (`ekhodzitsky/rupunct-small-onnx`).
- Model weights are never committed (`/models` is gitignored); exact
  dependency tree with licenses: `THIRD-PARTY.md` (regenerate:
  `scripts/gen-third-party.ps1`); compatibility gate: `cargo deny check licenses`.

## Project layout

```
crates/flowcore/          text formatting engine + flowfmt CLI + tests
crates/punct/             RUPunct neural punctuation (flowpunct crate)
crates/app/               flowvoice binary (hook, audio, STT, commands, paste, GUI, journal)
native/vosk-sys/          vendored FFI bindings, runtime-loaded vosk.dll
assets/flowvoice.ico      app icon (stamped by scripts/stamp-icon.ps1 after release builds)
models/whisper/           ggml-large-v3-turbo.bin (downloaded, gitignored)
models/punct/             rupunct_small_int8.onnx + tokenizer.json (downloaded, gitignored)
scripts/get-native.ps1    downloads binaries, models (resumable, SHA256-verified)
scripts/install.ps1       one-file install (build + models + Desktop shortcut)
scripts/uninstall.ps1     one-command removal (+report, optional -WipeData)
scripts/check.ps1         local checker: fmt + clippy + tests (default and audio)
scripts/stamp-icon.ps1    stamps the icon into the release exe
docs/JOURNAL.md           per-replica journal schema
.github/workflows/ci.yml  fmt + clippy + test + release build (Windows & Linux),
                          license gate, coverage artifact; no stubs allowed
```

## License

MIT

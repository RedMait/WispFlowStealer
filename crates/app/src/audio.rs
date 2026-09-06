// SPDX-License-Identifier: MIT
//! Microphone capture plus fully local, offline speech-to-text.
//!
//! Only compiled for Windows with the `audio` feature enabled.
//!
//! Flow: capture mono PCM at the device's native sample rate -> resample to
//! 16 kHz -> feed the Vosk recognizer -> return the recognized text.

use cpal::traits::DeviceTrait;
use flowcore::Language;
use flowpunct::Punctuator;
use vosk::{CompleteResult, Model, Recognizer};

use std::path::PathBuf;
use std::sync::OnceLock;

const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Model path override via the `FLOWVOICE_MODEL` env var.
/// Defaults to `models/ru` because the Russian dictation demo needs it.
fn default_model_path() -> String {
    std::env::var("FLOWVOICE_MODEL").unwrap_or_else(|_| "models/ru".to_string())
}

/// Vosk model shared by every dictation, loaded exactly once.
///
/// Reloading it per hotkey press cost seconds (the full RU model is ~1.8 GB)
/// and — worse — the microphone only opened *after* the load, so the start
/// of the utterance was silently lost. Warmed up by [`preload`].
static MODEL: OnceLock<Result<Model, String>> = OnceLock::new();

fn model_instance() -> Result<&'static Model, String> {
    MODEL
        .get_or_init(|| {
            let path = default_model_path();
            Model::new(path.clone()).ok_or_else(|| format!("cannot load vosk model from `{path}`"))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// Load the heavy models in the background at startup so the first hotkey
/// press starts recording immediately instead of hanging on model load.
/// Backend chain: Groq Cloud (needs `GROQ_API_KEY`) -> resident local
/// whisper-server -> Vosk fallback; narrowed by the GUI/backend preference.
pub fn preload() {
    std::thread::spawn(|| {
        let pref = pref();
        if pref.allows_groq() && crate::groq::available() {
            crate::state::emit("[ready] groq backend (cloud whisper)");
            set_label("groq cloud");
        } else if pref.allows_local() && crate::whisper::available() {
            match crate::whisper::ensure_server() {
                Ok(p) => {
                    crate::state::emit(&format!("[ready] whisper server on 127.0.0.1:{p}"));
                    set_label("whisper local");
                }
                Err(e) => crate::state::emit(&format!("[whisper] {e} (vosk fallback)")),
            }
        } else if pref.allows_vosk() && model_instance().is_ok() {
            crate::state::emit("[ready] speech model loaded (vosk fallback)");
            set_label("vosk");
        } else {
            crate::state::emit(
                "[audio] no speech backend: set GROQ_API_KEY or run scripts/get-native.ps1",
            );
        }
        if punct_instance().is_some() {
            crate::state::emit("[ready] punctuation model loaded");
        }
    });
}

/// Active backend preference: GUI setting when attached, else
/// `FLOWVOICE_BACKEND` env, else auto.
fn pref() -> crate::state::BackendPref {
    if let Some(s) = crate::state::get() {
        if let Ok(p) = s.backend_pref.lock() {
            return *p;
        }
    }
    crate::state::BackendPref::from_env().unwrap_or(crate::state::BackendPref::Auto)
}

fn set_label(label: &str) {
    if let Some(s) = crate::state::get() {
        s.set_backend_label(label);
    }
}

/// Directory holding the RUPunct punctuation model, or `None` when absent.
/// Override via the `FLOWPUNCT_MODEL` env var.
fn punct_dir() -> PathBuf {
    std::env::var_os("FLOWPUNCT_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|cwd| cwd.join("models").join("punct"))
                .unwrap_or_else(|_| PathBuf::from("models/punct"))
        })
}

/// Lazily load the neural punctuator; `None` when the model is not installed.
fn punct_instance() -> Option<&'static Punctuator> {
    static PUNCT: OnceLock<Option<Punctuator>> = OnceLock::new();
    PUNCT
        .get_or_init(|| {
            let dir = punct_dir();
            let onnx = dir.join("rupunct_small_int8.onnx");
            let tokenizer = dir.join("tokenizer.json");
            if !onnx.exists() || !tokenizer.exists() {
                return None;
            }
            Punctuator::load(&onnx.to_string_lossy(), &tokenizer.to_string_lossy()).ok()
        })
        .as_ref()
}

/// Post-process the raw transcript: neural punctuation for Russian when the
/// model is installed, otherwise the deterministic heuristic pipeline.
/// A fixed language setting (`FLOWVOICE_LANG=ru|en`) disables auto-detect.
/// Profile (`FLOWVOICE_PROFILE`) selects Chat/Mail/Code shaping.
fn finalize(text: String) -> String {
    let explicit = std::env::var("FLOWVOICE_LANG").ok();
    let lang = Language::resolve(explicit.as_deref(), &text);
    let profile = resolve_profile();
    let opts = flowcore::FormatOpts {
        numbers_words: numbers_words(),
    };
    // Chat replicas stay short and heuristic; Code keeps identifiers.
    if profile == flowcore::Profile::Chat {
        return flowcore::format_with(&text, lang, opts);
    }
    if profile == flowcore::Profile::Code {
        return flowcore::format_code(&text);
    }
    if lang == Language::Ru {
        if let Some(punct) = punct_instance() {
            let cleaned = flowcore::clean(&text, lang);
            // Short texts go through deterministic rules instead of the
            // neural model (J-09): less latency, fewer surprises.
            if flowcore::word_count(&cleaned) <= 10 {
                return flowcore::format_with(&text, lang, opts);
            }
            return punct
                .punct(&cleaned)
                .unwrap_or_else(|_| flowcore::format_with(&text, lang, opts));
        }
    }
    flowcore::format_with(&text, lang, opts)
}

/// One-shaped error context lives in [`crate::util::xerr`].
use crate::util::xerr;

/// `FLOWVOICE_NUMBERS=words` spells digit runs in Russian (F-10).
fn numbers_words() -> bool {
    std::env::var("FLOWVOICE_NUMBERS")
        .map(|v| v.eq_ignore_ascii_case("words"))
        .unwrap_or(false)
}

/// Raw mode (`FLOWVOICE_RAW=1`): skip all post-processing, return the bare
/// transcript. Also the off switch for post-processing (J-06).
fn is_raw() -> bool {
    crate::util::env_flag("FLOWVOICE_RAW")
}

/// Capture until the hotkey is released, recognize, return final text.
///
/// The microphone opens first so no speech is lost; the Vosk model is
/// cached (see [`preload`]), and Whisper runs through a resident server,
/// so neither backend reloads gigabytes per press.
///
/// Post-processing depends on the backend: Whisper (local or Groq Cloud)
/// already restores punctuation and casing, so it only needs filler
/// cleanup + heuristic commas ([`flowcore::format`]); Vosk outputs bare
/// words and goes through the full [`finalize`] pipeline (neural
/// punctuation for RU when present).
/// VAD amplitude threshold (`FLOWVOICE_VAD_LEVEL`, default 200): samples
/// quieter than this count as silence for trimming (E-01/E-02/E-04).
fn vad_level() -> i16 {
    std::env::var("FLOWVOICE_VAD_LEVEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
}

/// Strip leading/trailing silence (E-01/E-02). All-silent input is kept
/// as-is: the engines report it better than an empty buffer does.
fn trim_silence(pcm: &[i16]) -> Vec<i16> {
    let level = vad_level().abs() as i32;
    let mut start = 0;
    while start < pcm.len() && (pcm[start] as i32).abs() < level {
        start += 1;
    }
    let mut end = pcm.len();
    while end > start && (pcm[end - 1] as i32).abs() < level {
        end -= 1;
    }
    if start >= end {
        return pcm.to_vec();
    }
    pcm[start..end].to_vec()
}

/// Optional noise gate (`FLOWVOICE_NOISE_GATE=1`): zero sub-threshold
/// samples before recognition (C-16). Off by default.
fn noise_gate(pcm: &mut [i16]) {
    let on = std::env::var("FLOWVOICE_NOISE_GATE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !on {
        return;
    }
    let level = vad_level().abs() as i32;
    for s in pcm.iter_mut() {
        if (*s as i32).abs() < level {
            *s = 0;
        }
    }
}

pub fn transcribe() -> Result<String, String> {
    let (pcm, rate) = capture_until_stop()?;
    // Accidental taps (<200 ms hold) never reach STT: no empty replicas,
    // no wasted API calls (D-10).
    let min_hold_ms = crate::util::env_u64("FLOWVOICE_MIN_HOLD_MS", 200);
    if let Some(t0) = crate::win::press_instant() {
        if !crate::hotkey::meets_min_hold(t0.elapsed().as_millis() as u64, min_hold_ms) {
            return Ok(String::new());
        }
    }
    let mut pcm16k = resample_to_16k(&pcm, rate);
    noise_gate(&mut pcm16k);
    let pcm16k = trim_silence(&pcm16k);
    let pref = pref();

    // Pure priority table (unit-tested in `backend::select`).
    let engine = crate::backend::select(
        pref,
        crate::groq::available(),
        crate::whisper::available(),
        // Probing the multi-GB Vosk model per press would defeat caching;
        // assume present here and let `model_instance` fail loudly instead.
        true,
    )
    .map_err(|e| e.to_string())?;
    set_label(engine.label());

    match engine {
        crate::backend::Backend::Groq => {
            let raw = crate::groq::transcribe_pcm(&pcm16k)?;
            Ok(apply_commands(format_segments(strip_stage_directions(raw))))
        }
        crate::backend::Backend::Local => {
            let raw = crate::whisper::transcribe_pcm(&pcm16k)?;
            Ok(apply_commands(format_segments(strip_stage_directions(raw))))
        }
        crate::backend::Backend::Vosk => {
            let model = model_instance()?;
            let raw = recognize_with(model, &pcm16k)?;
            if is_raw() {
                return Ok(raw);
            }
            // The neural punctuator may glue marks across words; collapse runs.
            Ok(apply_commands(flowcore::collapse_punctuation(&finalize(
                raw,
            ))))
        }
    }
}

/// Drop bracketed stage directions without digits (`[музыка]`, `(смеётся)`)
/// that recognizers sprinkle into transcripts (AM-19). Spans with digits
/// (`(712) 555-01-01`) are kept: they may be phone numbers.
fn strip_stage_directions(raw: Vec<String>) -> Vec<String> {
    raw.into_iter().map(|seg| strip_brackets(&seg)).collect()
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let open = chars[i];
        let close = match open {
            '[' => ']',
            '(' => ')',
            _ => {
                out.push(open);
                i += 1;
                continue;
            }
        };
        if let Some(rel) = chars[i..].iter().position(|&c| c == close) {
            let span: String = chars[i + 1..i + rel].iter().collect();
            if span.chars().any(|c| c.is_ascii_digit()) {
                out.push_str(&span_surround(open, &span, close));
            } else if span.trim().is_empty() {
                out.push(open);
                out.push(close);
            }
            // Digit-free spans vanish (the direction itself is dropped).
            i += rel + 1;
        } else {
            out.push(open);
            i += 1;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn span_surround(open: char, span: &str, close: char) -> String {
    format!("{open}{span}{close}")
}

/// Voice commands (`переведи:`, `сократи:`, `замени`, `перепиши`, `отмени`,
/// `транслит:`): transform instead of dictating. Raw mode skips them along
/// with all other post-processing.
fn apply_commands(text: String) -> String {
    if is_raw() || text.is_empty() {
        return text;
    }
    // Voice macros first: an exact phrase presses keys instead of
    // dictating anything (AJ-01/AJ-02/AJ-03).
    if let Some(m) = run_macro(&text) {
        return m;
    }
    let Some(cmd) = crate::command::parse(&text) else {
        return text;
    };
    match cmd {
        crate::command::Command::Transliterate { text } => flowcore::transliterate_ru(&text),
        crate::command::Command::Replace { from, to } => {
            let prev = last_replica_text();
            let next = prev.replacen(&from, &to, 1);
            if next == prev {
                return text;
            }
            set_undo_slot(prev);
            next
        }
        crate::command::Command::Undo => match take_undo_slot() {
            Some(prev) => prev,
            None => text,
        },
        crate::command::Command::Translate { target, text } => {
            match crate::groq::chat(
                &format!(
                    "Translate the following text to language code '{target}'. Return only the translation, no quotes, no commentary."
                ),
                &text,
            ) {
                Ok(t) => {
                    set_undo_slot(last_replica_text());
                    t
                }
                Err(e) => format!("{text} [команда не выполнена: {e}]"),
            }
        }
        crate::command::Command::Summarize { text } => {
            match crate::groq::chat(
                "Сократи следующий текст, сохранив смысл. Верни только сокращённый текст, без кавычек и комментариев.",
                &text,
            ) {
                Ok(t) => {
                    set_undo_slot(last_replica_text());
                    t
                }
                Err(e) => format!("{text} [{e}]"),
            }
        }
        crate::command::Command::Rewrite => {
            let prev = last_replica_text();
            if prev.is_empty() {
                return text;
            }
            match crate::groq::chat(
                "Перепиши следующий текст другими словами, сохранив смысл. Верни только переписанный текст.",
                &prev,
            ) {
                Ok(t) => {
                    set_undo_slot(prev);
                    t
                }
                Err(e) => format!("{text} [{e}]"),
            }
        }
    }
}

/// Active post-processing profile: explicit setting wins, otherwise the
/// foreground app decides (J-04/J-05), defaulting to Mail.
fn resolve_profile() -> flowcore::Profile {
    let pref = flowcore::Profile::parse(&std::env::var("FLOWVOICE_PROFILE").unwrap_or_default());
    pref.resolve(&crate::win::foreground_title())
}

/// Format every whisper segment as its own sentence and join them.
/// Segments track utterance boundaries, so each one gets its own terminal
/// mark ("…скрипт. Хочу…"); filler-only fragments format to empty and drop.
/// Raw mode returns the segments untouched.
fn format_segments(raw: Vec<String>) -> String {
    if is_raw() {
        return raw.join(" ");
    }
    if resolve_profile() == flowcore::Profile::Code {
        return raw
            .into_iter()
            .map(|seg| flowcore::format_code(&seg))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    let explicit = std::env::var("FLOWVOICE_LANG").ok();
    let opts = flowcore::FormatOpts {
        numbers_words: numbers_words(),
    };
    raw.into_iter()
        .map(|seg| {
            let lang = flowcore::Language::resolve(explicit.as_deref(), &seg);
            flowcore::format_with(&seg, lang, opts)
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run the cached Vosk model over buffered audio, return the raw transcript.
fn recognize_with(model: &Model, pcm16k: &[i16]) -> Result<String, String> {
    let mut recognizer = Recognizer::new(model, TARGET_SAMPLE_RATE as f32)
        .ok_or_else(|| "cannot create vosk recognizer".to_string())?;

    for chunk in pcm16k.chunks(TARGET_SAMPLE_RATE as usize * 2) {
        let _ = recognizer.accept_waveform(chunk);
    }

    let completed = recognizer.final_result();
    Ok(transcript(&completed))
}

/// Last finalized replica (for `замени`/`перепиши` context).
fn last_replica_text() -> String {
    crate::state::get()
        .and_then(|s| s.last_text.lock().ok().map(|t| t.clone()))
        .unwrap_or_default()
}

/// Voice macros from `%APPDATA%/WispFlowStealer/macros.json`
/// (`[{"phrase": "сохрани", "keys": "ctrl+s"}]`, AJ-02). Missing or broken
/// file simply means no macros.
fn load_macros() -> Vec<crate::macros::Macro> {
    let path = crate::state::app_dir().join("macros.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    value
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            Some(crate::macros::Macro {
                phrase: m.get("phrase")?.as_str()?.to_string(),
                keys: m.get("keys")?.as_str()?.to_string(),
            })
        })
        .collect()
}

/// Exact-phrase macro execution: press the keys, paste nothing.
fn run_macro(text: &str) -> Option<String> {
    let macros = load_macros();
    if macros.is_empty() {
        return None;
    }
    let hit = crate::macros::find_match(&macros, text)?;
    let strokes = crate::macros::parse_combo(&hit.keys)?;
    crate::win::send_combo(&strokes);
    crate::state::emit(&format!("[macro] {} -> {}", hit.phrase, hit.keys));
    Some(String::new())
}

/// Stash pre-edit text for `отмени` (J-15).
fn set_undo_slot(prev: String) {
    if let Some(s) = crate::state::get() {
        if let Ok(mut slot) = s.undo_slot.lock() {
            *slot = Some(prev);
        }
    }
}

fn take_undo_slot() -> Option<String> {
    crate::state::get().and_then(|s| s.undo_slot.lock().ok().and_then(|mut slot| slot.take()))
}

fn transcript(result: &CompleteResult<'_>) -> String {
    match result {
        CompleteResult::Single(single) => single.text.trim().to_string(),
        CompleteResult::Multiple(multi) => multi
            .alternatives
            .first()
            .map(|a| a.text.trim().to_string())
            .unwrap_or_default(),
    }
}

/// Case-insensitive substring match for `--device`/`FLOWVOICE_DEVICE`.
fn device_matches(name: &str, want: &str) -> bool {
    name.to_lowercase().contains(&want.to_lowercase())
}

/// Microphone failure, translated into a next step (X-10/X-11): access
/// denials name the Windows privacy toggle instead of dumping drivers.
fn mic_help(detail: &str) -> String {
    let low = detail.to_lowercase();
    if low.contains("denied")
        || low.contains("permission")
        || low.contains("access")
        || low.contains("unavailable")
        || low.contains("no default")
        || low.contains("not found")
    {
        return format!(
            "{detail}. Разрешите микрофон: Параметры Windows → Конфиденциальность → Микрофон → доступ для классических приложений, затем нажмите хоткей снова"
        );
    }
    detail.to_string()
}

/// Transcribe one audio file without a microphone (N-01..N-05, Y-01).
/// `path` may be `-` for stdin bytes (Y-08). Groq accepts wav/mp3/m4a/
/// ogg/flac/webm; the local/Vosk engines only take WAV (converted to
/// 16 kHz mono on the fly, N-06/N-07).
pub fn transcribe_file(path: &str, opts: &crate::FileOpts) -> Result<String, String> {
    let (bytes, fname) = read_audio_input(path)?;
    if bytes.is_empty() {
        return Err("empty audio input (0 bytes)".to_string());
    }
    let pref = pref();
    let engine = crate::backend::select(
        pref,
        crate::groq::available(),
        crate::whisper::available(),
        true,
    )
    .map_err(|e| e.to_string())?;
    set_label(engine.label());

    let spans = match engine {
        crate::backend::Backend::Groq => {
            let tmp = stage_temp(&bytes, &fname)?;
            let out = crate::groq::transcribe_file(&tmp);
            let _ = std::fs::remove_file(&tmp);
            out?
        }
        crate::backend::Backend::Local | crate::backend::Backend::Vosk => {
            if !is_wav_name(&fname) {
                return Err(format!(
                    "local engines need WAV; set GROQ_API_KEY for {fname} or convert it first"
                ));
            }
            let (pcm, rate) = crate::whisper::decode_wav(&bytes)?;
            let mut pcm16k = resample_to_16k(&pcm, rate);
            noise_gate(&mut pcm16k);
            let pcm16k = trim_silence(&pcm16k);
            match engine {
                crate::backend::Backend::Local => crate::whisper::transcribe_pcm(&pcm16k)?
                    .into_iter()
                    .map(|text| crate::whisper::Span {
                        text,
                        start: 0.0,
                        end: 0.0,
                    })
                    .collect(),
                _ => {
                    let model = model_instance()?;
                    let raw = recognize_with(model, &pcm16k)?;
                    vec![crate::whisper::Span {
                        text: raw,
                        start: 0.0,
                        end: 0.0,
                    }]
                }
            }
        }
    };
    // Total duration for timestamp fallback (local WAV only).
    let total_secs = if is_wav_name(&fname) {
        crate::whisper::decode_wav(&bytes)
            .ok()
            .map(|(pcm, rate)| pcm.len() as f32 / rate.max(1) as f32)
    } else {
        None
    };
    let out = render_spans(&spans, opts, total_secs);
    if opts.save && path != "-" {
        let txt = std::path::Path::new(path).with_extension("txt");
        xerr(
            std::fs::write(&txt, &out),
            &format!("cannot write {}", txt.display()),
        )?;
    }
    Ok(out)
}

/// Batch-transcribe a directory of audio files (N-09, Y-09).
/// (N-13), joined with spaces. When the engine gives no times but the total
/// duration is known (local WAV), times are split proportionally by length.
fn render_spans(
    spans: &[crate::whisper::Span],
    opts: &crate::FileOpts,
    total_secs: Option<f32>,
) -> String {
    let explicit = std::env::var("FLOWVOICE_LANG").ok();
    let mut parts = Vec::with_capacity(spans.len());
    let need_times = opts.timestamps
        && spans.iter().all(|s| s.end <= s.start)
        && total_secs.unwrap_or(0.0) > 0.0;
    let total_chars: usize = spans.iter().map(|s| s.text.chars().count().max(1)).sum();
    let mut cursor = 0.0f32;
    for span in spans {
        let formatted = if is_raw() {
            span.text.clone()
        } else {
            let lang = flowcore::Language::resolve(explicit.as_deref(), &span.text);
            flowcore::format_with(
                &span.text,
                lang,
                flowcore::FormatOpts {
                    numbers_words: numbers_words(),
                },
            )
        };
        if formatted.trim().is_empty() {
            continue;
        }
        let (start, end) = if span.end > span.start {
            (span.start, span.end)
        } else if need_times {
            let share = total_secs.unwrap_or(0.0) * span.text.chars().count().max(1) as f32
                / total_chars.max(1) as f32;
            let s = cursor;
            cursor += share;
            (s, cursor)
        } else {
            (0.0, 0.0)
        };
        if opts.timestamps && end > start {
            parts.push(format!(
                "[{:02}:{:04.1}] {formatted}",
                (start / 60.0) as u32,
                start % 60.0
            ));
        } else {
            parts.push(formatted);
        }
    }
    parts.join(" ")
}

/// Batch-transcribe a directory of audio files (N-09, Y-09).
/// Progress goes to stderr (Y-15); each result is printed and, with
/// `--save`, stored next to its input (N-12). One bad file never stops
/// the batch: it is reported and skipped.
pub fn transcribe_dir(dir: &str, opts: &crate::FileOpts) -> Result<usize, String> {
    const EXTS: &[&str] = &["wav", "mp3", "m4a", "ogg", "flac", "webm"];
    let mut files: Vec<std::path::PathBuf> =
        xerr(std::fs::read_dir(dir), &format!("cannot list {dir}"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| EXTS.contains(&x.to_ascii_lowercase().as_str()))
                    .unwrap_or(false)
            })
            .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no audio files in {dir}"));
    }
    let mut done = 0usize;
    for (i, path) in files.iter().enumerate() {
        eprintln!("[{}/{}] {}", i + 1, files.len(), path.display());
        match transcribe_file(&path.to_string_lossy(), opts) {
            Ok(out) => {
                done += 1;
                println!("{out}");
            }
            Err(e) => eprintln!("skip {}: {e}", path.display()),
        }
    }
    Ok(done)
}

fn read_audio_input(path: &str) -> Result<(Vec<u8>, String), String> {
    if path == "-" {
        use std::io::Read as _;
        let mut buf = Vec::new();
        xerr(std::io::stdin().read_to_end(&mut buf), "stdin")?;
        return Ok((buf, "stdin.wav".to_string()));
    }
    xerr(std::fs::read(path), &format!("cannot read {path} (N-14)")).map(|b| {
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.wav")
            .to_string();
        (b, name)
    })
}

fn stage_temp(bytes: &[u8], fname: &str) -> Result<std::path::PathBuf, String> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let safe: String = fname
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = std::env::temp_dir().join(format!("flowvoice-file-{n}-{safe}"));
    xerr(std::fs::write(&path, bytes), "cannot stage file")?;
    Ok(path)
}

fn is_wav_name(fname: &str) -> bool {
    fname.to_ascii_lowercase().ends_with(".wav")
}

/// Record mono 16-bit PCM from the chosen input device until the key is up.
/// `FLOWVOICE_DEVICE` substring selects a non-default microphone (Y-05).
fn capture_until_stop() -> Result<(Vec<i16>, u32), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let wanted = std::env::var("FLOWVOICE_DEVICE").ok();
    let device = match wanted {
        Some(want) => xerr(host.input_devices(), "cannot list input devices")?
            .find(|d| d.name().map(|n| device_matches(&n, &want)).unwrap_or(false))
            .ok_or_else(|| format!("no input device matching `{want}`"))?,
        None => host.default_input_device().ok_or_else(|| {
            mic_help("no default audio input device: the OS gave us no microphone")
        })?,
    };

    let supported = xerr(device.default_input_config(), "cannot query input config")?;

    let channels = supported.channels() as usize;
    let rate = supported.sample_rate().0;

    let config = cpal::StreamConfig {
        channels: supported.channels(),
        sample_rate: supported.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    let storage: std::sync::Arc<std::sync::Mutex<Vec<i16>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    // Fresh meter + peak per utterance.
    use std::sync::atomic::Ordering;
    MIC_LEVEL.store(0, Ordering::SeqCst);
    MIC_PEAK.store(0, Ordering::SeqCst);

    let sink = storage.clone();
    let err_fn = |e| eprintln!("[audio] stream error: {e}");

    let stream = build_stream(
        &device,
        &config,
        supported.sample_format(),
        channels,
        sink,
        err_fn,
    )
    .map_err(|e| mic_help(&e.to_string()))?;

    stream
        .play()
        .map_err(|e| mic_help(&format!("cannot start audio stream: {e}")))?;

    // Wait for the hotkey to be released.
    while !crate::win::is_stop_requested() {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    xerr(stream.pause(), "cannot pause audio stream")?;

    let data = storage
        .lock()
        .map_err(|_| "audio buffer poisoned".to_string())?;
    // The stream object dies here: mic released right after the replica
    // (AG-01/AG-03) and the meter visibly falls to zero.
    drop(stream);
    MIC_LEVEL.store(0, Ordering::SeqCst);
    // Zero-signal warning (C-08): capture ran but the mic stayed silent.
    if MIC_PEAK.load(Ordering::SeqCst) < vad_level().max(0) as u32 {
        crate::state::emit("[warn] нулевой уровень сигнала — проверьте микрофон");
    }
    // Clipping warning (AI-01): hot overloaded input instead of silence.
    let total = data.len().max(1) as f32;
    let clipped = data.iter().filter(|s| (**s as i32).abs() >= 32760).count() as f32;
    if clipped / total > 0.05 {
        crate::state::emit("[warn] вход перегружен (клиппинг) — убавьте усиление микрофона");
    }
    Ok((data.clone(), rate))
}

/// Build an input stream for i16/f32/f64 data, downmixing to mono i16.
#[allow(clippy::too_many_arguments)]
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    channels: usize,
    sink: std::sync::Arc<std::sync::Mutex<Vec<i16>>>,
    err_fn: impl Fn(cpal::StreamError) + Send + Sync + 'static,
) -> Result<cpal::Stream, String> {
    match format {
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                config,
                move |data: &[i16], _| push_mono(&sink, data, channels),
                err_fn,
                None,
            )
            .map_err(|e| e.to_string()),
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                config,
                move |data: &[f32], _| {
                    let mono: Vec<i16> = data
                        .chunks(channels)
                        .map(|f| f32_to_i16(f.iter().sum::<f32>() / f.len() as f32))
                        .collect();
                    push_mono(&sink, &mono, 1);
                },
                err_fn,
                None,
            )
            .map_err(|e| e.to_string()),
        cpal::SampleFormat::F64 => device
            .build_input_stream(
                config,
                move |data: &[f64], _| {
                    let mono: Vec<i16> = data
                        .chunks(channels)
                        .map(|f| f32_to_i16((f.iter().sum::<f64>() / f.len() as f64) as f32))
                        .collect();
                    push_mono(&sink, &mono, 1);
                },
                err_fn,
                None,
            )
            .map_err(|e| e.to_string()),
        other => Err(format!("unsupported sample format {other}")),
    }
}

/// Downmix interleaved frames into a single mono i16 stream in the callback.
/// Also feeds the live level meter + session peak (C-07/C-08).
fn push_mono(sink: &std::sync::Arc<std::sync::Mutex<Vec<i16>>>, data: &[i16], channels: usize) {
    use std::sync::atomic::Ordering;
    let mut buf = match sink.lock() {
        Ok(buf) => buf,
        Err(_) => return,
    };
    let mut peak = 0i32;
    for frame in data.chunks(channels) {
        let sum: i32 = frame.iter().map(|s| i32::from(*s)).sum();
        let mono = (sum / frame.len() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let a = (mono as i32).abs();
        if a > peak {
            peak = a;
        }
        buf.push(mono);
    }
    MIC_LEVEL.store(
        ((peak * 100) / 32768).clamp(0, 100) as u32,
        Ordering::SeqCst,
    );
    MIC_PEAK.fetch_max(peak as u32, Ordering::SeqCst);
}

/// Convert an f32 sample in [-1, 1] to i16.
fn f32_to_i16(sample: f32) -> i16 {
    (sample * 32767.0).clamp(-32768.0, 32767.0) as i16
}

/// Live input level 0..100 for the GUI meter (C-07), updated per callback.
pub static MIC_LEVEL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Session peak amplitude (for the zero-signal warning, C-08).
static MIC_PEAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Simple linear resampler: input mono PCM at `rate` Hz -> 16 kHz.
fn resample_to_16k(input: &[i16], rate: u32) -> Vec<i16> {
    if input.is_empty() || rate == TARGET_SAMPLE_RATE {
        return input.to_vec();
    }

    let ratio = TARGET_SAMPLE_RATE as f64 / f64::from(rate);
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);

    for k in 0..out_len {
        let pos = k as f64 / ratio;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(input.len() - 1);
        let frac = (pos - pos.floor()) as f32;

        let v0 = f32::from(input[i0]);
        let v1 = f32::from(input[i1]);
        let v = v0 * (1.0 - frac) + v1 * frac;
        out.push(v.round().clamp(-32768.0, 32767.0) as i16);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_match_is_case_insensitive_substring() {
        assert!(device_matches("Microphone (USB Audio)", "usb"));
        assert!(device_matches("Микрофон", "МИКРО"));
        assert!(!device_matches("Speakers", "mic"));
    }

    #[test]
    fn brackets_without_digits_vanish() {
        assert_eq!(strip_brackets("привет [музыка] мир"), "привет мир");
        assert_eq!(strip_brackets("а (смеётся) б"), "а б");
        assert_eq!(
            strip_brackets("позвони (712) 555-01-01 завтра"),
            "позвони (712) 555-01-01 завтра"
        );
        assert_eq!(strip_brackets("без закрытия"), "без закрытия");
    }

    #[test]
    fn render_adds_timestamps() {
        let spans = vec![
            crate::whisper::Span {
                text: "привет".to_string(),
                start: 61.5,
                end: 63.0,
            },
            crate::whisper::Span {
                text: "мир".to_string(),
                start: 0.0,
                end: 0.0,
            },
        ];
        let opts = crate::FileOpts {
            save: false,
            timestamps: true,
            json: false,
        };
        let out = render_spans(&spans, &opts, None);
        assert!(out.starts_with("[01:01.5] Привет."), "{out}");
        assert!(out.contains("Мир."), "{out}");
    }

    #[test]
    fn render_splits_times_proportionally() {
        let spans = vec![
            crate::whisper::Span {
                text: "раз два".to_string(),
                start: 0.0,
                end: 0.0,
            },
            crate::whisper::Span {
                text: "три".to_string(),
                start: 0.0,
                end: 0.0,
            },
        ];
        let opts = crate::FileOpts {
            save: false,
            timestamps: true,
            json: false,
        };
        // 7 + 3 chars over 10 s: first span owns [00:00, 00:07).
        let out = render_spans(&spans, &opts, Some(10.0));
        assert!(out.starts_with("[00:00.0]"), "{out}");
        assert!(out.contains("[00:07.0]"), "{out}");
        // No duration known: no prefixes at all.
        let plain = render_spans(&spans, &opts, None);
        assert!(!plain.contains('['), "{plain}");
    }

    #[test]
    fn resample_passthrough_and_ratio() {
        assert_eq!(resample_to_16k(&[], 44100), Vec::<i16>::new());
        let src = vec![0i16, 1000, 2000, 3000];
        assert_eq!(resample_to_16k(&src, 16_000), src);
        let up = resample_to_16k(&[0i16, 1000], 8000);
        assert_eq!(up.len(), 4);
    }

    #[test]
    fn silence_trim_keeps_speech_edges() {
        // Threshold default is 200; speech at +-1000 stays.
        let pcm = vec![0i16, 10, 50, 1000, -1500, 2000, 30, 5, 0];
        assert_eq!(trim_silence(&pcm), vec![1000, -1500, 2000]);
        // All-silent input is preserved (engines report it themselves).
        let hush = vec![0i16, 5, 10];
        assert_eq!(trim_silence(&hush), hush);
        assert_eq!(trim_silence(&[]), Vec::<i16>::new());
    }

    #[test]
    fn push_mono_downmix_preserves_every_frame() {
        use std::sync::{Arc, Mutex};
        let sink = Arc::new(Mutex::new(Vec::new()));
        // Stereo pairs average to mono (E-10: no samples lost).
        push_mono(&sink, &[1000, 3000, -2000, -4000], 2);
        assert_eq!(*sink.lock().unwrap(), vec![2000, -3000]);
        push_mono(&sink, &[500], 1);
        assert_eq!(*sink.lock().unwrap(), vec![2000, -3000, 500]);
    }
}

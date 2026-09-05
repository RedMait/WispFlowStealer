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
            println!("[ready] groq backend (cloud whisper)");
            set_label("groq cloud");
        } else if pref.allows_local() && crate::whisper::available() {
            match crate::whisper::ensure_server() {
                Ok(p) => {
                    println!("[ready] whisper server on 127.0.0.1:{p}");
                    set_label("whisper local");
                }
                Err(e) => eprintln!("[whisper] {e} (vosk fallback)"),
            }
        } else if pref.allows_vosk() && model_instance().is_ok() {
            println!("[ready] speech model loaded (vosk fallback)");
            set_label("vosk");
        } else {
            eprintln!("[audio] no speech backend: set GROQ_API_KEY or run scripts/get-native.ps1");
        }
        if punct_instance().is_some() {
            println!("[ready] punctuation model loaded");
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
fn finalize(text: String) -> String {
    let lang = Language::detect(&text);
    if lang == Language::Ru {
        if let Some(punct) = punct_instance() {
            let cleaned = flowcore::clean(&text, lang);
            return punct
                .punct(&cleaned)
                .unwrap_or_else(|_| flowcore::format(&text, lang));
        }
    }
    flowcore::format(&text, lang)
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
pub fn transcribe() -> Result<String, String> {
    let (pcm, rate) = capture_until_stop()?;
    let pcm16k = resample_to_16k(&pcm, rate);
    let pref = pref();

    if pref.allows_groq() && crate::groq::available() {
        let raw = crate::groq::transcribe_pcm(&pcm16k)?;
        set_label("groq cloud");
        return Ok(format_segments(raw));
    }

    if pref.allows_local() && crate::whisper::available() {
        let raw = crate::whisper::transcribe_pcm(&pcm16k)?;
        set_label("whisper local");
        return Ok(format_segments(raw));
    }

    if pref.allows_vosk() {
        let model = model_instance()?;
        set_label("vosk");
        return Ok(finalize(recognize_with(model, &pcm16k)?));
    }

    Err("no speech backend enabled: set GROQ_API_KEY or run scripts/get-native.ps1".to_string())
}

/// Format every whisper segment as its own sentence and join them.
/// Segments track utterance boundaries, so each one gets its own terminal
/// mark ("…скрипт. Хочу…"); filler-only fragments format to empty and drop.
fn format_segments(raw: Vec<String>) -> String {
    raw.into_iter()
        .map(|seg| {
            let lang = Language::detect(&seg);
            flowcore::format(&seg, lang)
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

/// Record mono 16-bit PCM from the default input device until the key is up.
fn capture_until_stop() -> Result<(Vec<i16>, u32), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no default audio input device".to_string())?;

    let supported = device
        .default_input_config()
        .map_err(|e| format!("cannot query input config: {e}"))?;

    let channels = supported.channels() as usize;
    let rate = supported.sample_rate().0;

    let config = cpal::StreamConfig {
        channels: supported.channels(),
        sample_rate: supported.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    let storage: std::sync::Arc<std::sync::Mutex<Vec<i16>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    let sink = storage.clone();
    let err_fn = |e| eprintln!("[audio] stream error: {e}");

    let stream = build_stream(
        &device,
        &config,
        supported.sample_format(),
        channels,
        sink,
        err_fn,
    )?;

    stream
        .play()
        .map_err(|e| format!("cannot start audio stream: {e}"))?;

    // Wait for the hotkey to be released.
    while !crate::win::is_stop_requested() {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    stream
        .pause()
        .map_err(|e| format!("cannot pause audio stream: {e}"))?;

    let data = storage
        .lock()
        .map_err(|_| "audio buffer poisoned".to_string())?;
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
fn push_mono(sink: &std::sync::Arc<std::sync::Mutex<Vec<i16>>>, data: &[i16], channels: usize) {
    let mut buf = match sink.lock() {
        Ok(buf) => buf,
        Err(_) => return,
    };
    for frame in data.chunks(channels) {
        let sum: i32 = frame.iter().map(|s| i32::from(*s)).sum();
        let mono = (sum / frame.len() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        buf.push(mono);
    }
}

/// Convert an f32 sample in [-1, 1] to i16.
fn f32_to_i16(sample: f32) -> i16 {
    (sample * 32767.0).clamp(-32768.0, 32767.0) as i16
}

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

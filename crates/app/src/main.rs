// SPDX-License-Identifier: MIT
// GUI builds are windowed apps: double-clicking the exe opens no console.
// Terminal modes (--help/--demo/console hook) reattach explicitly via
// `win::ensure_console()`.
#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]

mod demo;

// Pure, platform-free modules. Compiled on Windows always and anywhere
// for tests, so the logic is covered on plain CI too.
#[cfg(any(all(windows, any(feature = "audio", feature = "gui")), test))]
mod backend;
#[cfg(any(all(windows, feature = "audio"), test))]
mod command;
#[cfg(any(windows, test))]
mod hotkey;
#[cfg(any(all(windows, any(feature = "audio", feature = "gui")), test))]
mod journal;

#[cfg(all(windows, feature = "audio"))]
mod audio;

#[cfg(all(windows, feature = "audio"))]
mod groq;

#[cfg(all(windows, feature = "audio"))]
mod whisper;

#[cfg(windows)]
mod win;

#[cfg(all(windows, any(feature = "audio", feature = "gui")))]
mod state;

#[cfg(all(windows, feature = "gui"))]
mod gui;

/// Console builds always have a terminal; windowed GUI builds reattach
/// explicitly for terminal modes.
#[cfg(all(windows, feature = "gui"))]
fn ensure_console() {
    win::ensure_console();
}

#[cfg(not(all(windows, feature = "gui")))]
fn ensure_console() {}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        ensure_console();
        println!("flowvoice {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // CLI overrides for the run (same names as the env vars, Y-05..Y-07).
    apply_env_flag(&args, "--model", "FLOWVOICE_GROQ_MODEL");
    apply_env_flag(&args, "--lang", "FLOWVOICE_LANG");
    apply_env_flag(&args, "--backend", "FLOWVOICE_BACKEND");
    apply_env_flag(&args, "--device", "FLOWVOICE_DEVICE");
    if args.iter().any(|a| a == "--raw" || a == "--no-format") {
        std::env::set_var("FLOWVOICE_RAW", "1");
    }

    if args.iter().any(|a| a == "-h" || a == "--help") {
        ensure_console();
        print_help();
        return;
    }

    if let Some(key) = flag_value(&args, "--set-key") {
        ensure_console();
        #[cfg(all(windows, feature = "audio"))]
        {
            match crate::groq::save_key_to_store(&key) {
                Ok(()) => println!("Groq key saved to Windows Credential Manager"),
                Err(e) => {
                    eprintln!("cannot save key: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        #[cfg(not(all(windows, feature = "audio")))]
        {
            let _ = key;
            eprintln!("--set-key needs Windows and the `audio` feature");
            std::process::exit(2);
        }
    }

    if args.iter().any(|a| a == "--stats") {
        ensure_console();
        #[cfg(all(windows, any(feature = "audio", feature = "gui")))]
        {
            print_stats();
            return;
        }
        #[cfg(not(all(windows, any(feature = "audio", feature = "gui"))))]
        {
            eprintln!("--stats needs a Windows build with `audio` or `gui`");
            std::process::exit(2);
        }
    }

    if let Some(text) = parse_demo_arg(&args) {
        ensure_console();
        if args.iter().any(|a| a == "--json") {
            print_demo_json(&text);
        } else {
            demo::run(&text);
        }
        return;
    }

    if let Some(path) = flag_value(&args, "--file") {
        ensure_console();
        #[cfg(all(windows, feature = "audio"))]
        {
            let opts = FileOpts::from_args(&args);
            match crate::audio::transcribe_file(&path, &opts) {
                Ok(out) => {
                    if opts.json {
                        print_text_json(&out);
                    } else {
                        println!("{out}");
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        #[cfg(not(all(windows, feature = "audio")))]
        {
            let _ = &path;
            eprintln!("--file needs Windows and the `audio` feature");
            std::process::exit(2);
        }
    }

    if let Some(dir) = flag_value(&args, "--dir") {
        ensure_console();
        #[cfg(all(windows, feature = "audio"))]
        {
            let opts = FileOpts::from_args(&args);
            match crate::audio::transcribe_dir(&dir, &opts) {
                Ok(n) => println!("transcribed {n} file(s)"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        #[cfg(not(all(windows, feature = "audio")))]
        {
            let _ = &dir;
            eprintln!("--dir needs Windows and the `audio` feature");
            std::process::exit(2);
        }
    }

    if args.iter().any(|a| a == "--gui") {
        #[cfg(all(windows, feature = "gui"))]
        {
            // Windowed on purpose: no console in GUI mode.
            gui::run();
            return;
        }
        #[cfg(not(all(windows, feature = "gui")))]
        {
            eprintln!("GUI needs Windows and the `gui` feature:");
            eprintln!("  cargo run -p flowvoice --features gui -- --gui");
            eprintln!("(full app: --features audio,gui)");
            std::process::exit(2);
        }
    }

    #[cfg(windows)]
    {
        ensure_console();
        let key = parse_key_arg(&args).unwrap_or_default();
        win::run(key);
    }

    #[cfg(not(windows))]
    {
        eprintln!("Hotkey dictation currently requires Windows.");
        eprintln!("On any platform you can try the demo:");
        eprintln!("  cargo run -p flowvoice -- --demo \"hello world\"");
        std::process::exit(2);
    }
}

/// `--flag value` lookup.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().cloned();
        }
    }
    None
}

/// `--flag value` becomes a process env override for this run.
fn apply_env_flag(args: &[String], flag: &str, env: &str) {
    if let Some(v) = flag_value(args, flag) {
        std::env::set_var(env, v);
    }
}

/// File/batch transcription options (N-12/N-13, Y-02).
#[cfg(all(windows, feature = "audio"))]
#[derive(Debug, Clone)]
pub struct FileOpts {
    pub save: bool,
    pub timestamps: bool,
    pub json: bool,
}

#[cfg(all(windows, feature = "audio"))]
impl FileOpts {
    fn from_args(args: &[String]) -> Self {
        Self {
            save: args.iter().any(|a| a == "--save"),
            timestamps: args.iter().any(|a| a == "--timestamps"),
            json: args.iter().any(|a| a == "--json"),
        }
    }
}

/// Print `--demo`/`--file` output as one JSON object (Y-02).
fn print_text_json(text: &str) {
    let lang = flowcore::Language::detect(text);
    let kind = flowcore::classify(text, lang);
    println!(
        "{{\"text\":{},\"lang\":\"{lang}\",\"kind\":\"{kind}\"}}",
        json_escape(text)
    );
}

fn print_demo_json(text: &str) {
    print_text_json(&flowcore::format_raw(text));
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One-line journal summary for `--stats` (T-15).
#[cfg(all(windows, any(feature = "audio", feature = "gui")))]
fn print_stats() {
    let entries = crate::journal::read_all(&crate::state::journal_path());
    let total = crate::journal::stats_since(&entries, 0);
    println!("replicas: {}", total.replicas);
    println!("words: {}", total.words);
    println!("avg_secs: {:.2}", total.avg_secs);
    println!("avg_wpm: {:.1}", total.avg_wpm);
    println!("best_wpm: {:.1}", total.best_wpm);
}

/// `--demo <text...>` runs the full pipeline on static text.
/// Useful for slides, CI, and machines without a microphone.
fn parse_demo_arg(args: &[String]) -> Option<String> {
    let mut rest: Vec<String> = Vec::new();
    let mut found = false;
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "--demo" {
            found = true;
            for a in iter {
                rest.push(a.clone());
            }
            break;
        }
    }
    if found && !rest.is_empty() {
        Some(rest.join(" "))
    } else {
        None
    }
}

#[cfg(windows)]
fn parse_key_arg(args: &[String]) -> Option<win::Hotkey> {
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "--key" {
            if let Some(name) = iter.next() {
                return win::Hotkey::parse(name);
            }
        }
    }
    None
}

fn print_help() {
    println!("flowvoice - hold-to-talk voice dictation with auto-formatting");
    println!();
    println!("USAGE:");
    println!("  flowvoice [--key <F8|RCONTROL|F7|...>]   hotkey dictation (Windows only)");
    println!("  flowvoice --demo <text...> [--json]      run the formatting pipeline on text");
    println!("  flowvoice --file <path|-> [--save] [--timestamps] [--json]");
    println!("                                           transcribe an audio file or stdin");
    println!("  flowvoice --dir <path> [--save]          batch-transcribe a directory");
    println!("  flowvoice --gui                          desktop window: status, settings, history (needs `gui` feature)");
    println!("  flowvoice --stats                        journal summary (replicas, delays, WPM)");
    println!("  flowvoice --set-key <KEY>                store the Groq key in Credential Manager");
    println!("  flowvoice --version                      print version");
    println!();
    println!("RUN FLAGS (override env for this run):");
    println!("  --model <id>     Groq model (default whisper-large-v3-turbo)");
    println!("  --lang <ru|en|auto>  recognition + formatting language");
    println!("  --backend <auto|groq|local|vosk>");
    println!("  --device <name>  microphone substring match");
    println!("  --raw, --no-format   skip all post-processing");
    println!();
    println!("ENV:");
    println!("  GROQ_API_KEY         Groq Cloud whisper (primary when set; keep it secret)");
    println!("  FLOWVOICE_GROQ_MODEL Groq model id (default whisper-large-v3-turbo)");
    println!("  FLOWVOICE_GROQ_PROMPT vocabulary hint, e.g. domain words (optional)");
    println!("  FLOWVOICE_CHAT_MODEL chat model for voice commands (default openai/gpt-oss-20b)");
    println!("  FLOWVOICE_MODEL    path to a Vosk model directory (default models/ru)");
    println!("  FLOWVOICE_WHISPER_MODEL local whisper model file (default models/whisper/ggml-large-v3-turbo.bin)");
    println!("  FLOWVOICE_WHISPER_BIN  local whisper-server.exe (default native/whisper/whisper-server.exe)");
    println!("  FLOWVOICE_WHISPER_PORT local server port, +2 fallback (default 8178)");
    println!("  FLOWVOICE_THREADS  local whisper threads (default server auto)");
    println!("  FLOWVOICE_PROFILE  post profile: auto|chat|mail|code (default auto)");
    println!("  FLOWVOICE_RAW=1    raw transcript, no post-processing");
    println!("  FLOWVOICE_NO_HISTORY=1  privacy mode: no history, no journal");
    println!("  FLOWVOICE_SOUND=0  mute record beeps");
    println!("  FLOWVOICE_TIMEOUT_S Groq timeout, seconds (default 180)");
    println!("  FLOWVOICE_PASTE_DELAY_MS pause before Ctrl+V (default 0)");
    println!("  FLOWVOICE_RESTORE_MS clipboard restore delay (default 400, 0 keeps ours)");
    println!("  FLOWVOICE_LEADING_SPACE=1 space before alphanumeric replicas");
    println!("  FLOWPUNCT_MODEL    dir with rupunct_small_int8.onnx + tokenizer.json (default models/punct)");
    println!();
    println!("DEFAULT HOTKEY: hold Right Ctrl to dictate, release to insert formatted text");
}

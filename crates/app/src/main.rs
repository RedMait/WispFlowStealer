// GUI builds are windowed apps: double-clicking the exe opens no console.
// Terminal modes (--help/--demo/console hook) reattach explicitly via
// `win::ensure_console()`.
#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]

mod demo;

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

    if args.iter().any(|a| a == "-h" || a == "--help") {
        ensure_console();
        print_help();
        return;
    }

    if let Some(text) = parse_demo_arg(&args) {
        ensure_console();
        demo::run(&text);
        return;
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
    println!("  flowvoice --demo <text...>               run the formatting pipeline on text");
    println!("  flowvoice --gui                          desktop window: status, settings, history (needs `gui` feature)");
    println!();
    println!("ENV:");
    println!("  GROQ_API_KEY         Groq Cloud whisper (primary when set; keep it secret)");
    println!("  FLOWVOICE_GROQ_MODEL Groq model id (default whisper-large-v3-turbo)");
    println!("  FLOWVOICE_GROQ_PROMPT vocabulary hint, e.g. domain words (optional)");
    println!("  FLOWVOICE_MODEL    path to a Vosk model directory (default models/ru)");
    println!("  FLOWPUNCT_MODEL    dir with rupunct_small_int8.onnx + tokenizer.json (default models/punct)");
    println!();
    println!("DEFAULT HOTKEY: hold Right Ctrl to dictate, release to insert formatted text");
}

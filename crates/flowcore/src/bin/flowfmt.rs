use std::io::{self, Read};

use flowcore::{format, Language};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut lang_override: Option<Language> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--lang" => {
                if let Some(v) = it.next() {
                    lang_override = match v.as_str() {
                        "ru" => Some(Language::Ru),
                        "en" => Some(Language::En),
                        _ => {
                            eprintln!("unknown language: {v} (use ru or en)");
                            std::process::exit(2);
                        }
                    };
                }
            }
            "-h" | "--help" => {
                println!("Reads dictation text from stdin (or args), prints formatted text.");
                println!("Usage: flowfmt [--lang <ru|en>] [TEXT...]");
                return;
            }
            _ => rest.push(arg.clone()),
        }
    }

    let (input, lang) = if rest.is_empty() {
        let mut buf = String::new();
        if io::stdin().read_to_string(&mut buf).is_err() {
            eprintln!("failed to read stdin");
            std::process::exit(2);
        }
        let lang = lang_override.unwrap_or_else(|| Language::detect(&buf));
        (buf, lang)
    } else {
        let text = rest.join(" ");
        let lang = lang_override.unwrap_or_else(|| Language::detect(&text));
        (text, lang)
    };

    println!("{}", format(&input, lang));
}

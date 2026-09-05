use flowcore::{classify, format, Language};

/// Run the full dictation pipeline on static text and print every stage.
/// This mirrors exactly what happens with real microphone input, just
/// with a canned transcript instead of a speech model.
pub fn run(raw: &str) {
    let lang = Language::detect(raw);
    let kind = classify(raw, lang);
    let formatted = format(raw, lang);

    println!("flowvoice demo");
    println!("  raw:       {raw:?}");
    println!("  language:  {lang}");
    println!("  sentence:  {kind} ({})", kind.punctuation());
    println!("  formatted: {formatted}");

    if formatted.is_empty() {
        println!("note: the transcript contained no speech after cleanup");
    }

    #[cfg(feature = "audio")]
    copy_to_clipboard(&formatted);
}

#[cfg(feature = "audio")]
fn copy_to_clipboard(text: &str) {
    if !text.is_empty() {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if cb.set_text(text).is_ok() {
                println!("  (copied to clipboard)");
            }
        }
    }
}

// SPDX-License-Identifier: MIT
//! Voice commands: prefix instructions that transform instead of dictate.
//!
//! Pure parser (no I/O, no network): unit-tested on every platform.
//! Execution lives in `audio.rs` (LLM-backed ones need `GROQ_API_KEY`).
//!
//! Supported (case-insensitive, leading fillers tolerated):
//! * `переведи на английский: <текст>` / `переведи: <текст>` (J-10)
//! * `сократи: <текст>` (J-11)
//! * `замени <X> на <Y>` applied to the last replica (J-13)
//! * `перепиши` — restyle the last replica (J-14)
//! * `отмени` — undo the last command edit (J-15)
//! * `транслит: <текст>` — crude RU→Latin transliteration, offline (I-08)

/// A parsed voice command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Translate { target: String, text: String },
    Summarize { text: String },
    Replace { from: String, to: String },
    Rewrite,
    Undo,
    Transliterate { text: String },
}

/// Target language name (RU) -> ISO code for the translate prompt.
pub fn lang_name_to_code(name: &str) -> Option<&'static str> {
    match name.trim().to_lowercase().as_str() {
        "английский" | "английкого" | "английски" => Some("en"),
        "русский" | "русски" => Some("ru"),
        "немецкий" => Some("de"),
        "французский" => Some("fr"),
        "испанский" => Some("es"),
        "итальянский" => Some("it"),
        "китайский" => Some("zh"),
        "татарский" => Some("tt"),
        _ => None,
    }
}

/// Strip leading filler words so `ну переведи: ...` still parses.
fn ltrim_fillers(s: &str) -> &str {
    const FILLERS: &[&str] = &[
        "ну",
        "эм",
        "э",
        "мм",
        "типа",
        "как бы",
        "вот",
        "короче",
        "значит",
        "так",
    ];
    let mut rest = s.trim_start();
    loop {
        let mut cut = false;
        for f in FILLERS {
            if let Some(tail) = rest.strip_prefix(f) {
                if tail.starts_with([' ', ',', '.', '!', '?']) || tail.is_empty() {
                    rest = tail.trim_start_matches([' ', ',', '.', '!', '?', ' ']);
                    cut = true;
                    break;
                }
            }
        }
        if !cut {
            return rest;
        }
    }
}

/// Parse a voice command. Returns `None` for plain dictation.
pub fn parse(text: &str) -> Option<Command> {
    let t = ltrim_fillers(text);
    let low = t.to_lowercase();

    for prefix in ["переведи на ", "перевести на "] {
        if let Some(rest) = low.strip_prefix(prefix) {
            let orig_rest = &t[prefix.len()..];
            if let Some((lang_part, body)) = rest.split_once([':', '—', '-']) {
                let body = body.trim();
                if body.is_empty() {
                    return None;
                }
                let code = lang_name_to_code(lang_part.trim()).unwrap_or("en");
                let take = orig_rest
                    .split_once([':', '—', '-'])
                    .map(|(_, b)| b.trim())
                    .unwrap_or("");
                return Some(Command::Translate {
                    target: code.to_string(),
                    text: take.to_string(),
                });
            }
        }
    }
    for prefix in ["переведи:", "переведи ", "перевести:"] {
        if low.starts_with(prefix) {
            let body = t[prefix.len()..].trim().trim_start_matches(':').trim();
            if body.is_empty() {
                return None;
            }
            return Some(Command::Translate {
                target: "en".to_string(),
                text: body.to_string(),
            });
        }
    }
    for prefix in ["сократи:", "сократи ", "сократить:"] {
        if low.starts_with(prefix) {
            let body = t[prefix.len()..].trim().trim_start_matches(':').trim();
            if body.is_empty() {
                return None;
            }
            return Some(Command::Summarize {
                text: body.to_string(),
            });
        }
    }
    for prefix in ["транслит:", "транслит "] {
        if low.starts_with(prefix) {
            let body = t[prefix.len()..].trim().trim_start_matches(':').trim();
            if body.is_empty() {
                return None;
            }
            return Some(Command::Transliterate {
                text: body.to_string(),
            });
        }
    }
    if low.starts_with("замени ") || low.starts_with("заменить ") {
        let kw = if low.starts_with("замени ") {
            "замени "
        } else {
            "заменить "
        };
        let rest = t[kw.len()..].trim();
        if let Some((from, to)) = rest.split_once(" на ") {
            let (from, to) = (from.trim(), to.trim());
            if !from.is_empty() && !to.is_empty() {
                return Some(Command::Replace {
                    from: from.to_string(),
                    to: to.to_string(),
                });
            }
        }
        return None;
    }
    if matches!(
        low.trim(),
        "перепиши" | "переписать" | "перепиши последнее" | "перепиши последнее предложение"
    ) {
        return Some(Command::Rewrite);
    }
    if matches!(low.trim(), "отмени" | "отмена" | "отменить") {
        return Some(Command::Undo);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_with_target() {
        assert_eq!(
            parse("переведи на английский: привет мир"),
            Some(Command::Translate {
                target: "en".to_string(),
                text: "привет мир".to_string(),
            })
        );
    }

    #[test]
    fn translate_default_target() {
        assert_eq!(
            parse("Переведи: доброе утро"),
            Some(Command::Translate {
                target: "en".to_string(),
                text: "доброе утро".to_string(),
            })
        );
    }

    #[test]
    fn translate_tolerates_fillers() {
        assert!(matches!(
            parse("ну эм сократи: очень длинный текст"),
            Some(Command::Summarize { .. })
        ));
    }

    #[test]
    fn summarize() {
        assert_eq!(
            parse("сократи: раз два три четыре"),
            Some(Command::Summarize {
                text: "раз два три четыре".to_string(),
            })
        );
    }

    #[test]
    fn replace_split() {
        assert_eq!(
            parse("замени Маша на маржа"),
            Some(Command::Replace {
                from: "Маша".to_string(),
                to: "маржа".to_string(),
            })
        );
    }

    #[test]
    fn rewrite_and_undo() {
        assert_eq!(parse("перепиши"), Some(Command::Rewrite));
        assert_eq!(parse("Отмени"), Some(Command::Undo));
    }

    #[test]
    fn transliterate_cmd() {
        assert_eq!(
            parse("транслит: привет"),
            Some(Command::Transliterate {
                text: "привет".to_string(),
            })
        );
    }

    #[test]
    fn plain_dictation_is_none() {
        assert_eq!(parse("привет мир"), None);
        assert_eq!(parse("замени"), None);
        assert_eq!(parse("переведи:"), None);
        assert_eq!(parse("я сказал замени потом"), None);
    }
}

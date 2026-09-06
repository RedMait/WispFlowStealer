// SPDX-License-Identifier: MIT
//! Voice macros (AJ-01/AJ-02/AJ-03): exact spoken phrases that press keys
//! instead of dictating text. Defined in `%APPDATA%\WispFlowStealer\macros.json`:
//! `[{"phrase": "сохрани", "keys": "ctrl+s"}]`. Pure parser, unit-tested
//! everywhere; execution (keybd) lives in `win.rs`.

/// One macro: spoken phrase -> key combo spec (`ctrl+s`, `alt+f4`, `enter`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Macro {
    pub phrase: String,
    pub keys: String,
}

/// Normalize a transcript for exact matching: lowercase, trimmed,
///
/// punctuation stripped.
pub fn normalize(text: &str) -> String {
    text.to_lowercase()
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// Find the first macro whose phrase equals the transcript.
pub fn find_match<'a>(macros: &'a [Macro], text: &str) -> Option<&'a Macro> {
    let want = normalize(text);
    if want.is_empty() {
        return None;
    }
    macros.iter().find(|m| normalize(&m.phrase) == want)
}

/// One virtual key with direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStroke {
    pub vk: u8,
    pub down: bool,
}

/// Parse `ctrl+shift+s` / `alt+f4` / `enter` into down/up strokes.
/// Modifiers go down first and up last. Unknown names fail the whole combo.
pub fn parse_combo(spec: &str) -> Option<Vec<KeyStroke>> {
    let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let mut mods: Vec<u8> = Vec::new();
    let mut main: Option<u8> = None;
    for part in parts {
        let low = part.to_ascii_lowercase();
        let vk: u8 = match low.as_str() {
            "ctrl" | "control" => 0x11,
            "alt" | "menu" => 0x12,
            "shift" => 0x10,
            "win" | "meta" => 0x5B,
            "enter" | "return" => 0x0D,
            "esc" | "escape" => 0x1B,
            "tab" => 0x09,
            "space" => 0x20,
            "backspace" => 0x08,
            "delete" | "del" => 0x2E,
            "insert" | "ins" => 0x2D,
            "home" => 0x24,
            "end" => 0x23,
            "pgup" | "pageup" => 0x21,
            "pgdn" | "pagedown" => 0x22,
            "up" => 0x26,
            "down" => 0x28,
            "left" => 0x25,
            "right" => 0x27,
            s if s.len() == 1 => {
                let c = s.as_bytes()[0];
                match c {
                    b'a'..=b'z' => c - b'a' + b'A',
                    b'0'..=b'9' => c,
                    _ => return None,
                }
            }
            s if s.starts_with('f') => {
                let n: u8 = s[1..].parse().ok()?;
                if !(1..=24).contains(&n) {
                    return None;
                }
                0x6F + n
            }
            _ => return None,
        };
        match low.as_str() {
            "ctrl" | "control" | "alt" | "menu" | "shift" | "win" | "meta" => mods.push(vk),
            _ => {
                if main.is_some() {
                    return None; // two main keys: ambiguous
                }
                main = Some(vk);
            }
        }
    }
    let main = main?;
    let mut out = Vec::with_capacity(mods.len() * 2 + 2);
    for m in &mods {
        out.push(KeyStroke { vk: *m, down: true });
    }
    out.push(KeyStroke {
        vk: main,
        down: true,
    });
    out.push(KeyStroke {
        vk: main,
        down: false,
    });
    for m in mods.iter().rev() {
        out.push(KeyStroke {
            vk: *m,
            down: false,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_case_and_marks() {
        assert_eq!(normalize("  Сохрани! "), "сохрани");
        assert_eq!(normalize("..."), "");
    }

    #[test]
    fn exact_match_only() {
        let macros = vec![Macro {
            phrase: "сохрани".to_string(),
            keys: "ctrl+s".to_string(),
        }];
        assert!(find_match(&macros, "Сохрани.").is_some());
        assert!(find_match(&macros, "сохрани файл").is_none());
        assert!(find_match(&macros, "").is_none());
    }

    #[test]
    fn combos() {
        assert_eq!(
            parse_combo("ctrl+s").unwrap(),
            vec![
                KeyStroke {
                    vk: 0x11,
                    down: true
                },
                KeyStroke {
                    vk: b'S',
                    down: true
                },
                KeyStroke {
                    vk: b'S',
                    down: false
                },
                KeyStroke {
                    vk: 0x11,
                    down: false
                },
            ]
        );
        assert_eq!(
            parse_combo("alt+F4").unwrap()[1],
            KeyStroke {
                vk: 0x73,
                down: true
            }
        );
        assert_eq!(parse_combo("enter").unwrap().len(), 2);
        assert!(parse_combo("ctrl+").is_none());
        assert!(parse_combo("hyper+x").is_none());
        assert!(parse_combo("a+b").is_none());
        assert!(parse_combo("f25").is_none());
    }
}

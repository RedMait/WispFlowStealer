//! Pure, dependency-free text post-processing engine.
//!
//! Replicates the writing "magic" of Wispr-like dictation assistants:
//!
//! * removes filler words (`um`, `uh`, `ну`, `типа`, ...)
//! * collapses duplicated words (`и и` -> `и`, `the the` -> `the`)
//! * detects the sentence type and appends `?`, `!` or `.`
//! * capitalizes the first letter
//! * cleans up spacing around punctuation
//!
//! Everything is deterministic and fully unit-tested, so the whole
//! formatting pipeline can be verified on any machine — no audio
//! hardware, no neural network, no network access required.

use std::fmt;

/// Supported dictation languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Ru,
    En,
}

impl Language {
    /// Guess the language from the presence of Cyrillic letters.
    pub fn detect(text: &str) -> Language {
        if text
            .chars()
            .any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
        {
            Language::Ru
        } else {
            Language::En
        }
    }

    /// Filler phrases (words to strip from dictation), ordered longest-first
    /// so greedy matching removes multi-word fillers before the individual
    /// words they are made of.
    fn fillers(self) -> &'static [&'static str] {
        match self {
            Language::Ru => &[
                "в общем",
                "в принципе",
                "это самое",
                "так сказать",
                "по сути",
                "ну вот",
                "ну и",
                "как бы",
                "эм",
                "э-э",
                "ээ",
                "мм",
                "м-м",
                "короче",
                "кстати",
                "скажем",
                "допустим",
                "значит",
                "типа",
                "блин",
                "ну",
                "хм",
                "э",
                "м",
                "а",
            ],
            Language::En => &[
                "you know",
                "i mean",
                "kind of",
                "sort of",
                "so basically",
                "literally",
                "basically",
                "actually",
                "honestly",
                "um",
                "uh",
                "hmm",
                "kinda",
                "like",
                "ah",
                "er",
                "hm",
            ],
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Language::Ru => write!(f, "ru"),
            Language::En => write!(f, "en"),
        }
    }
}

/// The detected kind of a sentence, rendered as `?`, `!` or `.`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentenceKind {
    Statement,
    Question,
    Exclamation,
}

impl SentenceKind {
    /// The punctuation character that terminates this sentence.
    pub fn punctuation(self) -> char {
        match self {
            SentenceKind::Statement => '.',
            SentenceKind::Question => '?',
            SentenceKind::Exclamation => '!',
        }
    }
}

impl fmt::Display for SentenceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SentenceKind::Statement => write!(f, "statement"),
            SentenceKind::Question => write!(f, "question"),
            SentenceKind::Exclamation => write!(f, "exclamation"),
        }
    }
}

/// Characters allowed to stay inside a "word" token.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '-' || c == '_'
}

/// A minimal tokenizer split between words and punctuation marks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Punct(char),
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    for c in input.chars() {
        if is_word_char(c) {
            word.push(c);
        } else {
            if !word.is_empty() {
                tokens.push(Token::Word(std::mem::take(&mut word)));
            }
            if !c.is_whitespace() {
                tokens.push(Token::Punct(c));
            }
        }
    }
    if !word.is_empty() {
        tokens.push(Token::Word(word));
    }
    tokens
}

/// Returns `true` at every word position that survives filler removal.
fn filler_mask(words: &[String], lang: Language) -> Vec<bool> {
    let fillers: Vec<Vec<String>> = lang
        .fillers()
        .iter()
        .map(|f| f.split(' ').map(|w| w.to_lowercase()).collect())
        .collect();

    let mut keep = vec![true; words.len()];
    let mut i = 0;
    while i < words.len() {
        let mut jumped = None;
        for filler in &fillers {
            if i + filler.len() <= words.len()
                && filler.iter().enumerate().all(|(k, fw)| words[i + k] == *fw)
            {
                jumped = Some(filler.len());
                break;
            }
        }
        match jumped {
            Some(len) => {
                for kept in keep.iter_mut().skip(i).take(len) {
                    *kept = false;
                }
                i += len;
            }
            None => i += 1,
        }
    }
    keep
}

/// Collapse a word repeated back-to-back (`и и` -> `и`).
fn collapse_repeats(words: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words {
        if let Some(last) = out.last() {
            if last.eq_ignore_ascii_case(w) {
                continue;
            }
        }
        out.push(w.clone());
    }
    out
}

/// Returns an explicit terminal punctuation mark if the raw text already
/// ends with one of `?`, `!` or `.`.
fn raw_terminal(raw: &str) -> Option<char> {
    raw.trim_end()
        .chars()
        .next_back()
        .filter(|c| matches!(c, '?' | '!' | '.'))
}

/// Returns the whole trailing run of terminal punctuation (`?!`, `??`, ...).
fn terminal_seq(raw: &str) -> &str {
    let end = raw.trim_end();
    let len: usize = end
        .chars()
        .rev()
        .take_while(|c| matches!(c, '?' | '!' | '.'))
        .map(char::len_utf8)
        .sum();
    &end[end.len() - len..]
}

/// Words whose presence at the *start* of a sentence makes it a question.
fn question_starters(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Ru => &[
            "что",
            "как",
            "почему",
            "зачем",
            "где",
            "куда",
            "когда",
            "кто",
            "чей",
            "сколько",
            "можно",
            "нельзя",
            "правда",
            "разве",
            "неужели",
        ],
        Language::En => &[
            "who", "what", "why", "when", "where", "which", "how", "whose", "whom", "does", "did",
            "do", "is", "are", "was", "were", "can", "could", "should", "would", "will", "shall",
            "may", "might", "have", "has", "had", "am",
        ],
    }
}

/// Words that make a sentence emphatic enough to end with `!`.
fn exclamation_words(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Ru => &[
            "очень",
            "супер",
            "ура",
            "вау",
            "ого",
            "круто",
            "класс",
            "отлично",
            "прекрасно",
            "прекрасный",
            "великолепно",
            "восхитительно",
            "невероятно",
            "потрясающе",
            "замечательно",
            "ужасно",
            "офигенно",
        ],
        Language::En => &[
            "wow",
            "awesome",
            "amazing",
            "great",
            "fantastic",
            "incredible",
            "terrible",
            "awful",
            "horrible",
            "excellent",
            "brilliant",
            "perfect",
            "wonderful",
            "yay",
            "cool",
            "superb",
            "phenomenal",
            "whoa",
        ],
    }
}

/// Classify a sentence into statement / question / exclamation.
///
/// Explicit punctuation in the input wins, then heuristic triggers are
/// consulted: emphatic words for `!`, question starters for `?`.
pub fn classify(raw: &str, lang: Language) -> SentenceKind {
    if let Some(term) = raw_terminal(raw) {
        return match term {
            '?' => SentenceKind::Question,
            '!' => SentenceKind::Exclamation,
            _ => SentenceKind::Statement,
        };
    }

    let lower: Vec<String> = tokenize(raw)
        .into_iter()
        .filter_map(|t| match t {
            Token::Word(w) => Some(w.to_lowercase()),
            Token::Punct(_) => None,
        })
        .collect();

    let mask = filler_mask(&lower, lang);
    let kept: Vec<String> = lower
        .iter()
        .zip(&mask)
        .filter(|(_, keep)| **keep)
        .map(|(w, _)| w.clone())
        .collect();

    let words = collapse_repeats(&kept);
    let first = words.first().map(String::as_str).unwrap_or("");

    let emphatic = words
        .iter()
        .any(|w| exclamation_words(lang).contains(&w.as_str()));
    if emphatic {
        return SentenceKind::Exclamation;
    }

    if question_starters(lang).contains(&first) {
        return SentenceKind::Question;
    }

    // "Какой красивый кролик!" — "какой" at the start prefers an exclamation,
    // but only when no explicit question trigger was detected above.
    if lang == Language::Ru && matches!(first, "какой" | "какая" | "какое" | "какие")
    {
        return SentenceKind::Exclamation;
    }

    SentenceKind::Statement
}

/// Join tokens back into a single string with sane spacing rules.
fn join_tokens(tokens: &[Token], lang: Language) -> String {
    let mut s = String::new();
    for token in tokens {
        match token {
            Token::Word(w) => {
                if !s.is_empty() && !s.ends_with(char::is_whitespace) {
                    s.push(' ');
                }
                let word = if lang == Language::En && w.eq_ignore_ascii_case("i") {
                    "I"
                } else {
                    w
                };
                s.push_str(word);
            }
            Token::Punct(c) => {
                if s.is_empty() {
                    continue; // never start with stray punctuation
                }
                s.push(*c);
                s.push(' ');
            }
        }
    }
    s.trim().to_string()
}

/// Uppercase the first alphabetic character of a string.
fn capitalize_first(s: &mut String) {
    if let Some((i, c)) = s.char_indices().find(|(_, c)| c.is_alphabetic()) {
        let upper: String = c.to_uppercase().collect();
        s.replace_range(i..i + c.len_utf8(), &upper);
    }
}

/// Format a raw dictation string into final, publish-ready text.
///
/// Runs the full pipeline: cleanup -> filler removal -> dedup ->
/// sentence classification -> punctuation -> capitalization.
pub fn format(raw: &str, lang: Language) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }

    let kind = classify(raw, lang);

    // Strip an explicit terminal punctuation; we restore it verbatim below
    // (this preserves user-input sequences like "?!" or "...").
    let term = terminal_seq(raw);
    let bare = if term.is_empty() {
        raw.to_string()
    } else {
        let end = raw.trim_end();
        end[..end.len() - term.len()].to_string()
    };

    let tokens = tokenize(&bare);

    // Which word tokens survive filler removal?
    let lower: Vec<String> = tokens
        .iter()
        .filter_map(|t| match t {
            Token::Word(w) => Some(w.to_lowercase()),
            Token::Punct(_) => None,
        })
        .collect();
    let mask = filler_mask(&lower, lang);

    // Rebuild tokens, dropping removed fillers and back-to-back duplicates.
    let mut rebuilt: Vec<Token> = Vec::new();
    let mut mi = 0usize;
    for token in &tokens {
        match token {
            Token::Punct(c) => rebuilt.push(Token::Punct(*c)),
            Token::Word(w) => {
                mi += 1;
                if !mask[mi - 1] {
                    continue;
                }
                if let Some(Token::Word(last)) = rebuilt.last() {
                    if last.eq_ignore_ascii_case(w) {
                        continue;
                    }
                }
                rebuilt.push(Token::Word(w.clone()));
            }
        }
    }

    let mut text = join_tokens(&rebuilt, lang);
    if text.is_empty() {
        return String::new();
    }

    capitalize_first(&mut text);
    if term.is_empty() {
        text.push(kind.punctuation());
    } else {
        text.push_str(term);
    }
    text
}

/// Convenience wrapper around [`format`] that auto-detects the language.
pub fn format_raw(raw: &str) -> String {
    format(raw, Language::detect(raw))
}

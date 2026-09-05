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
                // NOTE: no bare "а" — it is a live conjunction
                // ("а также", "а потом") and must survive.
                "аа",
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

/// Phrases that take a trailing comma when the sentence continues
/// (`например` -> `например,`). Ordered longest-first per language.
fn comma_after_phrases(lang: Language) -> &'static [&'static [&'static str]] {
    match lang {
        Language::Ru => &[
            &["конечно", "же"],
            &["честно", "говоря"],
            &["иными", "словами"],
            &["между", "прочим"],
            &["тем", "не", "менее"],
            &["к", "сожалению"],
            &["к", "счастью"],
            &["например"],
            &["конечно"],
            &["наверное"],
            &["пожалуй"],
            &["видимо"],
            &["кажется"],
            &["по-моему"],
            &["во-первых"],
            &["во-вторых"],
            &["в-третьих"],
            &["итак"],
            &["впрочем"],
        ],
        Language::En => &[
            &["however"],
            &["therefore"],
            &["anyway"],
            &["meanwhile"],
            &["finally"],
            &["obviously"],
            &["apparently"],
            &["unfortunately"],
            &["fortunately"],
            &["besides"],
            &["moreover"],
        ],
    }
}

/// Phrases that take a leading comma when they begin a subordinate clause
/// (`я подумал что ...` -> `я подумал, что ...`). Matched only mid-sentence.
fn comma_before_phrases(lang: Language) -> &'static [&'static [&'static str]] {
    match lang {
        Language::Ru => &[
            &["потому", "что"],
            &["так", "как"],
            &["что"],
            &["чтобы"],
            &["если"],
            &["хотя"],
            &["но"],
            &["поэтому"],
            &["который"],
            &["которая"],
            &["которое"],
            &["которые"],
            &["чей"],
            &["чья"],
            &["чьи"],
        ],
        Language::En => &[
            &["because"],
            &["though"],
            &["although"],
            &["while"],
            &["so"],
            &["yet"],
            &["but"],
        ],
    }
}

/// Insert heuristic commas into the token stream, leaving explicit
/// punctuation untouched. Kept deliberately conservative to avoid
/// embarrassing errors in dictation output.
fn insert_commas(tokens: &[Token], lang: Language) -> Vec<Token> {
    let mut after: Vec<&[&str]> = comma_after_phrases(lang).to_vec();
    let mut before: Vec<&[&str]> = comma_before_phrases(lang).to_vec();
    after.sort_by_key(|p| std::cmp::Reverse(p.len()));
    before.sort_by_key(|p| std::cmp::Reverse(p.len()));

    fn phrase_len(tokens: &[Token], i: usize, phrases: &[&[&str]]) -> Option<usize> {
        'outer: for p in phrases {
            if i + p.len() <= tokens.len() {
                for (k, pw) in p.iter().enumerate() {
                    match &tokens[i + k] {
                        Token::Word(w) if w.eq_ignore_ascii_case(pw) => {}
                        _ => continue 'outer,
                    }
                }
                return Some(p.len());
            }
        }
        None
    }

    let mut out: Vec<Token> = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Punct(c) => {
                out.push(Token::Punct(*c));
                i += 1;
            }
            Token::Word(_) => {
                if let Some(len) = phrase_len(tokens, i, &before) {
                    let sentence_open = !out.iter().any(|t| matches!(t, Token::Word(_)));
                    if !sentence_open && !matches!(out.last(), Some(Token::Punct(_))) {
                        out.push(Token::Punct(','));
                    }
                    for w in tokens.iter().skip(i).take(len) {
                        if let Token::Word(w) = w {
                            out.push(Token::Word(w.clone()));
                        }
                    }
                    i += len;
                    continue;
                }
                if let Some(len) = phrase_len(tokens, i, &after) {
                    let prev_word = out.iter().any(|t| matches!(t, Token::Word(_)));
                    if prev_word && !matches!(out.last(), Some(Token::Punct(_))) {
                        out.push(Token::Punct(','));
                    }
                    for w in tokens.iter().skip(i).take(len) {
                        if let Token::Word(w) = w {
                            out.push(Token::Word(w.clone()));
                        }
                    }
                    if matches!(tokens.get(i + len), Some(Token::Word(_)))
                        && !matches!(out.last(), Some(Token::Punct(_)))
                    {
                        out.push(Token::Punct(','));
                    }
                    i += len;
                    continue;
                }
                if let Token::Word(w) = &tokens[i] {
                    out.push(Token::Word(w.clone()));
                }
                i += 1;
            }
        }
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
    // A dot/comma between digits is a decimal separator, not a boundary:
    // glue it tight (`3 . 14` -> `3.14`).
    fn decimal_glue(tokens: &[Token], i: usize, c: char) -> bool {
        if !(c == '.' || c == ',') {
            return false;
        }
        let prev_digit = tokens[..i].iter().rev().find_map(|t| match t {
            Token::Word(w) => w.chars().last(),
            Token::Punct(p) => Some(*p),
        });
        let next_digit = tokens.get(i + 1).and_then(|t| match t {
            Token::Word(w) => w.chars().next(),
            Token::Punct(_) => None,
        });
        matches!(prev_digit, Some(d) if d.is_ascii_digit())
            && matches!(next_digit, Some(d) if d.is_ascii_digit())
    }

    let mut s = String::new();
    // Set when the previous mark was glued decimal-tight (`3.` so far):
    // the next word continues the number without a space.
    let mut glued = false;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Word(w) => {
                if !s.is_empty() && !s.ends_with(char::is_whitespace) && !glued {
                    s.push(' ');
                }
                glued = false;
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
                if decimal_glue(tokens, i, *c) {
                    while s.ends_with(char::is_whitespace) {
                        s.pop();
                    }
                    s.push(*c);
                    glued = true;
                    continue; // no trailing space either: `3.14`, not `3. 14`
                }
                glued = false;
                // Glue to the previous token: no space before a mark and
                // none between back-to-back marks ("12% ," -> "12%,",
                // "готово. ," -> "готово.,").
                while s.ends_with(char::is_whitespace) {
                    s.pop();
                }
                s.push(*c);
                s.push(' ');
            }
        }
    }
    s.trim().to_string()
}

/// Marks that collapse when stuck together (`.,` -> `.`, `,,` -> `,`).
const RUN_MARKS: [char; 7] = [',', '.', ';', ':', '!', '?', '…'];

/// Collapse runs of adjacent punctuation marks (`.,` -> `.`, `,,` -> `,`,
/// `!?` -> `?!`), preserving `...`, `?!` and single marks. Decimal points
/// and in-number commas survive (digits break the runs).
pub fn collapse_punctuation(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if !RUN_MARKS.contains(&chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && RUN_MARKS.contains(&chars[j]) {
            j += 1;
        }
        out.push_str(&collapse_run(&chars[i..j]));
        i = j;
    }
    out
}

fn collapse_run(run: &[char]) -> String {
    if run.iter().filter(|&&c| c == '.').count() >= 3 {
        return "...".to_string();
    }
    if run.contains(&'…') {
        return "…".to_string();
    }
    if run.contains(&'?') && run.contains(&'!') {
        return "?!".to_string();
    }
    if let Some(&c) = run.iter().find(|c| matches!(c, '.' | '?' | '!')) {
        return c.to_string();
    }
    // Weak-only run (`,`, `;`, `:`): keep the first mark.
    run.first().map(|c| c.to_string()).unwrap_or_default()
}

/// Uppercase the first letter of every sentence (`все. жду` -> `Все. Жду`).
/// A dot between digits (`3.14`) is not a boundary.
fn capitalize_sentences(s: &mut String) {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut after_terminator = true; // start of text is a boundary
    for (i, &c) in chars.iter().enumerate() {
        if c.is_whitespace() {
            out.push(c);
            continue;
        }
        if after_terminator && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            after_terminator = false;
            continue;
        }
        out.push(c);
        if c.is_alphabetic() {
            after_terminator = false;
        } else if matches!(c, '?' | '!' | '…') || (c == '.' && !is_decimal_dot(&chars, i)) {
            after_terminator = true;
        }
    }
    *s = out;
}

fn is_decimal_dot(chars: &[char], i: usize) -> bool {
    i > 0 && i + 1 < chars.len() && chars[i - 1].is_ascii_digit() && chars[i + 1].is_ascii_digit()
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

    let mut text = join_tokens(&insert_commas(&rebuilt, lang), lang);
    if text.is_empty() {
        return String::new();
    }
    if term.is_empty() {
        text.push(kind.punctuation());
    } else {
        text.push_str(term);
    }
    // The recognizer may glue or double marks ("12% ,", "готово.,");
    // collapse runs, then case every sentence start.
    text = collapse_punctuation(&text);
    capitalize_sentences(&mut text);
    text
}

/// Reduce raw dictation to a bare lowercase word sequence: fillers removed,
/// repeats collapsed, no punctuation and no casing. This is the right input
/// shape for downstream neural post-processors (e.g. punctuation models).
pub fn clean(raw: &str, lang: Language) -> String {
    let tokens = tokenize(raw.trim());
    let lower: Vec<String> = tokens
        .iter()
        .filter_map(|t| match t {
            Token::Word(w) => Some(w.to_lowercase()),
            Token::Punct(_) => None,
        })
        .collect();
    let mask = filler_mask(&lower, lang);

    let mut words: Vec<String> = Vec::new();
    let mut mi = 0usize;
    for token in &tokens {
        if let Token::Word(w) = token {
            mi += 1;
            if !mask[mi - 1] {
                continue;
            }
            let lower = w.to_lowercase();
            if let Some(last) = words.last() {
                if last == &lower {
                    continue;
                }
            }
            words.push(lower);
        }
    }
    words.join(" ")
}

/// Convenience wrapper around [`format`] that auto-detects the language.
pub fn format_raw(raw: &str) -> String {
    format(raw, Language::detect(raw))
}

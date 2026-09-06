// SPDX-License-Identifier: MIT
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

    /// Explicit user choice wins (`FLOWVOICE_LANG`), otherwise auto-detect.
    /// A fixed language setting disables auto-detection (I-07).
    pub fn resolve(explicit: Option<&str>, text: &str) -> Language {
        match explicit.map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("ru") | Some("russian") | Some("rus") => Language::Ru,
            Some("en") | Some("english") | Some("eng") => Language::En,
            _ => Language::detect(text),
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

/// Formatting switches beyond the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FormatOpts {
    /// Rewrite digit runs as Russian words (`12` -> `двенадцать`, F-10).
    /// Off by default: recognizers emit digits and users expect digits.
    pub numbers_words: bool,
}

/// Format a raw dictation string into final, publish-ready text.
///
/// Runs the full pipeline: cleanup -> filler removal -> dedup ->
/// sentence classification -> punctuation -> capitalization.
pub fn format(raw: &str, lang: Language) -> String {
    format_with(raw, lang, FormatOpts::default())
}

/// [`format`] with explicit switches.
pub fn format_with(raw: &str, lang: Language, opts: FormatOpts) -> String {
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
    // collapse runs, normalize dates/times, optionally spell out numbers,
    // then case every sentence start.
    text = collapse_punctuation(&text);
    text = normalize_dates(&text);
    text = normalize_times(&text);
    if opts.numbers_words {
        text = digits_to_words(&text, true);
    }
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

/// Words in a bare transcript. Short texts (`<= 10` words) skip the neural
/// punctuator in favor of deterministic rules (J-09).
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Minimum-length gate for finalized replicas ("порог короткой реплики"):
/// stray one-char marks never become pastes. Counts Unicode chars.
pub fn meets_min_len(text: &str, min_chars: usize) -> bool {
    text.chars().count() >= min_chars
}

/// Spell an integer in Russian words (F-10 `FLOWVOICE_NUMBERS=words`).
/// Covers 0..=999_999_999; out-of-range input passes through unchanged.
pub fn num_to_ru_words(n: i64) -> Option<String> {
    if !(0..=999_999_999).contains(&n) {
        return None;
    }
    const HUNDREDS: &[&str] = &[
        "",
        "сто",
        "двести",
        "триста",
        "четыреста",
        "пятьсот",
        "шестьсот",
        "семьсот",
        "восемьсот",
        "девятьсот",
    ];
    const TENS: &[&str] = &[
        "",
        "",
        "двадцать",
        "тридцать",
        "сорок",
        "пятьдесят",
        "шестьдесят",
        "семьдесят",
        "восемьдесят",
        "девяносто",
    ];
    const TEENS: &[&str] = &[
        "десять",
        "одиннадцать",
        "двенадцать",
        "тринадцать",
        "четырнадцать",
        "пятнадцать",
        "шестнадцать",
        "семнадцать",
        "восемнадцать",
        "девятнадцать",
    ];
    const ONES_M: &[&str] = &[
        "",
        "один",
        "два",
        "три",
        "четыре",
        "пять",
        "шесть",
        "семь",
        "восемь",
        "девять",
    ];
    const ONES_F: &[&str] = &[
        "",
        "одна",
        "две",
        "три",
        "четыре",
        "пять",
        "шесть",
        "семь",
        "восемь",
        "девять",
    ];
    fn group(v: i64, ones: &[&str], out: &mut Vec<String>) {
        let h = (v / 100) as usize;
        let t = ((v % 100) / 10) as usize;
        let o = (v % 10) as usize;
        if h > 0 {
            out.push(HUNDREDS[h].to_string());
        }
        if t == 1 {
            out.push(TEENS[o].to_string());
        } else {
            if t > 1 {
                out.push(TENS[t].to_string());
            }
            if o > 0 {
                out.push(ones[o].to_string());
            }
        }
    }
    fn plural(n: i64, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
        let n10 = n % 10;
        let n100 = n % 100;
        if n10 == 1 && n100 != 11 {
            one
        } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
            few
        } else {
            many
        }
    }
    if n == 0 {
        return Some("ноль".to_string());
    }
    let mut out: Vec<String> = Vec::new();
    let millions = n / 1_000_000;
    if millions > 0 {
        group(millions, ONES_M, &mut out);
        out.push(plural(millions, "миллион", "миллиона", "миллионов").to_string());
    }
    let thousands = (n % 1_000_000) / 1000;
    if thousands > 0 {
        group(thousands, ONES_F, &mut out);
        out.push(plural(thousands, "тысяча", "тысячи", "тысяч").to_string());
    }
    group(n % 1000, ONES_M, &mut out);
    Some(out.join(" "))
}

/// Rewrite digit runs as Russian words when `words` mode is on (F-10).
/// Runs attached to letters (`COVID-19`, `gpt-4`) are left alone.
pub fn digits_to_words(text: &str, words_mode: bool) -> String {
    if !words_mode {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        let run: String = chars[i..j].iter().collect();
        // Attached to letters (COVID-19) or separators of dates/times
        // (12.03, 15:30, 12/03): leave alone.
        let attached = (i > 0
            && (chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '.' | '/' | '-' | ':')))
            || (j < chars.len()
                && (chars[j].is_alphanumeric() || matches!(chars[j], '.' | '/' | '-' | ':')));
        if attached {
            out.push_str(&run);
        } else if let Ok(n) = run.parse::<i64>() {
            match num_to_ru_words(n) {
                Some(w) => out.push_str(&w),
                None => out.push_str(&run),
            }
        } else {
            out.push_str(&run);
        }
        i = j;
    }
    out
}

/// Normalize dates to `DD.MM[.YYYY]` (F-11): `12.03`, `12/03/2024`,
/// `12 марта`, `12 марта 2024` (genitive month names).
pub fn normalize_dates(text: &str) -> String {
    const MONTHS: &[(&str, &str)] = &[
        ("января", "01"),
        ("февраля", "02"),
        ("марта", "03"),
        ("апреля", "04"),
        ("мая", "05"),
        ("июня", "06"),
        ("июля", "07"),
        ("августа", "08"),
        ("сентября", "09"),
        ("октября", "10"),
        ("ноября", "11"),
        ("декабря", "12"),
    ];
    let mut out = text.to_string();
    for (name, num) in MONTHS {
        // `12 марта [2024]` -> `12.03[.2024]`, word-boundary aware.
        let mut res = String::with_capacity(out.len());
        let chars: Vec<char> = out.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_ascii_digit() {
                let mut j = i;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                let day: String = chars[i..j].iter().collect();
                let mut k = j;
                while k < chars.len() && (chars[k] == ' ' || chars[k] == '\u{a0}') {
                    k += 1;
                }
                let name_chars: Vec<char> = name.chars().collect();
                let matches_month = day.len() <= 2
                    && day
                        .parse::<u32>()
                        .map(|d| (1..=31).contains(&d))
                        .unwrap_or(false)
                    && chars[k..].starts_with(&name_chars);
                if matches_month {
                    let after = k + name_chars.len();
                    let after_ok = chars.get(after).map(|c| !c.is_alphabetic()).unwrap_or(true);
                    if after_ok {
                        res.push_str(&format!("{:0>2}.{num}", day.parse::<u32>().unwrap()));
                        // Optional year right after: `12 марта 2024`.
                        let mut y = after;
                        while y < chars.len() && chars[y].is_whitespace() {
                            y += 1;
                        }
                        let mut z = y;
                        while z < chars.len() && chars[z].is_ascii_digit() {
                            z += 1;
                        }
                        if z - y == 4 {
                            res.push('.');
                            res.push_str(&chars[y..z].iter().collect::<String>());
                            i = z;
                        } else {
                            i = after;
                        }
                        continue;
                    }
                }
                res.push_str(&day);
                i = j;
                continue;
            }
            res.push(chars[i]);
            i += 1;
        }
        out = res;
    }
    // Numeric `D/M[/Y]` with slashes or dashes -> dots: `12/03` -> `12.03`.
    let mut res = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < chars.len()
                && (chars[j] == '/' || chars[j] == '-')
                && j + 1 < chars.len()
                && chars[j + 1].is_ascii_digit()
            {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let (a, b) = (
                    &chars[i..j].iter().collect::<String>(),
                    &chars[j + 1..k].iter().collect::<String>(),
                );
                if a.len() <= 2 && b.len() == 2 {
                    res.push_str(&format!("{a:0>2}.{b}"));
                    i = k;
                    continue;
                }
            }
            res.push_str(&chars[i..j].iter().collect::<String>());
            i = j;
            continue;
        }
        res.push(chars[i]);
        i += 1;
    }
    res
}

/// Trailing sentence marks carried by a consumed word (`часа.` -> `.`).
fn trailing_marks(word: &str) -> String {
    word.chars()
        .rev()
        .take_while(|c| matches!(c, '.' | ',' | '?' | '!'))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Normalize spoken times to `H:MM` (F-12): `15:30` stays, `в 3 часа`,
/// `5 часов 20 минут`, `в 7 вечера` (evening +12h, best-effort).
pub fn normalize_times(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let words: Vec<&str> = text.split(' ').collect();
    let mut i = 0;
    while i < words.len() {
        if let Ok(h) = words[i].parse::<u32>() {
            if h <= 23 && i + 1 < words.len() {
                let next = words[i + 1].to_lowercase();
                // Bare evening marker: `в 7 вечера` -> `19:00`.
                if ["вечера", "вечером", "ночи", "ночью"].contains(&next.as_str()) && h < 12
                {
                    out.push_str(&format!("{}:00{} ", h + 12, trailing_marks(words[i + 1])));
                    i += 2;
                    continue;
                }
                let is_hour = next.starts_with("час");
                if is_hour {
                    let mut mm = "00".to_string();
                    let mut skip = 2;
                    if i + 2 < words.len() {
                        if let Ok(m) = words[i + 2].parse::<u32>() {
                            if m <= 59
                                && i + 3 < words.len()
                                && words[i + 3].to_lowercase().starts_with("минут")
                            {
                                mm = format!("{m:02}");
                                skip = 4;
                            }
                        }
                    }
                    // Evening marker after the hour word(s).
                    let mut hh = h;
                    if i + skip < words.len()
                        && ["вечера", "вечером", "ночи", "ночью"]
                            .contains(&words[i + skip].to_lowercase().as_str())
                        && hh < 12
                    {
                        hh += 12;
                        skip += 1;
                    }
                    let tail = trailing_marks(words[i + skip - 1]);
                    out.push_str(&format!("{hh}:{mm}{tail} "));
                    i += skip;
                    continue;
                }
            }
        }
        out.push_str(words[i]);
        out.push(' ');
        i += 1;
    }
    out.trim_end().to_string()
}

/// Post-processing profile (J-01/J-02/J-04/J-05): how much the pipeline may
/// reshape a replica. `Auto` resolves from the foreground app title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Auto,
    Chat,
    Mail,
    Code,
}

impl Profile {
    pub fn parse(s: &str) -> Self {
        // Full Unicode lowercase: aliases include Cyrillic ("ПОЧТА").
        match s.to_lowercase().as_str() {
            "chat" | "messenger" | "мессенджер" => Self::Chat,
            "mail" | "post" | "почта" => Self::Mail,
            "code" | "код" => Self::Code,
            _ => Self::Auto,
        }
    }

    /// Guess the profile from a foreground window title (J-04).
    pub fn detect(app_title: &str) -> Self {
        let t = app_title.to_lowercase();
        const CHAT: &[&str] = &["telegram", "whatsapp", "discord", "slack", "skype", "viber"];
        const MAIL: &[&str] = &["outlook", "thunderbird", "gmail", "mail", "почта"];
        const CODE: &[&str] = &[
            "visual studio",
            "vscode",
            "idea",
            "pycharm",
            "terminal",
            "powershell",
            "cmd",
            "neovim",
            "vim",
        ];
        if CHAT.iter().any(|k| t.contains(k)) {
            Self::Chat
        } else if MAIL.iter().any(|k| t.contains(k)) {
            Self::Mail
        } else if CODE.iter().any(|k| t.contains(k)) {
            Self::Code
        } else {
            Self::Mail
        }
    }

    /// Resolve `Auto` through the app title; explicit choice wins (J-05).
    pub fn resolve(self, app_title: &str) -> Self {
        match self {
            Self::Auto => Self::detect(app_title),
            other => other,
        }
    }
}

/// Code-dictation formatting (J-03): keep identifiers verbatim — no filler
/// removal, no case changes, no appended terminal mark. Only whitespace is
/// tidied and repeats collapsed.
pub fn format_code(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for w in raw.split_whitespace() {
        if let Some(last) = out.last() {
            if last.eq_ignore_ascii_case(w) {
                continue;
            }
        }
        out.push(w.to_string());
    }
    out.join(" ")
}

/// Leading separator between consecutive replicas (AM-03): with the option
/// on, an alphanumeric start gains one leading space so back-to-back
/// pastes don't glue words together.
pub fn pad_replica_start(text: &str, enabled: bool) -> String {
    if enabled {
        if let Some(c) = text.chars().next() {
            if c.is_alphanumeric() {
                return format!(" {text}");
            }
        }
    }
    text.to_string()
}

/// Crude Russian→Latin transliteration for the `транслит:` voice command
/// (I-08). Offline lookup table, no model involved.
pub fn transliterate_ru(text: &str) -> String {
    fn map(c: char) -> &'static str {
        match c {
            'а' => "a",
            'б' => "b",
            'в' => "v",
            'г' => "g",
            'д' => "d",
            'е' | 'ё' => "e",
            'ж' => "zh",
            'з' => "z",
            'и' => "i",
            'й' => "y",
            'к' => "k",
            'л' => "l",
            'м' => "m",
            'н' => "n",
            'о' => "o",
            'п' => "p",
            'р' => "r",
            'с' => "s",
            'т' => "t",
            'у' => "u",
            'ф' => "f",
            'х' => "kh",
            'ц' => "ts",
            'ч' => "ch",
            'ш' => "sh",
            'щ' => "shch",
            'ъ' | 'ь' => "",
            'ы' => "y",
            'э' => "e",
            'ю' => "yu",
            'я' => "ya",
            'А' => "A",
            'Б' => "B",
            'В' => "V",
            'Г' => "G",
            'Д' => "D",
            'Е' | 'Ё' => "E",
            'Ж' => "Zh",
            'З' => "Z",
            'И' => "I",
            'Й' => "Y",
            'К' => "K",
            'Л' => "L",
            'М' => "M",
            'Н' => "N",
            'О' => "O",
            'П' => "P",
            'Р' => "R",
            'С' => "S",
            'Т' => "T",
            'У' => "U",
            'Ф' => "F",
            'Х' => "Kh",
            'Ц' => "Ts",
            'Ч' => "Ch",
            'Ш' => "Sh",
            'Щ' => "Shch",
            'Ъ' | 'Ь' => "",
            'Ы' => "Y",
            'Э' => "E",
            'Ю' => "Yu",
            'Я' => "Ya",
            _ => "",
        }
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c);
        } else if matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё') {
            out.push_str(map(c));
        } else {
            out.push(c);
        }
    }
    out
}

/// Convenience wrapper around [`format`] that auto-detects the language.
pub fn format_raw(raw: &str) -> String {
    format(raw, Language::detect(raw))
}

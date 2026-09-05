//! Russian punctuation + capitalization restoration.
//!
//! Runs the `RUPunct/RUPunct_small` INT8 ONNX model with the pure-Rust
//! `tract-onnx` runtime: no native DLLs, no network, no Python. The model
//! classifies every input sub-token into one of 33 labels (3 case modes
//! x 11 punctuation classes) and restores commas, periods, question marks,
//! capitalization and more on plain lowercase dictation output.
//!
//! Model files are downloaded by `scripts/get-native.ps1` into
//! `models/punct/`:
//!   * `rupunct_small_int8.onnx`
//!   * `tokenizer.json`  (HF WordPiece tokenizer)
//!
//! The heavy `tract-onnx` dependency only builds when the `onnx` feature
//! is enabled, so the hermetic default workspace build stays lightweight.

#![cfg(feature = "onnx")]

use std::collections::HashMap;
use tract_onnx::prelude::*;

const CLS: i64 = 2;
const SEP: i64 = 3;
const UNK: i64 = 1;
const MAX_SEQ: usize = 512;

/// `id2label` order taken from the model's `config.json` (33 labels).
const LABELS: [&str; 33] = [
    "UPPER_PERIOD",
    "LOWER_PERIOD",
    "UPPER_TOTAL_PERIOD",
    "UPPER_COMMA",
    "LOWER_COMMA",
    "UPPER_TOTAL_COMMA",
    "UPPER_QUESTION",
    "LOWER_QUESTION",
    "UPPER_TOTAL_QUESTION",
    "UPPER_TIRE",
    "LOWER_TIRE",
    "UPPER_TOTAL_TIRE",
    "UPPER_VOSKL",
    "LOWER_VOSKL",
    "UPPER_TOTAL_VOSKL",
    "UPPER_DVOETOCHIE",
    "LOWER_DVOETOCHIE",
    "UPPER_TOTAL_DVOETOCHIE",
    "UPPER_PERIODCOMMA",
    "LOWER_PERIODCOMMA",
    "UPPER_TOTAL_PERIODCOMMA",
    "UPPER_DEFIS",
    "LOWER_DEFIS",
    "UPPER_TOTAL_DEFIS",
    "UPPER_QUESTIONVOSKL",
    "LOWER_QUESTIONVOSKL",
    "UPPER_TOTAL_QUESTIONVOSKL",
    "UPPER_MNOGOTOCHIE",
    "LOWER_MNOGOTOCHIE",
    "UPPER_TOTAL_MNOGOTOCHIE",
    "UPPER_O",
    "LOWER_O",
    "UPPER_TOTAL_O",
];

type Runner = Box<dyn Fn(&[i64], &[i64], &[i64], usize) -> Result<Vec<f32>, String> + Send + Sync>;

/// Russian punctuation restorer backed by a neural token classifier.
pub struct Punctuator {
    vocab: HashMap<String, i64>,
    runner: Runner,
}

impl Punctuator {
    /// Load the ONNX model and the WordPiece tokenizer from disk.
    pub fn load(model_path: &str, tokenizer_path: &str) -> Result<Self, String> {
        let model = tract_onnx::onnx()
            .model_for_path(model_path)
            .map_err(|e| format!("cannot load onnx model: {e}"))?;
        let model = model.into_optimized().map_err(|e| e.to_string())?;
        let model = model.into_runnable().map_err(|e| e.to_string())?;

        let runner: Runner = Box::new(move |ids, attn, types, seq| {
            let batch = ids.len() / seq;
            let t_ids = Tensor::from_shape(&[batch, seq], ids).map_err(|e| e.to_string())?;
            let t_attn = Tensor::from_shape(&[batch, seq], attn).map_err(|e| e.to_string())?;
            let t_types = Tensor::from_shape(&[batch, seq], types).map_err(|e| e.to_string())?;
            let mut outputs = model
                .run(tvec!(t_ids.into(), t_attn.into(), t_types.into()))
                .map_err(|e| e.to_string())?;
            let out = outputs
                .pop()
                .ok_or("model produced no output".to_string())?;
            let tensor = out.into_tensor();
            let view = tensor.to_array_view::<f32>().map_err(|e| e.to_string())?;
            Ok(view.iter().copied().collect())
        });

        let vocab = parse_vocab(tokenizer_path)?;
        Ok(Self { vocab, runner })
    }

    /// Restore punctuation and casing on lowercase dictation text.
    pub fn punct(&self, text: &str) -> Result<String, String> {
        let input = text.trim().to_lowercase();
        let pieces = basic_split(&input);
        let (ids, attn, types, first_of_piece) = self.encode(&input);
        if pieces.is_empty() {
            return Ok(String::new());
        }
        let logits = (self.runner)(&ids, &attn, &types, ids.len())?;

        let mut parts: Vec<String> = Vec::with_capacity(pieces.len());
        for (pi, piece) in pieces.iter().enumerate() {
            let pos = first_of_piece[pi];
            if pos >= ids.len() {
                parts.push(piece.clone());
                continue;
            }
            let start = pos * 33;
            let slice = &logits[start..start + 33];
            let (li, _) = slice
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                .expect("33 labels per position");
            parts.push(process_piece(piece, LABELS[li]));
        }
        Ok(parts.join(" ").trim().to_string())
    }

    /// BERT-style encode: `[CLS]` + WordPiece ids + `[SEP]`.
    fn encode(&self, text: &str) -> (Vec<i64>, Vec<i64>, Vec<i64>, Vec<usize>) {
        let pieces = basic_split(text);
        let mut ids: Vec<i64> = vec![CLS];
        let mut first_of_piece: Vec<usize> = Vec::with_capacity(pieces.len());
        for piece in &pieces {
            first_of_piece.push(ids.len());
            for id in self.wordpiece(piece) {
                ids.push(id);
            }
        }
        ids.push(SEP);
        if ids.len() > MAX_SEQ {
            ids.truncate(MAX_SEQ - 1);
            ids.push(SEP);
        }
        let n = ids.len();
        (ids, vec![1i64; n], vec![0i64; n], first_of_piece)
    }

    /// Greedy BERT WordPiece with the `##` continuation prefix.
    fn wordpiece(&self, word: &str) -> Vec<i64> {
        let chars: Vec<char> = word.chars().collect();
        if chars.is_empty() || chars.len() > 100 {
            return vec![UNK];
        }
        let mut tokens = Vec::new();
        let mut start = 0usize;
        while start < chars.len() {
            let mut end = chars.len();
            let mut found: Option<i64> = None;
            while start < end {
                let sub: String = if start == 0 {
                    chars[start..end].iter().collect()
                } else {
                    let mut s = String::from("##");
                    s.extend(chars[start..end].iter());
                    s
                };
                if let Some(id) = self.vocab.get(&sub) {
                    found = Some(*id);
                    break;
                }
                end -= 1;
            }
            match found {
                Some(id) => {
                    tokens.push(id);
                    start = end;
                }
                None => return vec![UNK],
            }
        }
        tokens
    }
}

/// Split like BERT's `BasicTokenizer`: whitespace first, then any
/// punctuation — ASCII ranges plus Unicode punctuation (`—`, `«»`, ...).
fn is_bert_punct(c: char) -> bool {
    !c.is_whitespace() && !c.is_alphanumeric()
}

fn split_on_punct(s: &str, out: &mut Vec<String>) {
    let mut cur = String::new();
    for c in s.chars() {
        if is_bert_punct(c) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
}

fn basic_split(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in text.split_whitespace() {
        split_on_punct(chunk, &mut out);
    }
    out
}

/// Read the WordPiece vocab from a HF `tokenizer.json`.
fn parse_vocab(path: &str) -> Result<HashMap<String, i64>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read tokenizer: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("bad tokenizer.json: {e}"))?;
    let vocab = value["model"]["vocab"]
        .as_object()
        .ok_or("tokenizer.json has no model.vocab".to_string())?;
    vocab
        .iter()
        .map(|(k, v)| {
            let id = v
                .as_i64()
                .ok_or_else(|| format!("bad vocab id for {k:?}"))?;
            Ok((k.clone(), id))
        })
        .collect()
}

/// Apply case + trailing punctuation from a RUPunct label.
fn process_piece(token: &str, label: &str) -> String {
    let mut parts = label.split('_');
    let case = parts.next().unwrap_or("LOWER");
    let mut kind = parts.next().unwrap_or("O");
    let total = kind == "TOTAL";
    if total {
        kind = parts.next().unwrap_or("O");
    }

    let word = match (case, total) {
        ("UPPER", true) => token.to_uppercase(),
        ("UPPER", false) => capitalize(token),
        _ => token.to_string(),
    };
    let tail = match kind {
        "O" => "",
        "PERIOD" => ".",
        "COMMA" => ",",
        "QUESTION" => "?",
        "TIRE" => " —",
        "VOSKL" => "!",
        "DVOETOCHIE" => ":",
        "PERIODCOMMA" => ";",
        "DEFIS" => "-",
        "MNOGOTOCHIE" => "...",
        "QUESTIONVOSKL" => "?!",
        _ => "",
    };
    format!("{word}{tail}")
}

/// Python's `str.capitalize()`: first char uppercased, the rest lowercased.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_uppercase().collect::<String>();
            out.push_str(&chars.as_str().to_lowercase());
            out
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_o_labels() {
        assert_eq!(process_piece("привет", "LOWER_O"), "привет");
        assert_eq!(process_piece("привет", "UPPER_O"), "Привет");
        assert_eq!(process_piece("привет", "UPPER_TOTAL_O"), "ПРИВЕТ");
    }

    #[test]
    fn process_punctuation_labels() {
        assert_eq!(process_piece("мир", "LOWER_COMMA"), "мир,");
        assert_eq!(process_piece("мир", "UPPER_PERIOD"), "Мир.");
        assert_eq!(process_piece("стоп", "UPPER_TOTAL_VOSKL"), "СТОП!");
        assert_eq!(process_piece("да", "LOWER_QUESTION"), "да?");
        assert_eq!(process_piece("да", "LOWER_MNOGOTOCHIE"), "да...");
        assert_eq!(process_piece("так", "LOWER_TIRE"), "так —");
    }

    #[test]
    fn basic_split_keeps_punctuation_pieces() {
        assert_eq!(basic_split("привет, мир!"), ["привет", ",", "мир", "!"]);
        assert_eq!(basic_split("hello world"), ["hello", "world"]);
        assert_eq!(basic_split("а—б"), ["а", "—", "б"]);
    }
}

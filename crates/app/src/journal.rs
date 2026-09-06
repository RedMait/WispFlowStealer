// SPDX-License-Identifier: MIT
//! Per-replica journal: one JSON object per line (T-01), local file.
//!
//! Dependency-free on purpose (`std` only) so statistics work on every
//! platform and in every feature combination. Each line carries the fields
//! the checkers require: unique id (T-13), UTC timestamp (T-12), backend,
//! language, foreground app, sizes, latencies and WPM (AL-02).

use std::sync::atomic::{AtomicU64, Ordering};

/// One journaled replica.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Unique id: `<unix_nanos>-<process_counter>`.
    pub id: String,
    /// Unix seconds (UTC). Rendered with timezone by consumers (T-12).
    pub ts: u64,
    /// Engine used: `groq cloud` / `whisper local` / `vosk`.
    pub backend: String,
    /// `ru` / `en` / `auto`.
    pub lang: String,
    /// Foreground window title at release time (may be empty).
    pub app: String,
    /// Final text length in chars.
    pub chars: usize,
    /// Word count of the final text.
    pub words: usize,
    /// Seconds from hotkey release to finished paste.
    pub secs: f32,
    /// Words per minute over the captured audio span (0 when unknown).
    pub wpm: f32,
    /// Captured audio span in seconds (0 when unknown).
    pub audio_secs: f32,
    /// Delivery method: `paste` / `clipboard-fallback`.
    pub method: String,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build an entry id unique within and across runs.
pub fn make_id(ts_nanos: u128) -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{ts_nanos}-{n}")
}

/// Words per minute for `words` over `audio_secs` (0 when not measurable).
pub fn wpm(words: usize, audio_secs: f32) -> f32 {
    if audio_secs > 0.0 {
        words as f32 / (audio_secs / 60.0)
    } else {
        0.0
    }
}

fn esc_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

/// Serialize one entry as a single JSON line (T-01).
pub fn to_line(e: &Entry) -> String {
    let mut s = String::with_capacity(256);
    s.push('{');
    s.push_str("\"id\":\"");
    esc_into(&mut s, &e.id);
    s.push_str("\",\"ts\":");
    s.push_str(&e.ts.to_string());
    s.push_str(",\"backend\":\"");
    esc_into(&mut s, &e.backend);
    s.push_str("\",\"lang\":\"");
    esc_into(&mut s, &e.lang);
    s.push_str("\",\"app\":\"");
    esc_into(&mut s, &e.app);
    s.push_str("\",\"chars\":");
    s.push_str(&e.chars.to_string());
    s.push_str(",\"words\":");
    s.push_str(&e.words.to_string());
    s.push_str(",\"secs\":");
    s.push_str(&format!("{:.2}", e.secs));
    s.push_str(",\"wpm\":");
    s.push_str(&format!("{:.1}", e.wpm));
    s.push_str(",\"audio_secs\":");
    s.push_str(&format!("{:.2}", e.audio_secs));
    s.push_str(",\"method\":\"");
    esc_into(&mut s, &e.method);
    s.push_str("\"}");
    s
}

/// Append one entry to the journal file (creates parent dirs).
pub fn append(path: &std::path::Path, e: &Entry) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{}", to_line(e))?;
    Ok(())
}

/// Minimal parser for our own flat lines: strings, integers, floats.
/// Corrupt lines are skipped by the caller (fail-safe journal reads).
fn parse_line(line: &str) -> Option<Entry> {
    let body = line.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut id = None;
    let mut ts = None;
    let mut backend = None;
    let mut lang = None;
    let mut app = None;
    let mut chars = None;
    let mut words = None;
    let mut secs = None;
    let mut wpm_v = None;
    let mut audio_secs = None;
    let mut method = None;
    for field in split_fields(body) {
        let (k, v) = field.split_once(':')?;
        let k = unquote(k.trim())?;
        match k.as_str() {
            "id" => id = unquote(v.trim()),
            "ts" => ts = v.trim().parse().ok(),
            "backend" => backend = unquote(v.trim()),
            "lang" => lang = unquote(v.trim()),
            "app" => app = unquote(v.trim()),
            "chars" => chars = v.trim().parse().ok(),
            "words" => words = v.trim().parse().ok(),
            "secs" => secs = v.trim().parse().ok(),
            "wpm" => wpm_v = v.trim().parse().ok(),
            "audio_secs" => audio_secs = v.trim().parse().ok(),
            "method" => method = unquote(v.trim()),
            _ => {}
        }
    }
    Some(Entry {
        id: id?,
        ts: ts?,
        backend: backend?,
        lang: lang?,
        app: app?,
        chars: chars?,
        words: words?,
        secs: secs?,
        wpm: wpm_v?,
        audio_secs: audio_secs?,
        method: method?,
    })
}

/// Split top-level `a:b` fields, respecting quoted strings.
fn split_fields(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in body.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' if in_str => esc = true,
            '"' => in_str = !in_str,
            ',' if !in_str => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&body[start..]);
    out
}

/// Unquote a `"..."` JSON string (handles the escapes we emit).
fn unquote(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next()? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let hex: String = [it.next()?, it.next()?, it.next()?, it.next()?]
                    .iter()
                    .collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Read all valid entries, skipping corrupt lines (O-11 style fail-safe).
pub fn read_all(path: &std::path::Path) -> Vec<Entry> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines().filter_map(parse_line).collect()
}

/// Aggregate statistics over entries (T-03/T-05, AL-04/05/06/10/14).
#[derive(Debug, Default, PartialEq)]
pub struct Stats {
    pub replicas: usize,
    pub words: usize,
    pub avg_secs: f32,
    pub avg_wpm: f32,
    pub best_wpm: f32,
}

/// Stats over entries from the last `day_start_ts` (unix secs) onwards;
/// pass 0 for all-time.
pub fn stats_since(entries: &[Entry], day_start_ts: u64) -> Stats {
    let sel: Vec<&Entry> = entries.iter().filter(|e| e.ts >= day_start_ts).collect();
    if sel.is_empty() {
        return Stats::default();
    }
    let n = sel.len() as f32;
    // WPM averages skip unmeasurable replicas (GUI re-pastes carry 0).
    let measured: Vec<&Entry> = sel.iter().filter(|e| e.wpm > 0.0).copied().collect();
    let m = measured.len() as f32;
    Stats {
        replicas: sel.len(),
        words: sel.iter().map(|e| e.words).sum(),
        avg_secs: sel.iter().map(|e| e.secs).sum::<f32>() / n,
        avg_wpm: if m > 0.0 {
            measured.iter().map(|e| e.wpm).sum::<f32>() / m
        } else {
            0.0
        },
        best_wpm: sel.iter().map(|e| e.wpm).fold(0.0f32, f32::max),
    }
}

/// CSV export with wpm + word columns (T-08, AL-12). GUI feature.
#[cfg(feature = "gui")]
pub fn to_csv(entries: &[Entry]) -> String {
    let mut s = "id,ts,backend,lang,app,chars,words,secs,wpm,audio_secs,method\n".to_string();
    for e in entries {
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{:.2},{:.1},{:.2},{}\n",
            csv_cell(&e.id),
            e.ts,
            csv_cell(&e.backend),
            csv_cell(&e.lang),
            csv_cell(&e.app),
            e.chars,
            e.words,
            e.secs,
            e.wpm,
            e.audio_secs,
            csv_cell(&e.method),
        ));
    }
    s
}

#[cfg(feature = "gui")]
fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Entry {
        Entry {
            id: "1-0".to_string(),
            ts: 1_700_000_000,
            backend: "groq cloud".to_string(),
            lang: "ru".to_string(),
            app: "Telegram".to_string(),
            chars: 12,
            words: 2,
            secs: 2.5,
            wpm: 96.0,
            audio_secs: 1.25,
            method: "paste".to_string(),
        }
    }

    #[test]
    fn roundtrip_escapes() {
        let mut e = sample();
        e.app = "a\"b\\c\nnewline, comma".to_string();
        let line = to_line(&e);
        assert!(!line.contains('\n'));
        let back = parse_line(&line).expect("parses own output");
        assert_eq!(back, e);
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let good = to_line(&sample());
        let text = format!("{good}\nnot json\n{{\"id\":1}}\n{good}\n");
        let parsed: Vec<Entry> = text.lines().filter_map(parse_line).collect();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn stats_math() {
        let mut a = sample();
        a.words = 10;
        a.secs = 2.0;
        a.wpm = 100.0;
        let mut b = sample();
        b.ts += 10;
        b.words = 20;
        b.secs = 4.0;
        b.wpm = 200.0;
        let s = stats_since(&[a, b], 0);
        assert_eq!(s.replicas, 2);
        assert_eq!(s.words, 30);
        assert!((s.avg_secs - 3.0).abs() < 1e-6);
        assert!((s.avg_wpm - 150.0).abs() < 1e-6);
        assert!((s.best_wpm - 200.0).abs() < 1e-6);
        let day = stats_since(&[sample()], sample().ts + 1);
        assert_eq!(day, Stats::default());
    }

    #[test]
    fn avg_wpm_skips_unmeasured() {
        let mut a = sample();
        a.wpm = 100.0;
        let mut b = sample();
        b.wpm = 0.0;
        let s = stats_since(&[a, b], 0);
        assert!((s.avg_wpm - 100.0).abs() < 1e-6);
        assert_eq!(s.replicas, 2);
    }

    #[cfg(feature = "gui")]
    #[test]
    fn csv_shape() {
        let csv = to_csv(&[sample()]);
        let mut lines = csv.lines();
        assert_eq!(
            lines.next().unwrap(),
            "id,ts,backend,lang,app,chars,words,secs,wpm,audio_secs,method"
        );
        let row = lines.next().unwrap();
        assert!(row.contains("groq cloud"));
        assert!(row.contains("96.0"));
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn wpm_guards() {
        assert!((wpm(120, 60.0) - 120.0).abs() < 1e-6);
        assert_eq!(wpm(10, 0.0), 0.0);
    }

    #[test]
    fn ids_unique() {
        assert_ne!(make_id(1), make_id(1));
    }

    #[test]
    fn append_creates_dirs_and_reads_back() {
        let dir = std::env::temp_dir().join(format!(
            "flowvoice-jtest-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let path = dir.join("sub").join("journal.jsonl");
        append(&path, &sample()).expect("append works");
        let back = read_all(&path);
        assert_eq!(back, vec![sample()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

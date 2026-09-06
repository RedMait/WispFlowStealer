// SPDX-License-Identifier: MIT
//! Groq Cloud speech-to-text (primary backend when configured).
//!
//! Sends the buffered utterance to
//! `https://api.groq.com/openai/v1/audio/transcriptions`
//! (`whisper-large-v3-turbo` by default) via `curl.exe` — the system curl
//! owns TLS, so Rust needs no HTTP/TLS dependencies and the hermetic
//! default build stays untouched.
//!
//! Setup: create a key at https://console.groq.com/keys and expose it as
//! `GROQ_API_KEY` (process env only, never committed). Without a key this
//! backend is silently skipped and the local engines are used.
//!
//! Chain: Groq (key set) -> resident whisper-server (files present) ->
//! Vosk (model present) -> error. See [`crate::audio::transcribe`].

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Default Groq model: best price/performance for multilingual dictation.
/// `whisper-large-v3` is the accuracy-first alternative ($0.111/hr vs $0.04).
const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";

/// Groq model override via `FLOWVOICE_GROQ_MODEL`.
fn model() -> String {
    std::env::var("FLOWVOICE_GROQ_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string())
}

/// API key: process env wins, otherwise Windows Credential Manager
/// (`flowvoice/GROQ_API_KEY`, written by `--set-key`). Never logged (P-07).
fn api_key() -> Result<String, String> {
    if let Ok(k) = std::env::var("GROQ_API_KEY") {
        if !k.is_empty() {
            return Ok(k);
        }
    }
    read_key_from_store()
        .ok()
        .flatten()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| "GROQ_API_KEY is not set (env or `flowvoice --set-key ...`)".to_string())
}

/// True when a key is reachable (env or OS store).
pub fn available() -> bool {
    api_key().is_ok()
}

/// Transcribe 16 kHz mono i16 PCM through the Groq API, return raw
/// per-segment texts (`verbose_json` tracks sentence boundaries).
pub fn transcribe_pcm(pcm: &[i16]) -> Result<Vec<String>, String> {
    let key = api_key()?;

    let wav = crate::whisper::encode_wav(pcm);
    let path = temp_wav_path();
    std::fs::write(&path, &wav).map_err(|e| format!("cannot stage wav: {e}"))?;
    let result = spans_from_value(&post_audio(&path, &key, "audio/wav")?)
        .map(|spans| spans.into_iter().map(|s| s.text).collect());
    let _ = std::fs::remove_file(&path);
    result
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_wav_path() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("flowvoice-{}-{n}.wav", std::process::id()))
}

/// Optional style/vocabulary hint (`FLOWVOICE_GROQ_PROMPT`), e.g.
/// domain words the recognizer keeps misspelling ("маржа, выручка,
/// тенге, созвонимся"). Sent as the `prompt` field; empty by default.
/// Groq caps prompts at 896 chars, so longer values are clipped at a
/// word boundary instead of failing the whole request.
fn prompt() -> String {
    clip_prompt(&std::env::var("FLOWVOICE_GROQ_PROMPT").unwrap_or_default())
}

/// Clip a prompt to Groq's 896-char cap at a word boundary (last comma
/// or space wins); short values pass through untouched.
fn clip_prompt(full: &str) -> String {
    const MAX: usize = 896;
    if full.chars().count() <= MAX {
        return full.to_string();
    }
    let mut end = MAX;
    while !full.is_char_boundary(end) {
        end -= 1;
    }
    let back_to_sep = full[..end]
        .trim_end_matches(|c: char| c != ',' && !c.is_whitespace())
        .trim_end_matches([',', ' ', '\t'])
        .trim_end();
    back_to_sep.to_string()
}

/// Transcribe any local audio file (wav/mp3/m4a/ogg/flac/webm, N-01..N-05)
/// through Groq, with per-segment times for `--timestamps` (N-13).
pub fn transcribe_file(path: &std::path::Path) -> Result<Vec<crate::whisper::Span>, String> {
    let key = api_key()?;
    spans_from_value(&post_audio(path, &key, "")?)
}

/// Shared reply handling: API error surface, then segment split.
fn spans_from_value(value: &serde_json::Value) -> Result<Vec<crate::whisper::Span>, String> {
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown groq error");
        return Err(format!("groq error: {msg}"));
    }
    let spans = crate::whisper::segment_spans(value);
    if spans.is_empty() {
        return Err("groq reply has no text".to_string());
    }
    Ok(spans)
}

/// POST one audio file, return the parsed reply body.
fn post_audio(path: &std::path::Path, key: &str, mime: &str) -> Result<serde_json::Value, String> {
    use std::os::windows::process::CommandExt as _;

    let file_arg = if mime.is_empty() {
        format!("file=@{}", path.display())
    } else {
        format!("file=@{};type={mime}", path.display())
    };
    let timeout_s: u64 = std::env::var("FLOWVOICE_TIMEOUT_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);
    let mut cmd = Command::new("curl.exe");
    // Console child of a windowless parent would flash a terminal and
    // steal focus (breaking the Ctrl+V paste that follows).
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd.arg("-sS")
        .arg("--max-time")
        .arg(timeout_s.min(900).to_string())
        .arg("--retry")
        .arg("2")
        .arg("-X")
        .arg("POST")
        .arg("https://api.groq.com/openai/v1/audio/transcriptions")
        .arg("-H")
        .arg(format!("Authorization: Bearer {key}"))
        .arg("-F")
        .arg(file_arg)
        .arg("-F")
        .arg(format!("model={}", model()));
    // No "auto" code on the API side: omitting the field enables detection.
    if crate::whisper::lang() != "auto" {
        cmd.arg("-F")
            .arg(format!("language={}", crate::whisper::lang()));
    }
    cmd.arg("-F")
        .arg("response_format=verbose_json")
        .arg("-F")
        .arg("temperature=0.0");
    if !prompt().is_empty() {
        let full_len = std::env::var("FLOWVOICE_GROQ_PROMPT")
            .map(|v| v.chars().count())
            .unwrap_or(0);
        if full_len > prompt().chars().count() {
            crate::state::emit("[groq] prompt clipped to fit the 896-char limit");
        }
        cmd.arg("-F").arg(format!("prompt={}", prompt()));
    }
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run curl.exe: {e}"))?;

    if !out.status.success() {
        let tail = String::from_utf8_lossy(&out.stderr);
        let tail = tail.trim().chars().take(300).collect::<String>();
        return Err(format!("groq curl failed: {tail}"));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("bad groq json: {e}"))
}

/// Test seam for reply handling: parse a body, then split segments.
#[cfg(test)]
fn extract_text(body: &[u8]) -> Result<Vec<String>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("bad groq json: {e}"))?;
    spans_from_value(&value).map(|spans| spans.into_iter().map(|s| s.text).collect())
}

/// Chat model for voice commands (translate/summarize/rewrite).
/// `openai/gpt-oss-20b` is fast and cheap; override with
/// `FLOWVOICE_CHAT_MODEL` (e.g. `whisper-large-v3` is NOT a chat model).
fn chat_model() -> String {
    std::env::var("FLOWVOICE_CHAT_MODEL").unwrap_or_else(|_| "openai/gpt-oss-20b".to_string())
}

/// One Groq chat completion: system instruction + user text -> reply text.
/// Used by voice commands (J-10/J-11/J-14); STT itself never calls an LLM.
pub fn chat(system: &str, user: &str) -> Result<String, String> {
    let key = api_key()?;
    let body = serde_json::json!({
        "model": chat_model(),
        "temperature": 0.0,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    let dir = std::env::temp_dir();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = dir.join(format!("flowvoice-{}-{n}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string(&body).unwrap_or_default())
        .map_err(|e| format!("cannot stage chat body: {e}"))?;
    // The JSON rides in a file (not argv): Cyrillic survives any console
    // encoding, and the key never appears in process listings beyond -H.
    let timeout_s: u64 = std::env::var("FLOWVOICE_TIMEOUT_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);
    let mut cmd = Command::new("curl.exe");
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .arg("-sS")
        .arg("--max-time")
        .arg(timeout_s.min(900).to_string())
        .arg("--retry")
        .arg("2")
        .arg("-X")
        .arg("POST")
        .arg("https://api.groq.com/openai/v1/chat/completions")
        .arg("-H")
        .arg(format!("Authorization: Bearer {key}"))
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("--data-binary")
        .arg(format!("@{}", path.display()))
        .output()
        .map_err(|e| format!("cannot run curl.exe: {e}"));
    let _ = std::fs::remove_file(&path);
    let out = out?;
    if !out.status.success() {
        let tail = String::from_utf8_lossy(&out.stderr);
        return Err(format!("groq chat failed: {}", tail.trim()));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("bad groq json: {e}"))?;
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown groq error");
        return Err(format!("groq error: {msg}"));
    }
    value
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "groq chat reply has no text".to_string())
}

// --- Windows Credential Manager (`flowvoice/GROQ_API_KEY`, P-06) ---

#[link(name = "advapi32")]
unsafe extern "system" {
    fn CredWriteW(cred: *const CredentialW, flags: u32) -> i32;
    fn CredReadW(target: *const u16, kind: u32, flags: u32, out: *mut *mut CredentialW) -> i32;
    fn CredFree(ptr: *const std::os::raw::c_void);
}

#[repr(C)]
struct CredentialW {
    flags: u32,
    kind: u32,
    target: *const u16,
    comment: *const u16,
    last_written_lo: u32,
    last_written_hi: u32,
    blob_size: u32,
    blob: *mut u8,
    persist: u32,
    attr_count: u32,
    attrs: *const std::os::raw::c_void,
    target_alias: *const u16,
    username: *const u16,
}

const CRED_TYPE_GENERIC: u32 = 1;
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
const CRED_TARGET: &str = "flowvoice/GROQ_API_KEY";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Store the key in the OS credential vault (`--set-key`).
pub fn save_key_to_store(secret: &str) -> Result<(), String> {
    let target = wide(CRED_TARGET);
    let mut blob: Vec<u8> = Vec::with_capacity(secret.len() * 2);
    for u in secret.encode_utf16() {
        blob.extend_from_slice(&u.to_le_bytes());
    }
    let cred = CredentialW {
        flags: 0,
        kind: CRED_TYPE_GENERIC,
        target: target.as_ptr(),
        comment: std::ptr::null(),
        last_written_lo: 0,
        last_written_hi: 0,
        blob_size: blob.len() as u32,
        blob: blob.as_mut_ptr(),
        persist: CRED_PERSIST_LOCAL_MACHINE,
        attr_count: 0,
        attrs: std::ptr::null(),
        target_alias: std::ptr::null(),
        username: std::ptr::null(),
    };
    let ok = unsafe { CredWriteW(&cred, 0) };
    if ok == 0 {
        return Err("Credential Manager refused the key".to_string());
    }
    Ok(())
}

/// Read the key back from the OS vault (env wins when set).
fn read_key_from_store() -> Result<Option<String>, String> {
    let target = wide(CRED_TARGET);
    let mut out: *mut CredentialW = std::ptr::null_mut();
    let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut out) };
    if ok == 0 || out.is_null() {
        return Ok(None);
    }
    let (text, ok) = unsafe {
        let c = &*out;
        let units = std::slice::from_raw_parts(c.blob as *const u16, c.blob_size as usize / 2);
        let text = String::from_utf16_lossy(units);
        CredFree(out as *const std::os::raw::c_void);
        (text, true)
    };
    let _ = ok;
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_reply() {
        let body = "{\"text\":\"  Привет мир. \",\"x_groq\":{\"id\":\"req_1\"}}";
        assert_eq!(extract_text(body.as_bytes()).unwrap(), ["Привет мир."]);
    }

    #[test]
    fn prefers_segments_over_text() {
        let body = "{\"text\":\"all\",\"segments\":[{\"text\":\"Один.\"},{\"text\":\"Два?\"}]}";
        assert_eq!(extract_text(body.as_bytes()).unwrap(), ["Один.", "Два?"]);
    }

    #[test]
    fn surfaces_api_errors() {
        let body = br#"{"error":{"message":"Invalid API Key","type":"invalid_request_error"}}"#;
        let err = extract_text(body).unwrap_err();
        assert!(err.contains("Invalid API Key"), "{err}");
    }

    #[test]
    fn rejects_garbage() {
        assert!(extract_text(b"not json").is_err());
        assert!(extract_text(br#"{"x_groq":{}}"#).is_err());
    }

    #[test]
    fn temp_names_are_unique() {
        assert_ne!(temp_wav_path(), temp_wav_path());
    }

    #[test]
    fn prompt_clipping() {
        assert_eq!(clip_prompt("а, б"), "а, б");
        let long = "слово,".repeat(200);
        let clipped = clip_prompt(&long);
        assert!(clipped.chars().count() <= 896);
        assert!(clipped.ends_with("слово,") || clipped.ends_with("слово"));
        assert!(!clipped.ends_with("слов"));
    }
}

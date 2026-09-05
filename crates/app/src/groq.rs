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

/// True when a non-empty `GROQ_API_KEY` is present in the environment.
pub fn available() -> bool {
    std::env::var_os("GROQ_API_KEY").is_some_and(|k| !k.is_empty())
}

/// Transcribe 16 kHz mono i16 PCM through the Groq API, return raw text.
pub fn transcribe_pcm(pcm: &[i16]) -> Result<String, String> {
    let key = std::env::var("GROQ_API_KEY").map_err(|_| "GROQ_API_KEY is not set".to_string())?;
    if key.is_empty() {
        return Err("GROQ_API_KEY is empty".to_string());
    }

    let wav = crate::whisper::encode_wav(pcm);
    let path = temp_wav_path();
    std::fs::write(&path, &wav).map_err(|e| format!("cannot stage wav: {e}"))?;
    let result = post_wav(&path, &key);
    let _ = std::fs::remove_file(&path);
    result
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_wav_path() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("flowvoice-{}-{n}.wav", std::process::id()))
}

/// Upload one wav file with curl.exe, extract `{"text": ...}`.
fn post_wav(path: &std::path::Path, key: &str) -> Result<String, String> {
    let file_arg = format!("file=@{};type=audio/wav", path.display());
    let out = Command::new("curl.exe")
        .arg("-sS")
        .arg("--max-time")
        .arg("180")
        .arg("-X")
        .arg("POST")
        .arg("https://api.groq.com/openai/v1/audio/transcriptions")
        .arg("-H")
        .arg(format!("Authorization: Bearer {key}"))
        .arg("-F")
        .arg(file_arg)
        .arg("-F")
        .arg(format!("model={}", model()))
        .arg("-F")
        .arg(format!("language={}", crate::whisper::lang()))
        .arg("-F")
        .arg("response_format=json")
        .arg("-F")
        .arg("temperature=0.0")
        .output()
        .map_err(|e| format!("cannot run curl.exe: {e}"))?;

    if !out.status.success() {
        let tail = String::from_utf8_lossy(&out.stderr);
        let tail = tail.trim().chars().take(300).collect::<String>();
        return Err(format!("groq curl failed: {tail}"));
    }
    extract_text(&out.stdout)
}

/// Pull the transcript out of a `response_format=json` reply.
fn extract_text(body: &[u8]) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("bad groq json: {e}"))?;
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown groq error");
        return Err(format!("groq error: {msg}"));
    }
    value
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "groq reply has no text".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_reply() {
        let body = "{\"text\":\"  Привет мир. \",\"x_groq\":{\"id\":\"req_1\"}}";
        assert_eq!(extract_text(body.as_bytes()).unwrap(), "Привет мир.");
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
}

//! Whisper speech-to-text through a resident `whisper-server` process.
//!
//! The server is a prebuilt whisper.cpp binary (fetched by
//! `scripts/get-native.ps1`, no Rust/C++ toolchain needed). It loads the
//! model once and stays alive, so every utterance pays inference cost only —
//! unlike `whisper-cli`, which would reload ~1.5 GB per hotkey press.
//!
//! HTTP is spoken with `std::net::TcpStream` only (hand-rolled
//! `multipart/form-data` POST to `/inference`); the only extra dependency
//! is `serde_json` for the `{"text": ...}` reply, and only under the
//! `audio` feature, so the hermetic default build stays untouched.
//!
//! If the server is already listening on the port (even an orphan from a
//! previous session), it is reused instead of spawning a new instance.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default TCP port of the local whisper server.
const DEFAULT_PORT: u16 = 8178;
/// How long to wait for the first server start (1.5 GB model load).
const START_TIMEOUT: Duration = Duration::from_secs(240);
/// Per-utterance inference cap (short dictations take seconds).
const INFER_TIMEOUT: Duration = Duration::from_secs(180);

/// Prebuilt `whisper-server.exe`, override with `FLOWVOICE_WHISPER_BIN`.
fn server_bin() -> PathBuf {
    std::env::var_os("FLOWVOICE_WHISPER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|cwd| {
                    cwd.join("native")
                        .join("whisper")
                        .join("whisper-server.exe")
                })
                .unwrap_or_else(|_| PathBuf::from("native/whisper/whisper-server.exe"))
        })
}

/// Whisper ggml model file, override with `FLOWVOICE_WHISPER_MODEL`.
fn model_file() -> PathBuf {
    std::env::var_os("FLOWVOICE_WHISPER_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|cwd| {
                    cwd.join("models")
                        .join("whisper")
                        .join("ggml-large-v3-turbo.bin")
                })
                .unwrap_or_else(|_| PathBuf::from("models/whisper/ggml-large-v3-turbo.bin"))
        })
}

fn port() -> u16 {
    std::env::var("FLOWVOICE_WHISPER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Recognition language (`FLOWVOICE_LANG`, default `ru`).
/// Server `-l` flag and Groq `language` field both take ISO-639-1.
pub(crate) fn lang() -> String {
    std::env::var("FLOWVOICE_LANG").unwrap_or_else(|_| "ru".to_string())
}

/// True when both the server binary and the model file are on disk.
pub fn available() -> bool {
    server_bin().is_file() && model_file().is_file()
}

/// Keeps the spawned server child alive for the process lifetime.
/// Never killed on purpose: the next app start reuses it via the port probe.
static CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// Make sure something answers on 127.0.0.1:port: reuse a running server
/// or spawn a new one and wait until it accepts connections.
/// The TCP probe is sub-millisecond, so calling this per utterance is free
/// and self-healing (a dead server is simply respawned).
pub fn ensure_server() -> Result<u16, String> {
    let p = port();
    if TcpStream::connect(("127.0.0.1", p)).is_ok() {
        return Ok(p);
    }
    let bin = server_bin();
    let model = model_file();
    if !bin.is_file() {
        return Err(format!(
            "whisper-server not found at `{}` (run scripts/get-native.ps1)",
            bin.display()
        ));
    }
    if !model.is_file() {
        return Err(format!(
            "whisper model not found at `{}` (run scripts/get-native.ps1)",
            model.display()
        ));
    }

    let mut cmd = Command::new(&bin);
    cmd.arg("-m")
        .arg(&model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(p.to_string())
        .arg("-l")
        .arg(lang())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // whisper.dll / ggml-*.dll sit next to the exe; run from its directory.
    if let Some(dir) = bin.parent() {
        if dir.is_dir() {
            cmd.current_dir(dir);
        }
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("cannot start whisper-server: {e}"))?;
    CHILD
        .lock()
        .map_err(|_| "whisper server lock poisoned".to_string())?
        .replace(child);

    let start = Instant::now();
    loop {
        // The server listens only after the model is loaded, so an
        // accepted connection means it is ready for /inference.
        if TcpStream::connect(("127.0.0.1", p)).is_ok() {
            return Ok(p);
        }
        if start.elapsed() > START_TIMEOUT {
            return Err("whisper-server did not come up in time".to_string());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Send 16 kHz mono i16 PCM to `/inference`, return raw per-segment texts.
/// `verbose_json` segments track sentence boundaries, so callers can
/// punctuate every sentence instead of only terminating the whole utterance.
pub fn transcribe_pcm(pcm: &[i16]) -> Result<Vec<String>, String> {
    let p = ensure_server()?;
    let wav = encode_wav(pcm);
    let boundary = format!("----flowvoice{}", std::process::id());
    let body = encode_multipart(&boundary, &wav);
    let head = format!(
        "POST /inference HTTP/1.1\r\n\
         Host: 127.0.0.1:{p}\r\n\
         Content-Type: multipart/form-data; boundary={boundary}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );

    let mut sock = TcpStream::connect(("127.0.0.1", p))
        .map_err(|e| format!("whisper-server connect failed: {e}"))?;
    sock.set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    sock.set_read_timeout(Some(INFER_TIMEOUT))
        .map_err(|e| e.to_string())?;
    sock.write_all(head.as_bytes())
        .and_then(|()| sock.write_all(&body))
        .map_err(|e| format!("whisper-server request failed: {e}"))?;

    let mut resp = Vec::new();
    sock.read_to_end(&mut resp)
        .map_err(|e| format!("whisper-server read failed: {e}"))?;
    parse_inference_response(&resp)
}

/// Minimal 16 kHz mono 16-bit WAV encoder (what `/inference` expects,
/// and what the Groq backend uploads).
pub(crate) fn encode_wav(pcm: &[i16]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&16_000u32.to_le_bytes()); // sample rate
    out.extend_from_slice(&(16_000u32 * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// `multipart/form-data` body: the wav file plus decoding parameters.
fn encode_multipart(boundary: &str, wav: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 512);
    let part = |body: &mut Vec<u8>, headers: &str| {
        body.extend_from_slice(format!("--{boundary}\r\n{headers}\r\n\r\n").as_bytes());
    };
    part(
        &mut body,
        "Content-Disposition: form-data; name=\"file\"; filename=\"utterance.wav\"\r\n\
         Content-Type: audio/wav",
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(b"\r\n");
    for (name, value) in [("temperature", "0.0"), ("response_format", "verbose_json")] {
        part(
            &mut body,
            &format!("Content-Disposition: form-data; name=\"{name}\""),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// Pull per-segment transcripts out of an `/inference` HTTP reply.
/// Prefers the `verbose_json` `segments[]` array (one entry per sentence);
/// falls back to the top-level `text` when segments are absent.
fn parse_inference_response(resp: &[u8]) -> Result<Vec<String>, String> {
    let split = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "bad http reply from whisper-server".to_string())?;
    let head = String::from_utf8_lossy(&resp[..split]);
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "bad http status from whisper-server".to_string())?;
    let body = &resp[split + 4..];
    if status != 200 {
        let preview = String::from_utf8_lossy(&body[..body.len().min(200)]);
        return Err(format!("whisper-server http {status}: {preview}"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("bad whisper json: {e}"))?;
    let texts = segment_texts(&value);
    if texts.is_empty() {
        return Err("whisper-server reply has no text".to_string());
    }
    Ok(texts)
}

/// Split a `verbose_json`-style reply into trimmed non-empty segment texts.
/// Shared with the Groq backend (same response shape).
pub(crate) fn segment_texts(value: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(segments) = value.get("segments").and_then(|s| s.as_array()) {
        for seg in segments {
            if let Some(t) = seg.get("text").and_then(|t| t.as_str()) {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    out.push(t);
                }
            }
        }
    }
    if out.is_empty() {
        if let Some(t) = value.get("text").and_then(|t| t.as_str()) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                out.push(t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_16k_mono_pcm() {
        let wav = encode_wav(&[0i16, 1, -1, 32767]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1); // PCM
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1); // mono
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 8);
        assert_eq!(wav.len(), 44 + 8);
    }

    #[test]
    fn multipart_wraps_wav_with_params() {
        let body = encode_multipart("BND", &[1, 2, 3]);
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--BND\r\n"));
        assert!(text.contains("filename=\"utterance.wav\""));
        assert!(text.contains("name=\"temperature\""));
        assert!(text.contains("name=\"response_format\""));
        assert!(text.contains("verbose_json"));
        assert!(text.ends_with("--BND--\r\n"));
        assert!(body.windows(3).any(|w| w == [1, 2, 3]));
    }

    #[test]
    fn parses_json_text_reply() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 26\r\n\r\n{\"text\":\"  \xd0\x9f\xd1\x80\xd0\xb8\xd0\xb2\xd0\xb5\xd1\x82 \"}";
        assert_eq!(parse_inference_response(raw).unwrap(), ["Привет"]);
    }

    #[test]
    fn prefers_verbose_segments() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"text\":\"a b\",\"segments\":[{\"text\":\"  a. \"},{\"text\":\"\"},{\"text\":\"b?\"}]}";
        assert_eq!(parse_inference_response(raw).unwrap(), ["a.", "b?"]);
    }

    #[test]
    fn rejects_http_errors() {
        let raw = b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 11\r\n\r\nmodel busy";
        assert!(parse_inference_response(raw).is_err());
    }
}

// SPDX-License-Identifier: MIT
//! Shared runtime state between the hotkey hook thread and the GUI.
//!
//! Windows-only. Dependency-free (`std` only): timestamps are stored as
//! `SystemTime` and formatted wherever `chrono` is available (the GUI).

use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};

/// Backend preference lives in [`crate::backend`]; re-exported so existing
/// `state::BackendPref` paths keep working.
pub use crate::backend::BackendPref;

/// GUI-editable settings (persisted as JSON by the GUI; env wins at runtime).
#[cfg(feature = "gui")]
#[derive(Debug, Clone)]
pub struct Settings {
    pub hotkey: String,
    pub lang: String,
    pub groq_model: String,
    pub backend: BackendPref,
    pub profile: String,
    pub sound: bool,
    /// Second hotkey for edit mode (`выкл` disables, D-14).
    pub edit_key: String,
    /// Start with Windows via HKCU Run (B-10).
    pub autostart: bool,
    /// Paste engine: `clipboard` (Ctrl+V) or `unicode` keystrokes (L-03).
    pub paste_method: String,
    /// Encrypt history.json with DPAPI (M-17).
    pub history_encrypt: bool,
    /// Simple noise gate on capture (C-16).
    #[cfg(feature = "audio")]
    pub noise_gate: bool,
}

#[cfg(feature = "gui")]
impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "RCONTROL".to_string(),
            lang: "ru".to_string(),
            groq_model: "whisper-large-v3-turbo".to_string(),
            backend: BackendPref::Auto,
            profile: "auto".to_string(),
            sound: true,
            edit_key: "выкл".to_string(),
            autostart: false,
            paste_method: "clipboard".to_string(),
            history_encrypt: false,
            #[cfg(feature = "audio")]
            noise_gate: false,
        }
    }
}

/// One pasted dictation. Fields are consumed by the GUI; without it the
/// entries are still collected (cheap) but never read.
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub struct HistoryEntry {
    pub when: std::time::SystemTime,
    pub text: String,
    /// Foreground window title at capture time (M-02, may be empty).
    pub app: String,
}

/// Live state shared by the hook thread (producer) and the GUI (consumer).
pub struct AppState {
    /// True while the hotkey is held / audio is being captured.
    pub recording: AtomicBool,
    record_started: Mutex<Option<std::time::Instant>>,
    /// Last finalized text (shown in the status panel).
    pub last_text: Mutex<String>,
    /// Human-readable backend in use, e.g. `groq cloud`.
    pub backend_label: Mutex<String>,
    /// Recent `[listening] / [final] / [error]` lines for the log panel.
    log: Mutex<VecDeque<String>>,
    /// Pasted dictations (persisted by the GUI).
    pub history: Mutex<Vec<HistoryEntry>>,
    /// Set when the history changed and needs a disk save.
    pub history_dirty: AtomicBool,
    /// Pre-edit text for the `отмени` voice command (J-15).
    #[cfg(feature = "audio")]
    pub undo_slot: Mutex<Option<String>>,
    /// Active backend preference (GUI setting or `FLOWVOICE_BACKEND`).
    pub backend_pref: Mutex<BackendPref>,
    /// egui handle for cross-thread repaint kicks (GUI mode only).
    #[cfg(feature = "gui")]
    pub egui_ctx: OnceLock<eframe::egui::Context>,
}

impl AppState {
    #[cfg(feature = "gui")]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            recording: AtomicBool::new(false),
            record_started: Mutex::new(None),
            last_text: Mutex::new(String::new()),
            backend_label: Mutex::new("starting…".to_string()),
            log: Mutex::new(VecDeque::with_capacity(200)),
            history: Mutex::new(Vec::new()),
            history_dirty: AtomicBool::new(false),
            #[cfg(feature = "audio")]
            undo_slot: Mutex::new(None),
            backend_pref: Mutex::new(BackendPref::from_env().unwrap_or(BackendPref::Auto)),
            #[cfg(feature = "gui")]
            egui_ctx: OnceLock::new(),
        })
    }

    pub fn set_recording(&self, on: bool) {
        self.recording.store(on, Ordering::SeqCst);
        if let Ok(mut started) = self.record_started.lock() {
            *started = on.then(std::time::Instant::now);
        }
        self.kick_ui();
    }

    #[cfg(feature = "gui")]
    pub fn recording_secs(&self) -> u64 {
        self.record_started
            .lock()
            .ok()
            .and_then(|s| *s)
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }

    pub fn push_log(&self, line: String) {
        if let Ok(mut log) = self.log.lock() {
            if log.len() >= 200 {
                log.pop_front();
            }
            log.push_back(line);
        }
        self.kick_ui();
    }

    #[cfg(feature = "gui")]
    pub fn recent_log(&self) -> Vec<String> {
        self.log
            .lock()
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[cfg(feature = "audio")]
    pub fn push_history(&self, text: String, app: String) {
        if let Ok(mut history) = self.history.lock() {
            if history.len() >= 100 {
                history.remove(0);
            }
            history.push(HistoryEntry {
                when: std::time::SystemTime::now(),
                text,
                app,
            });
        }
        self.history_dirty.store(true, Ordering::SeqCst);
        self.kick_ui();
    }

    #[cfg(feature = "audio")]
    pub fn set_backend_label(&self, label: &str) {
        if let Ok(mut current) = self.backend_label.lock() {
            *current = label.to_string();
        }
    }

    #[cfg(feature = "gui")]
    fn kick_ui(&self) {
        if let Some(ctx) = self.egui_ctx.get() {
            ctx.request_repaint();
        }
    }

    #[cfg(not(feature = "gui"))]
    fn kick_ui(&self) {}
}

/// Global handle so `win`/`audio` (which predate the GUI) can reach state
/// without signature churn. `None` in console mode without `--gui`.
static STATE: OnceLock<Arc<AppState>> = OnceLock::new();

#[cfg(feature = "gui")]
pub fn attach(state: Arc<AppState>) {
    let _ = STATE.set(state);
}

pub fn get() -> Option<Arc<AppState>> {
    STATE.get().cloned()
}

/// Emit a status line: GUI log panel when attached, stdout otherwise.
///
/// Windowed GUI builds may have no valid stdout at all (double-clicked
/// exe), where `println!` would panic — so GUI-attached code must use
/// this instead of printing directly.
///
/// `[error]` lines are additionally appended to a local error log (O-17).
pub fn emit(line: &str) {
    if let Some(s) = get() {
        s.push_log(line.to_string());
    } else {
        println!("{line}");
    }
    if line.starts_with("[error]") {
        append_error_log(line);
    }
}

/// Local error log with size rotation (O-17/O-19): `errors.log`, older
/// content moves to `errors.old.log` past 1 MiB.
#[cfg(any(feature = "audio", feature = "gui"))]
fn append_error_log(line: &str) {
    use std::io::Write as _;
    let path = app_dir().join("errors.log");
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 1024 * 1024 {
        let old = app_dir().join("errors.old.log");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&path, &old);
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts}] {line}");
        ensure_private(&path);
    }
}

/// Owner-only file permissions, best-effort (P-11): strip inheritance,
/// grant the current user read/write. Failures are ignored (portable
/// installs, unusual ACLs) — the files simply keep inherited rights.
#[cfg(any(feature = "audio", feature = "gui"))]
fn ensure_private(path: &std::path::Path) {
    use std::sync::Once;
    static DONE: Once = Once::new();
    DONE.call_once(|| {
        let _ = std::process::Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!(
                "{}:(R,W)",
                std::env::var("USERNAME").unwrap_or_else(|_| "OWNER RIGHTS".to_string())
            ))
            .output();
    });
}

/// Crash-recovery marker (O-09): written at capture start, removed after
/// paste. A leftover at startup means the previous run died mid-replica.
/// Absent in tests: no FS writes from unit tests.
#[cfg(all(any(feature = "audio", feature = "gui"), not(test)))]
pub(crate) fn write_pending() {
    let path = app_dir().join("pending.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::write(&path, format!("{{\"started\":{ts}}}"));
}

/// Returns the leftover marker message, if any, and clears it (O-10).
#[cfg(any(feature = "audio", feature = "gui"))]
pub(crate) fn take_pending_notice() -> Option<String> {
    let path = app_dir().join("pending.json");
    let body = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(format!(
        "[recover] previous run stopped mid-replica ({body}); dictate it again"
    ))
}

#[cfg(any(feature = "audio", feature = "gui"))]
pub(crate) fn clear_pending() {
    let _ = std::fs::remove_file(app_dir().join("pending.json"));
}

/// `%APPDATA%/WispFlowStealer` (settings, history, journal live here).
#[cfg(any(feature = "audio", feature = "gui"))]
pub(crate) fn app_dir() -> std::path::PathBuf {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("WispFlowStealer")
}

/// `journal.jsonl` path (per-replica JSON lines, T-01).
#[cfg(any(feature = "audio", feature = "gui"))]
pub(crate) fn journal_path() -> std::path::PathBuf {
    app_dir().join("journal.jsonl")
}

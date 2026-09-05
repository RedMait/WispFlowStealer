//! Shared runtime state between the hotkey hook thread and the GUI.
//!
//! Windows-only. Dependency-free (`std` only): timestamps are stored as
//! `SystemTime` and formatted wherever `chrono` is available (the GUI).

use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};

/// Speech backend preference (GUI setting + `FLOWVOICE_BACKEND` env).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPref {
    Auto,
    Groq,
    Local,
    Vosk,
}

impl BackendPref {
    pub fn all() -> &'static [BackendPref] {
        &[Self::Auto, Self::Groq, Self::Local, Self::Vosk]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Groq => "groq cloud",
            Self::Local => "whisper local",
            Self::Vosk => "vosk",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "groq" | "groq cloud" => Self::Groq,
            "local" | "whisper" | "whisper local" => Self::Local,
            "vosk" => Self::Vosk,
            _ => Self::Auto,
        }
    }

    /// Env override (`FLOWVOICE_BACKEND`), `None` when unset/invalid.
    pub fn from_env() -> Option<Self> {
        std::env::var("FLOWVOICE_BACKEND")
            .ok()
            .map(|s| Self::parse(&s))
            .filter(|p| *p != Self::Auto)
            .or_else(|| {
                std::env::var("FLOWVOICE_BACKEND")
                    .ok()
                    .filter(|s| s.eq_ignore_ascii_case("auto"))
                    .map(|_| Self::Auto)
            })
    }

    pub fn allows_groq(self) -> bool {
        matches!(self, Self::Auto | Self::Groq)
    }

    pub fn allows_local(self) -> bool {
        matches!(self, Self::Auto | Self::Local)
    }

    pub fn allows_vosk(self) -> bool {
        matches!(self, Self::Auto | Self::Vosk)
    }
}

/// GUI-editable settings (persisted as JSON by the GUI; env wins at runtime).
#[derive(Debug, Clone)]
pub struct Settings {
    pub hotkey: String,
    pub lang: String,
    pub groq_model: String,
    pub backend: BackendPref,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "RCONTROL".to_string(),
            lang: "ru".to_string(),
            groq_model: "whisper-large-v3-turbo".to_string(),
            backend: BackendPref::Auto,
        }
    }
}

/// One pasted dictation.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub when: std::time::SystemTime,
    pub text: String,
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
    /// Active backend preference (GUI setting or `FLOWVOICE_BACKEND`).
    pub backend_pref: Mutex<BackendPref>,
    /// egui handle for cross-thread repaint kicks (GUI mode only).
    #[cfg(feature = "gui")]
    pub egui_ctx: OnceLock<eframe::egui::Context>,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            recording: AtomicBool::new(false),
            record_started: Mutex::new(None),
            last_text: Mutex::new(String::new()),
            backend_label: Mutex::new("starting…".to_string()),
            log: Mutex::new(VecDeque::with_capacity(200)),
            history: Mutex::new(Vec::new()),
            history_dirty: AtomicBool::new(false),
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

    pub fn recent_log(&self) -> Vec<String> {
        self.log
            .lock()
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn push_history(&self, text: String) {
        if let Ok(mut history) = self.history.lock() {
            if history.len() >= 100 {
                history.remove(0);
            }
            history.push(HistoryEntry {
                when: std::time::SystemTime::now(),
                text,
            });
        }
        self.history_dirty.store(true, Ordering::SeqCst);
        self.kick_ui();
    }

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

pub fn attach(state: Arc<AppState>) {
    let _ = STATE.set(state);
}

pub fn get() -> Option<Arc<AppState>> {
    STATE.get().cloned()
}

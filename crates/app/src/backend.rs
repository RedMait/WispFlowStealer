// SPDX-License-Identifier: MIT
//! Speech backend preference and selection.
//!
//! Chain: Groq Cloud (needs `GROQ_API_KEY`) -> resident local
//! whisper-server -> Vosk fallback. Pure selection logic lives here so the
//! priority table is unit-tested; availability probes stay in the backends.

/// Speech backend preference (GUI setting + `FLOWVOICE_BACKEND` env).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPref {
    Auto,
    Groq,
    Local,
    Vosk,
}

impl BackendPref {
    #[cfg(feature = "gui")]
    pub fn all() -> &'static [BackendPref] {
        &[Self::Auto, Self::Groq, Self::Local, Self::Vosk]
    }

    #[cfg(feature = "gui")]
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
    #[cfg(any(feature = "audio", feature = "gui"))]
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("FLOWVOICE_BACKEND").ok()?;
        if raw.eq_ignore_ascii_case("auto") {
            return Some(Self::Auto);
        }
        let parsed = Self::parse(&raw);
        if parsed == Self::Auto {
            return None;
        }
        Some(parsed)
    }

    #[cfg(feature = "audio")]
    pub fn allows_groq(self) -> bool {
        matches!(self, Self::Auto | Self::Groq)
    }

    #[cfg(feature = "audio")]
    pub fn allows_local(self) -> bool {
        matches!(self, Self::Auto | Self::Local)
    }

    #[cfg(feature = "audio")]
    pub fn allows_vosk(self) -> bool {
        matches!(self, Self::Auto | Self::Vosk)
    }
}

/// Concrete engine chosen for one utterance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(feature = "audio")]
pub enum Backend {
    Groq,
    Local,
    Vosk,
}

#[cfg(feature = "audio")]
impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Groq => "groq cloud",
            Self::Local => "whisper local",
            Self::Vosk => "vosk",
        }
    }
}

/// Pick the first available backend the preference allows.
/// Pure priority table: Groq -> local whisper -> Vosk.
#[cfg(feature = "audio")]
pub fn select(
    pref: BackendPref,
    groq_ok: bool,
    local_ok: bool,
    vosk_ok: bool,
) -> Result<Backend, &'static str> {
    use BackendPref as P;
    if groq_ok && matches!(pref, P::Auto | P::Groq) {
        return Ok(Backend::Groq);
    }
    if local_ok && matches!(pref, P::Auto | P::Local) {
        return Ok(Backend::Local);
    }
    if vosk_ok && matches!(pref, P::Auto | P::Vosk) {
        return Ok(Backend::Vosk);
    }
    Err("no speech backend enabled: set GROQ_API_KEY or run scripts/get-native.ps1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "audio")]
    #[test]
    fn auto_prefers_groq_then_local_then_vosk() {
        assert_eq!(
            select(BackendPref::Auto, true, true, true),
            Ok(Backend::Groq)
        );
        assert_eq!(
            select(BackendPref::Auto, false, true, true),
            Ok(Backend::Local)
        );
        assert_eq!(
            select(BackendPref::Auto, false, false, true),
            Ok(Backend::Vosk)
        );
    }

    #[cfg(feature = "audio")]
    #[test]
    fn forced_backend_skips_others() {
        assert_eq!(
            select(BackendPref::Local, true, true, true),
            Ok(Backend::Local)
        );
        assert_eq!(
            select(BackendPref::Vosk, true, true, true),
            Ok(Backend::Vosk)
        );
        assert_eq!(
            select(BackendPref::Groq, true, true, true),
            Ok(Backend::Groq)
        );
    }

    #[cfg(feature = "audio")]
    #[test]
    fn forced_but_missing_backend_errors() {
        assert!(select(BackendPref::Groq, false, true, true).is_err());
        assert!(select(BackendPref::Local, true, false, true).is_err());
        assert!(select(BackendPref::Vosk, true, true, false).is_err());
    }

    #[cfg(feature = "audio")]
    #[test]
    fn nothing_available_errors() {
        assert!(select(BackendPref::Auto, false, false, false).is_err());
    }

    #[test]
    fn parse_names() {
        assert_eq!(BackendPref::parse("groq"), BackendPref::Groq);
        assert_eq!(BackendPref::parse("WHISPER LOCAL"), BackendPref::Local);
        assert_eq!(BackendPref::parse("vosk"), BackendPref::Vosk);
        assert_eq!(BackendPref::parse("nonsense"), BackendPref::Auto);
    }
}

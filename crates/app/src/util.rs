// SPDX-License-Identifier: MIT
//! Shared plumbing: one-shaped error contexts (AC-08).

/// Attach a static context to any `Display` error, replacing dozens of
/// identical `.map_err(|e| format!(...))` closures across the app.
#[cfg(any(feature = "audio", test))]
pub fn xerr<T, E: std::fmt::Display>(r: Result<T, E>, ctx: &str) -> Result<T, String> {
    r.map_err(|e| format!("{ctx}: {e}"))
}

/// Read an env flag (`1`/`true` on, anything else off).
#[cfg(any(feature = "audio", feature = "gui", test))]
pub fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Read an env integer with a default.
#[cfg(any(feature = "audio", feature = "gui", test))]
pub fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xerr_keeps_ok_and_contexts_err() {
        assert_eq!(xerr::<i32, String>(Ok(1), "ctx"), Ok(1));
        assert_eq!(
            xerr::<i32, String>(Err("boom".to_string()), "ctx"),
            Err("ctx: boom".to_string())
        );
    }

    #[test]
    fn env_flag_parsing() {
        std::env::remove_var("FLOWVOICE_T_FLAG");
        assert!(!env_flag("FLOWVOICE_T_FLAG"));
        std::env::set_var("FLOWVOICE_T_FLAG", "true");
        assert!(env_flag("FLOWVOICE_T_FLAG"));
        std::env::set_var("FLOWVOICE_T_FLAG", "0");
        assert!(!env_flag("FLOWVOICE_T_FLAG"));
        std::env::remove_var("FLOWVOICE_T_FLAG");
        assert_eq!(env_u64("FLOWVOICE_T_FLAG", 7), 7);
        std::env::set_var("FLOWVOICE_T_FLAG", "42");
        assert_eq!(env_u64("FLOWVOICE_T_FLAG", 7), 42);
        std::env::remove_var("FLOWVOICE_T_FLAG");
    }
}

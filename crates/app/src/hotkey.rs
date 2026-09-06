// SPDX-License-Identifier: MIT
//! Pure recording state machine: which hotkey events start/stop capture.
//!
//! The OS hook (`win.rs`) feeds key events here. The transition table lives
//! in pure code so flag/comparison swaps break unit tests on every platform
//! (mutation-testing note from the hackathon review).

/// A hotkey press or release (already filtered to `code >= 0` by the hook).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Press { vk: u16 },
    Release { vk: u16 },
}

/// What the hook should do with the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Start,
    Stop,
    Ignore,
}

/// Next capture state + action.
///
/// `recording` is the current capture state, `enabled` the master pause
/// switch, `hotkey` the configured virtual-key code.
pub fn decide(
    recording: bool,
    enabled: bool,
    hotkey: u16,
    ev: HotkeyEvent,
) -> (bool, HotkeyAction) {
    let vk = match ev {
        HotkeyEvent::Press { vk } | HotkeyEvent::Release { vk } => vk,
    };
    if vk != hotkey {
        return (recording, HotkeyAction::Ignore);
    }
    if !enabled {
        return (recording, HotkeyAction::Ignore);
    }
    match ev {
        HotkeyEvent::Press { .. } if recording => (true, HotkeyAction::Ignore),
        HotkeyEvent::Press { .. } => (true, HotkeyAction::Start),
        HotkeyEvent::Release { .. } if recording => (false, HotkeyAction::Stop),
        HotkeyEvent::Release { .. } => (false, HotkeyAction::Ignore),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOTKEY: u16 = 0xA3;
    const OTHER: u16 = 0x41;

    #[test]
    fn press_starts_when_idle() {
        assert_eq!(
            decide(false, true, HOTKEY, HotkeyEvent::Press { vk: HOTKEY }),
            (true, HotkeyAction::Start)
        );
    }

    #[test]
    fn auto_repeat_press_is_ignored() {
        assert_eq!(
            decide(true, true, HOTKEY, HotkeyEvent::Press { vk: HOTKEY }),
            (true, HotkeyAction::Ignore)
        );
    }

    #[test]
    fn release_stops_recording() {
        assert_eq!(
            decide(true, true, HOTKEY, HotkeyEvent::Release { vk: HOTKEY }),
            (false, HotkeyAction::Stop)
        );
    }

    #[test]
    fn stray_release_is_ignored() {
        assert_eq!(
            decide(false, true, HOTKEY, HotkeyEvent::Release { vk: HOTKEY }),
            (false, HotkeyAction::Ignore)
        );
    }

    #[test]
    fn wrong_key_is_ignored() {
        assert_eq!(
            decide(false, true, HOTKEY, HotkeyEvent::Press { vk: OTHER }),
            (false, HotkeyAction::Ignore)
        );
        assert_eq!(
            decide(true, true, HOTKEY, HotkeyEvent::Release { vk: OTHER }),
            (true, HotkeyAction::Ignore)
        );
    }

    #[test]
    fn disabled_switch_ignores_everything() {
        assert_eq!(
            decide(false, false, HOTKEY, HotkeyEvent::Press { vk: HOTKEY }),
            (false, HotkeyAction::Ignore)
        );
        assert_eq!(
            decide(true, false, HOTKEY, HotkeyEvent::Release { vk: HOTKEY }),
            (true, HotkeyAction::Ignore)
        );
    }

    #[test]
    fn wrong_hotkey_config_never_fires() {
        assert_eq!(
            decide(false, true, OTHER, HotkeyEvent::Press { vk: HOTKEY }),
            (false, HotkeyAction::Ignore)
        );
    }

    #[test]
    fn release_then_press_cycles_cleanly() {
        let (rec, _) = decide(true, true, HOTKEY, HotkeyEvent::Release { vk: HOTKEY });
        assert!(!rec);
        let (rec, action) = decide(rec, true, HOTKEY, HotkeyEvent::Press { vk: HOTKEY });
        assert!(rec);
        assert_eq!(action, HotkeyAction::Start);
    }
}

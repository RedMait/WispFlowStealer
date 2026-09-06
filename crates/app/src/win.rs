// SPDX-License-Identifier: MIT
//! Windows-only implementation of the hold-to-talk UX using raw WinAPI.
//!
//! A low-level keyboard hook (`WH_KEYBOARD_LL`) detects when the hotkey is
//! pressed down and released. On press a recorder thread starts; on release
//! it is told to stop, the transcript is formatted and pasted as Ctrl+V.

use std::io::Write;
use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

#[cfg(any(feature = "audio", feature = "gui"))]
use crate::state::self;
#[cfg(feature = "audio")]
use crate::state::AppState;

const WH_KEYBOARD_LL: c_int = 13;
const WM_KEYDOWN: u32 = 0x0100;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_QUIT: u32 = 0x0012;

#[cfg(any(feature = "audio", feature = "gui"))]
const KEYEVENTF_KEYUP: u32 = 0x0002;
#[cfg(any(feature = "audio", feature = "gui"))]
const VK_CONTROL: u16 = 0x11;
#[cfg(any(feature = "audio", feature = "gui"))]
const VK_V: u16 = 0x56;
const VK_F7: u16 = 0x76;
const VK_F8: u16 = 0x77;
const VK_F9: u16 = 0x78;
const VK_RCONTROL: u16 = 0xA3;

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowsHookExW(
        id_hook: c_int,
        lpfn: *const c_void,
        h_mod: *const c_void,
        dw_thread_id: u32,
    ) -> isize;
    fn UnhookWindowsHookEx(hhk: isize) -> c_int;
    fn CallNextHookEx(hhk: isize, n_code: c_int, w_param: usize, l_param: isize) -> isize;
    fn GetMessageW(msg: *mut Msg, hwnd: *const c_void, min: u32, max: u32) -> c_int;
    fn TranslateMessage(msg: *const Msg) -> c_int;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn GetModuleHandleW(name: *const u16) -> *const c_void;
    fn GetLastError() -> u32;
    #[cfg(any(feature = "audio", feature = "gui"))]
    fn GetForegroundWindow() -> *const c_void;
    #[cfg(any(feature = "audio", feature = "gui"))]
    fn GetWindowTextW(hwnd: *const c_void, text: *mut u16, max: c_int) -> c_int;
    fn OpenInputDesktop(flags: u32, inherit: c_int, access: u32) -> *const c_void;
    fn CloseDesktop(handle: *const c_void) -> c_int;
    #[cfg(any(feature = "audio", feature = "gui"))]
    fn keybd_event(b_vk: u8, b_scan: u8, dw_flags: u32, dw_extra: usize);
}

#[cfg(feature = "gui")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetConsoleWindow() -> *const c_void;
    fn AttachConsole(process_id: u32) -> c_int;
    fn AllocConsole() -> c_int;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(attrs: *const c_void, owned: c_int, name: *const u16) -> *const c_void;
    fn Beep(freq: u32, duration_ms: u32) -> c_int;
}

#[link(name = "user32")]
unsafe extern "system" {
    #[cfg(feature = "gui")]
    fn MessageBoxW(hwnd: *const c_void, text: *const u16, caption: *const u16, kind: u32) -> c_int;
}

#[repr(C)]
struct Msg {
    hwnd: *const c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: Point,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

static HOTKEY: AtomicU16 = AtomicU16::new(VK_RCONTROL);
static RECORDING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
/// Master switch for dictation (GUI pause button / tray). When off, hotkey
/// presses are ignored entirely; an in-flight recording finishes normally.
static ENABLED: AtomicBool = AtomicBool::new(true);
/// When the hotkey was last released: start of the release→paste latency.
static KEYUP_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
/// When the current capture started (for WPM over the audio span).
#[cfg(any(feature = "audio", feature = "gui"))]
static KEYDOWN_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Foreground window title at the moment of the call (target app journal).
/// Empty when unavailable (locked screen, elevated window, …).
#[cfg(any(feature = "audio", feature = "gui"))]
pub(crate) fn foreground_title() -> String {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as c_int);
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

/// Change the hotkey at runtime (GUI settings). Takes effect immediately.
pub fn set_hotkey_vk(vk: u16) {
    HOTKEY.store(vk, Ordering::SeqCst);
}

/// Pause (`false`) or resume (`true`) dictation without touching the UI.
#[cfg(feature = "gui")]
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

#[cfg(feature = "gui")]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Make sure terminal modes have somewhere to print.
///
/// GUI builds are windowed (`windows_subsystem`), so a double-clicked exe
/// has no console at all. When started from a terminal we attach to the
/// parent console; otherwise a fresh one is allocated.
#[cfg(feature = "gui")]
pub fn ensure_console() {
    const ATTACH_PARENT: u32 = 0xFFFFFFFF;
    unsafe {
        if !GetConsoleWindow().is_null() {
            return;
        }
        if AttachConsole(ATTACH_PARENT) == 0 {
            AllocConsole();
        }
    }
}

/// Last-resort error popup for GUI startup failures (no console to print
/// to in windowed mode).
#[cfg(feature = "gui")]
pub fn fatal_popup(text: &str) {
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let text = wide(text);
    let caption = wide("flowvoice");
    unsafe {
        MessageBoxW(std::ptr::null(), text.as_ptr(), caption.as_ptr(), 0x10);
    }
}

#[cfg(feature = "audio")]
pub fn is_stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}

/// Second copy exits with a message instead of fighting over the hotkey
/// and the microphone (O-14).
pub fn ensure_single_instance() -> bool {
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let name = wide("Local\\WispFlowStealer");
    unsafe {
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        GetLastError() != 183 // ERROR_ALREADY_EXISTS
    }
}

/// True on a locked workstation / secure desktop: hotkeys must not record
/// there (AG-07/AG-08). `OpenInputDesktop` fails when the default desktop
/// is not interactive.
pub fn workstation_locked() -> bool {
    const GENERIC_READ: u32 = 0x80000000;
    unsafe {
        let h = OpenInputDesktop(0, 0, GENERIC_READ);
        if h.is_null() {
            return true;
        }
        CloseDesktop(h);
        false
    }
}

/// Sound signals on/off (`FLOWVOICE_SOUND=0` disables, AG-06).
fn sound_on() -> bool {
    std::env::var("FLOWVOICE_SOUND")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true)
}

fn beep(freq: u32, ms: u32) {
    if sound_on() {
        unsafe {
            Beep(freq, ms);
        }
    }
}

/// Error signal: distinct low tone (AG-15).
#[cfg(feature = "audio")]
pub fn beep_error() {
    beep(220, 200);
}

extern "system" fn hook_proc(code: c_int, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && !workstation_locked() {
        // KBDLLHOOKSTRUCT starts with vkCode: u32, scanCode: u32, ...
        let vk = unsafe { *(l_param as *const u32) } as u16;
        let down = w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize;
        let ev = if down {
            crate::hotkey::HotkeyEvent::Press { vk }
        } else {
            crate::hotkey::HotkeyEvent::Release { vk }
        };
        // The transition table is pure and unit-tested (`hotkey::decide`).
        let (next, action) = crate::hotkey::decide(
            RECORDING.load(Ordering::SeqCst),
            ENABLED.load(Ordering::SeqCst),
            HOTKEY.load(Ordering::SeqCst),
            ev,
        );
        RECORDING.store(next, Ordering::SeqCst);
        match action {
            crate::hotkey::HotkeyAction::Start => handle_keydown(),
            crate::hotkey::HotkeyAction::Stop => handle_keyup(),
            crate::hotkey::HotkeyAction::Ignore => {}
        }
    }
    unsafe { CallNextHookEx(0, code, w_param, l_param) }
}

fn handle_keydown() {
    STOP.store(false, Ordering::SeqCst);
    beep(880, 90);
    #[cfg(any(feature = "audio", feature = "gui"))]
    state::emit("[listening] hold the key and speak...");
    #[cfg(not(any(feature = "audio", feature = "gui")))]
    {
        println!("[listening] hold the key and speak...");
        let _ = std::io::stdout().flush();
    }
    #[cfg(any(feature = "audio", feature = "gui"))]
    {
        if let Some(s) = state::get() {
            s.set_recording(true);
        }
        if let Ok(mut slot) = KEYDOWN_AT.lock() {
            *slot = Some(std::time::Instant::now());
        }
        state::write_pending();
    }
    std::thread::spawn(spawn_recorder);
}

fn handle_keyup() {
    STOP.store(true, Ordering::SeqCst);
    beep(660, 90);
    if let Ok(mut slot) = KEYUP_AT.lock() {
        *slot = Some(std::time::Instant::now());
    }
}

fn spawn_recorder() {
    #[cfg(feature = "audio")]
    {
        let finish_with = |text: String, state: Option<std::sync::Arc<AppState>>| {
            if let Some(s) = state {
                s.set_recording(false);
                if !text.is_empty() && !no_history() {
                    s.push_history(text.clone(), foreground_title());
                    if let Ok(mut last) = s.last_text.lock() {
                        *last = text.clone();
                    }
                }
            }
            finish(&text);
        };
        let shared = state::get();
        match crate::audio::transcribe() {
            Ok(text) => finish_with(text, shared),
            Err(e) => {
                beep_error();
                state::emit(&format!("[error] {e}"));
                if let Some(s) = &shared {
                    s.set_recording(false);
                }
            }
        }
    }

    #[cfg(not(feature = "audio"))]
    {
        eprintln!(
            "[audio] not available: rebuild with `cargo build -p flowvoice --features audio`"
        );
        STOP.store(true, Ordering::SeqCst);
    }
}

#[cfg(feature = "audio")]
/// `transcribe()` already returns finalized text (neural punctuation for RU
/// when available, heuristic formatting otherwise) — paste it as-is.
fn finish(text: &str) {
    if text.is_empty() {
        state::emit("[done] no speech detected");
        return;
    }

    // Optional separator so back-to-back replicas don't glue (AM-03).
    let spaced = std::env::var("FLOWVOICE_LEADING_SPACE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let text = flowcore::pad_replica_start(text, spaced);
    state::emit(&format!("[final] {text}"));
    paste(&text);
}

/// Copy the text to the clipboard and simulate Ctrl+V. Pasting is the
/// fastest way to enter arbitrary Unicode text reliably.
///
/// Returns keyup→paste seconds (0 when unknown). The user's clipboard is
/// restored afterwards (L-01); failures are reported, never silent.
#[cfg(any(feature = "audio", feature = "gui"))]
pub(crate) fn paste(text: &str) -> f32 {
    // Optional pause before pasting (AM-20 `FLOWVOICE_PASTE_DELAY_MS`).
    let delay_ms: u64 = std::env::var("FLOWVOICE_PASTE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms.min(5000)));
    }

    // Remember the user's clipboard: text, or None when empty/unreadable.
    let mut cb = match arboard::Clipboard::new() {
        Ok(cb) => cb,
        Err(e) => {
            state::emit(&format!(
                "[error] cannot open clipboard ({e}); text kept in history, paste it manually"
            ));
            return 0.0;
        }
    };
    let saved = cb.get_text().ok();
    if let Err(e) = cb.set_text(text) {
        state::emit(&format!("[error] cannot write clipboard ({e})"));
        return 0.0;
    }

    unsafe {
        keybd_event(VK_CONTROL as u8, 0, 0, 0);
        keybd_event(VK_V as u8, 0, 0, 0);
        keybd_event(VK_V as u8, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_CONTROL as u8, 0, KEYEVENTF_KEYUP, 0);
    }

    // Latency from hotkey release to the finished paste, straight to the log.
    // The release instant is taken once and shared with the journal below.
    let keyup = KEYUP_AT.lock().ok().and_then(|mut slot| slot.take());
    let keydown = KEYDOWN_AT.lock().ok().and_then(|mut slot| slot.take());
    let secs = keyup.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
    if keyup.is_some() {
        state::emit(&format!("[timing] {secs:.1}s keyup->paste"));
    }

    // Restore what the user had (L-01); empty stays empty (L-04).
    // Best-effort: a busy clipboard only costs the restore, never the paste.
    let restore_ms: u64 = std::env::var("FLOWVOICE_RESTORE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    if restore_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(restore_ms.min(5000)));
        let _ = match saved {
            Some(prev) => cb.set_text(prev),
            None => cb.clear(),
        };
    }

    journal_append(text, secs, keydown, keyup, "paste");
    state::clear_pending();
    secs
}

/// Append one journal line for a pasted replica (skipped in no-history
/// mode alongside the visible history).
#[cfg(any(feature = "audio", feature = "gui"))]
fn journal_append(
    text: &str,
    secs: f32,
    keydown: Option<std::time::Instant>,
    keyup: Option<std::time::Instant>,
    method: &str,
) {
    if no_history() {
        return;
    }
    let Some(s) = state::get() else {
        return;
    };
    let audio_secs = match (keydown, keyup) {
        (Some(a), Some(b)) if b >= a => (b - a).as_secs_f32(),
        _ => 0.0,
    };
    let now = std::time::SystemTime::now();
    let nanos = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let words = text.split_whitespace().count();
    let entry = crate::journal::Entry {
        id: crate::journal::make_id(nanos),
        ts: (nanos / 1_000_000_000) as u64,
        backend: s
            .backend_label
            .lock()
            .map(|b| b.clone())
            .unwrap_or_default(),
        lang: flowcore::Language::detect(text).to_string(),
        app: foreground_title(),
        chars: text.chars().count(),
        words,
        secs,
        wpm: crate::journal::wpm(words, audio_secs),
        audio_secs,
        method: method.to_string(),
    };
    if let Err(e) = crate::journal::append(&state::journal_path(), &entry) {
        state::emit(&format!("[error] journal append failed: {e}"));
    }
}

/// Privacy mode (`FLOWVOICE_NO_HISTORY=1`): no history, no journal (P-12).
#[cfg(any(feature = "audio", feature = "gui"))]
fn no_history() -> bool {
    std::env::var("FLOWVOICE_NO_HISTORY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Install the low-level keyboard hook and pump messages until quit.
/// Blocks the calling thread; GUI mode runs the pump on a background
/// thread via [`spawn_pump`].
pub fn run(hotkey: Hotkey) {
    if !ensure_single_instance() {
        eprintln!("flowvoice is already running (single instance)");
        std::process::exit(1);
    }
    #[cfg(any(feature = "audio", feature = "gui"))]
    if let Some(msg) = state::take_pending_notice() {
        state::emit(&msg);
    }
    set_hotkey_vk(hotkey.to_vk());

    // Warm up the speech + punctuation models in the background so the
    // first hotkey press records immediately instead of hanging on load.
    #[cfg(feature = "audio")]
    crate::audio::preload();

    println!(
        "flowvoice: hold {:?} to dictate, release to insert text (Ctrl+C to quit)",
        hotkey.label()
    );
    let _ = std::io::stdout().flush();

    pump();
}

/// Same hook as [`run`], but pumped on a new background thread (for GUI
/// mode, where the main thread owns the UI event loop).
#[cfg(feature = "gui")]
pub fn spawn_pump(hotkey: Hotkey) {
    set_hotkey_vk(hotkey.to_vk());
    std::thread::spawn(pump);
}

fn pump() {
    unsafe {
        let hmod = GetModuleHandleW(std::ptr::null());
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, hook_proc as *const c_void, hmod, 0);
        if hook == 0 {
            eprintln!(
                "[error] cannot install keyboard hook (error {})",
                GetLastError()
            );
            std::process::exit(1);
        }

        println!("flowvoice: hotkey hook installed");
        let _ = std::io::stdout().flush();

        loop {
            let mut msg = Msg {
                hwnd: std::ptr::null(),
                message: 0,
                w_param: 0,
                l_param: 0,
                time: 0,
                pt: Point { x: 0, y: 0 },
            };
            let ret = GetMessageW(&mut msg, std::ptr::null(), 0, 0);
            if ret <= 0 || msg.message == WM_QUIT {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        UnhookWindowsHookEx(hook);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hotkey {
    F7,
    F8,
    F9,
    #[default]
    RightControl,
}

impl Hotkey {
    pub fn parse(name: &str) -> Option<Hotkey> {
        match name.to_ascii_uppercase().as_str() {
            "F7" => Some(Hotkey::F7),
            "F8" => Some(Hotkey::F8),
            "F9" => Some(Hotkey::F9),
            "RCONTROL" | "RIGHT_CONTROL" | "RCTRL" => Some(Hotkey::RightControl),
            _ => None,
        }
    }

    pub fn to_vk(self) -> u16 {
        match self {
            Hotkey::F7 => VK_F7,
            Hotkey::F8 => VK_F8,
            Hotkey::F9 => VK_F9,
            Hotkey::RightControl => VK_RCONTROL,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Hotkey::F7 => "F7",
            Hotkey::F8 => "F8",
            Hotkey::F9 => "F9",
            Hotkey::RightControl => "Right Ctrl",
        }
    }
}

//! Windows-only implementation of the hold-to-talk UX using raw WinAPI.
//!
//! A low-level keyboard hook (`WH_KEYBOARD_LL`) detects when the hotkey is
//! pressed down and released. On press a recorder thread starts; on release
//! it is told to stop, the transcript is formatted and pasted as Ctrl+V.

use std::io::Write;
use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

#[cfg(any(feature = "audio", feature = "gui"))]
use crate::state::{self, AppState};

const WH_KEYBOARD_LL: c_int = 13;
const WM_KEYDOWN: u32 = 0x0100;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_QUIT: u32 = 0x0012;

#[cfg(feature = "audio")]
const KEYEVENTF_KEYUP: u32 = 0x0002;
#[cfg(feature = "audio")]
const VK_CONTROL: u16 = 0x11;
#[cfg(feature = "audio")]
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
    #[cfg(feature = "audio")]
    fn keybd_event(b_vk: u8, b_scan: u8, dw_flags: u32, dw_extra: usize);
}

#[cfg(feature = "gui")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetConsoleWindow() -> *const c_void;
    fn AttachConsole(process_id: u32) -> c_int;
    fn AllocConsole() -> c_int;
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

extern "system" fn hook_proc(code: c_int, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && ENABLED.load(Ordering::SeqCst) {
        // KBDLLHOOKSTRUCT starts with vkCode: u32, scanCode: u32, ...
        let vk = unsafe { *(l_param as *const u32) } as u16;
        let hotkey = HOTKEY.load(Ordering::SeqCst);

        if vk == hotkey {
            let down = w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize;
            if down {
                if !RECORDING.swap(true, Ordering::SeqCst) {
                    handle_keydown();
                }
            } else if RECORDING.swap(false, Ordering::SeqCst) {
                handle_keyup();
            }
        }
    }
    unsafe { CallNextHookEx(0, code, w_param, l_param) }
}

fn handle_keydown() {
    STOP.store(false, Ordering::SeqCst);
    #[cfg(any(feature = "audio", feature = "gui"))]
    state::emit("[listening] hold the key and speak...");
    #[cfg(not(any(feature = "audio", feature = "gui")))]
    {
        println!("[listening] hold the key and speak...");
        let _ = std::io::stdout().flush();
    }
    #[cfg(any(feature = "audio", feature = "gui"))]
    if let Some(s) = state::get() {
        s.set_recording(true);
    }
    std::thread::spawn(spawn_recorder);
}

fn handle_keyup() {
    STOP.store(true, Ordering::SeqCst);
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
                if !text.is_empty() {
                    s.push_history(text.clone());
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

    state::emit(&format!("[final] {text}"));
    paste(text);
}

#[cfg(feature = "audio")]
/// Copy the text to the clipboard and simulate Ctrl+V. Pasting is the
/// fastest way to enter arbitrary Unicode text reliably.
fn paste(text: &str) {
    #[cfg(feature = "audio")]
    {
        let mut cb = match arboard::Clipboard::new() {
            Ok(cb) => cb,
            Err(e) => {
                state::emit(&format!("[error] cannot open clipboard: {e}"));
                return;
            }
        };
        if let Err(e) = cb.set_text(text) {
            state::emit(&format!("[error] cannot write clipboard: {e}"));
            return;
        }
    }

    unsafe {
        keybd_event(VK_CONTROL as u8, 0, 0, 0);
        keybd_event(VK_V as u8, 0, 0, 0);
        keybd_event(VK_V as u8, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_CONTROL as u8, 0, KEYEVENTF_KEYUP, 0);
    }

    // Latency from hotkey release to the finished paste, straight to the log.
    if let Ok(mut slot) = KEYUP_AT.lock() {
        if let Some(t0) = slot.take() {
            state::emit(&format!(
                "[timing] {:.1}s keyup->paste",
                t0.elapsed().as_secs_f32()
            ));
        }
    }
}

/// Install the low-level keyboard hook and pump messages until quit.
/// Blocks the calling thread; GUI mode runs the pump on a background
/// thread via [`spawn_pump`].
pub fn run(hotkey: Hotkey) {
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

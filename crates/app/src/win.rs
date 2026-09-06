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
use crate::state;
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
    #[cfg(any(feature = "audio", feature = "gui"))]
    fn SendInput(n_inputs: u32, inputs: *const KeyInput, struct_size: c_int) -> u32;
}

/// One keyboard input for `SendInput` (INPUT union, keyboard view).
/// Sized to the real 40-byte INPUT so `cbSize` is always right
/// (compile-time asserted below).
#[repr(C)]
#[derive(Clone, Copy)]
struct KeyInput {
    kind: u32,      // 0
    _pad: u32,      // 4
    vk: u16,        // 8
    scan: u16,      // 10
    flags: u32,     // 12
    time: u32,      // 16
    _pad2: u32,     // 20: align `extra`
    extra: usize,   // 24
    _tail: [u8; 8], // 32..40
}

const _: () = assert!(std::mem::size_of::<KeyInput>() == 40);

/// One planned key event: either a UTF-16 unit typed directly
/// (`KEYEVENTF_UNICODE`, no layout involved) or a virtual key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(any(feature = "audio", feature = "gui", test))]
enum KeyPlan {
    Unicode { unit: u16, up: bool },
    Vk { vk: u8, up: bool },
}

/// Keystroke plan for typing text without touching the clipboard (L-03).
/// `\n` becomes Enter; everything else rides as UTF-16 units, so any
/// keyboard layout receives the exact characters.
#[cfg(any(feature = "audio", feature = "gui", test))]
fn plan_unicode_typing(text: &str) -> Vec<KeyPlan> {
    // Normalize line endings first: lone \r never reaches the field.
    let clean = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::with_capacity(clean.len() * 2);
    for unit in clean.encode_utf16() {
        if unit == b'\n' as u16 {
            out.push(KeyPlan::Vk {
                vk: 0x0D,
                up: false,
            });
            out.push(KeyPlan::Vk { vk: 0x0D, up: true });
        } else {
            out.push(KeyPlan::Unicode { unit, up: false });
            out.push(KeyPlan::Unicode { unit, up: true });
        }
    }
    out
}

/// Play a [`plan_unicode_typing`] plan through `SendInput`, batched.
#[cfg(any(feature = "audio", feature = "gui"))]
fn send_unicode_plan(plan: &[KeyPlan]) {
    const INPUT_KEYBOARD: u32 = 1;
    const KEYEVENTF_UNICODE: u32 = 0x0004;
    for chunk in plan.chunks(64) {
        let mut inputs = Vec::with_capacity(chunk.len());
        for ev in chunk {
            let (vk, scan, flags) = match *ev {
                KeyPlan::Unicode { unit, up } => {
                    let mut f = KEYEVENTF_UNICODE;
                    if up {
                        f |= KEYEVENTF_KEYUP;
                    }
                    (0u16, unit, f)
                }
                KeyPlan::Vk { vk, up } => {
                    let mut f = 0u32;
                    if up {
                        f |= KEYEVENTF_KEYUP;
                    }
                    (vk as u16, 0u16, f)
                }
            };
            inputs.push(KeyInput {
                kind: INPUT_KEYBOARD,
                _pad: 0,
                vk,
                scan,
                flags,
                time: 0,
                _pad2: 0,
                extra: 0,
                _tail: [0; 8],
            });
        }
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<KeyInput>() as c_int,
            );
        }
    }
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
    #[cfg(not(test))]
    fn Beep(freq: u32, duration_ms: u32) -> c_int;
    #[cfg(feature = "gui")]
    fn LocalFree(ptr: *const c_void) -> *const c_void;
}

#[cfg(feature = "gui")]
#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        hive: *const c_void,
        subkey: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        attrs: *const c_void,
        out_key: *mut *const c_void,
        disposition: *mut u32,
    ) -> i32;
    fn RegSetValueExW(
        key: *const c_void,
        name: *const u16,
        reserved: u32,
        kind: u32,
        data: *const u8,
        size: u32,
    ) -> i32;
    fn RegDeleteValueW(key: *const c_void, name: *const u16) -> i32;
    fn RegCloseKey(key: *const c_void) -> i32;
}

#[cfg(feature = "gui")]
#[link(name = "crypt32")]
unsafe extern "system" {
    fn CryptProtectData(
        data_in: *const DataBlob,
        descr: *const u16,
        entropy: *const DataBlob,
        reserved: *const c_void,
        prompt: *const c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> c_int;
    fn CryptUnprotectData(
        data_in: *const DataBlob,
        descr: *mut *mut u16,
        entropy: *const DataBlob,
        reserved: *const c_void,
        prompt: *const c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> c_int;
}

/// Byte blob for DPAPI (`DATA_BLOB`).
#[repr(C)]
#[cfg(feature = "gui")]
struct DataBlob {
    size: u32,
    data: *mut u8,
}

/// DPAPI-protect bytes for the current user (no UI). Used for the
/// encrypted history file (M-17) and readable back on this machine only.
#[cfg(feature = "gui")]
pub(crate) fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    unsafe {
        let input = DataBlob {
            size: plain.len() as u32,
            data: plain.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            size: 0,
            data: std::ptr::null_mut(),
        };
        if CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        ) == 0
        {
            return Err("CryptProtectData failed".to_string());
        }
        let bytes = std::slice::from_raw_parts(output.data, output.size as usize).to_vec();
        LocalFree(output.data as *const c_void);
        Ok(bytes)
    }
}

/// Reverse [`dpapi_protect`]. Fails on other machines/users (by design).
#[cfg(feature = "gui")]
pub(crate) fn dpapi_unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    unsafe {
        let input = DataBlob {
            size: blob.len() as u32,
            data: blob.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            size: 0,
            data: std::ptr::null_mut(),
        };
        let mut descr: *mut u16 = std::ptr::null_mut();
        if CryptUnprotectData(
            &input,
            &mut descr,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        ) == 0
        {
            return Err("CryptUnprotectData failed".to_string());
        }
        if !descr.is_null() {
            LocalFree(descr as *const c_void);
        }
        let bytes = std::slice::from_raw_parts(output.data, output.size as usize).to_vec();
        LocalFree(output.data as *const c_void);
        Ok(bytes)
    }
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
/// Second hotkey: edit mode (D-14). 0 = disabled.
static HOTKEY_EDIT: AtomicU16 = AtomicU16::new(0);
/// Set by the edit hotkey press, taken by the next paste for the journal
/// method mark.
static EDIT_ARMED: AtomicBool = AtomicBool::new(false);
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

/// Capture-start instant for hold-duration checks (D-10). Read without
/// taking: `paste()` takes the slot later for journal timings.
#[cfg(feature = "audio")]
pub(crate) fn press_instant() -> Option<std::time::Instant> {
    KEYDOWN_AT.lock().ok().and_then(|slot| *slot)
}

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

/// Change the edit-mode hotkey (0 disables it).
pub fn set_edit_hotkey_vk(vk: u16) {
    HOTKEY_EDIT.store(vk, Ordering::SeqCst);
}

/// Parse an edit-key name (`выкл`/empty disables); mirrors `Hotkey::parse`.
pub fn parse_edit_key(name: &str) -> u16 {
    let n = name.trim().to_ascii_uppercase();
    if n.is_empty() || n == "OFF" || n == "ВЫКЛ" {
        return 0;
    }
    Hotkey::parse(&n).map(|h| h.to_vk()).unwrap_or(0)
}

/// Pause (`false`) or resume (`true`) dictation without touching the UI.
#[cfg(any(feature = "gui", test))]
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

#[cfg(any(feature = "gui", test))]
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

/// UTF-16 + NUL for WinAPI calls.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Autostart via HKCU...\Run (B-10). `on` writes
/// `"flowvoice"="<exe>" --gui`, off removes the value.
#[cfg(feature = "gui")]
pub fn set_autostart(on: bool) -> Result<(), String> {
    const HKEY_CURRENT_USER: *const c_void = 0x80000001 as *const c_void;
    const KEY_WRITE: u32 = 0x20006;
    const REG_SZ: u32 = 1;
    let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = wide("flowvoice");
    unsafe {
        let mut key: *const c_void = std::ptr::null();
        let mut disposition = 0u32;
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_WRITE,
            std::ptr::null(),
            &mut key,
            &mut disposition,
        ) != 0
        {
            return Err("cannot open Run key".to_string());
        }
        let result = if on {
            let exe = std::env::current_exe().map_err(|e| format!("cannot locate exe: {e}"))?;
            let cmd = format!("\"{}\" --gui", exe.display());
            let blob: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            let rc = RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                blob.as_ptr() as *const u8,
                (blob.len() * 2) as u32,
            );
            if rc != 0 {
                Err("cannot write Run value".to_string())
            } else {
                Ok(())
            }
        } else {
            // Missing value on disable is fine (already off).
            RegDeleteValueW(key, name.as_ptr());
            Ok(())
        };
        RegCloseKey(key);
        result
    }
}

/// Second copy exits with a message instead of fighting over the hotkey
/// and the microphone (O-14).
pub fn ensure_single_instance() -> bool {
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

/// Audible signals. Silent in tests: `cargo test` must not beep.
#[cfg(not(test))]
fn beep(freq: u32, ms: u32) {
    if sound_on() {
        unsafe {
            Beep(freq, ms);
        }
    }
}

#[cfg(test)]
fn beep(_freq: u32, _ms: u32) {}

/// Error signal: distinct low tone (AG-15).
#[cfg(feature = "audio")]
pub fn beep_error() {
    beep(220, 200);
}

extern "system" fn hook_proc(code: c_int, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && !workstation_locked() {
        // KBDLLHOOKSTRUCT starts with vkCode: u32, scanCode: u32, ...
        let vk = unsafe { *(l_param as *const u32) } as u16;
        handle_key_event(w_param, vk);
    }
    unsafe { CallNextHookEx(0, code, w_param, l_param) }
}

/// Route one key event through the transition table. Split out for tests:
/// no desktop check, no hook chain here — just the state machine.
/// The edit hotkey (D-14) starts the same pipeline flagged for edit mode.
fn handle_key_event(w_param: usize, vk: u16) {
    let down = w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize;
    let hotkey = HOTKEY.load(Ordering::SeqCst);
    let edit_key = HOTKEY_EDIT.load(Ordering::SeqCst);
    let target = if vk == hotkey {
        Some(false)
    } else if edit_key != 0 && vk == edit_key {
        Some(true)
    } else {
        None
    };
    let Some(edit) = target else {
        return;
    };
    if down && edit {
        EDIT_ARMED.store(true, Ordering::SeqCst);
    }
    // The transition table matches on the pressed key: feed it the key
    // that actually fired (main or edit), not always the main one.
    let matched_vk = if edit { edit_key } else { hotkey };
    let ev = if down {
        crate::hotkey::HotkeyEvent::Press { vk: matched_vk }
    } else {
        crate::hotkey::HotkeyEvent::Release { vk: matched_vk }
    };
    // The transition table is pure and unit-tested (`hotkey::decide`).
    // Handlers own their transitions (see `try_begin_recording`): the
    // stored state here is only advisory, so a repeated press can never
    // spawn a second recorder over a live one.
    let (_, action) = crate::hotkey::decide(
        RECORDING.load(Ordering::SeqCst),
        ENABLED.load(Ordering::SeqCst),
        matched_vk,
        ev,
    );
    match action {
        crate::hotkey::HotkeyAction::Start => handle_keydown(),
        crate::hotkey::HotkeyAction::Stop => handle_keyup(),
        crate::hotkey::HotkeyAction::Ignore => {}
    }
}

/// Attempt the idle→recording transition. Returns true exactly once per
/// utterance: the caller owns the new recording and must spawn one worker.
/// A repeated press over a live recording returns false — no second thread,
/// no second capture over the first one.
fn try_begin_recording() -> bool {
    RECORDING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

fn handle_keydown() {
    if !try_begin_recording() {
        return;
    }
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
        // No FS writes in tests: a stale marker would fake a crash notice.
        #[cfg(not(test))]
        state::write_pending();
    }
    launch_recorder();
}

fn handle_keyup() {
    STOP.store(true, Ordering::SeqCst);
    beep(660, 90);
    if let Ok(mut slot) = KEYUP_AT.lock() {
        *slot = Some(std::time::Instant::now());
    }
    RECORDING.store(false, Ordering::SeqCst);
}

/// Spawn the recorder worker. In tests only a counter moves (no threads,
/// no microphone, no files): the idempotency under test is the guard above.
#[cfg(test)]
static SPAWNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn launch_recorder() {
    #[cfg(test)]
    {
        SPAWNS.fetch_add(1, Ordering::SeqCst);
        // Keep the production worker linked in test builds (no dead code):
        // the thread spawn above is what the idempotency tests count.
        let _ = spawn_recorder as fn();
    }
    #[cfg(not(test))]
    {
        std::thread::spawn(spawn_recorder);
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
    // Short-replica threshold (`FLOWVOICE_MIN_CHARS`, default 2): stray
    // marks from accidental taps never reach the field.
    let min_chars: usize = std::env::var("FLOWVOICE_MIN_CHARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    if !flowcore::meets_min_len(text, min_chars) {
        state::emit("[done] no speech detected");
        return;
    }

    // Optional separator so back-to-back replicas don't glue (AM-03).
    let spaced = std::env::var("FLOWVOICE_LEADING_SPACE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let text = flowcore::pad_replica_start(text, spaced);
    state::emit(&format!("[final] {text}"));
    let total = paste(&text);
    if total > 0.0 {
        state::emit(&format!("[timing] {total:.1}s keyup->paste"));
    }
}

/// Paste method select: `unicode` types keystrokes directly (L-03),
/// anything else uses clipboard + Ctrl+V (default).
#[cfg(any(feature = "audio", feature = "gui", test))]
fn paste_method_is_unicode() -> bool {
    std::env::var("FLOWVOICE_PASTE_METHOD")
        .map(|v| v.eq_ignore_ascii_case("unicode"))
        .unwrap_or(false)
}

/// Play parsed macro keystrokes through keybd_event (AJ-03).
#[cfg(feature = "audio")]
pub(crate) fn send_combo(strokes: &[crate::macros::KeyStroke]) {
    unsafe {
        for s in strokes {
            if s.down {
                keybd_event(s.vk, 0, 0, 0);
            } else {
                keybd_event(s.vk, 0, KEYEVENTF_KEYUP, 0);
            }
        }
    }
}

/// Copy the text to the clipboard and simulate Ctrl+V. Pasting is the
/// fastest way to enter arbitrary Unicode text reliably.
///
/// Returns keyup→paste seconds (0 when unknown). The user's clipboard is
/// restored afterwards (L-01); failures are reported, never silent.
#[cfg(any(feature = "audio", feature = "gui"))]
pub(crate) fn paste(text: &str) -> f32 {
    // Optional pause before pasting (AM-20 `FLOWVOICE_PASTE_DELAY_MS`).
    let delay_ms = crate::util::env_u64("FLOWVOICE_PASTE_DELAY_MS", 0);
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms.min(5000)));
    }

    if paste_method_is_unicode() {
        send_unicode_plan(&plan_unicode_typing(text));
    } else {
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

        // Restore what the user had (L-01); empty stays empty (L-04).
        // Best-effort: a busy clipboard only costs the restore, never the paste.
        let restore_ms = crate::util::env_u64("FLOWVOICE_RESTORE_MS", 400);
        if restore_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(restore_ms.min(5000)));
            let _ = match saved {
                Some(prev) => cb.set_text(prev),
                None => cb.clear(),
            };
        }
    }

    // Latency from hotkey release to the finished paste. The release
    // instant is taken once and shared with the journal below; the
    // breakdown line itself is emitted by `finish()`.
    let keyup = KEYUP_AT.lock().ok().and_then(|mut slot| slot.take());
    let keydown = KEYDOWN_AT.lock().ok().and_then(|mut slot| slot.take());
    let secs = keyup.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);

    journal_append(
        text,
        secs,
        keydown,
        keyup,
        if EDIT_ARMED.swap(false, Ordering::SeqCst) {
            state::emit("[edit-mode] replacing selection");
            "paste-edit"
        } else {
            "paste"
        },
    );
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
    crate::util::env_flag("FLOWVOICE_NO_HISTORY")
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
    apply_key_env();

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
    apply_key_env();
    std::thread::spawn(pump);
}

/// Second-hotkey env (`FLOWVOICE_EDIT_KEY` / `--edit-key`, D-14).
fn apply_key_env() {
    if let Ok(name) = std::env::var("FLOWVOICE_EDIT_KEY") {
        set_edit_hotkey_vk(parse_edit_key(&name));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    const KEY: u16 = VK_RCONTROL;
    const OTHER: u16 = 0x41;
    const WM_KEYUP: usize = 0x0101;

    /// Tests share process-wide hook statics: serialize them.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap();
        RECORDING.store(false, SeqCst);
        STOP.store(true, SeqCst);
        ENABLED.store(true, SeqCst);
        HOTKEY.store(KEY, SeqCst);
        HOTKEY_EDIT.store(0, SeqCst);
        EDIT_ARMED.store(false, SeqCst);
        SPAWNS.store(0, SeqCst);
        guard
    }

    /// Drive the routing layer directly (what `hook_proc` calls after the
    /// code/desktop guards): no hook chain, no desktop check here.
    fn key(vk: u16, down: bool) {
        handle_key_event(if down { WM_KEYDOWN as usize } else { WM_KEYUP }, vk);
    }

    fn spawns() -> usize {
        SPAWNS.load(SeqCst)
    }

    #[test]
    fn repeat_press_over_live_recording_spawns_once() {
        let _guard = reset();
        key(KEY, true);
        assert!(RECORDING.load(SeqCst));
        assert_eq!(spawns(), 1);
        // Auto-repeat / shaky finger: still exactly one recorder.
        key(KEY, true);
        key(KEY, true);
        assert!(RECORDING.load(SeqCst));
        assert_eq!(spawns(), 1);
        // Release ends it; the next press is a fresh recording.
        key(KEY, false);
        assert!(!RECORDING.load(SeqCst));
        key(KEY, true);
        assert_eq!(spawns(), 2);
        key(KEY, false);
    }

    #[test]
    fn try_begin_is_idempotent() {
        let _guard = reset();
        assert!(try_begin_recording());
        assert!(!try_begin_recording());
        assert!(!try_begin_recording());
        handle_keyup();
        assert!(try_begin_recording());
    }

    #[test]
    fn stray_release_and_wrong_key_do_nothing() {
        let _guard = reset();
        key(KEY, false);
        assert!(!RECORDING.load(SeqCst));
        assert_eq!(spawns(), 0);
        key(OTHER, true);
        key(OTHER, false);
        assert!(!RECORDING.load(SeqCst));
        assert_eq!(spawns(), 0);
    }

    #[test]
    fn disabled_switch_blocks_start_but_keeps_state() {
        let _guard = reset();
        assert!(is_enabled());
        set_enabled(false);
        assert!(!is_enabled());
        key(KEY, true);
        assert!(!RECORDING.load(SeqCst));
        assert_eq!(spawns(), 0);
        set_enabled(true);
        key(KEY, true);
        assert_eq!(spawns(), 1);
        key(KEY, false);
    }

    #[test]
    fn negative_code_passes_through_untouched() {
        let _guard = reset();
        let vk32 = KEY as u32;
        hook_proc(-1, WM_KEYDOWN as usize, &vk32 as *const u32 as isize);
        assert!(!RECORDING.load(SeqCst));
        assert_eq!(spawns(), 0);
    }

    #[test]
    fn edit_key_runs_same_pipeline_flagged() {
        let _guard = reset();
        set_edit_hotkey_vk(0x77); // F8
        assert_eq!(parse_edit_key("выкл"), 0);
        assert_eq!(parse_edit_key(""), 0);
        assert_eq!(parse_edit_key("F8"), 0x77);
        // Edit press starts recording like the main key.
        key(0x77, true);
        assert!(RECORDING.load(SeqCst));
        assert_eq!(spawns(), 1);
        assert!(EDIT_ARMED.load(SeqCst));
        // Main key over a live edit recording is ignored, not doubled.
        key(KEY, true);
        assert_eq!(spawns(), 1);
        key(0x77, false);
        assert!(!RECORDING.load(SeqCst));
        set_edit_hotkey_vk(0);
    }

    #[test]
    fn sound_setting_roundtrip() {
        let _guard = reset();
        std::env::remove_var("FLOWVOICE_SOUND");
        assert!(sound_on());
        std::env::set_var("FLOWVOICE_SOUND", "0");
        assert!(!sound_on());
        std::env::remove_var("FLOWVOICE_SOUND");
    }

    #[cfg(feature = "gui")]
    #[test]
    fn dpapi_roundtrip() {
        let secret = "секретный текст history 123".as_bytes();
        let blob = dpapi_protect(secret).expect("protect works headless");
        assert_ne!(blob, secret);
        let back = dpapi_unprotect(&blob).expect("unprotect works headless");
        assert_eq!(back, secret);
        assert!(dpapi_unprotect(b"garbage bytes here").is_err());
    }

    #[test]
    fn unicode_plan_types_exact_units() {
        // "А\nё" in UTF-16 units + down/up each, newline as Enter.
        let plan = plan_unicode_typing("А\nё");
        let units: Vec<u16> = "А".encode_utf16().chain("ё".encode_utf16()).collect();
        assert_eq!(
            plan,
            vec![
                KeyPlan::Unicode {
                    unit: units[0],
                    up: false
                },
                KeyPlan::Unicode {
                    unit: units[0],
                    up: true
                },
                KeyPlan::Vk {
                    vk: 0x0D,
                    up: false
                },
                KeyPlan::Vk { vk: 0x0D, up: true },
                KeyPlan::Unicode {
                    unit: units[1],
                    up: false
                },
                KeyPlan::Unicode {
                    unit: units[1],
                    up: true
                },
            ]
        );
        // CRLF folds to one Enter; lone CR too.
        assert_eq!(
            plan_unicode_typing("a\r\nb\rc").len(),
            plan_unicode_typing("a\nb\nc").len()
        );
    }

    #[test]
    fn paste_method_select() {
        let _guard = reset();
        std::env::remove_var("FLOWVOICE_PASTE_METHOD");
        assert!(!paste_method_is_unicode());
        std::env::set_var("FLOWVOICE_PASTE_METHOD", "UNICODE");
        assert!(paste_method_is_unicode());
        std::env::remove_var("FLOWVOICE_PASTE_METHOD");
    }
}

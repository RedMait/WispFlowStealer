// SPDX-License-Identifier: MIT
/* Vendored fork of the `vosk-sys` crate (MIT, https://github.com/Bear-03/vosk-rs).
 *
 * The original exposes vosk functions as ordinary link-time imports, which forces
 * vosk.dll to be loadable whenever the binary starts (even for --demo mode).
 * This fork instead resolves the functions lazily at runtime with LoadLibrary /
 * GetProcAddress, so the process only needs vosk.dll when dictation is actually
 * attempted.
 *
 * Search order for the native library:
 *   1. standard search (exe directory, PATH, system directories) - "vosk.dll"
 *   2. "native\\vosk.dll" and "native/vosk.dll" relative to the working dir
 */

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use std::os::raw::{c_char, c_int, c_short, c_void};
use std::sync::OnceLock;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VoskModel {
    _unused: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VoskSpkModel {
    _unused: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VoskRecognizer {
    _unused: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VoskBatchModel {
    _unused: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VoskBatchRecognizer {
    _unused: [u8; 0],
}

type Hmodule = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    /// LoadLibraryExW with custom search flags.
    ///
    /// The returned HMODULE must not be FreeLibrary'd by us: the process keeps
    /// the library loaded for its lifetime.
    fn LoadLibraryExW(name: *const u16, file: Hmodule, flags: u32) -> Hmodule;
    fn GetProcAddress(h_module: Hmodule, name: *const c_char) -> *mut c_void;
}

// LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS.
//
// The default dirs flag alone does NOT include the directory the loaded DLL
// lives in, but the mingw runtime (libgcc_s, libstdc++-6, libwinpthread)
// stored next to vosk.dll must be resolvable — hence DLL_LOAD_DIR.
const LOAD_FLAGS: u32 = 0x100 | 0x010;

/// Raw handles are not Send/Sync, so we cache them as usize and convert back.
static VOSK_DLL: OnceLock<Option<usize>> = OnceLock::new();

fn get_dll() -> Option<Hmodule> {
    match VOSK_DLL.get_or_init(load_dll) {
        Some(h) => Some(*h as Hmodule),
        None => None,
    }
}

fn load_dll() -> Option<usize> {
    let mut candidates: Vec<String> = vec!["vosk.dll".into()];
    // LOAD_LIBRARY_SEARCH_* flags exclude the current directory, so relative
    // candidates must be made absolute ourselves before passing them along.
    if let Ok(cwd) = std::env::current_dir() {
        for native in ["native", "native/"] {
            let p = cwd.join(native).join("vosk.dll");
            candidates.push(p.to_string_lossy().into_owned());
        }
    }
    for candidate in &candidates {
        let wide_name: Vec<u16> = candidate.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: wide_name is a valid NUL-terminated UTF-16 path.
        let handle = unsafe { LoadLibraryExW(wide_name.as_ptr(), std::ptr::null_mut(), LOAD_FLAGS) };
        if !handle.is_null() {
            return Some(handle as usize);
        }
    }
    None
}

/// Resolve `name` (without trailing NUL) in the loaded library.
unsafe fn symbol<T>(name: &str) -> Option<T> {
    let dll = get_dll()?;
    let c_name = std::ffi::CString::new(name).ok()?;
    let ptr = GetProcAddress(dll, c_name.as_ptr());
    if ptr.is_null() {
        None
    } else {
        // Function pointers are pointer-sized on all supported Windows targets,
        // so copying the raw pointer bytes into the fn type is sound.
        Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&ptr) })
    }
}

/// Static NUL-terminated C string returned by string-returning wrappers when
/// the library is unavailable, so callers never dereference a null pointer.
static EMPTY_CSTR: &[u8] = b"\0";
const EMPTY_PTR: *const c_char = EMPTY_CSTR.as_ptr() as *const c_char;

pub unsafe fn vosk_model_new(model_path: *const c_char) -> *mut VoskModel {
    unsafe {
        match symbol::<unsafe extern "C" fn(*const c_char) -> *mut VoskModel>("vosk_model_new") {
            Some(f) => f(model_path),
            None => std::ptr::null_mut(),
        }
    }
}

pub unsafe fn vosk_model_free(model: *mut VoskModel) {
    unsafe {
        if let Some(f) = symbol::<unsafe extern "C" fn(*mut VoskModel)>("vosk_model_free") {
            f(model);
        }
    }
}

pub unsafe fn vosk_model_find_word(model: *mut VoskModel, word: *const c_char) -> c_int {
    unsafe {
        match symbol::<unsafe extern "C" fn(*mut VoskModel, *const c_char) -> c_int>(
            "vosk_model_find_word",
        ) {
            Some(f) => f(model, word),
            None => -1,
        }
    }
}

pub unsafe fn vosk_spk_model_new(model_path: *const c_char) -> *mut VoskSpkModel {
    unsafe {
        match symbol::<unsafe extern "C" fn(*const c_char) -> *mut VoskSpkModel>(
            "vosk_spk_model_new",
        ) {
            Some(f) => f(model_path),
            None => std::ptr::null_mut(),
        }
    }
}

pub unsafe fn vosk_spk_model_free(model: *mut VoskSpkModel) {
    unsafe {
        if let Some(f) = symbol::<unsafe extern "C" fn(*mut VoskSpkModel)>("vosk_spk_model_free") {
            f(model);
        }
    }
}

pub unsafe fn vosk_recognizer_new(model: *mut VoskModel, sample_rate: f32) -> *mut VoskRecognizer {
    unsafe {
        match symbol::<unsafe extern "C" fn(*mut VoskModel, f32) -> *mut VoskRecognizer>(
            "vosk_recognizer_new",
        ) {
            Some(f) => f(model, sample_rate),
            None => std::ptr::null_mut(),
        }
    }
}

pub unsafe fn vosk_recognizer_new_spk(
    model: *mut VoskModel,
    sample_rate: f32,
    spk_model: *mut VoskSpkModel,
) -> *mut VoskRecognizer {
    unsafe {
        match symbol::<
            unsafe extern "C" fn(*mut VoskModel, f32, *mut VoskSpkModel) -> *mut VoskRecognizer,
        >("vosk_recognizer_new_spk")
        {
            Some(f) => f(model, sample_rate, spk_model),
            None => std::ptr::null_mut(),
        }
    }
}

pub unsafe fn vosk_recognizer_new_grm(
    model: *mut VoskModel,
    sample_rate: f32,
    grammar: *const c_char,
) -> *mut VoskRecognizer {
    unsafe {
        match symbol::<
            unsafe extern "C" fn(*mut VoskModel, f32, *const c_char) -> *mut VoskRecognizer,
        >("vosk_recognizer_new_grm")
        {
            Some(f) => f(model, sample_rate, grammar),
            None => std::ptr::null_mut(),
        }
    }
}

pub unsafe fn vosk_recognizer_set_spk_model(
    recognizer: *mut VoskRecognizer,
    spk_model: *mut VoskSpkModel,
) {
    unsafe {
        if let Some(f) = symbol::<unsafe extern "C" fn(*mut VoskRecognizer, *mut VoskSpkModel)>(
            "vosk_recognizer_set_spk_model",
        ) {
            f(recognizer, spk_model);
        }
    }
}

pub unsafe fn vosk_recognizer_set_max_alternatives(
    recognizer: *mut VoskRecognizer,
    max_alternatives: c_int,
) {
    unsafe {
        if let Some(f) =
            symbol::<unsafe extern "C" fn(*mut VoskRecognizer, c_int)>("vosk_recognizer_set_max_alternatives")
        {
            f(recognizer, max_alternatives);
        }
    }
}

pub unsafe fn vosk_recognizer_set_words(recognizer: *mut VoskRecognizer, words: c_int) {
    unsafe {
        if let Some(f) =
            symbol::<unsafe extern "C" fn(*mut VoskRecognizer, c_int)>("vosk_recognizer_set_words")
        {
            f(recognizer, words);
        }
    }
}

pub unsafe fn vosk_recognizer_set_partial_words(
    recognizer: *mut VoskRecognizer,
    partial_words: c_int,
) {
    unsafe {
        if let Some(f) = symbol::<unsafe extern "C" fn(*mut VoskRecognizer, c_int)>(
            "vosk_recognizer_set_partial_words",
        ) {
            f(recognizer, partial_words);
        }
    }
}

pub unsafe fn vosk_recognizer_set_nlsml(recognizer: *mut VoskRecognizer, nlsml: c_int) {
    unsafe {
        if let Some(f) =
            symbol::<unsafe extern "C" fn(*mut VoskRecognizer, c_int)>("vosk_recognizer_set_nlsml")
        {
            f(recognizer, nlsml);
        }
    }
}

pub unsafe fn vosk_recognizer_accept_waveform(
    recognizer: *mut VoskRecognizer,
    data: *const c_char,
    length: c_int,
) -> c_int {
    unsafe {
        match symbol::<unsafe extern "C" fn(*mut VoskRecognizer, *const c_char, c_int) -> c_int>(
            "vosk_recognizer_accept_waveform",
        ) {
            Some(f) => f(recognizer, data, length),
            None => -1,
        }
    }
}

pub unsafe fn vosk_recognizer_accept_waveform_s(
    recognizer: *mut VoskRecognizer,
    data: *const c_short,
    length: c_int,
) -> c_int {
    unsafe {
        match symbol::<
            unsafe extern "C" fn(*mut VoskRecognizer, *const c_short, c_int) -> c_int,
        >("vosk_recognizer_accept_waveform_s")
        {
            Some(f) => f(recognizer, data, length),
            None => -1,
        }
    }
}

pub unsafe fn vosk_recognizer_accept_waveform_f(
    recognizer: *mut VoskRecognizer,
    data: *const f32,
    length: c_int,
) -> c_int {
    unsafe {
        match symbol::<unsafe extern "C" fn(*mut VoskRecognizer, *const f32, c_int) -> c_int>(
            "vosk_recognizer_accept_waveform_f",
        ) {
            Some(f) => f(recognizer, data, length),
            None => -1,
        }
    }
}

pub unsafe fn vosk_recognizer_result(recognizer: *mut VoskRecognizer) -> *const c_char {
    unsafe {
        match symbol::<unsafe extern "C" fn(*mut VoskRecognizer) -> *const c_char>(
            "vosk_recognizer_result",
        ) {
            Some(f) => f(recognizer),
            None => EMPTY_PTR,
        }
    }
}

pub unsafe fn vosk_recognizer_partial_result(recognizer: *mut VoskRecognizer) -> *const c_char {
    unsafe {
        match symbol::<unsafe extern "C" fn(*mut VoskRecognizer) -> *const c_char>(
            "vosk_recognizer_partial_result",
        ) {
            Some(f) => f(recognizer),
            None => EMPTY_PTR,
        }
    }
}

pub unsafe fn vosk_recognizer_final_result(recognizer: *mut VoskRecognizer) -> *const c_char {
    unsafe {
        match symbol::<unsafe extern "C" fn(*mut VoskRecognizer) -> *const c_char>(
            "vosk_recognizer_final_result",
        ) {
            Some(f) => f(recognizer),
            None => EMPTY_PTR,
        }
    }
}

pub unsafe fn vosk_recognizer_reset(recognizer: *mut VoskRecognizer) {
    unsafe {
        if let Some(f) = symbol::<unsafe extern "C" fn(*mut VoskRecognizer)>("vosk_recognizer_reset")
        {
            f(recognizer);
        }
    }
}

pub unsafe fn vosk_recognizer_free(recognizer: *mut VoskRecognizer) {
    unsafe {
        if let Some(f) =
            symbol::<unsafe extern "C" fn(*mut VoskRecognizer)>("vosk_recognizer_free")
        {
            f(recognizer);
        }
    }
}

pub unsafe fn vosk_set_log_level(log_level: c_int) {
    unsafe {
        if let Some(f) = symbol::<unsafe extern "C" fn(c_int)>("vosk_set_log_level") {
            f(log_level);
        }
    }
}
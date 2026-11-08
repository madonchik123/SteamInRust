//! Keyboard and mouse for the in-game overlay.
//!
//! We subclass the game's window (the swap chain's output window): Shift+Tab
//! toggles the overlay, and while it's open the window's mouse, keyboard and
//! raw-input messages go to the app instead of the game.
//!
//! Games also lock and hide the cursor (ClipCursor, SetCursorPos, ShowCursor,
//! SetCursor) and read input outside the window procedure (GetRawInputData,
//! GetRawInputBuffer, GetAsyncKeyState, GetKeyState, GetCursorPos). While the
//! overlay is open those calls from the game's modules are answered by us,
//! by patching the game modules' import tables (never our own).

use super::shared;
use super::wire::{self, Input};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicUsize, Ordering};
use windows::core::BOOL;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCSTR, w};

/// The overlay has the input right now (read from hooks, so no locks).
static OPEN: AtomicBool = AtomicBool::new(false);
static HWND_NOW: AtomicIsize = AtomicIsize::new(0);
static REAL_WNDPROC: AtomicIsize = AtomicIsize::new(0);
static MSG_CAPTURE: AtomicU32 = AtomicU32::new(0);
static BB_W: AtomicU32 = AtomicU32::new(0);
static BB_H: AtomicU32 = AtomicU32::new(0);
/// Swallow the WM_CHAR '\t' that follows our Shift+Tab.
static EAT_TAB_CHAR: AtomicBool = AtomicBool::new(false);
/// A UTF-16 high surrogate waiting for its pair.
static HIGH_SURROGATE: AtomicU32 = AtomicU32::new(0);
/// ShowCursor(TRUE) calls we made to show the cursor, undone on close.
static SHOWN: AtomicI32 = AtomicI32::new(0);
/// The game's ShowCursor calls while open, applied on close.
static GAME_SHOW_DELTA: AtomicI32 = AtomicI32::new(0);
/// The game's cursor clip when the overlay opened (or set while open).
static SAVED_CLIP: std::sync::Mutex<Option<RECT>> = std::sync::Mutex::new(None);
/// Cursor position the game sees while the overlay is open.
static FROZEN: std::sync::Mutex<Option<POINT>> = std::sync::Mutex::new(None);

pub fn set_size(size: (u32, u32)) {
    BB_W.store(size.0, Ordering::Relaxed);
    BB_H.store(size.1, Ordering::Relaxed);
}

/// Subclass the game's window (again if the game made a new one).
pub fn attach(hwnd: HWND) {
    if hwnd.is_invalid() || HWND_NOW.load(Ordering::Acquire) == hwnd.0 as isize {
        return;
    }
    unsafe {
        if MSG_CAPTURE.load(Ordering::Relaxed) == 0 {
            MSG_CAPTURE.store(RegisterWindowMessageW(w!("SteamInRust.Overlay.Capture")), Ordering::Relaxed);
        }
        let old = HWND_NOW.swap(hwnd.0 as isize, Ordering::AcqRel);
        if old != 0 {
            // Give the old window its own procedure back.
            let prev = REAL_WNDPROC.load(Ordering::Acquire);
            if prev != 0 && IsWindow(Some(HWND(old as *mut c_void))).as_bool() {
                set_wndproc(HWND(old as *mut c_void), prev);
            }
        }
        let prev = set_wndproc(hwnd, wndproc as *const () as usize as isize);
        REAL_WNDPROC.store(prev, Ordering::Release);
        crate::log!("overlay: listening to the game's window");
    }
}

#[cfg(target_pointer_width = "64")]
unsafe fn set_wndproc(hwnd: HWND, proc: isize) -> isize {
    unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc) }
}
#[cfg(target_pointer_width = "32")]
unsafe fn set_wndproc(hwnd: HWND, proc: isize) -> isize {
    unsafe { SetWindowLongW(hwnd, GWLP_WNDPROC, proc as i32) as isize }
}

/// Take or give back the input. Cursor visibility is per thread, so that
/// part runs on the window's thread (a message to our window procedure).
pub fn capture(open: bool) {
    OPEN.store(open, Ordering::Release);
    if open {
        patch_imports();
    }
    let hwnd = HWND_NOW.load(Ordering::Acquire);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), MSG_CAPTURE.load(Ordering::Relaxed), WPARAM(open as usize), LPARAM(0));
        }
    }
}

/// On the window thread: show and free the cursor, or put back the game's.
unsafe fn apply_capture(hwnd: HWND, open: bool) {
    unsafe {
        let real_show: ShowCursorFn = std::mem::transmute(real(H_SHOW_CURSOR, ShowCursor as *const () as usize));
        let real_clip: ClipCursorFn = std::mem::transmute(real(H_CLIP_CURSOR, ClipCursor as *const () as usize));
        if open {
            let mut clip = RECT::default();
            let _ = GetClipCursor(&mut clip);
            *SAVED_CLIP.lock().unwrap_or_else(|e| e.into_inner()) = Some(clip);
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            *FROZEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
            let _ = real_clip(std::ptr::null());
            let mut n = 0;
            while real_show(BOOL(1)) < 0 && n < 64 {
                n += 1;
            }
            SHOWN.store(n + 1, Ordering::Relaxed);
            GAME_SHOW_DELTA.store(0, Ordering::Relaxed);
            set_cursor_shape();
            // The pointer starts where it is now.
            let mut c = p;
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut c);
            push_pointer(hwnd, c.x, c.y, None);
        } else {
            for _ in 0..SHOWN.swap(0, Ordering::Relaxed) {
                real_show(BOOL(0));
            }
            let delta = GAME_SHOW_DELTA.swap(0, Ordering::Relaxed);
            for _ in 0..delta.unsigned_abs() {
                real_show(BOOL((delta > 0) as i32));
            }
            if let Some(clip) = SAVED_CLIP.lock().unwrap_or_else(|e| e.into_inner()).take() {
                // Only a real clip (not "the whole screen") goes back.
                let screen = RECT { left: GetSystemMetrics(SM_XVIRTUALSCREEN), top: GetSystemMetrics(SM_YVIRTUALSCREEN), right: GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN), bottom: GetSystemMetrics(SM_YVIRTUALSCREEN) + GetSystemMetrics(SM_CYVIRTUALSCREEN) };
                if clip != screen {
                    let _ = real_clip(&clip);
                }
            }
            if let Some(p) = FROZEN.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = SetCursorPos(p.x, p.y);
            }
            shared::push_input(modifiers(), Input::Leave);
        }
    }
}

fn set_cursor_shape() {
    let id = match shared::cursor() {
        wire::cursor::HAND => IDC_HAND,
        wire::cursor::TEXT => IDC_IBEAM,
        wire::cursor::RESIZE_H => IDC_SIZEWE,
        wire::cursor::RESIZE_V => IDC_SIZENS,
        wire::cursor::RESIZE_NWSE => IDC_SIZENWSE,
        wire::cursor::RESIZE_NESW => IDC_SIZENESW,
        wire::cursor::MOVE => IDC_SIZEALL,
        wire::cursor::NOT_ALLOWED => IDC_NO,
        _ => IDC_ARROW,
    };
    unsafe {
        if let Ok(c) = LoadCursorW(None, id) {
            SetCursor(Some(c));
        }
    }
}

fn modifiers() -> u8 {
    let down = |vk: VIRTUAL_KEY| unsafe { GetKeyState(vk.0 as i32) } < 0;
    let mut m = 0;
    if down(VK_SHIFT) {
        m |= wire::MOD_SHIFT;
    }
    if down(VK_CONTROL) {
        m |= wire::MOD_CTRL;
    }
    if down(VK_MENU) {
        m |= wire::MOD_ALT;
    }
    m
}

/// Client pixels to back-buffer pixels (they differ when the game scales).
fn to_backbuffer(hwnd: HWND, x: i32, y: i32) -> (f32, f32) {
    let mut r = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut r) };
    let (cw, ch) = ((r.right - r.left).max(1) as f32, (r.bottom - r.top).max(1) as f32);
    let (bw, bh) = (BB_W.load(Ordering::Relaxed) as f32, BB_H.load(Ordering::Relaxed) as f32);
    if bw <= 0.0 || bh <= 0.0 {
        return (x as f32, y as f32);
    }
    (x as f32 * bw / cw, y as f32 * bh / ch)
}

fn push_pointer(hwnd: HWND, x: i32, y: i32, button: Option<(u8, bool)>) {
    let (x, y) = to_backbuffer(hwnd, x, y);
    let e = match button {
        Some((button, down)) => Input::Button { button, down, x, y },
        None => Input::Move { x, y },
    };
    shared::push_input(modifiers(), e);
}

fn clipboard_text() -> Option<String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        OpenClipboard(None).ok()?;
        let text = (|| {
            let h: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let _ = GlobalUnlock(g);
            Some(s)
        })();
        let _ = CloseClipboard();
        text
    }
}

const WM_MOUSELEAVE_: u32 = 0x02A3;

fn lo(l: LPARAM) -> i32 {
    (l.0 & 0xffff) as u16 as i16 as i32
}
fn hi(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xffff) as u16 as i16 as i32
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(|| unsafe { ours(hwnd, msg, wp, lp) });
    match r {
        Ok(Some(v)) => v,
        _ => unsafe { CallWindowProcW(std::mem::transmute::<isize, WNDPROC>(REAL_WNDPROC.load(Ordering::Acquire)), hwnd, msg, wp, lp) },
    }
}

/// `Some` when the overlay handled the message.
unsafe fn ours(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let open = OPEN.load(Ordering::Acquire);
    if msg == MSG_CAPTURE.load(Ordering::Relaxed) && msg != 0 {
        unsafe { apply_capture(hwnd, wp.0 != 0) };
        return Some(LRESULT(0));
    }
    // Shift+Tab, on key down only (not repeats).
    if (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) && wp.0 as u16 == VK_TAB.0 && lp.0 & (1 << 30) == 0 && unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0 && unsafe { GetKeyState(VK_CONTROL.0 as i32) } >= 0 {
        if shared::with(|s| s.connected) {
            EAT_TAB_CHAR.store(true, Ordering::Relaxed);
            shared::set_open(!open, true, true);
            return Some(LRESULT(0));
        }
    }
    if msg == WM_CHAR && wp.0 == 9 && EAT_TAB_CHAR.swap(false, Ordering::Relaxed) {
        return Some(LRESULT(0));
    }
    if !open {
        return None;
    }
    let handled = Some(LRESULT(0));
    match msg {
        WM_MOUSEMOVE => {
            push_pointer(hwnd, lo(lp), hi(lp), None);
            handled
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONDBLCLK | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONDBLCLK | WM_MBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONDBLCLK | WM_XBUTTONUP => {
            let (button, down) = match msg {
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => (0, true),
                WM_LBUTTONUP => (0, false),
                WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => (1, true),
                WM_RBUTTONUP => (1, false),
                WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => (2, true),
                WM_MBUTTONUP => (2, false),
                _ => (if (wp.0 >> 16) & 0xffff == 1 { 3 } else { 4 }, msg != WM_XBUTTONUP),
            };
            unsafe {
                if down {
                    SetCapture(hwnd);
                } else {
                    let _ = ReleaseCapture();
                }
            }
            push_pointer(hwnd, lo(lp), hi(lp), Some((button, down)));
            Some(LRESULT(if matches!(msg, WM_XBUTTONDOWN | WM_XBUTTONUP | WM_XBUTTONDBLCLK) { 1 } else { 0 }))
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let notches = ((wp.0 >> 16) & 0xffff) as u16 as i16 as f32 / 120.0;
            let e = if msg == WM_MOUSEWHEEL { Input::Wheel { x: 0.0, y: notches } } else { Input::Wheel { x: -notches, y: 0.0 } };
            shared::push_input(modifiers(), e);
            handled
        }
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let vk = wp.0 as u32;
            shared::push_input(modifiers(), Input::Key { vk, down, repeat: down && lp.0 & (1 << 30) != 0 });
            if down && vk == 'V' as u32 && modifiers() & wire::MOD_CTRL != 0 {
                if let Some(text) = clipboard_text() {
                    shared::push_input(modifiers(), Input::Paste(text));
                }
            }
            // Alt+F4 and Alt+Tab still work.
            if msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP {
                return Some(unsafe { DefWindowProcW(hwnd, msg, wp, lp) });
            }
            handled
        }
        WM_CHAR => {
            let unit = wp.0 as u32;
            let text = if (0xD800..0xDC00).contains(&unit) {
                HIGH_SURROGATE.store(unit, Ordering::Relaxed);
                None
            } else if (0xDC00..0xE000).contains(&unit) {
                let high = HIGH_SURROGATE.swap(0, Ordering::Relaxed);
                String::from_utf16(&[high as u16, unit as u16]).ok()
            } else {
                char::from_u32(unit).map(String::from)
            };
            if let Some(t) = text.filter(|t| t.chars().all(|c| !c.is_control())) {
                shared::push_input(modifiers(), Input::Text(t));
            }
            handled
        }
        WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => Some(unsafe { DefWindowProcW(hwnd, msg, wp, lp) }),
        // Raw input is the game's mouse look; let Windows clean it up.
        WM_INPUT => Some(unsafe { DefWindowProcW(hwnd, msg, wp, lp) }),
        WM_SETCURSOR if (lp.0 & 0xffff) as u32 == HTCLIENT => {
            set_cursor_shape();
            Some(LRESULT(1))
        }
        WM_MOUSELEAVE_ => {
            shared::push_input(modifiers(), Input::Leave);
            handled
        }
        _ => None,
    }
}

// ---- Import table hooks -------------------------------------------------

type ShowCursorFn = unsafe extern "system" fn(BOOL) -> i32;
type ClipCursorFn = unsafe extern "system" fn(*const RECT) -> BOOL;
type SetCursorPosFn = unsafe extern "system" fn(i32, i32) -> BOOL;
type GetCursorPosFn = unsafe extern "system" fn(*mut POINT) -> BOOL;
type SetCursorFn = unsafe extern "system" fn(*mut c_void) -> *mut c_void;
type KeyStateFn = unsafe extern "system" fn(i32) -> i16;
type RawBufferFn = unsafe extern "system" fn(*mut c_void, *mut u32, u32) -> u32;
type RawDataFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut u32, u32) -> u32;

struct Hook {
    name: &'static str,
    ours: usize,
}

const H_SHOW_CURSOR: usize = 0;
const H_CLIP_CURSOR: usize = 1;
const H_SET_CURSOR_POS: usize = 2;
const H_GET_CURSOR_POS: usize = 3;
const H_SET_CURSOR: usize = 4;
const H_ASYNC_KEY: usize = 5;
const H_KEY_STATE: usize = 6;
const H_RAW_BUFFER: usize = 7;
const H_RAW_DATA: usize = 8;
const N_HOOKS: usize = 9;

fn hooks() -> [Hook; N_HOOKS] {
    [
        Hook { name: "ShowCursor\0", ours: hook_show_cursor as *const () as usize },
        Hook { name: "ClipCursor\0", ours: hook_clip_cursor as *const () as usize },
        Hook { name: "SetCursorPos\0", ours: hook_set_cursor_pos as *const () as usize },
        Hook { name: "GetCursorPos\0", ours: hook_get_cursor_pos as *const () as usize },
        Hook { name: "SetCursor\0", ours: hook_set_cursor as *const () as usize },
        Hook { name: "GetAsyncKeyState\0", ours: hook_async_key as *const () as usize },
        Hook { name: "GetKeyState\0", ours: hook_key_state as *const () as usize },
        Hook { name: "GetRawInputBuffer\0", ours: hook_raw_buffer as *const () as usize },
        Hook { name: "GetRawInputData\0", ours: hook_raw_data as *const () as usize },
    ]
}

static REAL: [AtomicUsize; N_HOOKS] = [const { AtomicUsize::new(0) }; N_HOOKS];

/// The real user32 function (`fallback`: ours linked directly, same thing).
fn real(i: usize, fallback: usize) -> usize {
    match REAL[i].load(Ordering::Acquire) {
        0 => fallback,
        p => p,
    }
}

unsafe extern "system" fn hook_show_cursor(show: BOOL) -> i32 {
    if OPEN.load(Ordering::Acquire) {
        GAME_SHOW_DELTA.fetch_add(if show.as_bool() { 1 } else { -1 }, Ordering::Relaxed);
        return if show.as_bool() { 0 } else { -1 };
    }
    let f: ShowCursorFn = unsafe { std::mem::transmute(real(H_SHOW_CURSOR, ShowCursor as *const () as usize)) };
    unsafe { f(show) }
}

unsafe extern "system" fn hook_clip_cursor(rect: *const RECT) -> BOOL {
    if OPEN.load(Ordering::Acquire) {
        // Remember it for when the overlay closes.
        *SAVED_CLIP.lock().unwrap_or_else(|e| e.into_inner()) = if rect.is_null() { None } else { Some(unsafe { *rect }) };
        return BOOL(1);
    }
    let f: ClipCursorFn = unsafe { std::mem::transmute(real(H_CLIP_CURSOR, ClipCursor as *const () as usize)) };
    unsafe { f(rect) }
}

unsafe extern "system" fn hook_set_cursor_pos(x: i32, y: i32) -> BOOL {
    if OPEN.load(Ordering::Acquire) {
        if let Ok(mut f) = FROZEN.lock() {
            *f = Some(POINT { x, y });
        }
        return BOOL(1);
    }
    let f: SetCursorPosFn = unsafe { std::mem::transmute(real(H_SET_CURSOR_POS, SetCursorPos as *const () as usize)) };
    unsafe { f(x, y) }
}

unsafe extern "system" fn hook_get_cursor_pos(p: *mut POINT) -> BOOL {
    if OPEN.load(Ordering::Acquire) && !p.is_null() {
        if let Some(frozen) = *FROZEN.lock().unwrap_or_else(|e| e.into_inner()) {
            unsafe { *p = frozen };
            return BOOL(1);
        }
    }
    let f: GetCursorPosFn = unsafe { std::mem::transmute(real(H_GET_CURSOR_POS, GetCursorPos as *const () as usize)) };
    unsafe { f(p) }
}

unsafe extern "system" fn hook_set_cursor(c: *mut c_void) -> *mut c_void {
    let f: SetCursorFn = unsafe { std::mem::transmute(real(H_SET_CURSOR, SetCursor as *const () as usize)) };
    if OPEN.load(Ordering::Acquire) {
        return c;
    }
    unsafe { f(c) }
}

unsafe extern "system" fn hook_async_key(vk: i32) -> i16 {
    if OPEN.load(Ordering::Acquire) {
        return 0;
    }
    let f: KeyStateFn = unsafe { std::mem::transmute(real(H_ASYNC_KEY, GetAsyncKeyState as *const () as usize)) };
    unsafe { f(vk) }
}

unsafe extern "system" fn hook_key_state(vk: i32) -> i16 {
    if OPEN.load(Ordering::Acquire) {
        return 0;
    }
    let f: KeyStateFn = unsafe { std::mem::transmute(real(H_KEY_STATE, GetKeyState as *const () as usize)) };
    unsafe { f(vk) }
}

unsafe extern "system" fn hook_raw_buffer(data: *mut c_void, size: *mut u32, header: u32) -> u32 {
    let f: RawBufferFn = unsafe { std::mem::transmute(real(H_RAW_BUFFER, 0)) };
    if OPEN.load(Ordering::Acquire) {
        // Drain it so it doesn't pile up, but the game gets nothing.
        if !data.is_null() {
            unsafe { f(data, size, header) };
        } else if !size.is_null() {
            unsafe { *size = 0 };
        }
        return 0;
    }
    unsafe { f(data, size, header) }
}

unsafe extern "system" fn hook_raw_data(handle: *mut c_void, command: u32, data: *mut c_void, size: *mut u32, header: u32) -> u32 {
    let f: RawDataFn = unsafe { std::mem::transmute(real(H_RAW_DATA, 0)) };
    if OPEN.load(Ordering::Acquire) {
        if data.is_null() {
            if !size.is_null() {
                unsafe { *size = 0 };
            }
            return 0;
        }
        return u32::MAX;
    }
    unsafe { f(handle, command, data, size, header) }
}

/// Point the game modules' user32 imports at our hooks. Runs on every open,
/// so modules loaded later are covered too; already patched slots no longer
/// match the real address and are left alone.
fn patch_imports() {
    use windows::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleHandleExW, GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::ProcessStatus::EnumProcessModules;
    use windows::Win32::System::Threading::GetCurrentProcess;
    let _ = std::panic::catch_unwind(|| unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return };
        let hooks = hooks();
        let mut table = Vec::new();
        for (i, h) in hooks.iter().enumerate() {
            let Some(p) = GetProcAddress(user32, PCSTR(h.name.as_ptr())) else { continue };
            REAL[i].store(p as usize, Ordering::Release);
            table.push((p as usize, h.ours));
        }
        let mut me = HMODULE::default();
        let _ = GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, windows::core::PCWSTR(patch_imports as *const u16), &mut me);
        let mut modules = vec![HMODULE::default(); 1024];
        let mut needed = 0u32;
        if EnumProcessModules(GetCurrentProcess(), modules.as_mut_ptr(), (modules.len() * size_of::<HMODULE>()) as u32, &mut needed).is_err() {
            return;
        }
        modules.truncate((needed as usize / size_of::<HMODULE>()).min(modules.len()));
        let mut patched = 0;
        for m in modules {
            if m == me || m == user32 {
                continue;
            }
            patched += patch_module(m.0 as *mut u8, &table);
        }
        if patched > 0 {
            crate::log!("overlay: {patched} input imports now pass through the overlay");
        }
    });
}

/// Rewrite matching import slots of one loaded module. Returns how many.
unsafe fn patch_module(base: *mut u8, table: &[(usize, usize)]) -> usize {
    use windows::Win32::System::Memory::{PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualProtect};
    unsafe {
        let rd32 = |off: usize| (base.add(off) as *const u32).read_unaligned() as usize;
        if (base as *const u16).read_unaligned() != 0x5A4D {
            return 0;
        }
        let nt = rd32(0x3C);
        if rd32(nt) != 0x4550 {
            return 0;
        }
        let opt = nt + 24;
        let magic = (base.add(opt) as *const u16).read_unaligned();
        let dirs = match magic {
            0x10b => opt + 96,
            0x20b => opt + 112,
            _ => return 0,
        };
        // Our own pointer size must match the module's.
        if (magic == 0x20b) != cfg!(target_pointer_width = "64") {
            return 0;
        }
        let (import_rva, import_size) = (rd32(dirs + 8), rd32(dirs + 12));
        if import_rva == 0 || import_size == 0 {
            return 0;
        }
        let mut count = 0;
        let mut desc = import_rva;
        loop {
            let (name, first_thunk) = (rd32(desc + 12), rd32(desc + 16));
            if name == 0 && first_thunk == 0 {
                break;
            }
            if first_thunk != 0 {
                let mut slot = base.add(first_thunk) as *mut usize;
                while *slot != 0 {
                    if let Some(&(_, ours)) = table.iter().find(|(real, _)| *real == *slot) {
                        let mut old = PAGE_PROTECTION_FLAGS(0);
                        if VirtualProtect(slot as *const c_void, size_of::<usize>(), PAGE_READWRITE, &mut old).is_ok() {
                            *slot = ours;
                            let _ = VirtualProtect(slot as *const c_void, size_of::<usize>(), old, &mut old);
                            count += 1;
                        }
                    }
                    slot = slot.add(1);
                }
            }
            desc += 20;
        }
        count
    }
}

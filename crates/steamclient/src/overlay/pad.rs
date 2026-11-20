//! Controllers and the overlay.
//!
//! - While the overlay is open the game reads a neutral controller: XInput's
//!   `XInputGetState` (and the Guide-aware ordinal 100, `XInputGetStateEx`)
//!   report nothing pressed and `XInputGetKeystroke` reports no key, in every
//!   XInput DLL the game loaded (`hook.rs` patches them in place).
//! - The Guide (Xbox) button opens and closes the overlay, like Steam's
//!   overlay with an Xbox controller; B also closes it. We read the
//!   controller ourselves through the original functions.
//!
//! Not covered: DirectInput and Windows.Gaming.Input controllers.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, PCWSTR, s};

const DLLS: [&str; 6] = ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll", "xinput1_2.dll", "xinput1_1.dll", "xinputuap.dll"];
const ERROR_SUCCESS: u32 = 0;
const ERROR_EMPTY: u32 = 4306;
const GUIDE: u16 = 0x0400;
const BUTTON_B: u16 = 0x2000;

/// The original functions (trampolines), per XInput DLL slot.
static STATE: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];
static STATE_EX: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];
static KEYSTROKE: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];
/// Slots already looked at (hooked or not hookable).
static DONE: Mutex<[bool; 6]> = Mutex::new([false; 6]);

type StateFn = unsafe extern "system" fn(u32, *mut u8) -> u32;
type KeystrokeFn = unsafe extern "system" fn(u32, u32, *mut u8) -> u32;

fn open() -> bool {
    super::input::is_open()
}

/// XINPUT_STATE is a packet number then 12 bytes of buttons, triggers and
/// sticks: keep the packet number, zero the rest.
unsafe fn neutral(state: *mut u8) {
    if !state.is_null() {
        unsafe { std::ptr::write_bytes(state.add(4), 0, 12) };
    }
}

macro_rules! detours {
    ($($n:literal => $state:ident, $state_ex:ident, $keystroke:ident;)*) => {
        $(
            unsafe extern "system" fn $state(user: u32, out: *mut u8) -> u32 {
                let real: StateFn = unsafe { std::mem::transmute(STATE[$n].load(Ordering::Acquire)) };
                let r = unsafe { real(user, out) };
                if r == ERROR_SUCCESS && open() {
                    unsafe { neutral(out) };
                }
                r
            }
            unsafe extern "system" fn $state_ex(user: u32, out: *mut u8) -> u32 {
                let real: StateFn = unsafe { std::mem::transmute(STATE_EX[$n].load(Ordering::Acquire)) };
                let r = unsafe { real(user, out) };
                if r == ERROR_SUCCESS && open() {
                    unsafe { neutral(out) };
                }
                r
            }
            unsafe extern "system" fn $keystroke(user: u32, reserved: u32, out: *mut u8) -> u32 {
                let real: KeystrokeFn = unsafe { std::mem::transmute(KEYSTROKE[$n].load(Ordering::Acquire)) };
                if !open() {
                    return unsafe { real(user, reserved, out) };
                }
                // Drop what's queued so it doesn't reach the game later.
                let mut scratch = [0u8; 16];
                for _ in 0..64 {
                    if unsafe { real(user, reserved, scratch.as_mut_ptr()) } != ERROR_SUCCESS {
                        break;
                    }
                }
                ERROR_EMPTY
            }
        )*
        fn detours() -> [(usize, usize, usize); 6] {
            [$(($state as *const () as usize, $state_ex as *const () as usize, $keystroke as *const () as usize)),*]
        }
    };
}

detours! {
    0 => state0, state_ex0, keystroke0;
    1 => state1, state_ex1, keystroke1;
    2 => state2, state_ex2, keystroke2;
    3 => state3, state_ex3, keystroke3;
    4 => state4, state_ex4, keystroke4;
    5 => state5, state_ex5, keystroke5;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Hook the XInput DLLs the game has loaded (call again later for ones it
/// loads afterwards; each is only done once).
pub fn hook_loaded() {
    let Ok(mut done) = DONE.lock() else { return };
    for (i, dll) in DLLS.iter().enumerate() {
        if done[i] {
            continue;
        }
        let name = wide(dll);
        let Ok(module) = (unsafe { GetModuleHandleW(PCWSTR(name.as_ptr())) }) else { continue };
        done[i] = true;
        let (d_state, d_state_ex, d_keystroke) = detours()[i];
        let hook = |proc: Option<unsafe extern "system" fn() -> isize>, detour: usize, slot: &AtomicUsize, what: &str| {
            let Some(p) = proc else { return };
            // The original goes into its slot before the jump is written.
            if !unsafe { super::hook::install(p as *const c_void, detour, |t| slot.store(t, Ordering::Release)) } {
                crate::log!("overlay: can't hook {dll} {what}; controllers may still reach the game");
            }
        };
        unsafe {
            hook(GetProcAddress(module, s!("XInputGetState")), d_state, &STATE[i], "XInputGetState");
            hook(GetProcAddress(module, PCSTR(100 as *const u8)), d_state_ex, &STATE_EX[i], "XInputGetStateEx");
            hook(GetProcAddress(module, s!("XInputGetKeystroke")), d_keystroke, &KEYSTROKE[i], "XInputGetKeystroke");
        }
        crate::log!("overlay: controller input from {dll} pauses while the overlay is open");
    }
}

/// Reads controllers for the Guide button (the original XInput, not the game's view).
pub struct Guide {
    read: Option<StateFn>,
    last: [u16; 4],
    connected: [bool; 4],
    next_scan: std::time::Instant,
}

impl Guide {
    pub fn new() -> Self {
        Self { read: None, last: [0; 4], connected: [true; 4], next_scan: std::time::Instant::now() }
    }

    fn reader(&mut self) -> Option<StateFn> {
        if self.read.is_none() {
            // The game's own XInput DLL if it has one (through our trampoline,
            // which sees the real buttons), else xinput1_4 loaded for us.
            for (i, slot) in STATE_EX.iter().enumerate() {
                let t = slot.load(Ordering::Acquire);
                if t != 0 {
                    crate::log!("overlay: Guide button read through {}", DLLS[i]);
                    self.read = Some(unsafe { std::mem::transmute::<usize, StateFn>(t) });
                    break;
                }
            }
            if self.read.is_none() {
                let name = wide("xinput1_4.dll");
                if let Ok(m) = unsafe { LoadLibraryW(PCWSTR(name.as_ptr())) } {
                    if let Some(p) = unsafe { GetProcAddress(m, PCSTR(100 as *const u8)) } {
                        // If it's the game's DLL too, hook it first so our
                        // reads use the original, not the neutral view.
                        hook_loaded();
                        let t = STATE_EX[0].load(Ordering::Acquire);
                        self.read = Some(if t != 0 { unsafe { std::mem::transmute::<usize, StateFn>(t) } } else { unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, StateFn>(p) } });
                    }
                }
            }
        }
        self.read
    }

    /// `Some(true)`: Guide pressed (toggle). `Some(false)`: B pressed while
    /// open (close). Call every ~50 ms.
    pub fn poll(&mut self, open: bool) -> Option<bool> {
        let read = self.reader()?;
        let now = std::time::Instant::now();
        // Asking about unplugged controllers is slow: check them rarely.
        let rescan = now >= self.next_scan;
        if rescan {
            self.next_scan = now + std::time::Duration::from_secs(2);
        }
        let mut result = None;
        for pad in 0..4u32 {
            if !self.connected[pad as usize] && !rescan {
                continue;
            }
            let mut state = [0u8; 16];
            let ok = unsafe { read(pad, state.as_mut_ptr()) } == ERROR_SUCCESS;
            self.connected[pad as usize] = ok;
            let buttons = if ok { u16::from_le_bytes([state[4], state[5]]) } else { 0 };
            let pressed = buttons & !self.last[pad as usize];
            self.last[pad as usize] = buttons;
            if pressed & GUIDE != 0 {
                result = Some(true);
            } else if open && pressed & BUTTON_B != 0 && result.is_none() {
                result = Some(false);
            }
        }
        result
    }
}

//! A stand-in for Valve's `steamclient64.dll`.
//!
//! A game's `steam_api64.dll` doesn't talk to Steam itself: it loads the
//! steamclient DLL named in `HKCU\Software\Valve\Steam\ActiveProcess`
//! (`SteamClientDll64`), calls `CreateInterface("SteamClient0xx")` and then
//! uses C++ interfaces (ISteamUser, ISteamUserStats…) plus a few plain
//! exports for callbacks (`Steam_BGetCallback`, `Steam_GetAPICallResult`…).
//!
//! SteamInRust points that registry value at this DLL for games it starts
//! (games without anti-cheat only) and gives them `STEAMINRUST_IPC`, the pipe
//! and token to reach the app. This DLL answers the game from what the app
//! sends over that pipe.
//!
//! Loaded into any other process (no `STEAMINRUST_IPC`, e.g. a game the
//! official Steam client started while the registry pointed here), it
//! loads Valve's real steamclient from the Steam folder and forwards every
//! export to it, so such games (and their anti-cheat) talk to real Steam.

#![allow(clippy::missing_safety_doc)]

pub mod protocol;

/// Vtable slot of an interface method (for test tools that call the DLL
/// directly, like the probe example).
#[doc(hidden)]
pub fn slot_of(version: &str, method: &str) -> Option<usize> {
    vt::layout(version)?.1.iter().position(|m| *m == method)
}

mod extras;
mod ifaces;
mod input;
mod inventory;
mod ipc;
mod jingle;
mod lobby;
mod p2p;
mod pseudotcp;
mod screenshots;
mod servers;
mod sns;
mod state;
mod tables;
#[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
mod tables32;
mod ugc;
mod vt;
mod win;

use std::ffi::c_char;
use std::sync::OnceLock;

pub mod logfile {
    use std::io::Write;

    pub fn write(args: std::fmt::Arguments) {
        let Some(path) = std::env::var_os(crate::protocol::ENV_LOG) else { return };
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let _ = writeln!(f, "[{t} pid {}] {args}", std::process::id());
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($a:tt)*) => { $crate::logfile::write(format_args!($($a)*)) };
}

enum Mode {
    /// A game SteamInRust started: we answer.
    Ours,
    /// Anything else: Valve's steamclient (module handle, 0 if missing).
    Forward(usize),
}

fn mode() -> &'static Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    MODE.get_or_init(|| {
        if ipc::target().is_some() {
            log!("steamclient: answering for SteamInRust (app {})", state::env_app_id());
            Mode::Ours
        } else {
            Mode::Forward(win::load_real_steamclient())
        }
    })
}

type Raw8 = unsafe extern "C" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;

/// Each export: forward to Valve's DLL in forward mode, else run ours.
/// Exports take at most eight pointer-sized slots; extra ones are just
/// ignored by the callee (the caller cleans up, x64 and 32-bit cdecl alike).
macro_rules! exports {
    ($($name:ident => $ours:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize, g: usize, h: usize) -> usize {
            match mode() {
                Mode::Forward(module) => match win::proc_address(*module, stringify!($name)) {
                    0 => 0,
                    p => unsafe { std::mem::transmute::<usize, Raw8>(p)(a, b, c, d, e, f, g, h) },
                },
                Mode::Ours => {
                    let _ = (g, h);
                    let ours: fn(usize, usize, usize, usize, usize, usize) -> usize = $ours;
                    ours(a, b, c, d, e, f)
                }
            }
        }
    )*};
}

/// Steam_GetAPICallResult(pipe, call, buffer, size, expected callback,
/// *failed). Typed, because the 64-bit call handle takes two stack slots
/// on 32-bit and would shift the generic six-argument form.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_GetAPICallResult(pipe: i32, call: u64, buf: *mut u8, size: i32, expected: i32, failed: *mut bool) -> bool {
    type Real = unsafe extern "C" fn(i32, u64, *mut u8, i32, i32, *mut bool) -> bool;
    match mode() {
        Mode::Forward(module) => match win::proc_address(*module, "Steam_GetAPICallResult") {
            0 => false,
            p => unsafe { std::mem::transmute::<usize, Real>(p)(pipe, call, buf, size, expected, failed) },
        },
        Mode::Ours => ifaces::call_result(call, buf, size, expected, failed),
    }
}

/// CallbackMsg_t
#[repr(C)]
struct CallbackMsg {
    user: i32,
    callback: i32,
    param: *mut u8,
    size: i32,
}

fn create_interface(version: usize, err: usize) -> usize {
    let p = ifaces::interface(version as *const c_char);
    if p.is_null() && err != 0 {
        unsafe { (err as *mut i32).write_unaligned(1) };
    }
    if !p.is_null() {
        // Load the session now so GetAppID and friends are right from the start.
        state::ensure_session();
    }
    p as usize
}

fn get_callback(_pipe: usize, msg: usize) -> usize {
    let msg = msg as *mut CallbackMsg;
    if msg.is_null() {
        return 0;
    }
    // Networking callbacks the game set as functions run here, on its thread.
    sns::run_callbacks();
    state::run_game_thread_tasks();
    let mut g = state::lock();
    let Some((callback, data)) = g.queue.pop_front() else { return 0 };
    g.current = Some(data);
    let cur = g.current.as_mut().expect("just set");
    unsafe { msg.write_unaligned(CallbackMsg { user: 1, callback, param: cur.as_mut_ptr(), size: cur.len() as i32 }) };
    1
}

fn free_last_callback(_pipe: usize) -> usize {
    state::lock().current = None;
    1
}

fn known_interface(version: usize) -> usize {
    let v = version as *const c_char;
    if v.is_null() {
        return 0;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(v) }.to_string_lossy();
    let known = vt::layout(&name).is_some();
    if !known {
        log!("game asked about unknown interface {name}");
    }
    known as usize
}

fn connected() -> usize {
    state::ensure_session() as usize
}

exports! {
    CreateInterface => |a, b, _, _, _, _| create_interface(a, b);
    Steam_BGetCallback => |a, b, _, _, _, _| get_callback(a, b);
    Steam_FreeLastCallback => |a, _, _, _, _, _| free_last_callback(a);
    Steam_IsKnownInterface => |a, _, _, _, _, _| known_interface(a);
    Steam_NotifyMissingInterface => |_, b, _, _, _, _| { known_interface(b); 0 };
    Steam_CreateSteamPipe => |_, _, _, _, _, _| 1;
    Steam_BReleaseSteamPipe => |_, _, _, _, _, _| 1;
    Steam_ConnectToGlobalUser => |_, _, _, _, _, _| connected();
    Steam_CreateGlobalUser => |a, _, _, _, _, _| { if a != 0 { unsafe { (a as *mut i32).write_unaligned(1) } } connected() };
    Steam_CreateLocalUser => |a, _, _, _, _, _| { if a != 0 { unsafe { (a as *mut i32).write_unaligned(1) } } 1 };
    Steam_ReleaseUser => |_, _, _, _, _, _| 0;
    Steam_BConnected => |_, _, _, _, _, _| connected();
    Steam_BLoggedOn => |_, _, _, _, _, _| (connected() != 0 && state::lock().session.as_ref().is_some_and(|s| !s.offline)) as usize;
    Steam_ReleaseThreadLocalMemory => |_, _, _, _, _, _| 0;
    Steam_SetLocalIPBinding => |_, _, _, _, _, _| 0;
    Steam_LogOn => |_, _, _, _, _, _| 0;
    Steam_LogOff => |_, _, _, _, _, _| 0;
    Steam_InitiateGameConnection => |_, _, _, _, _, _| 0;
    Steam_TerminateGameConnection => |_, _, _, _, _, _| 0;
    Steam_GetGSHandle => |_, _, _, _, _, _| 0;
    Steam_GSBLoggedOn => |_, _, _, _, _, _| 0;
    Steam_GSBSecure => |_, _, _, _, _, _| 0;
    Steam_GSGetSteamID => |_, _, _, _, _, _| 0;
    Steam_GSLogOff => |_, _, _, _, _, _| 0;
    Steam_GSLogOn => |_, _, _, _, _, _| 0;
    Steam_GSRemoveUserConnect => |_, _, _, _, _, _| 0;
    Steam_GSSendSteam2UserConnect => |_, _, _, _, _, _| 0;
    Steam_GSSendSteam3UserConnect => |_, _, _, _, _, _| 0;
    Steam_GSSendUserDisconnect => |_, _, _, _, _, _| 0;
    Steam_GSSendUserStatusResponse => |_, _, _, _, _, _| 0;
    Steam_GSSetServerType => |_, _, _, _, _, _| 0;
    Steam_GSSetSpawnCount => |_, _, _, _, _, _| 0;
    Steam_GSUpdateStatus => |_, _, _, _, _, _| 0;
    Steam_GSGetSteam2GetEncryptionKeyToSendToNewClient => |_, _, _, _, _, _| 0;
    Breakpad_SteamMiniDumpInit => |_, _, _, _, _, _| 0;
    Breakpad_SteamSendMiniDump => |_, _, _, _, _, _| 0;
    Breakpad_SteamSetAppID => |_, _, _, _, _, _| 0;
    Breakpad_SteamSetSteamID => |_, _, _, _, _, _| 0;
    Breakpad_SteamWriteMiniDumpSetComment => |_, _, _, _, _, _| 0;
    Breakpad_SteamWriteMiniDumpUsingExceptionInfoWithBuildId => |_, _, _, _, _, _| 0;
}

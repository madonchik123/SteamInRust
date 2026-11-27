//! Steam Play on a Mac: the small "Steam" that runs inside Wine.
//!
//! SteamInRust starts `rundll32.exe C:\SteamInRust\steamclient64.dll,SteamInRustLaunch`
//! in the game's Wine prefix. Like Proton's `steam.exe`, this stays running
//! while the game does, because `steam_api` checks that the process named in
//! `HKCU\Software\Valve\Steam\ActiveProcess\pid` is alive. It writes that key
//! (its own process id, this DLL as `SteamClientDll64`, the signed-in
//! account), starts the game from `STEAMINRUST_LAUNCH_*` and waits for it.
//! The game inherits `STEAMINRUST_IPC` (a `tcp:` localhost address, see
//! ipc.rs) and loads this DLL through the registry, like on Windows.

#[cfg(windows)]
use std::ffi::c_void;

/// What SteamInRust passes (environment variables, so paths with spaces
/// need no quoting through Wine's command line).
pub const ENV_EXE: &str = "STEAMINRUST_LAUNCH_EXE";
pub const ENV_DIR: &str = "STEAMINRUST_LAUNCH_DIR";
pub const ENV_ARGS: &str = "STEAMINRUST_LAUNCH_ARGS";
pub const ENV_ACCOUNT: &str = "STEAMINRUST_ACCOUNT_ID";

/// The game's command line: the program in quotes, then its arguments.
pub fn command_line(exe: &str, args: &str) -> String {
    let args = args.trim();
    if args.is_empty() { format!("\"{exe}\"") } else { format!("\"{exe}\" {args}") }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct StartupInfoW {
        pub cb: u32,
        pub reserved: *mut u16,
        pub desktop: *mut u16,
        pub title: *mut u16,
        pub x: u32,
        pub y: u32,
        pub x_size: u32,
        pub y_size: u32,
        pub x_chars: u32,
        pub y_chars: u32,
        pub fill: u32,
        pub flags: u32,
        pub show: u16,
        pub reserved2_len: u16,
        pub reserved2: *mut u8,
        pub std_in: *mut c_void,
        pub std_out: *mut c_void,
        pub std_err: *mut c_void,
    }

    #[repr(C)]
    pub struct ProcessInformation {
        pub process: *mut c_void,
        pub thread: *mut c_void,
        pub pid: u32,
        pub tid: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *const c_void, ta: *const c_void, inherit: i32, flags: u32, env: *const c_void, cwd: *const u16, si: *const StartupInfoW, pi: *mut ProcessInformation) -> i32;
        pub fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
        pub fn GetExitCodeProcess(h: *mut c_void, code: *mut u32) -> i32;
        pub fn CloseHandle(h: *mut c_void) -> i32;
        pub fn GetCurrentProcessId() -> u32;
        pub fn GetLastError() -> u32;
        pub fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
        pub fn GetModuleFileNameW(module: *mut c_void, buf: *mut u16, size: u32) -> u32;
        pub fn ExitProcess(code: u32) -> !;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        pub fn RegSetKeyValueW(key: isize, sub: *const u16, value: *const u16, kind: u32, data: *const c_void, len: u32) -> i32;
    }

    pub const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    pub const REG_SZ: u32 = 1;
    pub const REG_DWORD: u32 = 4;
    pub const INFINITE: u32 = 0xFFFF_FFFF;
    pub const FROM_ADDRESS: u32 = 0x4;
    pub const UNCHANGED_REFCOUNT: u32 = 0x2;
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
const ACTIVE: &str = "Software\\Valve\\Steam\\ActiveProcess";

#[cfg(windows)]
fn set_dword(name: &str, v: u32) {
    let (sub, name) = (wide(ACTIVE), wide(name));
    unsafe { sys::RegSetKeyValueW(sys::HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), sys::REG_DWORD, (&v as *const u32).cast(), 4) };
}

#[cfg(windows)]
fn set_string(name: &str, v: &str) {
    let (sub, name, data) = (wide(ACTIVE), wide(name), wide(v));
    unsafe { sys::RegSetKeyValueW(sys::HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), sys::REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
}

/// This DLL's own path.
#[cfg(windows)]
fn own_path() -> Option<String> {
    let mut module: *mut c_void = std::ptr::null_mut();
    let here = own_path as *const u16;
    if unsafe { sys::GetModuleHandleExW(sys::FROM_ADDRESS | sys::UNCHANGED_REFCOUNT, here, &mut module) } == 0 {
        return None;
    }
    let mut buf = vec![0u16; 1024];
    let n = unsafe { sys::GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]))
}

/// Point `ActiveProcess` at us, start the game, wait. Returns its exit code.
#[cfg(windows)]
fn run() -> i32 {
    let Ok(exe) = std::env::var(ENV_EXE) else {
        crate::log!("launcher: {ENV_EXE} not set");
        return 2;
    };
    let dir = std::env::var(ENV_DIR).ok().filter(|d| !d.is_empty());
    let args = std::env::var(ENV_ARGS).unwrap_or_default();
    let account: u32 = std::env::var(ENV_ACCOUNT).ok().and_then(|a| a.parse().ok()).unwrap_or(0);
    let me = own_path().unwrap_or_default();
    let pid = unsafe { sys::GetCurrentProcessId() };
    set_dword("pid", pid);
    set_dword("ActiveUser", account);
    set_dword("Universe", 1);
    if cfg!(target_pointer_width = "64") {
        set_string("SteamClientDll64", &me);
        // The 32-bit one, for 32-bit games, if it was shipped next to us.
        let dll32 = me.rsplit_once('\\').map(|(d, _)| format!("{d}\\steamclient.dll")).unwrap_or_default();
        if std::path::Path::new(&dll32).is_file() {
            set_string("SteamClientDll", &dll32);
        }
    } else {
        set_string("SteamClientDll", &me);
    }
    crate::log!("launcher: pid {pid}, starting {exe} in {dir:?}");
    let (app, mut cmd) = (wide(&exe), wide(&command_line(&exe, &args)));
    let cwd = dir.as_deref().map(wide);
    let si = sys::StartupInfoW {
        cb: std::mem::size_of::<sys::StartupInfoW>() as u32,
        reserved: std::ptr::null_mut(),
        desktop: std::ptr::null_mut(),
        title: std::ptr::null_mut(),
        x: 0,
        y: 0,
        x_size: 0,
        y_size: 0,
        x_chars: 0,
        y_chars: 0,
        fill: 0,
        flags: 0,
        show: 0,
        reserved2_len: 0,
        reserved2: std::ptr::null_mut(),
        std_in: std::ptr::null_mut(),
        std_out: std::ptr::null_mut(),
        std_err: std::ptr::null_mut(),
    };
    let mut pi = sys::ProcessInformation { process: std::ptr::null_mut(), thread: std::ptr::null_mut(), pid: 0, tid: 0 };
    let ok = unsafe {
        sys::CreateProcessW(app.as_ptr(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, 0, std::ptr::null(), cwd.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()), &si, &mut pi)
    };
    if ok == 0 {
        crate::log!("launcher: couldn't start the game (error {})", unsafe { sys::GetLastError() });
        set_dword("pid", 0);
        return 3;
    }
    unsafe {
        sys::CloseHandle(pi.thread);
        sys::WaitForSingleObject(pi.process, sys::INFINITE);
    }
    let mut code = 0u32;
    unsafe {
        sys::GetExitCodeProcess(pi.process, &mut code);
        sys::CloseHandle(pi.process);
    }
    crate::log!("launcher: game exited with {code}");
    set_dword("pid", 0);
    code as i32
}

/// `rundll32 steamclient64.dll,SteamInRustLaunch` (Unicode form, which
/// rundll32 looks for first).
#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SteamInRustLaunchW(_hwnd: *mut c_void, _inst: *mut c_void, _cmd: *const u16, _show: i32) {
    finish(run());
}

/// The ANSI form, for a rundll32 that doesn't look for the `W` one.
#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SteamInRustLaunch(_hwnd: *mut c_void, _inst: *mut c_void, _cmd: *const u8, _show: i32) {
    finish(run());
}

/// End rundll32 with the game's exit code. `ExitProcess` rather than Rust's
/// `process::exit`: the C runtime's exit handlers in a rundll32 that has no
/// console crashed under Wine Stable (0xc0000417) once the game had closed.
#[cfg(windows)]
fn finish(code: i32) -> ! {
    unsafe { sys::ExitProcess(code as u32) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn quotes_the_program() {
        assert_eq!(super::command_line(r"Z:\Users\me\My Game\game.exe", ""), r#""Z:\Users\me\My Game\game.exe""#);
        assert_eq!(super::command_line(r"Z:\g.exe", " -windowed +x "), r#""Z:\g.exe" -windowed +x"#);
    }
}

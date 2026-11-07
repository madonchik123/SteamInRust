//! The few Win32 calls the DLL needs, declared by hand (no extra crates).

#![allow(clippy::upper_case_acronyms)]

use std::ffi::c_void;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

#[cfg(windows)]
#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
}

#[cfg(windows)]
#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(hwnd: *mut c_void, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> *mut c_void;
}

const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const RRF_RT_REG_SZ: u32 = 0x2;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A string value under HKEY_CURRENT_USER.
#[cfg(windows)]
pub fn reg_string(sub: &str, value: &str) -> Option<String> {
    let (sub, value) = (wide(sub), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
    if r != 0 {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

#[cfg(not(windows))]
pub fn reg_string(_sub: &str, _value: &str) -> Option<String> {
    None
}

/// Load Valve's own steamclient from the Steam folder (for processes we
/// didn't start, e.g. games the official Steam client runs).
#[cfg(windows)]
pub fn load_real_steamclient() -> usize {
    let Some(root) = reg_string("Software\\Valve\\Steam", "SteamPath") else { return 0 };
    let file = if cfg!(target_pointer_width = "64") { "steamclient64.dll" } else { "steamclient.dll" };
    let path = format!("{}\\{}", root.replace('/', "\\"), file);
    let w = wide(&path);
    unsafe { LoadLibraryW(w.as_ptr()) as usize }
}

#[cfg(not(windows))]
pub fn load_real_steamclient() -> usize {
    0
}

#[cfg(windows)]
pub fn proc_address(module: usize, name: &str) -> usize {
    if module == 0 {
        return 0;
    }
    let mut n = name.as_bytes().to_vec();
    n.push(0);
    unsafe { GetProcAddress(module as *mut c_void, n.as_ptr()) as usize }
}

#[cfg(not(windows))]
pub fn proc_address(_module: usize, _name: &str) -> usize {
    0
}

/// Open a web page in the default browser (stands in for the Steam overlay).
pub fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(windows)]
    {
        let (op, file) = (wide("open"), wide(url));
        unsafe {
            ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
        }
    }
}

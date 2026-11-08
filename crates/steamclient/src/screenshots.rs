//! ISteamScreenshots: screenshots go into the player's Steam screenshot
//! library on this PC, where Steam keeps them
//! (`userdata/<account>/760/remote/<app>/screenshots/`, with thumbnails),
//! so Steam's client shows them and can upload them later.
//!
//! TriggerScreenshot asks the game (ScreenshotRequested) when it hooked
//! screenshots, else captures the game's window from the screen. Uploading
//! to the Steam Cloud screenshot library is left to Steam's client.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, slice, text};
use crate::state::{Bytes, lock};
use crate::vt::Fp;
use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const READY: i32 = 2301;
const REQUESTED: i32 = 2302;
const OK: i32 = 1;
const FAIL: i32 = 2;
const INVALID: u32 = 0;
const THUMB_WIDTH: u32 = 200;

static HOOKED: AtomicBool = AtomicBool::new(false);
static NEXT: AtomicU32 = AtomicU32::new(1);

pub fn imp(name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match name {
        "WriteScreenshot" => f!(write),
        "AddScreenshotToLibrary" => f!(add),
        "AddVRScreenshotToLibrary" => f!(add_vr),
        "TriggerScreenshot" => f!(trigger),
        "HookScreenshots" => f!(hook),
        "IsScreenshotsHooked" => f!(hooked),
        "SetLocation" | "TagUser" | "TagPublishedFile" => f!(tag),
        _ => None,
    }
}

/// `<Steam>/userdata/<account>/760/remote/<app>/screenshots`.
fn folder() -> Option<PathBuf> {
    let g = lock();
    let s = g.session.as_ref()?;
    if s.steam_root.is_empty() {
        return None;
    }
    let account = (s.steam_id & 0xFFFF_FFFF) as u32;
    Some(PathBuf::from(&s.steam_root).join("userdata").join(account.to_string()).join("760").join("remote").join(s.app_id.to_string()).join("screenshots"))
}

/// A new file name like Steam's: `YYYYMMDDHHMMSS_N.jpg`.
fn file_name(dir: &Path) -> PathBuf {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil date from days since 1970 (Howard Hinnant's algorithm), UTC.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    let stamp = format!("{y:04}{m:02}{d:02}{:02}{:02}{:02}", rem / 3600, rem / 60 % 60, rem % 60);
    (1..).map(|n| dir.join(format!("{stamp}_{n}.jpg"))).find(|p| !p.exists()).unwrap_or_else(|| dir.join(format!("{stamp}.jpg")))
}

/// Save an RGB image (and its thumbnail) into the library.
fn save_rgb(rgb: Vec<u8>, width: u32, height: u32) -> Result<PathBuf, String> {
    save_in(&folder().ok_or("no Steam folder for screenshots")?, rgb, width, height)
}

fn save_in(dir: &Path, rgb: Vec<u8>, width: u32, height: u32) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir.join("thumbnails")).map_err(|e| e.to_string())?;
    let img = image::RgbImage::from_raw(width, height, rgb).ok_or("bad image size")?;
    let path = file_name(dir);
    img.save_with_format(&path, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
    let th = THUMB_WIDTH.min(width).max(1);
    let thumb = image::imageops::thumbnail(&img, th, (height as u64 * th as u64 / width.max(1) as u64).max(1) as u32);
    let _ = thumb.save_with_format(dir.join("thumbnails").join(path.file_name().unwrap_or_default()), image::ImageFormat::Jpeg);
    Ok(path)
}

/// ScreenshotReady_t: handle, EResult.
fn ready(handle: u32, result: i32) {
    let mut b = Bytes::new(8);
    b.u32(0, handle).i32(4, result);
    lock().post(READY, b.0);
}

fn finish(r: Result<PathBuf, String>) -> u32 {
    let h = NEXT.fetch_add(1, Ordering::Relaxed);
    match r {
        Ok(p) => {
            crate::log!("screenshot saved: {}", p.display());
            ready(h, OK);
            h
        }
        Err(e) => {
            crate::log!("screenshot failed: {e}");
            ready(h, FAIL);
            INVALID
        }
    }
}

extern "C" fn write(_t: This, rgb: *const u8, size: u32, width: i32, height: i32) -> u32 {
    let (w, h) = (width.max(0) as u32, height.max(0) as u32);
    if rgb.is_null() || w == 0 || h == 0 || (size as u64) < w as u64 * h as u64 * 3 {
        return finish(Err("bad image".into()));
    }
    let data = slice(rgb, (w * h * 3) as usize).to_vec();
    finish(save_rgb(data, w, h))
}

/// Copy a file the game wrote (JPEG, or anything `image` reads) into the library.
fn add_file(file: &str) -> Result<PathBuf, String> {
    let img = image::open(file).map_err(|e| e.to_string())?.to_rgb8();
    let (w, h) = img.dimensions();
    save_rgb(img.into_raw(), w, h)
}

extern "C" fn add(_t: This, file: *const c_char, _thumbnail: *const c_char, _w: i32, _h: i32) -> u32 {
    let f = unsafe { text(file) };
    finish(add_file(&f))
}

extern "C" fn add_vr(_t: This, _kind: i32, file: *const c_char, _vr_file: *const c_char) -> u32 {
    let f = unsafe { text(file) };
    finish(add_file(&f))
}

extern "C" fn trigger(_t: This) {
    if HOOKED.load(Ordering::Relaxed) {
        lock().post(REQUESTED, vec![0]);
        return;
    }
    std::thread::spawn(|| {
        let r = capture::game_window().ok_or_else(|| "couldn't capture the game window".to_string()).and_then(|(rgb, w, h)| save_rgb(rgb, w, h));
        finish(r);
    });
}

extern "C" fn hook(_t: This, on: bool) {
    HOOKED.store(on, Ordering::Relaxed);
}

extern "C" fn hooked(_t: This) -> bool {
    HOOKED.load(Ordering::Relaxed)
}

/// Locations and tags only matter for the online library.
extern "C" fn tag(_t: This, handle: u32) -> bool {
    handle != INVALID
}

#[cfg(windows)]
mod capture {
    use std::ffi::c_void;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(f: extern "system" fn(*mut c_void, isize) -> i32, param: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
        fn GetWindowRect(hwnd: *mut c_void, rect: *mut [i32; 4]) -> i32;
        fn GetDC(hwnd: *mut c_void) -> *mut c_void;
        fn ReleaseDC(hwnd: *mut c_void, dc: *mut c_void) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
        fn CreateCompatibleBitmap(dc: *mut c_void, w: i32, h: i32) -> *mut c_void;
        fn SelectObject(dc: *mut c_void, obj: *mut c_void) -> *mut c_void;
        fn BitBlt(dst: *mut c_void, x: i32, y: i32, w: i32, h: i32, src: *mut c_void, sx: i32, sy: i32, rop: u32) -> i32;
        fn GetDIBits(dc: *mut c_void, bmp: *mut c_void, start: u32, lines: u32, bits: *mut c_void, info: *mut u8, usage: u32) -> i32;
        fn DeleteObject(obj: *mut c_void) -> i32;
        fn DeleteDC(dc: *mut c_void) -> i32;
    }

    /// The biggest visible window of this process.
    fn window() -> Option<(*mut c_void, [i32; 4])> {
        struct Best(*mut c_void, [i32; 4], i64);
        extern "system" fn each(hwnd: *mut c_void, param: isize) -> i32 {
            let best = unsafe { &mut *(param as *mut Best) };
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            if pid == std::process::id() && unsafe { IsWindowVisible(hwnd) } != 0 {
                let mut r = [0i32; 4];
                unsafe { GetWindowRect(hwnd, &mut r) };
                let area = (r[2] - r[0]) as i64 * (r[3] - r[1]) as i64;
                if area > best.2 {
                    *best = Best(hwnd, r, area);
                }
            }
            1
        }
        let mut best = Best(std::ptr::null_mut(), [0; 4], 0);
        unsafe { EnumWindows(each, &mut best as *mut Best as isize) };
        (!best.0.is_null()).then_some((best.0, best.1))
    }

    /// The game window as it is on screen: (RGB, width, height).
    pub fn game_window() -> Option<(Vec<u8>, u32, u32)> {
        let (_, r) = window()?;
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        if w <= 0 || h <= 0 {
            return None;
        }
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp);
            // SRCCOPY | CAPTUREBLT
            let ok = BitBlt(mem, 0, 0, w, h, screen, r[0], r[1], 0x00CC_0020 | 0x4000_0000) != 0;
            // BITMAPINFOHEADER: 32-bit top-down BGRA.
            let mut info = [0u8; 44];
            info[0..4].copy_from_slice(&40u32.to_le_bytes());
            info[4..8].copy_from_slice(&w.to_le_bytes());
            info[8..12].copy_from_slice(&(-h).to_le_bytes());
            info[12..14].copy_from_slice(&1u16.to_le_bytes());
            info[14..16].copy_from_slice(&32u16.to_le_bytes());
            let mut bgra = vec![0u8; (w * h * 4) as usize];
            let lines = if ok { GetDIBits(mem, bmp, 0, h as u32, bgra.as_mut_ptr().cast(), info.as_mut_ptr(), 0) } else { 0 };
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            if lines <= 0 {
                return None;
            }
            let rgb = bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
            Some((rgb, w as u32, h as u32))
        }
    }
}

#[cfg(not(windows))]
mod capture {
    pub fn game_window() -> Option<(Vec<u8>, u32, u32)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_like_steam() {
        let dir = std::env::temp_dir().join("sir-shots-test");
        let n = file_name(&dir);
        let name = n.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("_1.jpg") && name.len() == "20261108142500_1.jpg".len(), "{name}");
        assert!(name.starts_with("20"));
        let _ = std::fs::remove_dir_all(&dir);
        let p = save_in(&dir, vec![200; 640 * 360 * 3], 640, 360).unwrap();
        assert_eq!(image::open(&p).unwrap().width(), 640);
        let thumb = image::open(dir.join("thumbnails").join(p.file_name().unwrap())).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (200, 112));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

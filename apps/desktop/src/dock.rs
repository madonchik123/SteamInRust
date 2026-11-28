//! The Dock on macOS: whether SteamInRust shows there (Settings → Appearance
//! → "Show in the Dock"; off keeps it in the menu bar only) and the picture
//! it shows while running. Nothing happens on other systems.

use std::sync::atomic::{AtomicBool, Ordering};

/// "Show in the Dock" is off.
static HIDDEN: AtomicBool = AtomicBool::new(false);

/// Is SteamInRust kept out of the Dock? (The overlay asks on a Mac.)
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn hidden() -> bool {
    HIDDEN.load(Ordering::Relaxed)
}

/// Show SteamInRust in the Dock or keep it in the menu bar only.
pub fn set_hidden(hide: bool) {
    HIDDEN.store(hide, Ordering::Relaxed);
    #[cfg(target_os = "macos")]
    mac::policy(hide);
}

/// Without a Dock icon, macOS doesn't bring the app forward by itself when
/// its window is shown from the menu bar; this does.
pub fn activate() {
    #[cfg(target_os = "macos")]
    if hidden() {
        mac::activate();
    }
}

/// The Dock's picture while running: the logo picked in Settings on a Mac
/// tile, or the app's own icon (`None`).
#[cfg(target_os = "macos")]
pub fn set_icon(logo: Option<ui::brand::Logo>) {
    let png = logo.and_then(|l| {
        let size = 512;
        let img = image::RgbaImage::from_raw(size, size, ui::brand::mac_tile(l, size))?;
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some(out.into_inner())
    });
    mac::icon(png.as_deref());
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    fn app() -> Option<*mut AnyObject> {
        let cls = AnyClass::get(c"NSApplication")?;
        let app: *mut AnyObject = unsafe { msg_send![cls, sharedApplication] };
        (!app.is_null()).then_some(app)
    }

    /// NSApplicationActivationPolicyRegular = 0, Accessory = 1 (no Dock icon).
    pub fn policy(hide: bool) {
        let Some(app) = app() else { return };
        unsafe {
            let _: bool = msg_send![app, setActivationPolicy: if hide { 1isize } else { 0isize }];
        }
    }

    pub fn activate() {
        let Some(app) = app() else { return };
        unsafe {
            let _: () = msg_send![app, activateIgnoringOtherApps: true];
        }
    }

    /// `png` as the Dock icon, or the bundle's own icon for `None`.
    pub fn icon(png: Option<&[u8]>) {
        let Some(app) = app() else { return };
        unsafe {
            let image: *mut AnyObject = match png {
                Some(bytes) => {
                    let (Some(data_cls), Some(image_cls)) = (AnyClass::get(c"NSData"), AnyClass::get(c"NSImage")) else { return };
                    let data: *mut AnyObject = msg_send![data_cls, dataWithBytes: bytes.as_ptr().cast::<std::ffi::c_void>(), length: bytes.len()];
                    if data.is_null() {
                        return;
                    }
                    let alloc: *mut AnyObject = msg_send![image_cls, alloc];
                    msg_send![alloc, initWithData: data]
                }
                None => std::ptr::null_mut(),
            };
            let _: () = msg_send![app, setApplicationIconImage: image];
        }
    }
}

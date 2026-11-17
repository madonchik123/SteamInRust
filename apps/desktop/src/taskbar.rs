//! The running download's progress on the taskbar button (Windows) or a
//! percentage on the Dock icon (macOS), like Steam.

/// What the taskbar shows: nothing, or progress (0..1) and whether it's paused.
pub type Shown = Option<(f32, bool)>;

#[derive(Default)]
pub struct Taskbar {
    /// What was last sent, in tenths of a percent, and when.
    last: Option<(Option<(u16, bool)>, std::time::Instant)>,
    #[cfg(windows)]
    list: Option<windows::Win32::UI::Shell::ITaskbarList3>,
}

impl Taskbar {
    pub fn show(&mut self, frame: &eframe::Frame, shown: Shown) {
        let key = shown.map(|(f, paused)| ((f.clamp(0.0, 1.0) * 1000.0) as u16, paused));
        // Sent on change, and again now and then while showing progress:
        // Windows drops it if the taskbar button wasn't there yet (at start,
        // or after Explorer restarts).
        if let Some((last, at)) = &self.last
            && *last == key
            && (key.is_none() || at.elapsed() < std::time::Duration::from_secs(3))
        {
            return;
        }
        if self.apply(frame, key) {
            self.last = Some((key, std::time::Instant::now()));
        }
    }

    #[cfg(windows)]
    fn apply(&mut self, frame: &eframe::Frame, key: Option<(u16, bool)>) -> bool {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
        use windows::Win32::UI::Shell::{ITaskbarList3, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED, TaskbarList};
        let handle = match frame.window_handle() {
            Ok(h) => h,
            Err(e) => {
                tracing::debug!("taskbar: no window handle: {e}");
                return false;
            }
        };
        let RawWindowHandle::Win32(h) = handle.as_raw() else { return false };
        let hwnd = HWND(h.hwnd.get() as *mut _);
        if self.list.is_none() {
            // winit has usually set COM up on this thread already; asking
            // again is harmless.
            let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            let list = match unsafe { CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER) } {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!("taskbar progress unavailable: {e}");
                    return false;
                }
            };
            if let Err(e) = unsafe { list.HrInit() } {
                tracing::warn!("taskbar progress unavailable: {e}");
                return false;
            }
            self.list = Some(list);
        }
        let Some(list) = &self.list else { return false };
        unsafe {
            match key {
                None => list.SetProgressState(hwnd, TBPF_NOPROGRESS).is_ok(),
                Some((permille, paused)) => {
                    let _ = list.SetProgressState(hwnd, if paused { TBPF_PAUSED } else { TBPF_NORMAL });
                    let r = list.SetProgressValue(hwnd, permille as u64, 1000);
                    if let Err(e) = &r {
                        tracing::debug!("taskbar progress: {e}");
                    }
                    r.is_ok()
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn apply(&mut self, _frame: &eframe::Frame, key: Option<(u16, bool)>) -> bool {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};
        let label = key.map(|(permille, paused)| if paused { format!("{}% ❚❚", permille / 10) } else { format!("{}%", permille / 10) });
        unsafe {
            let Some(app_class) = AnyClass::get(c"NSApplication") else { return false };
            let app: *mut AnyObject = msg_send![app_class, sharedApplication];
            if app.is_null() {
                return false;
            }
            let tile: *mut AnyObject = msg_send![app, dockTile];
            if tile.is_null() {
                return false;
            }
            let text: *mut AnyObject = match label {
                Some(l) => {
                    let Some(ns) = AnyClass::get(c"NSString") else { return false };
                    let c = std::ffi::CString::new(l).unwrap_or_default();
                    msg_send![ns, stringWithUTF8String: c.as_ptr()]
                }
                None => std::ptr::null_mut(),
            };
            let _: () = msg_send![tile, setBadgeLabel: text];
        }
        true
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn apply(&mut self, _frame: &eframe::Frame, _key: Option<(u16, bool)>) -> bool {
        true
    }
}

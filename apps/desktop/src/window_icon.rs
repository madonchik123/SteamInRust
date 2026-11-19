//! Sharp app icons on Windows. One big picture that Windows shrinks itself
//! looks blurry on the taskbar and in the tray, so the logo is drawn at
//! exactly the sizes Windows shows it at for the display's scaling.

/// The main window's icon (title bar, taskbar, Alt+Tab): the logo picked
/// in Settings → Appearance, like the tray.
#[derive(Default)]
pub struct WindowIcon {
    /// The window's DPI and the logo the icons were drawn for.
    #[cfg(windows)]
    drawn: Option<(u32, ui::brand::Logo)>,
    /// Icons we made and handed to the window (freed when replaced; the
    /// last ones live as long as the window).
    #[cfg(windows)]
    made: Vec<windows_sys::Win32::UI::WindowsAndMessaging::HICON>,
}

impl WindowIcon {
    /// Hand the icons to the window again on the next frame.
    pub fn refresh(&mut self) {
        #[cfg(windows)]
        {
            self.drawn = None;
        }
    }

    /// Draws the icons for the window's scaling and `logo`, and again when
    /// either changes (moved to another display, another logo picked).
    /// Elsewhere the system takes the app's icon from its bundle.
    #[cfg(not(windows))]
    pub fn show(&mut self, _frame: &eframe::Frame, _logo: ui::brand::Logo) {}

    #[cfg(windows)]
    pub fn show(&mut self, frame: &eframe::Frame, logo: ui::brand::Logo) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
        use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, ICON_BIG, ICON_SMALL, SM_CXICON, SM_CXSMICON, SendMessageW, WM_SETICON};
        let Ok(handle) = frame.window_handle() else { return };
        let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
        let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        if self.drawn == Some((dpi, logo)) {
            return;
        }
        let mut made = Vec::new();
        for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = unsafe { GetSystemMetricsForDpi(metric, dpi) }.clamp(16, 256) as u32;
            let Some(icon) = make_icon(&logo.rgba(size), size) else { return };
            unsafe { SendMessageW(hwnd, WM_SETICON, kind as usize, icon as isize) };
            made.push(icon);
        }
        for old in std::mem::replace(&mut self.made, made) {
            unsafe { DestroyIcon(old) };
        }
        self.drawn = Some((dpi, logo));
    }
}

/// A window icon of `logo` at `size`, drawn once and kept (viewport
/// builders are made every frame).
pub fn icon_data(logo: ui::brand::Logo, size: u32) -> std::sync::Arc<egui::IconData> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    static CACHE: Mutex<Option<HashMap<(ui::brand::Logo, u32), Arc<egui::IconData>>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.get_or_insert_with(HashMap::new).entry((logo, size)).or_insert_with(|| Arc::new(egui::IconData { rgba: logo.rgba(size), width: size, height: size })).clone()
}

/// Size to draw the tray icon at: the small-icon size Windows uses for the
/// main display's scaling (32 elsewhere, scaled down by the system).
pub fn tray_size() -> u32 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
        use windows_sys::Win32::UI::WindowsAndMessaging::SM_CXSMICON;
        unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem().max(96)) }.clamp(16, 64) as u32
    }
    #[cfg(not(windows))]
    32
}

/// A `size` × `size` Windows icon from straight RGBA, with see-through edges.
#[cfg(windows)]
fn make_icon(rgba: &[u8], size: u32) -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON> {
    use windows_sys::Win32::UI::WindowsAndMessaging::CreateIcon;
    // RGBA → BGRA, which is what a 32-bit icon holds; the alpha does the
    // masking, so the 1-bit mask (rows padded to 16 bits) stays empty.
    let mut bgra = rgba.to_vec();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let mask = vec![0u8; size.div_ceil(16) as usize * 2 * size as usize];
    let icon = unsafe { CreateIcon(std::ptr::null_mut(), size as i32, size as i32, 1, 32, mask.as_ptr(), bgra.as_ptr()) };
    (!icon.is_null()).then_some(icon)
}

//! `steam://` links, single-instance forwarding, and desktop shortcuts.
//!
//! - SteamInRust can register itself as the handler for `steam://` links
//!   (Settings → Interface). The previous handler is saved so it can be
//!   restored with one click.
//! - Only one copy runs: a second launch (e.g. from a link or shortcut)
//!   forwards its link to the running window over localhost and exits.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;

const PORT: u16 = 47_630;

/// What a `steam://` link asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Run(u32),
    Install(u32),
    Uninstall(u32),
    Validate(u32),
    Store(u32),
    Page(&'static str),
    Show,
}

pub fn parse(url: &str) -> Option<Link> {
    let rest = url.trim().trim_matches('"');
    let rest = rest.strip_prefix("steam://")?.trim_end_matches('/');
    let mut parts = rest.split('/');
    let verb = parts.next()?.to_lowercase();
    let id = || parts.clone().next().and_then(|p| p.parse::<u32>().ok());
    Some(match verb.as_str() {
        "rungameid" | "run" | "launch" => Link::Run(id()?),
        "install" => Link::Install(id()?),
        "uninstall" => Link::Uninstall(id()?),
        "validate" => Link::Validate(id()?),
        "store" | "advertise" => Link::Store(id()?),
        "nav" | "open" => match parts.next().unwrap_or_default() {
            "downloads" => Link::Page("downloads"),
            "friends" => Link::Page("community"),
            "settings" => Link::Page("settings"),
            "games" | "library" => Link::Page("library"),
            _ => Link::Show,
        },
        "friends" => Link::Page("community"),
        _ => Link::Show,
    })
}

/// Try to hand `arg` to an already-running copy. Returns true if it did.
pub fn forward_to_running(arg: &str) -> bool {
    match TcpStream::connect(("127.0.0.1", PORT)) {
        Ok(mut s) => s.write_all(format!("{arg}\n").as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// Listen for links from later launches. Each line received is a link
/// (or "show" to just bring the window up).
pub fn listen(ctx: egui::Context) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    if let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)) {
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok() {
                    let _ = tx.send(line.trim().to_string());
                    ctx.request_repaint();
                }
            }
        });
    }
    rx
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
}

fn config_backup() -> PathBuf {
    client::store::config_dir().join("steam-protocol-backup.txt")
}

#[cfg(windows)]
fn reg(args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(windows)]
const KEY: &str = r"HKCU\Software\Classes\steam\shell\open\command";

/// The command currently registered for `steam://` links, if any.
#[cfg(windows)]
pub fn current_handler() -> Option<String> {
    let out = reg(&["query", KEY, "/ve"])?;
    let line = out.lines().find(|l| l.contains("REG_SZ"))?;
    Some(line.split("REG_SZ").nth(1)?.trim().to_string())
}

#[cfg(windows)]
pub fn is_default_handler() -> bool {
    current_handler().is_some_and(|h| h.to_lowercase().contains(&exe().to_lowercase()))
}

/// Make SteamInRust open `steam://` links (saves the old handler first).
#[cfg(windows)]
pub fn register() -> Result<(), String> {
    if let Some(old) = current_handler().filter(|h| !h.to_lowercase().contains(&exe().to_lowercase())) {
        let _ = std::fs::create_dir_all(client::store::config_dir());
        let _ = std::fs::write(config_backup(), old);
    }
    let cmd = format!("\"{}\" \"%1\"", exe());
    let ok = reg(&["add", r"HKCU\Software\Classes\steam", "/ve", "/d", "URL:steam protocol", "/f"]).is_some()
        && reg(&["add", r"HKCU\Software\Classes\steam", "/v", "URL Protocol", "/d", "", "/f"]).is_some()
        && reg(&["add", KEY, "/ve", "/d", &cmd, "/f"]).is_some();
    if ok { Ok(()) } else { Err("Windows refused the change".into()) }
}

/// Give `steam://` links back to whatever handled them before.
#[cfg(windows)]
pub fn unregister() -> Result<(), String> {
    match std::fs::read_to_string(config_backup()) {
        Ok(old) if !old.trim().is_empty() => {
            reg(&["add", KEY, "/ve", "/d", old.trim(), "/f"]).ok_or("Windows refused the change")?;
        }
        _ => {
            // Nothing saved: point back at the official client if it's installed.
            if let Some(root) = local::find_steam_root() {
                let cmd = format!("\"{}\" -- \"%1\"", root.join("steam.exe").display());
                reg(&["add", KEY, "/ve", "/d", &cmd, "/f"]).ok_or("Windows refused the change")?;
            }
        }
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn is_default_handler() -> bool {
    false
}
#[cfg(not(any(windows, target_os = "macos")))]
pub fn register() -> Result<(), String> {
    Err("Only supported on Windows and macOS for now".into())
}
#[cfg(not(any(windows, target_os = "macos")))]
pub fn unregister() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub use mac::{install_url_handler, is_default_handler, register, unregister};

/// macOS: links arrive as an Apple Event (not on the command line), and
/// Launch Services decides which app opens `steam://`.
#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, NSObject};
    use objc2::{ClassType, define_class, msg_send, sel};
    use std::ffi::{CStr, c_char, c_void};

    /// Our bundle id (see xtask's Info.plist).
    const BUNDLE_ID: &str = "io.github.madonchik123.steaminrust";
    /// The official client's, to give links back to.
    const STEAM_ID: &str = "com.valvesoftware.steam";
    const GURL: u32 = u32::from_be_bytes(*b"GURL");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "SteamInRustURLHandler"]
        struct UrlHandler;

        impl UrlHandler {
            #[unsafe(method(handleURL:withReply:))]
            fn handle_url(&self, event: &AnyObject, _reply: &AnyObject) {
                let url = unsafe {
                    let d: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
                    let s: *mut AnyObject = if d.is_null() { std::ptr::null_mut() } else { msg_send![d, stringValue] };
                    let p: *const c_char = if s.is_null() { std::ptr::null() } else { msg_send![s, UTF8String] };
                    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
                };
                if let Some(url) = url {
                    deliver(url);
                }
            }
        }
    );

    /// Hand the link to the window (which may still be starting).
    fn deliver(url: String) {
        std::thread::spawn(move || {
            for _ in 0..100 {
                if super::forward_to_running(&url) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
    }

    /// Receive `steam://` links macOS sends to the app. Call before the
    /// window opens, so the link that started the app isn't missed.
    pub fn install_url_handler() {
        unsafe {
            let handler: Retained<UrlHandler> = msg_send![UrlHandler::class(), new];
            let Some(cls) = AnyClass::get(c"NSAppleEventManager") else { return };
            let manager: *mut AnyObject = msg_send![cls, sharedAppleEventManager];
            if manager.is_null() {
                return;
            }
            let _: () = msg_send![manager, setEventHandler: &*handler, andSelector: sel!(handleURL:withReply:), forEventClass: GURL, andEventID: GURL];
            // Lives as long as the app.
            std::mem::forget(handler);
        }
    }

    type CFRef = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(alloc: CFRef, bytes: *const u8, len: isize, encoding: u32, external: u8) -> CFRef;
        fn CFStringGetCString(s: CFRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(cf: CFRef);
    }
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSSetDefaultHandlerForURLScheme(scheme: CFRef, bundle: CFRef) -> i32;
        fn LSCopyDefaultHandlerForURLScheme(scheme: CFRef) -> CFRef;
    }
    const UTF8: u32 = 0x0800_0100;

    fn cf(s: &str) -> CFRef {
        unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) }
    }

    fn current() -> Option<String> {
        unsafe {
            let scheme = cf("steam");
            let h = LSCopyDefaultHandlerForURLScheme(scheme);
            CFRelease(scheme);
            if h.is_null() {
                return None;
            }
            let mut buf = [0 as c_char; 512];
            let ok = CFStringGetCString(h, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0;
            CFRelease(h);
            ok.then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    fn set(bundle: &str) -> Result<(), String> {
        let r = unsafe {
            let (scheme, id) = (cf("steam"), cf(bundle));
            let r = LSSetDefaultHandlerForURLScheme(scheme, id);
            CFRelease(scheme);
            CFRelease(id);
            r
        };
        if r == 0 { Ok(()) } else { Err(format!("macOS refused the change ({r})")) }
    }

    pub fn is_default_handler() -> bool {
        current().is_some_and(|h| h.eq_ignore_ascii_case(BUNDLE_ID))
    }

    /// Make SteamInRust open `steam://` links. macOS only allows an app
    /// from its bundle (the copy in Applications), not a bare build.
    pub fn register() -> Result<(), String> {
        let in_bundle = std::env::current_exe().is_ok_and(|p| p.components().any(|c| c.as_os_str().to_string_lossy().ends_with(".app")));
        if !in_bundle {
            return Err(i18n::tr_dyn("Open SteamInRust from its app (SteamInRust.app) to handle steam:// links."));
        }
        if let Some(old) = current().filter(|h| !h.eq_ignore_ascii_case(BUNDLE_ID)) {
            let _ = std::fs::create_dir_all(client::store::config_dir());
            let _ = std::fs::write(super::config_backup(), old);
        }
        set(BUNDLE_ID)
    }

    /// Give `steam://` links back to whatever handled them before.
    pub fn unregister() -> Result<(), String> {
        let old = std::fs::read_to_string(super::config_backup()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        set(old.as_deref().unwrap_or(STEAM_ID))
    }
}

/// Put a shortcut to a game on the desktop (an internet shortcut to
/// `steam://rungameid/<id>`, with the game's own icon).
#[cfg(not(target_os = "macos"))]
pub fn desktop_shortcut(app_id: u32, name: &str, icon: Option<PathBuf>) -> Result<PathBuf, String> {
    let desktop = directories::UserDirs::new().and_then(|d| d.desktop_dir().map(|p| p.to_path_buf())).ok_or("No desktop folder found")?;
    let path = desktop.join(format!("{}.url", safe_name(name)));
    let mut text = format!("[InternetShortcut]\r\nURL=steam://rungameid/{app_id}\r\n");
    if let Some(icon) = icon {
        text.push_str(&format!("IconFile={}\r\nIconIndex=0\r\n", icon.display()));
    }
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// A file name from a game name.
fn safe_name(name: &str) -> String {
    let safe: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { ' ' } else { c }).collect();
    safe.trim().to_string()
}

/// macOS: a small app on the desktop, like Steam makes, that opens
/// `steam://rungameid/<id>` with SteamInRust (the running copy gets it).
#[cfg(target_os = "macos")]
pub fn desktop_shortcut(app_id: u32, name: &str, icon: Option<PathBuf>) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let desktop = directories::UserDirs::new().and_then(|d| d.desktop_dir().map(|p| p.to_path_buf())).ok_or("No desktop folder found")?;
    let bundle = desktop.join(format!("{}.app", safe_name(name)));
    let contents = bundle.join("Contents");
    let err = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(contents.join("MacOS")).map_err(err)?;
    std::fs::create_dir_all(contents.join("Resources")).map_err(err)?;
    let script = format!("#!/bin/sh\n# URL=steam://rungameid/{app_id}\nexec \"{}\" \"steam://rungameid/{app_id}\"\n", exe().replace('"', "\\\""));
    let launcher = contents.join("MacOS").join("launcher");
    std::fs::write(&launcher, script).map_err(err)?;
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).map_err(err)?;
    let has_icon = icon.is_some_and(|i| std::fs::copy(i, contents.join("Resources").join("icon.icns")).is_ok());
    let xml = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleName</key>\n\t<string>{}</string>\n\t<key>CFBundleExecutable</key>\n\t<string>launcher</string>\n\t<key>CFBundleIdentifier</key>\n\t<string>io.github.madonchik123.steaminrust.game{app_id}</string>\n\t<key>CFBundlePackageType</key>\n\t<string>APPL</string>\n{}</dict>\n</plist>\n",
        xml(name),
        if has_icon { "\t<key>CFBundleIconFile</key>\n\t<string>icon</string>\n" } else { "" }
    );
    std::fs::write(contents.join("Info.plist"), plist).map_err(err)?;
    Ok(bundle)
}

/// Delete desktop shortcuts that start this game (ours and Steam's both use
/// `steam://rungameid/<id>`). Returns how many were removed.
pub fn remove_desktop_shortcuts(app_id: u32) -> usize {
    let mut dirs = Vec::new();
    if let Some(d) = directories::UserDirs::new().and_then(|d| d.desktop_dir().map(|p| p.to_path_buf())) {
        dirs.push(d);
    }
    if let Some(public) = std::env::var_os("PUBLIC") {
        dirs.push(PathBuf::from(public).join("Desktop"));
    }
    let mut removed = 0;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            // macOS: our shortcut apps.
            if path.extension().is_some_and(|x| x == "app") {
                let launcher = path.join("Contents").join("MacOS").join("launcher");
                if std::fs::read_to_string(launcher).is_ok_and(|t| shortcut_runs(&t, app_id)) && std::fs::remove_dir_all(&path).is_ok() {
                    removed += 1;
                }
                continue;
            }
            if path.extension().and_then(|x| x.to_str()).is_none_or(|x| !x.eq_ignore_ascii_case("url")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            if shortcut_runs(&text, app_id) && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// Whether a .url file's target is exactly this game.
fn shortcut_runs(text: &str, app_id: u32) -> bool {
    text.lines().any(|l| {
        let l = l.trim().trim_start_matches("# ");
        l.strip_prefix("URL=").is_some_and(|u| {
            let u = u.trim().trim_end_matches('/');
            u == format!("steam://rungameid/{app_id}") || u == format!("steam://run/{app_id}")
        })
    })
}

/// Best icon for a game on macOS: the icon file inside its app bundle.
#[cfg(target_os = "macos")]
pub fn game_icon(app: Option<&model::OwnedApp>, install_dir: &std::path::Path) -> Option<PathBuf> {
    let opt = app?.launch.iter().find(|l| l.runs_on_this_os() && l.executable.to_lowercase().ends_with(".app"))?;
    let resources = install_dir.join(&opt.executable).join("Contents").join("Resources");
    let plist = std::fs::read_to_string(install_dir.join(&opt.executable).join("Contents").join("Info.plist")).unwrap_or_default();
    let named = plist.split("<key>CFBundleIconFile</key>").nth(1).and_then(|r| r.trim_start().strip_prefix("<string>")?.split("</string>").next()).map(|n| n.trim().to_string());
    let file = |n: String| resources.join(if n.ends_with(".icns") { n } else { format!("{n}.icns") });
    named.map(file).filter(|p| p.is_file()).or_else(|| std::fs::read_dir(&resources).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "icns")))
}

/// Best icon for a game: its main executable.
#[cfg(not(target_os = "macos"))]
pub fn game_icon(app: Option<&model::OwnedApp>, install_dir: &std::path::Path) -> Option<PathBuf> {
    let exe = app?.launch.iter().find(|l| l.runs_on_this_os() && l.executable.to_lowercase().ends_with(".exe"))?;
    let p = install_dir.join(exe.executable.replace('/', "\\"));
    p.exists().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        assert_eq!(parse("steam://rungameid/570"), Some(Link::Run(570)));
        assert_eq!(parse("\"steam://launch/730/\""), Some(Link::Run(730)));
        assert_eq!(parse("steam://install/440"), Some(Link::Install(440)));
        assert_eq!(parse("steam://store/620"), Some(Link::Store(620)));
        assert_eq!(parse("steam://open/downloads"), Some(Link::Page("downloads")));
        assert_eq!(parse("https://example.com"), None);
        assert!(shortcut_runs("[InternetShortcut]\r\nURL=steam://rungameid/570\r\n", 570));
        assert!(!shortcut_runs("[InternetShortcut]\r\nURL=steam://rungameid/5700\r\n", 570));
    }
}

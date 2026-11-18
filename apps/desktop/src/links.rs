//! `steam://` links, single-instance forwarding, and desktop shortcuts.
//!
//! - SteamInRust can register itself as the handler for `steam://` links
//!   (Settings → Interface). The previous handler is saved so it can be
//!   restored with one click.
//! - Only one copy runs: a second launch (e.g. from a link or shortcut)
//!   forwards its link to the running window over localhost and exits.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const PORT: u16 = 47_630;
const LISTEN_TIMEOUT: Duration = Duration::from_millis(500);
const FORWARD_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_LINE_BYTES: u64 = 2048;

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

fn token_path() -> PathBuf {
    client::store::config_dir().join("links.token")
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true).mode(0o700);
        b.create(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        if meta.file_type().is_symlink() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "config dir is a symlink"));
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

fn write_token_file(path: &Path, token: &str) -> std::io::Result<()> {
    let Some(dir) = path.parent() else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing token parent dir"));
    };
    ensure_private_dir(dir)?;
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "token path is a symlink"));
        }
    }
    let tmp = dir.join(format!(".links-token-{}-{}.tmp", std::process::id(), &generate_token()[..16]));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let write_res = (|| -> std::io::Result<()> {
        let mut f = opts.open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(token.as_bytes())?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn read_token_file(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return None;
    }
    let tok = std::fs::read_to_string(path).ok()?;
    let tok = tok.trim();
    (tok.len() >= 32 && tok.bytes().all(|b| b.is_ascii_hexdigit())).then(|| tok.to_string())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) fn is_allowed_payload(payload: &str) -> bool {
    let p = payload.trim();
    p == "show" || parse(p).is_some()
}

fn accept_one(mut stream: TcpStream, expected_token: &str) -> Option<String> {
    if !stream.peer_addr().is_ok_and(|a| a.ip().is_loopback()) {
        return None;
    }
    stream.set_read_timeout(Some(LISTEN_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(LISTEN_TIMEOUT)).ok()?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_LINE_BYTES)).read_line(&mut line).ok()?;
    if !line.ends_with('\n') {
        return None;
    }
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let (got_token, payload) = trimmed.split_once(' ')?;
    if expected_token.is_empty() || !ct_eq(got_token.as_bytes(), expected_token.as_bytes()) {
        return None;
    }
    let payload = payload.trim();
    if !is_allowed_payload(payload) {
        return None;
    }
    let _ = stream.write_all(b"ok\n").and_then(|_| stream.flush());
    Some(payload.to_string())
}

fn forward_with(addr: SocketAddr, token_file: &Path, arg: &str) -> bool {
    let arg = arg.trim();
    if arg.contains(['\r', '\n']) || !is_allowed_payload(arg) {
        return false;
    }
    let Some(token) = read_token_file(token_file) else { return false };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, FORWARD_TIMEOUT) else { return false };
    let _ = s.set_read_timeout(Some(FORWARD_TIMEOUT));
    let _ = s.set_write_timeout(Some(FORWARD_TIMEOUT));
    if s.write_all(format!("{token} {arg}\n").as_bytes()).and_then(|_| s.flush()).is_err() {
        return false;
    }
    let mut ack = String::new();
    BufReader::new(s.take(16)).read_line(&mut ack).is_ok() && ack.trim() == "ok"
}

/// Try to hand `arg` to an already-running copy. Returns true if it did.
pub fn forward_to_running(arg: &str) -> bool {
    forward_with(SocketAddr::from(([127, 0, 0, 1], PORT)), &token_path(), arg)
}

/// Listen for authenticated links from later launches. Each line received
/// carries `<token> <link|show>`.
pub fn listen(ctx: egui::Context) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    if let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)) {
        let token = generate_token();
        if let Err(e) = write_token_file(&token_path(), &token) {
            tracing::warn!("links listener disabled: couldn't write instance token: {e}");
            return rx;
        }
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(payload) = accept_one(stream, &token) {
                    let _ = tx.send(payload);
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

    #[test]
    fn authenticates_and_bounds_localhost_forwarding() {
        let dir = std::env::temp_dir().join(format!("sir-links-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tok_path = dir.join("cfg").join("links.token");
        let token = generate_token();
        write_token_file(&tok_path, &token).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let f_mode = std::fs::metadata(&tok_path).unwrap().permissions().mode() & 0o777;
            let d_mode = std::fs::metadata(tok_path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
            assert_eq!(f_mode, 0o600, "token file must be 0600");
            assert_eq!(d_mode, 0o700, "token dir must be 0700");
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let tok_srv = token.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(p) = accept_one(stream, &tok_srv) {
                    let _ = tx.send(p);
                }
            }
        });

        {
            let mut s = TcpStream::connect(addr).unwrap();
            s.write_all(b"steam://rungameid/570\n").unwrap();
        }
        {
            let mut s = TcpStream::connect(addr).unwrap();
            let bad = "0".repeat(64);
            s.write_all(format!("{bad} steam://rungameid/570\n").as_bytes()).unwrap();
        }
        {
            let mut s = TcpStream::connect(addr).unwrap();
            s.write_all(format!("{token} GET / HTTP/1.1\n").as_bytes()).unwrap();
        }
        {
            let mut s = TcpStream::connect(addr).unwrap();
            let big = format!("{token} steam://rungameid/{}\n", "9".repeat(2500));
            let _ = s.write_all(big.as_bytes());
        }
        let idle = TcpStream::connect(addr).unwrap();
        let t0 = std::time::Instant::now();

        assert!(forward_with(addr, &tok_path, "show"));
        assert!(t0.elapsed() < Duration::from_secs(3), "idle client hung listener!");
        drop(idle);
        assert!(forward_with(addr, &tok_path, "steam://rungameid/570"));
        assert!(!forward_with(addr, &tok_path, "https://evil.example"));

        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), "show");
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), "steam://rungameid/570");
        assert!(rx.try_recv().is_err(), "no invalid/unauthenticated messages may pass");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

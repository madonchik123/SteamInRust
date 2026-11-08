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
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn config_backup() -> PathBuf {
    client::store::config_dir().join("steam-protocol-backup.txt")
}

#[cfg(windows)]
fn reg(args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg")
        .args(args)
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

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
    if let Some(old) =
        current_handler().filter(|h| !h.to_lowercase().contains(&exe().to_lowercase()))
    {
        let _ = std::fs::create_dir_all(client::store::config_dir());
        let _ = std::fs::write(config_backup(), old);
    }
    let cmd = format!("\"{}\" \"%1\"", exe());
    let ok = reg(&[
        "add",
        r"HKCU\Software\Classes\steam",
        "/ve",
        "/d",
        "URL:steam protocol",
        "/f",
    ])
    .is_some()
        && reg(&[
            "add",
            r"HKCU\Software\Classes\steam",
            "/v",
            "URL Protocol",
            "/d",
            "",
            "/f",
        ])
        .is_some()
        && reg(&["add", KEY, "/ve", "/d", &cmd, "/f"]).is_some();
    if ok {
        Ok(())
    } else {
        Err("Windows refused the change".into())
    }
}

/// Give `steam://` links back to whatever handled them before.
#[cfg(windows)]
pub fn unregister() -> Result<(), String> {
    match std::fs::read_to_string(config_backup()) {
        Ok(old) if !old.trim().is_empty() => {
            reg(&["add", KEY, "/ve", "/d", old.trim(), "/f"])
                .ok_or("Windows refused the change")?;
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

#[cfg(not(windows))]
pub fn is_default_handler() -> bool {
    false
}
#[cfg(not(windows))]
pub fn register() -> Result<(), String> {
    Err("Only supported on Windows for now".into())
}
#[cfg(not(windows))]
pub fn unregister() -> Result<(), String> {
    Ok(())
}

/// Put a shortcut to a game on the desktop (an internet shortcut to
/// `steam://rungameid/<id>`, with the game's own icon).
pub fn desktop_shortcut(app_id: u32, name: &str, icon: Option<PathBuf>) -> Result<PathBuf, String> {
    let desktop = directories::UserDirs::new()
        .and_then(|d| d.desktop_dir().map(|p| p.to_path_buf()))
        .ok_or("No desktop folder found")?;
    let safe: String = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { ' ' } else { c })
        .collect();
    let path = desktop.join(format!("{}.url", safe.trim()));
    let mut text = format!("[InternetShortcut]\r\nURL=steam://rungameid/{app_id}\r\n");
    if let Some(icon) = icon {
        text.push_str(&format!("IconFile={}\r\nIconIndex=0\r\n", icon.display()));
    }
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Delete desktop shortcuts that start this game (ours and Steam's both use
/// `steam://rungameid/<id>`). Returns how many were removed.
pub fn remove_desktop_shortcuts(app_id: u32) -> usize {
    let mut dirs = Vec::new();
    if let Some(d) =
        directories::UserDirs::new().and_then(|d| d.desktop_dir().map(|p| p.to_path_buf()))
    {
        dirs.push(d);
    }
    if let Some(public) = std::env::var_os("PUBLIC") {
        dirs.push(PathBuf::from(public).join("Desktop"));
    }
    let mut removed = 0;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path
                .extension()
                .and_then(|x| x.to_str())
                .is_none_or(|x| !x.eq_ignore_ascii_case("url"))
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
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
        let l = l.trim();
        l.strip_prefix("URL=").is_some_and(|u| {
            let u = u.trim().trim_end_matches('/');
            u == format!("steam://rungameid/{app_id}") || u == format!("steam://run/{app_id}")
        })
    })
}

/// Best icon for a game: its main executable.
pub fn game_icon(app: Option<&model::OwnedApp>, install_dir: &std::path::Path) -> Option<PathBuf> {
    let exe = app?
        .launch
        .iter()
        .find(|l| l.runs_on_this_os() && l.executable.to_lowercase().ends_with(".exe"))?;
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
        assert_eq!(
            parse("steam://open/downloads"),
            Some(Link::Page("downloads"))
        );
        assert_eq!(parse("https://example.com"), None);
        assert!(shortcut_runs(
            "[InternetShortcut]\r\nURL=steam://rungameid/570\r\n",
            570
        ));
        assert!(!shortcut_runs(
            "[InternetShortcut]\r\nURL=steam://rungameid/5700\r\n",
            570
        ));
    }
}

//! The setup's file work: unpack the app, shortcuts, the "Apps & features"
//! entry, updating in place and uninstalling. Per-user only (no admin).

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const APP_EXE: &str = "steam-rs.exe";
pub const UNINSTALL_EXE: &str = "uninstall.exe";
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\SteamInRust";
const STEAM_LINK_KEY: &str = r"HKCU\Software\Classes\steam\shell\open\command";

/// The app files, zipped (see build.rs).
pub static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip"));

pub fn has_payload() -> bool {
    PAYLOAD.len() > 22
}

#[derive(Default, Clone)]
pub struct Progress {
    pub fraction: f32,
    pub file: String,
    /// What happened so far, for "Show details" (like Steam's setup).
    pub log: Vec<String>,
    /// Bytes unpacked / total (the updater shows "x of y KB").
    pub bytes: (u64, u64),
    pub done: bool,
    pub error: Option<String>,
}

pub type Shared = Arc<Mutex<Progress>>;

#[derive(Clone)]
pub struct Options {
    pub dir: PathBuf,
    pub desktop: bool,
    pub start_menu: bool,
    /// Updating: keep shortcuts as they are.
    pub update: bool,
}

fn set(p: &Shared, f: impl FnOnce(&mut Progress)) {
    if let Ok(mut g) = p.lock() {
        f(&mut g);
    }
}

fn log(p: &Shared, line: String) {
    set(p, |g| g.log.push(line));
}

pub fn default_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Programs").join("SteamInRust")
}

/// Where an earlier install lives, from its "Apps & features" entry.
pub fn installed_dir() -> Option<PathBuf> {
    let out = reg(&["query", UNINSTALL_KEY, "/v", "InstallLocation"])?;
    let line = out.lines().find(|l| l.contains("REG_SZ"))?;
    let path = PathBuf::from(line.split("REG_SZ").nth(1)?.trim());
    path.join(APP_EXE).exists().then_some(path)
}

/// Install or update. Waits up to `wait` for a running copy to close.
pub fn install(opts: &Options, p: &Shared, wait: Duration) -> Result<(), String> {
    std::fs::create_dir_all(&opts.dir).map_err(|e| format!("{}: {e}", opts.dir.display()))?;
    log(p, format!("Output folder: {}", opts.dir.display()));
    set(p, |g| g.file = i18n::tr_dyn("Waiting for SteamInRust to close…"));
    wait_unlocked(&opts.dir.join(APP_EXE), wait)?;

    let mut zip = zip::ZipArchive::new(Cursor::new(PAYLOAD)).map_err(|e| format!("payload: {e}"))?;
    let total: u64 = (0..zip.len()).filter_map(|i| zip.by_index(i).ok().map(|f| f.size())).sum::<u64>().max(1);
    let mut written = 0u64;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else { continue };
        let out = opts.dir.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let name = rel.display().to_string();
        set(p, |g| g.file = format!("Extract: {name}"));
        let mut data = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        replace_file(&out, &data)?;
        written += data.len() as u64;
        log(p, format!("Extract: {}... 100%", rel.display()));
        set(p, |g| {
            g.fraction = 0.9 * written as f32 / total as f32;
            g.bytes = (written, total);
        });
    }

    // The uninstaller is this setup (it knows `--uninstall`).
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let un = opts.dir.join(UNINSTALL_EXE);
    if me != un {
        let bytes = std::fs::read(&me).map_err(|e| e.to_string())?;
        replace_file(&un, &bytes)?;
        log(p, format!("Created uninstaller: {}", un.display()));
    }
    set(p, |g| {
        g.fraction = 0.95;
        g.file = i18n::tr_dyn("Creating shortcuts…");
    });
    let exe = opts.dir.join(APP_EXE);
    if !opts.update {
        let _ = remove_shortcuts();
        if opts.start_menu {
            shortcut(&start_menu_link(), &exe)?;
            log(p, format!("Create shortcut: {}", start_menu_link().display()));
        }
        if opts.desktop {
            let link = desktop_dir()?.join("SteamInRust.lnk");
            shortcut(&link, &exe)?;
            log(p, format!("Create shortcut: {}", link.display()));
        }
    }
    register(&opts.dir)?;
    log(p, "Completed".into());
    set(p, |g| {
        g.fraction = 1.0;
        g.file = i18n::tr_dyn("Completed");
        g.done = true;
    });
    Ok(())
}

/// Remove the app. `data`: also delete settings, saved sign-ins and caches.
pub fn uninstall(dir: &Path, data: bool, p: &Shared) -> Result<(), String> {
    set(p, |g| g.file = i18n::tr_dyn("Waiting for SteamInRust to close…"));
    wait_unlocked(&dir.join(APP_EXE), Duration::from_secs(20))?;
    set(p, |g| {
        g.fraction = 0.2;
        g.file = i18n::tr_dyn("Removing shortcuts…");
    });
    let _ = remove_shortcuts();
    restore_steam_links(dir);
    let _ = reg(&["delete", UNINSTALL_KEY, "/f"]);
    // Startup entry, if the app added one.
    let _ = reg(&["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "SteamInRust", "/f"]);
    set(p, |g| {
        g.fraction = 0.4;
        g.file = i18n::tr_dyn("Removing files…");
    });
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.file_name().is_some_and(|n| n == UNINSTALL_EXE) {
                continue;
            }
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            log(p, format!("Delete file: {}", path.display()));
        }
    }
    if data {
        set(p, |g| {
            g.fraction = 0.7;
            g.file = i18n::tr_dyn("Removing your settings…");
        });
        for d in data_dirs() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    // This program can't delete itself while running: a short-lived helper
    // removes the folder a moment after we exit.
    schedule_delete(dir);
    log(p, "Completed".into());
    set(p, |g| {
        g.fraction = 1.0;
        g.file = i18n::tr_dyn("Completed");
        g.done = true;
    });
    Ok(())
}

/// Settings, sign-ins and caches the app keeps outside its folder.
fn data_dirs() -> Vec<PathBuf> {
    // %APPDATA%\SteamInRust and %LOCALAPPDATA%\SteamInRust (the program
    // itself is in %LOCALAPPDATA%\Programs, a different folder).
    ["APPDATA", "LOCALAPPDATA"].iter().filter_map(|v| std::env::var_os(v)).map(|b| PathBuf::from(b).join("SteamInRust")).collect()
}

/// Write `data` to `path`, also when an old copy is in the way.
fn replace_file(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("new");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(data).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let old = path.with_extension("old");
        let _ = std::fs::remove_file(&old);
        if std::fs::rename(path, &old).is_err() {
            std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let _ = std::fs::remove_file(&old);
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Wait until `exe` isn't running (it can be renamed again).
fn wait_unlocked(exe: &Path, wait: Duration) -> Result<(), String> {
    if !exe.exists() {
        return Ok(());
    }
    let probe = exe.with_extension("probe");
    let start = Instant::now();
    loop {
        if std::fs::rename(exe, &probe).is_ok() {
            let _ = std::fs::rename(&probe, exe);
            return Ok(());
        }
        if start.elapsed() > wait {
            return Err(i18n::tr_dyn("SteamInRust is still running. Close it (also from the tray) and try again."));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn register(dir: &Path) -> Result<(), String> {
    let exe = dir.join(APP_EXE).display().to_string();
    let un = dir.join(UNINSTALL_EXE).display().to_string();
    let size_kb: u64 = std::fs::read_dir(dir).map(|r| r.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum::<u64>() / 1024).unwrap_or(0);
    let values: [(&str, &str, String); 10] = [
        ("DisplayName", "REG_SZ", "SteamInRust".into()),
        ("DisplayVersion", "REG_SZ", env!("CARGO_PKG_VERSION").into()),
        ("Publisher", "REG_SZ", "SteamInRust".into()),
        ("DisplayIcon", "REG_SZ", format!("{exe},0")),
        ("InstallLocation", "REG_SZ", dir.display().to_string()),
        ("UninstallString", "REG_SZ", format!("\"{un}\" --uninstall")),
        ("QuietUninstallString", "REG_SZ", format!("\"{un}\" --uninstall --quiet")),
        ("EstimatedSize", "REG_DWORD", size_kb.to_string()),
        ("NoModify", "REG_DWORD", "1".into()),
        ("NoRepair", "REG_DWORD", "1".into()),
    ];
    for (name, kind, value) in values {
        reg(&["add", UNINSTALL_KEY, "/v", name, "/t", kind, "/d", &value, "/f"]).ok_or_else(|| i18n::tr_dyn("Windows refused the change"))?;
    }
    Ok(())
}

/// If the app took over `steam://` links, give them back.
fn restore_steam_links(dir: &Path) {
    let Some(out) = reg(&["query", STEAM_LINK_KEY, "/ve"]) else { return };
    let ours = out.to_lowercase().contains(&dir.display().to_string().to_lowercase());
    if !ours {
        return;
    }
    let backup = directories::ProjectDirs::from("dev", "SteamInRust", "SteamInRust").and_then(|d| std::fs::read_to_string(d.config_dir().join("steam-protocol-backup.txt")).ok());
    match backup.filter(|b| !b.trim().is_empty()) {
        Some(old) => {
            let _ = reg(&["add", STEAM_LINK_KEY, "/ve", "/d", old.trim(), "/f"]);
        }
        None => {
            let _ = reg(&["delete", r"HKCU\Software\Classes\steam", "/f"]);
        }
    }
}

fn start_menu_link() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
    base.join(r"Microsoft\Windows\Start Menu\Programs\SteamInRust.lnk")
}

fn desktop_dir() -> Result<PathBuf, String> {
    let out = powershell("[Environment]::GetFolderPath('Desktop')")?;
    Ok(PathBuf::from(out.trim()))
}

fn remove_shortcuts() -> Result<(), String> {
    let _ = std::fs::remove_file(start_menu_link());
    let _ = std::fs::remove_file(desktop_dir()?.join("SteamInRust.lnk"));
    Ok(())
}

fn shortcut(link: &Path, target: &Path) -> Result<(), String> {
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let dir = target.parent().unwrap_or(target);
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.IconLocation='{},0';$s.Description='SteamInRust';$s.Save()",
        q(link),
        q(target),
        q(dir),
        q(target)
    );
    powershell(&script).map(|_| ())
}

fn schedule_delete(dir: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Waits until this uninstaller has closed, then removes the folder.
        let script = format!(
            "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; for ($i = 0; $i -lt 20; $i++) {{ Start-Sleep -Milliseconds 500; Remove-Item -LiteralPath '{dir}' -Recurse -Force -ErrorAction SilentlyContinue; if (-not (Test-Path -LiteralPath '{dir}')) {{ break }} }}",
            pid = std::process::id(),
            dir = dir.display().to_string().replace('\'', "''")
        );
        let _ = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script]).creation_flags(0x0800_0000 | 0x0000_0008).spawn();
    }
    #[cfg(not(windows))]
    let _ = dir;
}

/// The language picked in setup becomes the app's language on its first
/// start (the app reads this file only while it has no saved choice).
pub fn save_language(code: &str) {
    if let Some(d) = directories::ProjectDirs::from("dev", "SteamInRust", "SteamInRust") {
        let _ = std::fs::create_dir_all(d.config_dir());
        let _ = std::fs::write(d.config_dir().join("language"), code);
    }
}

/// Bytes the app needs (for "Space required").
pub fn payload_size() -> u64 {
    zip::ZipArchive::new(Cursor::new(PAYLOAD)).map(|mut z| (0..z.len()).filter_map(|i| z.by_index(i).ok().map(|f| f.size())).sum()).unwrap_or(0)
}

/// Free space on the drive of `dir` (for "Space available").
pub fn free_space(dir: &Path) -> Option<u64> {
    let drive = dir.components().next()?.as_os_str().to_string_lossy().trim_end_matches(':').to_string();
    let out = powershell(&format!("(Get-PSDrive -Name '{}').Free", drive.replace('\'', ""))).ok()?;
    out.trim().parse().ok()
}

/// Start the installed app.
pub fn launch(dir: &Path) {
    let _ = std::process::Command::new(dir.join(APP_EXE)).current_dir(dir).spawn();
}

fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn reg(args: &[&str]) -> Option<String> {
    let out = hidden(std::process::Command::new("reg").args(args)).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn powershell(script: &str) -> Result<String, String> {
    // UTF-8 output: paths like a OneDrive "Рабочий стол" desktop came back
    // garbled in the console's code page.
    let script = format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; {script}");
    let out = hidden(std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script])).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

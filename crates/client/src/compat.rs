//! Steam Play for the Mac: Windows games run through a Wine-based runner
//! (CrossOver, or a free Wine build), like Steam's Proton on Linux.
//!
//! - **Runners** ([`runners`]): CrossOver and Wine builds found on this Mac.
//! - **Choices** ([`Config`], `compat.json`): the default runner, whether
//!   Windows-only games use it, and games forced to a runner (Properties →
//!   Compatibility), which then get the Windows version even when a Mac one
//!   exists.
//! - **What installs** ([`plan`]): the Mac version, the Windows version
//!   through a runner, or nothing (no runner, or anti-cheat).
//! - **What's installed**: the app manifest records a Windows version on a
//!   Mac the way Steam does for Proton (`UserConfig/platform_override_source`
//!   "windows", `platform_override_dest` "macos"); see [`installed_platform`].
//!
//! Launching is in `launch.rs`: the runner starts `rundll32` on our Windows
//! steamclient DLL, which points the prefix's registry at itself, starts the
//! game and talks to this app over the localhost connection in `ipc.rs`.
//! Games with anti-cheat never start this way.

use model::{AppId, OwnedApp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Is Steam Play offered on this system? (Only the Mac for now.)
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// What kind of runner it is (they start differently).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// CodeWeavers' CrossOver: its `wine` script, one bottle for our games.
    CrossOver,
    /// A plain Wine build (WineHQ / Gcenx app, Homebrew, Game Porting Toolkit).
    Wine,
}

/// A Wine-based runner found on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runner {
    /// Stable id for settings ("crossover", "wine-devel", ...).
    pub id: String,
    /// What the player sees ("CrossOver 26.3", "Wine Devel 11.21").
    pub name: String,
    pub kind: Kind,
    /// The program that runs Windows programs.
    pub wine: PathBuf,
}

/// The player's Steam Play choices (`compat.json` in the app's data folder).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Runner id for games not set one by one ("" = the best one found).
    pub default: String,
    /// Like Steam's "Enable Steam Play for all other titles": Windows-only
    /// games install and run through the default runner.
    pub other_titles: bool,
    /// Games forced to a runner (Properties → Compatibility): the Windows
    /// version is used even when there's a Mac version.
    pub games: BTreeMap<AppId, String>,
    /// "Don't ask again" on the offer to install Wine (game pages of
    /// Windows-only games when there's no runner).
    pub no_wine_offer: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { default: String::new(), other_titles: true, games: BTreeMap::new(), no_wine_offer: false }
    }
}

fn config_path() -> PathBuf {
    crate::store::config_dir().join("compat.json")
}

/// The choices as last read or saved (screens ask every frame).
static CONFIG: Mutex<Option<Config>> = Mutex::new(None);

/// The saved choices (defaults if there are none yet).
pub fn load() -> Config {
    let mut cached = CONFIG.lock().unwrap();
    cached.get_or_insert_with(|| std::fs::read_to_string(config_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()).clone()
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    *CONFIG.lock().unwrap() = Some(cfg.clone());
    std::fs::write(config_path(), json).map_err(|e| e.to_string())
}

/// Why `app` can't be installed here, or `None` if it can (its own
/// version, or the Windows one through a runner), on this computer's system.
pub fn install_blocker(app: &OwnedApp, cfg: &Config, found: &[Runner]) -> Option<Blocker> {
    match plan(app, cfg, found) {
        Plan::Windows { .. } => None,
        Plan::Native if app.runs_on_this_os() => None,
        Plan::AntiCheat(name) => Some(Blocker::AntiCheat(name)),
        Plan::NoRunner if SUPPORTED => Some(Blocker::NoRunner),
        _ => Some(Blocker::OtherSystem),
    }
}

/// See [`install_blocker`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    /// Made for other systems only, and Steam Play can't help.
    OtherSystem,
    /// Windows-only and no CrossOver or Wine here (or Steam Play is off for it).
    NoRunner,
    /// Windows-only with anti-cheat: can't run on a Mac.
    AntiCheat(String),
}

// ------------------------------------------------------------ runners

/// What [`runners`] found last, and when.
static FOUND: Mutex<Option<(Instant, Vec<Runner>)>> = Mutex::new(None);

/// Runners on this computer, best first (CrossOver, then Wine builds).
/// Looked up again at most every 30 seconds; [`rescan`] forces it.
pub fn runners() -> Vec<Runner> {
    if !SUPPORTED {
        return Vec::new();
    }
    let mut found = FOUND.lock().unwrap();
    if let Some((at, list)) = found.as_ref().filter(|(at, _)| at.elapsed() < Duration::from_secs(30)) {
        let _ = at;
        return list.clone();
    }
    let list = scan(&search_roots(), std::env::var_os("STEAMINRUST_WINE").map(PathBuf::from));
    *found = Some((Instant::now(), list.clone()));
    list
}

/// Can `app` be played on this computer: its own version, or (on a Mac)
/// the Windows one through a runner? For list filters: no copying.
pub fn playable_here(app: &OwnedApp) -> bool {
    if app.runs_on_this_os() && !cfg!(target_os = "macos") {
        return true;
    }
    if !SUPPORTED {
        return app.runs_on_this_os();
    }
    let found = runners();
    let cfg = CONFIG.lock().unwrap();
    match cfg.as_ref() {
        Some(cfg) => install_blocker(app, cfg, &found).is_none(),
        None => {
            drop(cfg);
            install_blocker(app, &load(), &found).is_none()
        }
    }
}

/// Look again now (after installing CrossOver or Wine).
pub fn rescan() -> Vec<Runner> {
    *FOUND.lock().unwrap() = None;
    runners()
}

/// Folders apps are installed in: `/Applications`, `~/Applications`, and
/// ours (the Wine build SteamInRust installs, see [`install_wine`]).
fn search_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots.push(runners_dir());
    roots
}

/// Where SteamInRust puts the Wine build it installs itself.
pub fn runners_dir() -> PathBuf {
    crate::store::config_dir().join("compat").join("runners")
}

// ------------------------------------------- installing Wine for the player

/// The free Wine build SteamInRust offers to install when there's no
/// runner: Gcenx's macOS build of Wine (WineHQ's own Mac packages come from
/// it). The development series, because it's the one that ran games on
/// macOS 27 in testing (Wine Stable 11.0 crashed while setting up its
/// prefix there). Pinned with its SHA-256, so a changed file is refused.
pub const WINE_URL: &str = "https://github.com/Gcenx/macOS_Wine_builds/releases/download/11.21/wine-devel-11.21-osx64.tar.xz";
pub const WINE_SHA256: &str = "9909b486657e10ab66c7279371ae1f990345b596b3b1b3a0cf21f28af24fa0ce";
/// Its size in bytes (for the progress bar before the server says).
pub const WINE_SIZE: u64 = 225_612_636;
/// The app bundle inside the archive.
const WINE_BUNDLE: &str = "Wine Devel.app";

/// How an install of Wine is going (shared with the screen showing it).
#[derive(Debug, Default)]
pub struct WineInstall {
    pub downloaded: std::sync::atomic::AtomicU64,
    pub total: std::sync::atomic::AtomicU64,
    /// Unpacking (after the download).
    pub unpacking: std::sync::atomic::AtomicBool,
    /// Installing Rosetta 2 first (macOS is asking for the password).
    pub rosetta: std::sync::atomic::AtomicBool,
    pub cancel: std::sync::atomic::AtomicBool,
    /// `Some` when it's over: `Ok` or what went wrong.
    pub result: Mutex<Option<Result<(), String>>>,
}

impl WineInstall {
    pub fn fraction(&self) -> f32 {
        use std::sync::atomic::Ordering::Relaxed;
        let total = self.total.load(Relaxed).max(1);
        (self.downloaded.load(Relaxed) as f32 / total as f32).min(1.0)
    }

    pub fn finished(&self) -> Option<Result<(), String>> {
        self.result.lock().unwrap().clone()
    }
}

/// Download, check and unpack [`WINE_URL`] into [`runners_dir`]. The result
/// also lands in `progress.result`.
pub async fn install_wine(progress: std::sync::Arc<WineInstall>) -> Result<(), String> {
    let r = install_wine_inner(&progress).await;
    if r.is_err() {
        let _ = std::fs::remove_file(runners_dir().join(".wine-download.tar.xz"));
    }
    *progress.result.lock().unwrap() = Some(r.clone());
    if r.is_ok() {
        rescan();
    }
    r
}

async fn install_wine_inner(progress: &WineInstall) -> Result<(), String> {
    use sha2::Digest;
    use std::io::Write;
    use std::sync::atomic::Ordering::Relaxed;
    if !SUPPORTED {
        return Err("Steam Play is for macOS".into());
    }
    let dir = runners_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    let archive = dir.join(".wine-download.tar.xz");
    let client = reqwest::Client::builder().user_agent("SteamInRust").build().map_err(|e| e.to_string())?;
    let mut resp = client.get(WINE_URL).send().await.and_then(|r| r.error_for_status()).map_err(|e| format!("couldn't download Wine: {e}"))?;
    progress.total.store(resp.content_length().unwrap_or(WINE_SIZE), Relaxed);
    let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
    let mut hash = sha2::Sha256::new();
    loop {
        if progress.cancel.load(Relaxed) {
            return Err("cancelled".into());
        }
        let chunk = tokio::time::timeout(Duration::from_secs(60), resp.chunk()).await.map_err(|_| "the download stalled".to_string())?.map_err(|e| format!("couldn't download Wine: {e}"))?;
        let Some(chunk) = chunk else { break };
        hash.update(&chunk);
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        progress.downloaded.fetch_add(chunk.len() as u64, Relaxed);
        if progress.downloaded.load(Relaxed) > WINE_SIZE * 2 {
            return Err("the download is far bigger than expected".into());
        }
    }
    drop(file);
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != WINE_SHA256 {
        return Err("the downloaded Wine didn't match its checksum, so it wasn't installed".into());
    }
    progress.unpacking.store(true, Relaxed);
    let unpack = dir.join(".wine-unpack");
    let (a, u, d) = (archive.clone(), unpack.clone(), dir.clone());
    tokio::task::spawn_blocking(move || unpack_wine(&a, &u, &d)).await.map_err(|e| e.to_string())??;
    let _ = std::fs::remove_file(&archive);
    Ok(())
}

/// `tar -xf` (macOS's tar reads .tar.xz) into a scratch folder, then move
/// the app bundle into place, replacing an older one.
fn unpack_wine(archive: &Path, scratch: &Path, dir: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(scratch);
    std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("/usr/bin/tar").arg("-xf").arg(archive).arg("-C").arg(scratch).status().map_err(|e| format!("couldn't unpack Wine: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_dir_all(scratch);
        return Err(format!("couldn't unpack Wine ({status})"));
    }
    let bundle = scratch.join(WINE_BUNDLE);
    if !bundle.join("Contents/Resources/wine/bin/wine").is_file() {
        let _ = std::fs::remove_dir_all(scratch);
        return Err("the Wine download didn't have the expected app inside".into());
    }
    let dest = dir.join(WINE_BUNDLE);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&bundle, &dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(scratch);
    Ok(())
}

/// Is the Wine that SteamInRust installed there?
pub fn wine_installed_by_us() -> Option<PathBuf> {
    let app = runners_dir().join(WINE_BUNDLE);
    app.join("Contents/Resources/wine/bin/wine").is_file().then_some(app)
}

/// Remove the Wine SteamInRust installed (game prefixes stay).
pub fn remove_our_wine() -> Result<(), String> {
    let app = runners_dir().join(WINE_BUNDLE);
    std::fs::remove_dir_all(&app).map_err(|e| e.to_string())?;
    rescan();
    Ok(())
}

/// Wine builds for the Mac are Intel programs: on Apple Silicon they need
/// Rosetta 2. True on Intel Macs.
pub fn rosetta_ready() -> bool {
    if !cfg!(target_arch = "aarch64") {
        return true;
    }
    Path::new("/Library/Apple/usr/libexec/oah/libRosettaRuntime").exists()
}

/// Install Rosetta 2 with Apple's own tool; macOS asks for the password.
pub fn install_rosetta() -> Result<(), String> {
    let script = "do shell script \"/usr/sbin/softwareupdate --install-rosetta --agree-to-license\" with administrator privileges";
    let out = std::process::Command::new("/usr/bin/osascript").args(["-e", script]).output().map_err(|e| e.to_string())?;
    if out.status.success() || rosetta_ready() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.contains("-128") { "cancelled".into() } else { format!("Rosetta wasn't installed: {err}") })
    }
}

/// The app bundles we know, with the `wine` program inside them.
const BUNDLES: &[(&str, &str, Kind, &str)] = &[
    ("crossover", "CrossOver.app", Kind::CrossOver, "Contents/SharedSupport/CrossOver/bin/wine"),
    ("crossover-preview", "CrossOver Preview.app", Kind::CrossOver, "Contents/SharedSupport/CrossOver/bin/wine"),
    ("wine-stable", "Wine Stable.app", Kind::Wine, "Contents/Resources/wine/bin/wine"),
    ("wine-devel", "Wine Devel.app", Kind::Wine, "Contents/Resources/wine/bin/wine"),
    ("wine-staging", "Wine Staging.app", Kind::Wine, "Contents/Resources/wine/bin/wine"),
];

/// Wine programs outside app bundles (Homebrew, Game Porting Toolkit).
const PROGRAMS: &[(&str, &str, &str)] = &[
    ("wine-homebrew", "Wine (Homebrew)", "/opt/homebrew/bin/wine"),
    ("wine-homebrew-intel", "Wine (Homebrew)", "/usr/local/bin/wine"),
    ("gptk", "Game Porting Toolkit", "/usr/local/opt/game-porting-toolkit/bin/wine64"),
];

/// Find runners in `roots` (folders of apps), the fixed program paths, and
/// `custom` (`STEAMINRUST_WINE`, a wine program or a Wine app bundle).
fn scan(roots: &[PathBuf], custom: Option<PathBuf>) -> Vec<Runner> {
    let mut out: Vec<Runner> = Vec::new();
    for (id, bundle, kind, rel) in BUNDLES {
        let Some(app) = roots.iter().map(|r| r.join(bundle)).find(|a| a.join(rel).is_file()) else { continue };
        let base = bundle.trim_end_matches(".app");
        let name = match bundle_version(&app) {
            Some(v) => format!("{base} {v}"),
            None => base.to_string(),
        };
        out.push(Runner { id: id.to_string(), name, kind: *kind, wine: app.join(rel) });
    }
    for (id, name, path) in PROGRAMS {
        let p = PathBuf::from(path);
        // Homebrew's wine is often a link into a Wine app already listed.
        let real = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if p.is_file() && !out.iter().any(|r| std::fs::canonicalize(&r.wine).ok().as_ref() == Some(&real)) && !out.iter().any(|r| r.name == *name) {
            out.push(Runner { id: id.to_string(), name: name.to_string(), kind: Kind::Wine, wine: p });
        }
    }
    if let Some(c) = custom {
        let wine = if c.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) {
            BUNDLES.iter().map(|(_, _, _, rel)| c.join(rel)).find(|p| p.is_file())
        } else {
            c.is_file().then_some(c.clone())
        };
        if let Some(wine) = wine {
            let kind = if wine.to_string_lossy().contains("SharedSupport/CrossOver") { Kind::CrossOver } else { Kind::Wine };
            out.push(Runner { id: "custom".into(), name: format!("Wine ({})", c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), kind, wine });
        }
    }
    out
}

/// `CFBundleShortVersionString` from an app's Info.plist (XML ones only).
fn bundle_version(app: &Path) -> Option<String> {
    let text = std::fs::read_to_string(app.join("Contents/Info.plist")).ok()?;
    let after = text.split("<key>CFBundleShortVersionString</key>").nth(1)?;
    let v = after.trim_start().strip_prefix("<string>")?.split("</string>").next()?.trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// The runner for a game: its own choice, else the default, else the best
/// one found. `None` when none of those is on this computer.
pub fn runner_for(app_id: AppId, cfg: &Config, found: &[Runner]) -> Option<Runner> {
    let by_id = |id: &str| found.iter().find(|r| r.id == id).cloned();
    cfg.games.get(&app_id).and_then(|id| by_id(id)).or_else(|| by_id(&cfg.default)).or_else(|| found.first().cloned())
}

// ------------------------------------------------------- what installs

/// Steam category "Valve Anti-Cheat enabled".
const CATEGORY_VAC: u32 = 8;

/// How a game can be installed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// This system's own version (or Steam Play isn't involved).
    Native,
    /// The Windows version, played through `runner`.
    Windows { runner: Runner },
    /// Windows-only and no runner on this computer (or Steam Play is off
    /// for other titles).
    NoRunner,
    /// Uses anti-cheat: can't run through a runner.
    AntiCheat(String),
}

/// What to install for `app` on this system. Forced games get the Windows
/// version even if a native one exists; Windows-only games get it when
/// "other titles" is on. Known anti-cheat (VAC) rules the runner out.
pub fn plan(app: &OwnedApp, cfg: &Config, found: &[Runner]) -> Plan {
    plan_for(model::THIS_OS, app, cfg, found)
}

pub fn plan_for(os: &str, app: &OwnedApp, cfg: &Config, found: &[Runner]) -> Plan {
    if os == "windows" {
        return Plan::Native;
    }
    let forced = cfg.games.get(&app.app_id).is_some_and(|id| !id.is_empty());
    let native = app.runs_on(os);
    if native && !forced {
        return Plan::Native;
    }
    if !app.runs_on("windows") {
        return Plan::Native;
    }
    if !forced && !cfg.other_titles {
        return Plan::NoRunner;
    }
    if app.categories.contains(&CATEGORY_VAC) {
        // A forced game with a Mac version just stays native.
        return if native { Plan::Native } else { Plan::AntiCheat("Valve Anti-Cheat".into()) };
    }
    match runner_for(app.app_id, cfg, found) {
        Some(runner) => Plan::Windows { runner },
        None if native => Plan::Native,
        None => Plan::NoRunner,
    }
}

/// The system whose version is installed in `install_dir`, when it isn't
/// this system's (a Windows version on a Mac): from the app manifest's
/// `platform_override_source`, like Steam records Proton installs.
pub fn installed_platform(app_id: AppId, install_dir: &Path) -> Option<String> {
    let steamapps = install_dir.parent()?.parent()?;
    let text = std::fs::read_to_string(steamapps.join(format!("appmanifest_{app_id}.acf"))).ok()?;
    let doc = vdf::parse(&text).ok()?;
    let user = doc.path(&["AppState", "UserConfig"])?;
    let source = user.str("platform_override_source")?.trim().to_lowercase();
    (!source.is_empty() && source != model::THIS_OS).then_some(source)
}

// ---------------------------------------------------------- prefixes

/// Where a game's Wine prefix lives for one runner:
/// `<data>/compat/<app id>/<runner id>/pfx`, like Steam's
/// `compatdata/<app id>/pfx`. One per runner, because Wine can't go back to
/// an older version in a prefix a newer one made (its services crash).
/// CrossOver uses one bottle instead (see [`crossover_bottle`]).
pub fn prefix(app_id: AppId, runner_id: &str) -> PathBuf {
    let safe: String = runner_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    crate::store::config_dir().join("compat").join(app_id.to_string()).join(if safe.is_empty() { "wine".into() } else { safe }).join("pfx")
}

/// The CrossOver bottle our games share (shown in CrossOver's own window).
pub const CROSSOVER_BOTTLE: &str = "SteamInRust";

/// CrossOver's bottles folder, where [`CROSSOVER_BOTTLE`] is.
pub fn crossover_bottle() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/CrossOver/Bottles").join(CROSSOVER_BOTTLE))
}

/// The prefix folder a runner uses for a game.
pub fn prefix_for(runner: &Runner, app_id: AppId) -> Option<PathBuf> {
    match runner.kind {
        Kind::CrossOver => crossover_bottle(),
        Kind::Wine => Some(prefix(app_id, &runner.id)),
    }
}

/// The Windows user's folder inside a prefix (`drive_c/users/<name>`):
/// plain Wine names it after the Mac user, CrossOver bottles use
/// `crossover`.
pub fn wine_user_dir(prefix: &Path) -> PathBuf {
    let users = prefix.join("drive_c").join("users");
    let me = std::env::var("USER").ok().filter(|u| !u.is_empty());
    if let Some(me) = me.as_ref().filter(|u| users.join(u).is_dir()) {
        return users.join(me);
    }
    if users.join("crossover").is_dir() {
        return users.join("crossover");
    }
    let other = std::fs::read_dir(&users).ok().and_then(|rd| {
        rd.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().is_some_and(|n| !n.eq_ignore_ascii_case("public")))
    });
    other.unwrap_or_else(|| users.join(me.unwrap_or_else(|| "crossover".into())))
}

/// The Wine prefix of an installed Windows version (Steam Play), if this is one.
pub fn installed_prefix(app: &OwnedApp, install_dir: &Path) -> Option<PathBuf> {
    if !SUPPORTED || installed_platform(app.app_id, install_dir).is_none() {
        return None;
    }
    let runner = runner_for(app.app_id, &load(), &runners())?;
    prefix_for(&runner, app.app_id)
}

/// `C:\...` inside a prefix -> the Mac path (`drive_c/...`).
pub fn in_drive_c(prefix: &Path, windows_path: &str) -> PathBuf {
    let rel = windows_path.trim_start_matches(['C', 'c']).trim_start_matches(':').trim_start_matches(['\\', '/']);
    rel.split(['\\', '/']).filter(|p| !p.is_empty()).fold(prefix.join("drive_c"), |p, part| p.join(part))
}

/// A Mac path as Wine sees it: Wine's `Z:` drive is `/`.
pub fn windows_path(unix: &Path) -> String {
    format!("Z:{}", unix.to_string_lossy().replace('/', "\\"))
}

/// Where our steamclient DLL goes inside the prefix (a short path without
/// spaces, so `rundll32` reads it as one argument).
pub const DLL_IN_PREFIX: &str = r"C:\SteamInRust\steamclient64.dll";
pub const DLL32_IN_PREFIX: &str = r"C:\SteamInRust\steamclient.dll";

/// The export `rundll32` runs in our DLL: it fills in the prefix's
/// `ActiveProcess` registry key, starts the game and waits for it.
pub const LAUNCH_ENTRY: &str = "SteamInRustLaunch";

/// The Windows steamclient DLL shipped for Steam Play (64-bit, and 32-bit
/// if it's there), from `STEAMINRUST_STEAMCLIENT_WIN64` or next to the app
/// (`Contents/Resources/windows/` in the app bundle, `windows/` next to a
/// development build).
pub fn windows_dll() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("STEAMINRUST_STEAMCLIENT_WIN64").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let mut candidates = vec![exe_dir.join("windows").join("steamclient64.dll")];
    if let Some(contents) = exe_dir.parent() {
        candidates.push(contents.join("Resources").join("windows").join("steamclient64.dll"));
    }
    // A development build: `cargo xtask steamclient-win` puts it here.
    if let (Some(profile), Some(target)) = (exe_dir.file_name(), exe_dir.parent()) {
        candidates.push(target.join("x86_64-pc-windows-gnu").join(profile).join("steamclient.dll"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Copy our DLL into the prefix when it's missing or different. Returns
/// whether the 32-bit one is there too.
pub fn install_dll(prefix: &Path, dll64: &Path) -> Result<bool, String> {
    let dest = in_drive_c(prefix, DLL_IN_PREFIX);
    std::fs::create_dir_all(dest.parent().unwrap_or(prefix)).map_err(|e| format!("couldn't prepare the Wine prefix: {e}"))?;
    copy_if_changed(dll64, &dest)?;
    let dll32 = dll32_beside(dll64);
    let has32 = dll32.is_file() && copy_if_changed(&dll32, &in_drive_c(prefix, DLL32_IN_PREFIX)).is_ok();
    Ok(has32)
}

/// The 32-bit DLL shipped with `dll64`: `steamclient32.dll` next to it,
/// or (a development build) the i686 build in the same profile folder.
pub fn dll32_beside(dll64: &Path) -> PathBuf {
    let next = dll64.with_file_name("steamclient32.dll");
    let dev = dll64.parent().and_then(|p| Some((p.file_name()?.to_owned(), p.parent()?.parent()?.to_path_buf()))).map(|(profile, target)| target.join("i686-pc-windows-gnu").join(profile).join("steamclient.dll"));
    match dev {
        Some(d) if !next.is_file() && d.is_file() => d,
        _ => next,
    }
}

fn copy_if_changed(from: &Path, to: &Path) -> Result<(), String> {
    let same = std::fs::read(to).ok().is_some_and(|have| std::fs::read(from).ok().is_some_and(|want| want == have));
    if !same {
        std::fs::copy(from, to).map_err(|e| format!("couldn't copy {}: {e}", from.display()))?;
    }
    Ok(())
}

/// The command that runs a Windows program in this runner and prefix.
/// CrossOver's `wine` script takes the bottle by name (creating it first
/// if needed, see [`ensure_prefix`]); plain Wine takes `WINEPREFIX`.
pub fn wine_command(runner: &Runner, prefix: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(&runner.wine);
    match runner.kind {
        Kind::CrossOver => {
            cmd.arg("--bottle").arg(CROSSOVER_BOTTLE).arg("--no-convert");
        }
        Kind::Wine => {
            cmd.env("WINEPREFIX", prefix);
        }
    }
    if std::env::var_os("WINEDEBUG").is_none() {
        cmd.env("WINEDEBUG", "-all");
    }
    // Wine on macOS: faster syncing primitives where the build has them.
    cmd.env("WINEMSYNC", "1").env("WINEESYNC", "1");
    // MoltenVK (Vulkan on Metal) otherwise lists every feature on each start.
    if std::env::var_os("MVK_CONFIG_LOG_LEVEL").is_none() {
        cmd.env("MVK_CONFIG_LOG_LEVEL", "1");
    }
    cmd
}

/// Make sure the prefix exists (Wine creates it on first use, which takes
/// a while; doing it here keeps that out of the game's first start).
pub fn ensure_prefix(runner: &Runner, prefix: &Path) -> Result<(), String> {
    if prefix.join("system.reg").is_file() {
        return Ok(());
    }
    match runner.kind {
        Kind::CrossOver => {
            let cxbottle = runner.wine.with_file_name("cxbottle");
            let status = std::process::Command::new(&cxbottle)
                .args(["--bottle", CROSSOVER_BOTTLE, "--create", "--template", "win10_64", "--description", "SteamInRust games"])
                .status()
                .map_err(|e| format!("couldn't start CrossOver's cxbottle: {e}"))?;
            if !status.success() && !prefix.join("system.reg").is_file() {
                return Err(format!("CrossOver couldn't create the {CROSSOVER_BOTTLE} bottle ({status})"));
            }
        }
        Kind::Wine => {
            std::fs::create_dir_all(prefix).map_err(|e| format!("couldn't create the Wine prefix: {e}"))?;
            let status = wine_command(runner, prefix).args(["wineboot", "-u"]).status().map_err(|e| format!("couldn't start Wine: {e}"))?;
            if !status.success() {
                return Err(format!("Wine couldn't set up its prefix ({status})"));
            }
        }
    }
    Ok(())
}

/// Turn installer steps (Steam's redistributables) into Wine runs in the
/// prefix: installers run through the runner, registry values are imported
/// with `regedit`. Firewall rules don't apply. `done` remembers what ran in
/// this prefix already (their ids), so each runs once. Returns the ids of
/// the steps that ran, and what failed.
pub fn run_setup(runner: &Runner, prefix: &Path, steps: &[crate::installscript::Step], progress: &dyn Fn(usize, &str)) -> (Vec<String>, Vec<String>) {
    use crate::installscript::{Action, RegValue};
    let mut done = Vec::new();
    let mut errors = Vec::new();
    let mut reg = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
    let mut reg_ids = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        match &s.action {
            Action::Run { exe, args, .. } => {
                progress(i, &s.label);
                let lower = exe.to_ascii_lowercase();
                let mut cmd = wine_command(runner, prefix);
                if lower.ends_with(".msi") {
                    cmd.args(["msiexec", "/i"]);
                }
                // `exe` is a Windows path (Z:\...) here; Wine finds it.
                cmd.arg(exe).args(args.split_whitespace());
                match cmd.status() {
                    Ok(st) if st.success() || matches!(st.code(), Some(1638 | 1641 | 3010)) => done.push(s.id.clone()),
                    Ok(st) => errors.push(format!("{}: exited with {st}", s.label)),
                    Err(e) => errors.push(format!("{}: {e}", s.label)),
                }
            }
            Action::Registry { key, name, value } => {
                let v = match value {
                    RegValue::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
                    RegValue::Dword(d) => format!("dword:{d:08x}"),
                };
                reg.push_str(&format!("[{key}]\r\n\"{}\"={v}\r\n\r\n", name.replace('"', "\\\"")));
                reg_ids.push(s.id.clone());
            }
            Action::Firewall { .. } => done.push(s.id.clone()),
        }
    }
    if !reg_ids.is_empty() {
        let file = prefix.join("steaminrust-setup.reg");
        let imported = std::fs::write(&file, reg.as_bytes()).is_ok()
            && wine_command(runner, prefix).arg("regedit").arg("/S").arg(windows_path(&file)).status().is_ok_and(|s| s.success());
        let _ = std::fs::remove_file(&file);
        if imported {
            done.extend(reg_ids);
        } else {
            errors.push("registry values: regedit failed".into());
        }
    }
    (done, errors)
}

/// What's run in each prefix already (`compat/setup-done.json`).
fn setup_done_path() -> PathBuf {
    crate::store::config_dir().join("compat").join("setup-done.json")
}

pub fn setup_done(prefix: &Path) -> std::collections::HashSet<String> {
    let all: BTreeMap<String, Vec<String>> = std::fs::read_to_string(setup_done_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    all.get(&prefix.to_string_lossy().into_owned()).map(|v| v.iter().cloned().collect()).unwrap_or_default()
}

pub fn mark_setup_done(prefix: &Path, ids: &[String]) {
    let path = setup_done_path();
    let mut all: BTreeMap<String, Vec<String>> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let list = all.entry(prefix.to_string_lossy().into_owned()).or_default();
    for id in ids {
        if !list.contains(id) {
            list.push(id.clone());
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(&all) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{DepotInfo, LaunchOption};

    fn game(id: AppId, os_list: &str, categories: Vec<u32>) -> OwnedApp {
        OwnedApp { app_id: id, name: format!("game {id}"), os_list: os_list.into(), categories, ..Default::default() }
    }

    fn runner(id: &str) -> Runner {
        Runner { id: id.into(), name: id.into(), kind: Kind::Wine, wine: "/x/wine".into() }
    }

    #[test]
    fn windows_only_games_use_the_default_runner() {
        let cfg = Config::default();
        let found = vec![runner("crossover"), runner("wine-devel")];
        assert_eq!(plan_for("macos", &game(1, "windows", vec![]), &cfg, &found), Plan::Windows { runner: runner("crossover") });
        // A Mac version wins unless the game is forced.
        assert_eq!(plan_for("macos", &game(2, "windows,macos", vec![]), &cfg, &found), Plan::Native);
        let cfg = Config { default: "wine-devel".into(), ..Default::default() };
        assert_eq!(plan_for("macos", &game(1, "windows", vec![]), &cfg, &found), Plan::Windows { runner: runner("wine-devel") });
    }

    #[test]
    fn forcing_a_runner_picks_the_windows_version() {
        let found = vec![runner("crossover"), runner("wine-devel")];
        let mut cfg = Config::default();
        cfg.games.insert(2, "wine-devel".into());
        assert_eq!(plan_for("macos", &game(2, "windows,macos", vec![]), &cfg, &found), Plan::Windows { runner: runner("wine-devel") });
        // A forced runner that's gone falls back to the default one.
        cfg.games.insert(3, "gone".into());
        assert_eq!(plan_for("macos", &game(3, "windows", vec![]), &cfg, &found), Plan::Windows { runner: runner("crossover") });
    }

    #[test]
    fn no_runner_or_anti_cheat_means_no_windows_version() {
        let cfg = Config::default();
        assert_eq!(plan_for("macos", &game(1, "windows", vec![]), &cfg, &[]), Plan::NoRunner);
        assert_eq!(plan_for("macos", &game(1, "windows", vec![8]), &cfg, &[runner("wine-devel")]), Plan::AntiCheat("Valve Anti-Cheat".into()));
        let off = Config { other_titles: false, ..Default::default() };
        assert_eq!(plan_for("macos", &game(1, "windows", vec![]), &off, &[runner("wine-devel")]), Plan::NoRunner);
        // Linux-only games aren't for a Wine runner; Windows never uses one.
        assert_eq!(plan_for("macos", &game(4, "linux", vec![]), &cfg, &[runner("wine-devel")]), Plan::Native);
        assert_eq!(plan_for("windows", &game(1, "windows", vec![]), &cfg, &[runner("wine-devel")]), Plan::Native);
    }

    #[test]
    fn unknown_system_list_uses_launch_options() {
        let mut g = game(5, "", vec![]);
        g.launch = vec![LaunchOption { executable: "game.exe".into(), arguments: String::new(), working_dir: String::new(), description: String::new(), os_list: "windows".into() }];
        g.depots = vec![DepotInfo { id: 6, os: "windows".into(), ..Default::default() }];
        assert_eq!(plan_for("macos", &g, &Config::default(), &[runner("wine-devel")]), Plan::Windows { runner: runner("wine-devel") });
    }

    #[test]
    fn finds_runner_bundles() {
        let root = std::env::temp_dir().join(format!("sir-compat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for rel in ["CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine", "Wine Devel.app/Contents/Resources/wine/bin/wine"] {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"#!").unwrap();
        }
        std::fs::write(root.join("CrossOver.app/Contents/Info.plist"), "<plist><dict><key>CFBundleShortVersionString</key>\n<string>26.3</string></dict></plist>").unwrap();
        let found = scan(std::slice::from_ref(&root), None);
        let names: Vec<(&str, &str, Kind)> = found.iter().map(|r| (r.id.as_str(), r.name.as_str(), r.kind)).collect();
        assert!(names.starts_with(&[("crossover", "CrossOver 26.3", Kind::CrossOver), ("wine-devel", "Wine Devel", Kind::Wine)]), "{names:?}");
        // STEAMINRUST_WINE can name a Wine app bundle.
        let custom = scan(&[], Some(root.join("Wine Devel.app")));
        assert!(custom.iter().any(|r| r.id == "custom" && r.wine.ends_with("bin/wine")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_the_platform_override_from_the_manifest() {
        let lib = std::env::temp_dir().join(format!("sir-compat-acf-{}", std::process::id()));
        let dir = lib.join("steamapps").join("common").join("Game");
        std::fs::create_dir_all(&dir).unwrap();
        let acf = lib.join("steamapps").join("appmanifest_9.acf");
        std::fs::write(&acf, "\"AppState\" { \"appid\" \"9\" \"UserConfig\" { \"platform_override_source\" \"windows\" \"platform_override_dest\" \"macos\" } }").unwrap();
        let expect = (model::THIS_OS != "windows").then(|| "windows".to_string());
        assert_eq!(installed_platform(9, &dir), expect);
        std::fs::write(&acf, "\"AppState\" { \"appid\" \"9\" \"UserConfig\" { \"language\" \"english\" } }").unwrap();
        assert_eq!(installed_platform(9, &dir), None);
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn one_prefix_per_game_and_runner() {
        let a = prefix(5, "wine-stable");
        assert_ne!(a, prefix(5, "wine-devel"));
        assert!(a.ends_with(Path::new("compat").join("5").join("wine-stable").join("pfx")));
        assert!(prefix(5, "../x").ends_with(Path::new("5").join("___x").join("pfx")));
    }

    #[test]
    fn prefix_paths() {
        let pfx = Path::new("/p/pfx");
        assert_eq!(in_drive_c(pfx, DLL_IN_PREFIX), Path::new("/p/pfx/drive_c/SteamInRust/steamclient64.dll"));
        assert_eq!(windows_path(Path::new("/Users/me/Game/game.exe")), r"Z:\Users\me\Game\game.exe");
    }

    #[test]
    fn config_round_trip_and_defaults() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.other_titles && cfg.default.is_empty() && cfg.games.is_empty());
        let mut c = Config::default();
        c.games.insert(10, "crossover".into());
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}

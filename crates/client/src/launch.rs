//! Starting installed games, in one of three ways:
//!
//! - **Our steamclient** ([`LaunchMode::SteamInRust`]): games without
//!   anti-cheat get SteamInRust's own `steamclient` DLL (crates/steamclient)
//!   through the registry value Steam itself uses, so their Steam API calls
//!   (user, achievements, cloud files…) are answered by this app.
//! - **Official Steam** ([`LaunchMode::OfficialSteam`]): games with
//!   anti-cheat (VAC, Easy Anti-Cheat, BattlEye, …) never get our DLL; that
//!   would keep the anti-cheat from running. Valve's client is started
//!   silently and runs the game, so Steam and the anti-cheat work normally.
//! - **No Steam** ([`LaunchMode::NoSteam`]): when our DLL can't serve the
//!   game (32-bit games, DLL missing), it starts plain, as before.

use model::{AppId, OwnedApp};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    /// Steam features come from SteamInRust's own steamclient.
    SteamInRust,
    /// Runs through the official Steam client because of its anti-cheat
    /// (named in `anti_cheat`).
    OfficialSteam { anti_cheat: String },
    /// Starts without Steam features; `reason` says why.
    NoSteam { reason: String },
}

/// How Play will start this game. Reads the install folder (anti-cheat
/// files, the executable's bitness), so callers should cache it.
pub fn mode(app: &OwnedApp, install_dir: &Path) -> LaunchMode {
    if let Some(ac) = anti_cheat(app, install_dir) {
        return LaunchMode::OfficialSteam { anti_cheat: ac };
    }
    let Some(exe) = executable(app, install_dir) else {
        return LaunchMode::NoSteam { reason: "no launch option for this system".into() };
    };
    match pe_is_64bit(&exe) {
        Some(true) => {}
        Some(false) => return LaunchMode::NoSteam { reason: "32-bit game".into() },
        None => return LaunchMode::NoSteam { reason: "game file not found".into() },
    }
    if steamclient_dll().is_none() {
        return LaunchMode::NoSteam { reason: "steamclient.dll not found next to SteamInRust".into() };
    }
    LaunchMode::SteamInRust
}

/// [`mode`], remembered for a minute (for UI that asks every frame).
pub fn mode_cached(app: &OwnedApp, install_dir: &Path) -> LaunchMode {
    static CACHE: Mutex<Option<std::collections::HashMap<(AppId, PathBuf), (Instant, LaunchMode)>>> = Mutex::new(None);
    let key = (app.app_id, install_dir.to_path_buf());
    let mut cache = CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(Default::default);
    if let Some((_, m)) = map.get(&key).filter(|(at, _)| at.elapsed() < Duration::from_secs(60)) {
        return m.clone();
    }
    let m = mode(app, install_dir);
    map.insert(key, (Instant::now(), m.clone()));
    m
}

// ---------------------------------------------------------- anti-cheat

/// Files and folders that mean an anti-cheat ships with the game
/// (lower-case, matched against names in the install folder).
const ANTI_CHEAT_MARKERS: &[(&str, &str)] = &[
    ("easyanticheat", "Easy Anti-Cheat"),
    ("start_protected_game.exe", "Easy Anti-Cheat"),
    ("eac_launcher", "Easy Anti-Cheat"),
    ("battleye", "BattlEye"),
    ("beservice", "BattlEye"),
    ("beclient", "BattlEye"),
    ("eaanticheat", "EA Anti-Cheat"),
    ("pnkbstr", "PunkBuster"),
    ("punkbuster", "PunkBuster"),
    ("pbsvc", "PunkBuster"),
    ("xigncode", "XIGNCODE3"),
    ("gameguard", "nProtect GameGuard"),
    ("npgamemon", "nProtect GameGuard"),
    ("equ8", "EQU8"),
    ("mhyprot", "miHoYo Protect"),
    ("vgk.sys", "Vanguard"),
    ("anticheatexpert", "Anti-Cheat Expert"),
    ("ace-base", "Anti-Cheat Expert"),
    ("tp3helper", "TenProtect"),
    ("faceit", "FACEIT"),
    ("randgrid", "RICOCHET"),
    ("blackcipher", "BlackCipher"),
    ("nexonguard", "Nexon Guard"),
    ("hackshield", "HackShield"),
    ("anticheat", "anti-cheat"),
    ("anti-cheat", "anti-cheat"),
    ("anti_cheat", "anti-cheat"),
];

/// Steam category for "Valve Anti-Cheat enabled".
const CATEGORY_VAC: u32 = 8;

/// The anti-cheat this game uses, if any. Unsure (folder unreadable) counts
/// as anti-cheat: running it through Steam is the safe side.
pub fn anti_cheat(app: &OwnedApp, install_dir: &Path) -> Option<String> {
    if app.categories.contains(&CATEGORY_VAC) {
        return Some("Valve Anti-Cheat".into());
    }
    for opt in app.launch.iter().filter(|o| o.runs_on_this_os()) {
        let text = format!("{} {}", opt.executable, opt.arguments).to_lowercase();
        if let Some((_, name)) = ANTI_CHEAT_MARKERS.iter().find(|(m, _)| text.contains(m)) {
            return Some(name.to_string());
        }
    }
    if std::fs::read_dir(install_dir).is_err() {
        return Some("unknown (install folder unreadable)".into());
    }
    scan_markers(install_dir, 3).map(String::from)
}

fn scan_markers(dir: &Path, depth: u32) -> Option<&'static str> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if let Some((_, ac)) = ANTI_CHEAT_MARKERS.iter().find(|(m, _)| name.contains(m)) {
            return Some(ac);
        }
        if depth > 0 && e.file_type().is_ok_and(|t| t.is_dir()) {
            subdirs.push(e.path());
        }
    }
    subdirs.iter().find_map(|d| scan_markers(d, depth - 1))
}

// ---------------------------------------------------------- spawning

fn executable(app: &OwnedApp, install_dir: &Path) -> Option<PathBuf> {
    let opt = app.launch.iter().find(|o| o.runs_on_this_os() && !o.executable.is_empty())?;
    Some(install_dir.join(normalize(&opt.executable)))
}

/// 64-bit Windows executable? `None` if unreadable or not a PE file.
fn pe_is_64bit(path: &Path) -> Option<bool> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut mz = [0u8; 64];
    f.read_exact(&mut mz).ok()?;
    if &mz[..2] != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(mz[60..64].try_into().ok()?) as u64;
    f.seek(SeekFrom::Start(pe)).ok()?;
    let mut hdr = [0u8; 6];
    f.read_exact(&mut hdr).ok()?;
    if &hdr[..4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes([hdr[4], hdr[5]]) == 0x8664)
}

/// Our steamclient DLL: `steamclient64.dll` or `steamclient.dll` (the
/// cargo build name) next to the app, 64-bit only.
pub fn steamclient_dll() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("STEAMINRUST_STEAMCLIENT64").map(PathBuf::from) {
        return pe_is_64bit(&p).filter(|b| *b).map(|_| p);
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    ["steamclient64.dll", "steamclient.dll"].iter().map(|n| dir.join(n)).find(|p| pe_is_64bit(p) == Some(true))
}

pub fn spawn(app: &OwnedApp, install_dir: &Path, extra_args: &str) -> Result<Child, String> {
    command(app, install_dir, extra_args)?.spawn().map_err(|e| format!("could not start {}: {e}", app.name))
}

/// Start a game with our steamclient: environment like a Steam launch, plus
/// the pipe and token for the DLL. The registry swap must be in place.
pub fn spawn_with_steamclient(app: &OwnedApp, install_dir: &Path, extra_args: &str, ipc_env: &str, account: &str) -> Result<Child, String> {
    let mut cmd = command(app, install_dir, extra_args)?;
    let id = app.app_id.to_string();
    cmd.env(crate::ipc::protocol::ENV_IPC, ipc_env)
        .env("SteamOverlayGameId", &id)
        .env("SteamClientLaunch", "1")
        .env("SteamEnv", "1")
        .env("SteamAppUser", account)
        .env("SteamUser", account);
    if let Some(log) = std::env::var_os("STEAMINRUST_STEAMCLIENT_LOG") {
        cmd.env(crate::ipc::protocol::ENV_LOG, log);
    }
    cmd.spawn().map_err(|e| format!("could not start {}: {e}", app.name))
}

fn command(app: &OwnedApp, install_dir: &Path, extra_args: &str) -> Result<Command, String> {
    let opt = app
        .launch
        .iter()
        .find(|o| o.runs_on_this_os() && !o.executable.is_empty())
        .ok_or_else(|| format!("{} has no launch option for this OS", app.name))?;

    let exe = install_dir.join(normalize(&opt.executable));
    let cwd = if opt.working_dir.is_empty() {
        exe.parent().unwrap_or(install_dir).to_path_buf()
    } else {
        install_dir.join(normalize(&opt.working_dir))
    };
    tracing::info!("launching {} ({})", app.name, exe.display());

    let mut cmd = Command::new(&exe);
    cmd.args(split_args(&opt.arguments))
        .args(split_args(extra_args))
        .current_dir(cwd)
        .env("SteamAppId", app.app_id.to_string())
        .env("SteamGameId", app.app_id.to_string());
    Ok(cmd)
}

fn normalize(p: &str) -> String {
    if cfg!(windows) { p.replace('/', "\\") } else { p.replace('\\', "/") }
}

/// Minimal shell-style split that respects double quotes.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Kill a process and everything it started.
pub fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-TERM", &format!("-{pid}")]).status();
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    }
}

/// Close every process running from a game's folder (for games the
/// official client started, whose process ids we don't know).
pub fn kill_in_folder(dir: &Path) {
    for pid in win::processes_in(dir) {
        kill_tree(pid);
    }
}

pub fn wait(app_id: AppId, mut child: Child) -> AppId {
    let _ = child.wait();
    app_id
}

// ------------------------------------------------- the registry swap

/// `HKCU\Software\Valve\Steam\ActiveProcess`: steam_api reads the steamclient
/// path (`SteamClientDll64`) and Steam's process id (`pid`, must be alive)
/// from here.
const ACTIVE: &str = "Software\\Valve\\Steam\\ActiveProcess";

#[derive(Debug, Clone, Default)]
struct Saved {
    pid: Option<u32>,
    user: Option<u32>,
    dll64: Option<String>,
}

struct Swap {
    saved: Saved,
    /// What we wrote, so we only put back values nobody changed since
    /// (the official client rewrites them when it starts).
    ours: Saved,
    /// Launches whose game hasn't loaded the DLL yet.
    pending: std::collections::HashSet<String>,
    /// Launches still running.
    running: std::collections::HashSet<String>,
}

static SWAP: Mutex<Option<Swap>> = Mutex::new(None);

/// Point steam_api at our DLL for one launch.
pub fn swap_begin(token: &str, dll64: &Path, account_id: u32) -> Result<(), String> {
    let mut swap = SWAP.lock().unwrap();
    let state = swap.get_or_insert_with(|| Swap {
        saved: Saved { pid: win::reg_dword(ACTIVE, "pid"), user: win::reg_dword(ACTIVE, "ActiveUser"), dll64: win::reg_string(ACTIVE, "SteamClientDll64") },
        ours: Saved::default(),
        pending: Default::default(),
        running: Default::default(),
    });
    let dll = dll64.to_string_lossy().into_owned();
    let pid = std::process::id();
    win::set_string(ACTIVE, "SteamClientDll64", &dll)?;
    win::set_dword(ACTIVE, "pid", pid)?;
    win::set_dword(ACTIVE, "ActiveUser", account_id)?;
    state.ours = Saved { pid: Some(pid), user: Some(account_id), dll64: Some(dll) };
    state.pending.insert(token.to_string());
    state.running.insert(token.to_string());
    Ok(())
}

/// The game loaded our DLL (or never will): put the DLL path back right
/// away so nothing else picks it up. Steam's process id stays ours while
/// our games run, unless the official client is running.
pub fn swap_loaded(token: &str) {
    let mut swap = SWAP.lock().unwrap();
    let Some(s) = swap.as_mut() else { return };
    if !s.pending.remove(token) || !s.pending.is_empty() {
        return;
    }
    restore_dll(s);
}

/// One of our games closed.
pub fn swap_exited(token: &str) {
    let mut swap = SWAP.lock().unwrap();
    let Some(s) = swap.as_mut() else { return };
    s.pending.remove(token);
    s.running.remove(token);
    if s.pending.is_empty() {
        restore_dll(s);
    }
    if s.running.is_empty() {
        restore_all(s);
        *swap = None;
    }
}

/// Undo everything now (before the official client launches a game).
pub fn swap_restore_now() {
    let mut swap = SWAP.lock().unwrap();
    if let Some(s) = swap.as_mut() {
        restore_all(s);
    }
    *swap = None;
}

fn restore_dll(s: &Swap) {
    if win::reg_string(ACTIVE, "SteamClientDll64") == s.ours.dll64 {
        put_back_string("SteamClientDll64", &s.saved.dll64);
    }
    if win::reg_dword(ACTIVE, "ActiveUser") == s.ours.user {
        put_back_dword("ActiveUser", s.saved.user);
    }
    // The official client is running: its process id goes back too.
    if s.saved.pid.is_some_and(win::is_steam_process) && win::reg_dword(ACTIVE, "pid") == s.ours.pid {
        put_back_dword("pid", s.saved.pid);
    }
}

fn restore_all(s: &Swap) {
    restore_dll(s);
    if win::reg_dword(ACTIVE, "pid") == s.ours.pid {
        put_back_dword("pid", s.saved.pid);
    }
}

fn put_back_string(name: &str, v: &Option<String>) {
    let _ = match v {
        Some(v) => win::set_string(ACTIVE, name, v),
        None => win::delete_value(ACTIVE, name),
    };
}

fn put_back_dword(name: &str, v: Option<u32>) {
    let _ = match v {
        Some(v) => win::set_dword(ACTIVE, name, v),
        None => win::delete_value(ACTIVE, name),
    };
}

/// After a crash, the registry may still point at our DLL: put Valve's
/// back (only values whose process is gone).
pub fn repair_registry() {
    // Sign-in settings left from an earlier official-Steam launch.
    restore_login_later();
    if SWAP.lock().unwrap().is_some() {
        return;
    }
    let Some(dll) = win::reg_string(ACTIVE, "SteamClientDll64") else { return };
    let Some(root) = win::reg_string("Software\\Valve\\Steam", "SteamPath") else { return };
    let valve = PathBuf::from(root.replace('/', "\\")).join("steamclient64.dll");
    let pid = win::reg_dword(ACTIVE, "pid").unwrap_or(0);
    let ours = Path::new(&dll) != valve && !dll.to_lowercase().starts_with(&valve.parent().map(|p| p.to_string_lossy().to_lowercase()).unwrap_or_default());
    if ours && !win::is_alive(pid) {
        tracing::info!("putting Valve's steamclient back in the registry");
        let _ = win::set_string(ACTIVE, "SteamClientDll64", &valve.to_string_lossy());
        let _ = win::set_dword(ACTIVE, "pid", 0);
        let _ = win::set_dword(ACTIVE, "ActiveUser", 0);
    }
}

pub fn reg_string(sub: &str, value: &str) -> Option<String> {
    win::reg_string(sub, value)
}

// -------------------------------------------- the official Steam client

/// Where the official client is, from its own registry entry.
pub fn steam_exe() -> Option<PathBuf> {
    let p = win::reg_string("Software\\Valve\\Steam", "SteamExe")?;
    let p = PathBuf::from(p.replace('/', "\\"));
    p.is_file().then_some(p)
}

fn steam_running() -> bool {
    win::reg_dword(ACTIVE, "pid").is_some_and(win::is_steam_process)
}

fn signed_in() -> bool {
    steam_running() && win::reg_dword(ACTIVE, "ActiveUser").unwrap_or(0) != 0
}

fn app_running(app_id: AppId) -> bool {
    win::reg_dword(&format!("Software\\Valve\\Steam\\Apps\\{app_id}"), "Running").unwrap_or(0) == 1
}

fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    f()
}

/// Run a game through the official client (started with `-silent` if it
/// isn't running) and block until the game closes. `started` is called once
/// Steam reports the game running. `steam_id` is the SteamInRust account:
/// Steam signs in to it by itself when it remembers it. With `offline`,
/// Steam starts in its own offline mode (if it isn't running already).
/// Errors are for the player to read.
pub fn run_through_steam(app_id: AppId, args: &str, steam_id: u64, offline: bool, started: impl FnOnce()) -> Result<(), String> {
    swap_restore_now();
    // However this ends, Steam's sign-in settings go back once it closes.
    struct RestoreLogin;
    impl Drop for RestoreLogin {
        fn drop(&mut self) {
            restore_login_later();
        }
    }
    let _restore = RestoreLogin;
    let exe = steam_exe().ok_or_else(|| "This game uses anti-cheat and runs through the official Steam client, which isn't installed. Install Steam from store.steampowered.com, sign in, then press Play again.".to_string())?;
    if offline && !steam_running() && remembered_account(steam_id).is_none() {
        return Err("This game uses anti-cheat and runs through the official Steam client. Steam can only start offline for an account it remembers: sign in to Steam once with \"Remember me\" while online.".into());
    }
    if !steam_running() {
        start_steam(&exe, steam_id, offline)?;
    }
    if !wait_until(Duration::from_secs(20), signed_in) && win::reg_dword(ACTIVE, "ActiveUser").unwrap_or(0) == 0 {
        // Waiting at the account picker or the sign-in window: nobody is
        // signed in, so restarting it signed in to this account loses nothing.
        if remembered_account(steam_id).is_some() {
            tracing::info!("Steam is waiting for an account; restarting it signed in to this one");
            quiet(Command::new(&exe).arg("-shutdown")).spawn().map_err(|e| format!("Couldn't restart Steam: {e}"))?;
            wait_until(Duration::from_secs(60), || !steam_running());
            start_steam(&exe, steam_id, offline)?;
        }
    }
    if !wait_until(Duration::from_secs(60), signed_in) {
        return Err("Steam is running but not signed in. Sign in to the Steam window, then press Play again.".into());
    }
    restore_login_user();
    let mut cmd = Command::new(&exe);
    cmd.arg("-applaunch").arg(app_id.to_string()).args(split_args(args));
    quiet(&mut cmd).spawn().map_err(|e| format!("Couldn't ask Steam to start the game: {e}"))?;
    // Steam may update the game first; give it a while.
    if !wait_until(Duration::from_secs(600), || app_running(app_id)) {
        return Err("Steam didn't start the game. Check the Steam window for a message.".into());
    }
    started();
    while app_running(app_id) {
        std::thread::sleep(Duration::from_secs(2));
    }
    Ok(())
}

fn start_steam(exe: &Path, steam_id: u64, offline: bool) -> Result<(), String> {
    match auto_login(steam_id, offline) {
        Some(account) => tracing::info!("starting the official Steam client in the background, signed in as {account}"),
        None => tracing::info!("starting the official Steam client in the background (it doesn't remember this account)"),
    }
    quiet(Command::new(exe).arg("-silent")).spawn().map_err(|e| format!("Couldn't start Steam: {e}"))?;
    if !wait_until(Duration::from_secs(90), steam_running) {
        return Err("Steam didn't start. Open Steam yourself, then press Play again.".into());
    }
    Ok(())
}

/// Steam's output isn't ours to show.
fn quiet(cmd: &mut Command) -> &mut Command {
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
}

fn steam_config(file: &str) -> Option<PathBuf> {
    let root = win::reg_string("Software\\Valve\\Steam", "SteamPath")?;
    Some(PathBuf::from(root.replace('/', "\\")).join("config").join(file))
}

/// The account's login name, if the official client remembers its login
/// ("Remember me" in `config/loginusers.vdf`).
fn remembered_account(steam_id: u64) -> Option<String> {
    let text = std::fs::read_to_string(steam_config("loginusers.vdf")?).ok()?;
    let kv = vdf::parse(&text).ok()?;
    let user = kv.obj("users")?.obj(&steam_id.to_string())?;
    (user.str("RememberPassword") == Some("1")).then(|| user.str("AccountName").map(String::from)).flatten()
}

/// Make the official client sign in to this account when it starts, the
/// way it does after you pick it in its account list. Steam must not be
/// running. Only accounts Steam already remembers; no password or token is
/// ever handed over. Returns the account name when set up.
///
/// The changes are temporary: what they replaced is saved to disk first,
/// `AutoLoginUser` goes back once Steam has signed in
/// ([`restore_login_user`]), the two files once Steam has closed
/// ([`restore_login_later`]); Steam rewrites them on exit, so earlier would
/// be undone.
fn auto_login(steam_id: u64, offline: bool) -> Option<String> {
    let account = remembered_account(steam_id)?;
    let users_path = steam_config("loginusers.vdf")?;
    let config_path = steam_config("config.vdf");
    let users = std::fs::read_to_string(&users_path).ok()?;
    let config = config_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let (new_users, new_config, mut changes) = plan_auto_login(&users, &config, steam_id, offline);
    if offline {
        tracing::info!("asking Steam to start in its offline mode");
    }
    let before_user = win::reg_string("Software\\Valve\\Steam", "AutoLoginUser");
    if before_user.as_deref() != Some(account.as_str()) {
        changes.auto_login_user = Some((before_user, account.clone()));
    }
    // Remember first, change second: a crash in between can only leave
    // nothing to undo.
    save_pending(&merge_changes(load_pending(), changes));
    win::set_string("Software\\Valve\\Steam", "AutoLoginUser", &account).ok()?;
    if new_users != users {
        let _ = std::fs::write(&users_path, new_users);
    }
    if let (Some(p), true) = (config_path, new_config != config) {
        tracing::info!("turning off Steam's account picker until Steam closes");
        let _ = std::fs::write(p, new_config);
    }
    Some(account)
}

/// What auto sign-in changed, to put back later. Each entry is
/// (value before, value we wrote); a value is only put back while it still
/// is ours, so anything the player changes in between stays.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct LoginChanges {
    /// Registry `AutoLoginUser` (`None` before = it didn't exist).
    auto_login_user: Option<(Option<String>, String)>,
    /// `loginusers.vdf` `AutoLogin` per account: (steam id, before, ours).
    auto_login: Vec<(String, String, String)>,
    /// `config.vdf` `AlwaysShowUserChooser`.
    chooser: Option<(String, String)>,
    /// `loginusers.vdf` `WantsOfflineMode` / `SkipOfflineModeWarning` for a
    /// game started while SteamInRust is offline: (key, steam id, before, ours).
    #[serde(default)]
    offline: Vec<(String, String, String, String)>,
}

impl LoginChanges {
    fn is_empty(&self) -> bool {
        self.auto_login_user.is_none() && self.auto_login.is_empty() && self.chooser.is_none() && self.offline.is_empty()
    }
}

/// Steam's own switches for starting in offline mode without asking.
const OFFLINE_KEYS: [&str; 2] = ["WantsOfflineMode", "SkipOfflineModeWarning"];

/// The two files with auto sign-in for `steam_id`, and what changed.
fn plan_auto_login(users: &str, config: &str, steam_id: u64, offline: bool) -> (String, String, LoginChanges) {
    let id = steam_id.to_string();
    let mut new_users = edit_key(users, "AutoLogin", |block, _| Some(if block == id { "1" } else { "0" }.to_string()));
    let mut offline_changes = Vec::new();
    if offline {
        for key in OFFLINE_KEYS {
            let before = key_values(&new_users, key).into_iter().find(|(b, _)| *b == id).map(|(_, v)| v);
            if let Some(before) = before.filter(|v| v != "1") {
                new_users = edit_key(&new_users, key, |block, _| (block == id).then(|| "1".to_string()));
                offline_changes.push((key.to_string(), id.clone(), before, "1".to_string()));
            }
        }
    }
    let new_config = edit_key(config, "AlwaysShowUserChooser", |_, _| Some("0".to_string()));
    let auto_login = key_values(users, "AutoLogin")
        .into_iter()
        .zip(key_values(&new_users, "AutoLogin"))
        .filter(|((_, before), (_, ours))| before != ours)
        .map(|((block, before), (_, ours))| (block, before, ours))
        .collect();
    let chooser = key_values(config, "AlwaysShowUserChooser").into_iter().next().map(|(_, v)| v).filter(|v| v != "0").map(|before| (before, "0".to_string()));
    (new_users, new_config, LoginChanges { auto_login_user: None, auto_login, chooser, offline: offline_changes })
}

/// Put back what [`plan_auto_login`] changed, where it's still ours.
fn undo_auto_login(users: &str, config: &str, c: &LoginChanges) -> (String, String) {
    let mut users = edit_key(users, "AutoLogin", |block, cur| {
        c.auto_login.iter().find(|(b, _, ours)| b == block && ours == cur).map(|(_, before, _)| before.clone())
    });
    for key in OFFLINE_KEYS {
        users = edit_key(&users, key, |block, cur| {
            c.offline.iter().find(|(k, b, _, ours)| k == key && b == block && ours == cur).map(|(_, _, before, _)| before.clone())
        });
    }
    let config = edit_key(config, "AlwaysShowUserChooser", |_, cur| c.chooser.as_ref().filter(|(_, ours)| ours == cur).map(|(before, _)| before.clone()));
    (users, config)
}

/// A second sign-in before the first was undone keeps the first "before".
fn merge_changes(old: Option<LoginChanges>, new: LoginChanges) -> LoginChanges {
    let Some(mut m) = old else { return new };
    match (&mut m.auto_login_user, new.auto_login_user) {
        (Some(o), Some(n)) => o.1 = n.1,
        (o @ None, n) => *o = n,
        _ => {}
    }
    for (block, before, ours) in new.auto_login {
        match m.auto_login.iter_mut().find(|(b, _, _)| *b == block) {
            Some(e) => e.2 = ours,
            None => m.auto_login.push((block, before, ours)),
        }
    }
    match (&mut m.chooser, new.chooser) {
        (Some(o), Some(n)) => o.1 = n.1,
        (o @ None, n) => *o = n,
        _ => {}
    }
    for (key, block, before, ours) in new.offline {
        match m.offline.iter_mut().find(|(k, b, _, _)| *k == key && *b == block) {
            Some(e) => e.3 = ours,
            None => m.offline.push((key, block, before, ours)),
        }
    }
    m
}

/// `"key" "value"` pairs in a KeyValues file, with the Steam id of the
/// account block they're in ("" outside one).
fn key_values(text: &str, key: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut block = String::new();
    for line in text.lines() {
        if let Some(id) = account_block(line) {
            block = id;
        }
        if let Some(v) = value_of(line, key) {
            out.push((block.clone(), v));
        }
    }
    out
}

/// Rewrite the value of `key` on every line where `f(block, current)` says
/// so; everything else stays byte for byte.
fn edit_key(text: &str, key: &str, mut f: impl FnMut(&str, &str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut block = String::new();
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        if let Some(id) = account_block(body) {
            block = id;
        }
        match value_of(body, key).and_then(|cur| f(&block, &cur).filter(|new| *new != cur).map(|new| (cur, new))) {
            Some((cur, new)) => {
                let at = body.rfind(&format!("\"{cur}\"")).unwrap_or(body.len());
                out.push_str(&body[..at]);
                out.push_str(&format!("\"{new}\""));
                out.push_str(&line[at + cur.len() + 2..]);
            }
            None => out.push_str(line),
        }
    }
    out
}

/// `"7656…"` on its own line opens an account block in loginusers.vdf.
fn account_block(line: &str) -> Option<String> {
    let t = line.trim();
    let inner = t.strip_prefix('"')?.strip_suffix('"')?;
    (inner.len() == 17 && inner.starts_with("7656") && inner.bytes().all(|b| b.is_ascii_digit())).then(|| inner.to_string())
}

fn value_of(line: &str, key: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix(&format!("\"{key}\""))?;
    let v = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
    Some(v.to_string())
}

fn pending_path() -> PathBuf {
    crate::store::config_dir().join("steam-sign-in-restore.json")
}

fn load_pending() -> Option<LoginChanges> {
    serde_json::from_str(&std::fs::read_to_string(pending_path()).ok()?).ok()
}

fn save_pending(c: &LoginChanges) {
    let path = pending_path();
    if c.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(path, json);
    }
}

static PENDING_LOCK: Mutex<()> = Mutex::new(());

/// Steam has signed in: `AutoLoginUser` goes back (Steam only reads it at start).
fn restore_login_user() {
    let _l = PENDING_LOCK.lock().unwrap();
    let Some(mut c) = load_pending() else { return };
    if let Some((before, ours)) = c.auto_login_user.take() {
        if win::reg_string("Software\\Valve\\Steam", "AutoLoginUser").as_deref() == Some(ours.as_str()) {
            let _ = match before {
                Some(v) => win::set_string("Software\\Valve\\Steam", "AutoLoginUser", &v),
                None => win::delete_value("Software\\Valve\\Steam", "AutoLoginUser"),
            };
        }
        save_pending(&c);
    }
}

/// Put everything back if Steam is closed. True when nothing is left to do.
fn restore_login_files() -> bool {
    let _l = PENDING_LOCK.lock().unwrap();
    let Some(c) = load_pending() else { return true };
    if steam_running() {
        return false;
    }
    if let Some((before, ours)) = &c.auto_login_user {
        if win::reg_string("Software\\Valve\\Steam", "AutoLoginUser").as_deref() == Some(ours.as_str()) {
            let _ = match before {
                Some(v) => win::set_string("Software\\Valve\\Steam", "AutoLoginUser", v),
                None => win::delete_value("Software\\Valve\\Steam", "AutoLoginUser"),
            };
        }
    }
    let read = |f: &str| steam_config(f).and_then(|p| Some((std::fs::read_to_string(&p).ok()?, p)));
    let (users, config) = (read("loginusers.vdf"), read("config.vdf"));
    let (u_text, c_text) = (users.as_ref().map(|u| u.0.as_str()).unwrap_or(""), config.as_ref().map(|c| c.0.as_str()).unwrap_or(""));
    let (new_users, new_config) = undo_auto_login(u_text, c_text, &c);
    if let Some((old, p)) = &users {
        if *old != new_users {
            let _ = std::fs::write(p, &new_users);
        }
    }
    if let Some((old, p)) = &config {
        if *old != new_config {
            let _ = std::fs::write(p, &new_config);
        }
    }
    tracing::info!("put Steam's sign-in settings back as they were");
    save_pending(&LoginChanges::default());
    true
}

/// Restore the sign-in settings as soon as Steam is closed: now, or from a
/// background thread that waits for Steam to exit (one at a time).
pub fn restore_login_later() {
    static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if restore_login_files() || WATCHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        while !restore_login_files() {
            std::thread::sleep(Duration::from_secs(5));
        }
        WATCHING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

// ------------------------------------------------------------ Win32

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};

    const HKCU: isize = 0x8000_0001u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x2;
    const RRF_RT_REG_DWORD: u32 = 0x10;
    const REG_SZ: u32 = 1;
    const REG_DWORD: u32 = 4;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const TH32CS_SNAPPROCESS: u32 = 0x2;

    #[repr(C)]
    struct ProcessEntry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        exe: [u16; 260],
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
        fn RegSetKeyValueW(key: isize, sub: *const u16, value: *const u16, kind: u32, data: *const c_void, len: u32) -> i32;
        fn RegDeleteKeyValueW(key: isize, sub: *const u16, value: *const u16) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
        fn QueryFullProcessImageNameW(process: *mut c_void, flags: u32, name: *mut u16, len: *mut u32) -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snap: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snap: *mut c_void, entry: *mut ProcessEntry) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn reg_string(sub: &str, value: &str) -> Option<String> {
        let (sub, value) = (wide(sub), wide(value));
        let mut buf = vec![0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe { RegGetValueW(HKCU, sub.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
        (r == 0).then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1).min(buf.len())]))
    }

    pub fn reg_dword(sub: &str, value: &str) -> Option<u32> {
        let (sub, value) = (wide(sub), wide(value));
        let mut v = 0u32;
        let mut len = 4u32;
        let r = unsafe { RegGetValueW(HKCU, sub.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut v as *mut u32).cast(), &mut len) };
        (r == 0).then_some(v)
    }

    pub fn set_string(sub: &str, value: &str, data: &str) -> Result<(), String> {
        let (s, v, d) = (wide(sub), wide(value), wide(data));
        let r = unsafe { RegSetKeyValueW(HKCU, s.as_ptr(), v.as_ptr(), REG_SZ, d.as_ptr().cast(), (d.len() * 2) as u32) };
        if r == 0 { Ok(()) } else { Err(format!("registry {sub}\\{value}: error {r}")) }
    }

    pub fn set_dword(sub: &str, value: &str, data: u32) -> Result<(), String> {
        let (s, v) = (wide(sub), wide(value));
        let r = unsafe { RegSetKeyValueW(HKCU, s.as_ptr(), v.as_ptr(), REG_DWORD, (&data as *const u32).cast(), 4) };
        if r == 0 { Ok(()) } else { Err(format!("registry {sub}\\{value}: error {r}")) }
    }

    pub fn delete_value(sub: &str, value: &str) -> Result<(), String> {
        let (s, v) = (wide(sub), wide(value));
        unsafe { RegDeleteKeyValueW(HKCU, s.as_ptr(), v.as_ptr()) };
        Ok(())
    }

    fn image(pid: u32) -> Option<(bool, String)> {
        if pid == 0 {
            return None;
        }
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut code = 0u32;
            let alive = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let name = if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0 { String::from_utf16_lossy(&buf[..len as usize]) } else { String::new() };
            CloseHandle(h);
            Some((alive, name))
        }
    }

    pub fn is_alive(pid: u32) -> bool {
        image(pid).is_some_and(|(alive, _)| alive)
    }

    pub fn is_steam_process(pid: u32) -> bool {
        image(pid).is_some_and(|(alive, name)| alive && name.to_lowercase().ends_with("\\steam.exe"))
    }

    pub fn processes_in(dir: &Path) -> Vec<u32> {
        let want = dir.to_string_lossy().to_lowercase();
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap as isize == -1 {
                return out;
            }
            let mut e: ProcessEntry = std::mem::zeroed();
            e.size = std::mem::size_of::<ProcessEntry>() as u32;
            let mut ok = Process32FirstW(snap, &mut e) != 0;
            while ok {
                if let Some((true, name)) = image(e.pid) {
                    if PathBuf::from(&name).starts_with(dir) || name.to_lowercase().starts_with(&want) {
                        out.push(e.pid);
                    }
                }
                ok = Process32NextW(snap, &mut e) != 0;
            }
            CloseHandle(snap);
        }
        out
    }
}

#[cfg(not(windows))]
mod win {
    use std::path::Path;
    pub fn reg_string(_: &str, _: &str) -> Option<String> {
        None
    }
    pub fn reg_dword(_: &str, _: &str) -> Option<u32> {
        None
    }
    pub fn set_string(_: &str, _: &str, _: &str) -> Result<(), String> {
        Err("no registry".into())
    }
    pub fn set_dword(_: &str, _: &str, _: u32) -> Result<(), String> {
        Err("no registry".into())
    }
    pub fn delete_value(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn is_alive(_: u32) -> bool {
        false
    }
    pub fn is_steam_process(_: u32) -> bool {
        false
    }
    pub fn processes_in(_: &Path) -> Vec<u32> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_args() {
        assert_eq!(split_args(r#"-a "b c" d"#), vec!["-a", "b c", "d"]);
    }

    fn app(categories: Vec<u32>, exe: &str) -> OwnedApp {
        OwnedApp {
            app_id: 1,
            name: "Test".into(),
            categories,
            launch: vec![model::LaunchOption { executable: exe.into(), arguments: String::new(), working_dir: String::new(), description: String::new(), os_list: "windows".into() }],
            ..Default::default()
        }
    }

    #[test]
    fn detects_anti_cheat() {
        let dir = std::env::temp_dir().join(format!("sir-ac-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        // VAC comes from the store category.
        assert_eq!(anti_cheat(&app(vec![8], "game.exe"), &dir).as_deref(), Some("Valve Anti-Cheat"));
        // Clean folder: none.
        assert_eq!(anti_cheat(&app(vec![2, 22], "game.exe"), &dir), None);
        // Launch option through EAC's bootstrapper.
        assert_eq!(anti_cheat(&app(vec![], "start_protected_game.exe"), &dir).as_deref(), Some("Easy Anti-Cheat"));
        // A BattlEye folder a level down.
        std::fs::create_dir_all(dir.join("bin").join("BattlEye")).unwrap();
        assert_eq!(anti_cheat(&app(vec![], "game.exe"), &dir).as_deref(), Some("BattlEye"));
        // Unreadable folder: treated as anti-cheat.
        assert!(anti_cheat(&app(vec![], "game.exe"), &dir.join("missing")).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    const USERS: &str = "\"users\"\r\n{\r\n\t\"76561198000000001\"\r\n\t{\r\n\t\t\"AccountName\"\t\t\"a\"\r\n\t\t\"AutoLogin\"\t\t\"1\"\r\n\t}\r\n\t\"76561198000000002\"\r\n\t{\r\n\t\t\"AccountName\"\t\t\"b\"\r\n\t\t\"AutoLogin\"\t\t\"0\"\r\n\t}\r\n}\r\n";
    const CONFIG: &str = "\"InstallConfigStore\"\n{\n\t\"AlwaysShowUserChooser\"\t\t\"1\"\n\t\"Other\"\t\t\"1\"\n}\n";

    #[test]
    fn auto_login_edits() {
        let (users, config, changes) = plan_auto_login(USERS, CONFIG, 76561198000000002, false);
        let kv = vdf::parse(&users).unwrap();
        let u = kv.obj("users").unwrap();
        assert_eq!(u.obj("76561198000000001").unwrap().str("AutoLogin"), Some("0"));
        assert_eq!(u.obj("76561198000000002").unwrap().str("AutoLogin"), Some("1"));
        assert!(users.contains("\r\n"), "line endings kept");
        assert!(config.contains("\"AlwaysShowUserChooser\"\t\t\"0\""));
        assert!(config.contains("\"Other\"\t\t\"1\""));
        assert_eq!(changes.auto_login.len(), 2);
        assert_eq!(changes.chooser, Some(("1".into(), "0".into())));
    }

    #[test]
    fn auto_login_restores_exactly() {
        let (users, config, changes) = plan_auto_login(USERS, CONFIG, 76561198000000002, false);
        // The changes survive a trip through the file on disk.
        let changes: LoginChanges = serde_json::from_str(&serde_json::to_string(&changes).unwrap()).unwrap();
        let (u, c) = undo_auto_login(&users, &config, &changes);
        assert_eq!(u, USERS);
        assert_eq!(c, CONFIG);
        // Values the player changed meanwhile stay as they are.
        let mine = config.replace("\"AlwaysShowUserChooser\"\t\t\"0\"", "\"AlwaysShowUserChooser\"\t\t\"2\"");
        let (_, c) = undo_auto_login(&users, &mine, &changes);
        assert!(c.contains("\"AlwaysShowUserChooser\"\t\t\"2\""));
        // A second sign-in before the undo keeps the very first values.
        let (users2, config2, again) = plan_auto_login(&users, &config, 76561198000000001, false);
        let merged = merge_changes(Some(changes), again);
        let (u, c) = undo_auto_login(&users2, &config2, &merged);
        assert_eq!(u, USERS);
        assert_eq!(c, CONFIG);
        // Nothing to do when everything already matches.
        let (_, _, none) = plan_auto_login(&users, &config, 76561198000000002, false);
        assert!(none.is_empty());
    }

    #[test]
    fn offline_mode_edits_and_restores() {
        let users = USERS.replace("\"AutoLogin\"\t\t\"0\"", "\"AutoLogin\"\t\t\"0\"\r\n\t\t\"WantsOfflineMode\"\t\t\"0\"\r\n\t\t\"SkipOfflineModeWarning\"\t\t\"0\"");
        let (new_users, config, changes) = plan_auto_login(&users, CONFIG, 76561198000000002, true);
        let kv = vdf::parse(&new_users).unwrap();
        let b = kv.obj("users").unwrap().obj("76561198000000002").unwrap();
        assert_eq!(b.str("WantsOfflineMode"), Some("1"));
        assert_eq!(b.str("SkipOfflineModeWarning"), Some("1"));
        assert_eq!(changes.offline.len(), 2);
        let (u, _) = undo_auto_login(&new_users, &config, &changes);
        assert_eq!(u, users);
        // Online launches leave Steam's offline switches alone.
        let (_, _, online) = plan_auto_login(&users, CONFIG, 76561198000000002, false);
        assert!(online.offline.is_empty());
    }

    #[test]
    fn reads_pe_bitness() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(pe_is_64bit(&exe), Some(cfg!(target_pointer_width = "64")));
        assert_eq!(pe_is_64bit(Path::new("Cargo.toml")), None);
    }
}

//! Steam's install scripts: the "first-time setup" before a game's first
//! launch. A game (and each shared redistributable it uses, like DirectX or
//! the Visual C++ runtime in `Steamworks Shared`) can ship `installscript.vdf`
//! files that run installers, write registry values and add firewall rules.
//!
//! Like Steam: the scripts are found through the app manifests
//! (`InstallScripts`, and `SharedDepots` for redistributables), anything
//! already done is skipped (a `HasRunKey` value Steam and we both write, an
//! existing registry value, or our own record), and the rest runs before the
//! game starts, elevated once through UAC when it needs an administrator.
//! Windows only: Steam doesn't run install scripts on a Mac either (there the
//! downloader already restores permissions and symlinks).

use model::AppId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One thing to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// Run an installer and wait. On success, `has_run` (key, value name)
    /// is set to 1 so it never runs again.
    Run { exe: String, args: String, as_user: bool, has_run: Option<(String, String, u32)> },
    /// Set a registry value (`key` starts with the hive, e.g. HKEY_LOCAL_MACHINE).
    Registry { key: String, name: String, value: RegValue },
    /// Let a program through the Windows firewall.
    Firewall { name: String, program: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RegValue {
    Str(String),
    Dword(u32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// What it's doing, for the progress line ("DirectX", "x64 14.51…").
    pub label: String,
    pub action: Action,
    /// Identifies the step in our own "already done" record.
    pub id: String,
}

impl Step {
    /// Only our own record says whether it ran (no registry value to look at).
    fn needs_record(&self) -> bool {
        matches!(self.action, Action::Firewall { .. } | Action::Run { has_run: None, .. })
    }

    /// Needs an administrator: machine-wide registry, installers that don't
    /// ask to run as the user, firewall rules.
    pub fn needs_admin(&self) -> bool {
        match &self.action {
            Action::Run { as_user, has_run, .. } => !as_user || has_run.as_ref().is_some_and(|(k, _, _)| !is_current_user(k)),
            Action::Registry { key, .. } => !is_current_user(key),
            Action::Firewall { .. } => true,
        }
    }
}

fn is_current_user(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    k.starts_with("HKEY_CURRENT_USER") || k.starts_with("HKCU")
}

/// Where the paths in a script point (`%INSTALLDIR%` and friends).
pub struct Vars {
    pub install_dir: PathBuf,
    pub steam_path: Option<PathBuf>,
}

impl Vars {
    fn expand(&self, s: &str) -> String {
        let dir = self.install_dir.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
        let mut out = String::new();
        let mut rest = s;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let Some(end) = after.find('%') else {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let name = &after[..end];
            let value = match name.to_ascii_uppercase().as_str() {
                "INSTALLDIR" => Some(dir.clone()),
                "ROOTDRIVE" => dir.chars().next().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_string()),
                "STEAMPATH" => self.steam_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                "USER_MYDOCS" => std::env::var("USERPROFILE").ok().map(|p| format!("{p}\\Documents")),
                "COMMON_MYDOCS" => std::env::var("PUBLIC").ok().map(|p| format!("{p}\\Documents")),
                _ => std::env::var(name).ok(),
            };
            match value {
                Some(v) => out.push_str(&v),
                None => {
                    out.push('%');
                    out.push_str(name);
                    out.push('%');
                }
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        out
    }
}

/// The steps in one `installscript.vdf`, in order: registry, firewall, then
/// installers. `language` picks a registry block per language (Steam's
/// default is english); `win64` is whether this is 64-bit Windows.
pub fn parse(text: &str, vars: &Vars, language: &str, win64: bool, script_id: &str) -> Vec<Step> {
    let Ok(doc) = vdf::parse(text) else { return Vec::new() };
    let Some(root) = doc.obj("InstallScript") else { return Vec::new() };
    let mut steps = Vec::new();
    if let Some(reg) = root.obj("Registry") {
        for (key, v) in reg.iter() {
            let Some(types) = v.as_obj() else { continue };
            for (kind, values) in types.iter() {
                let Some(values) = values.as_obj() else { continue };
                // Either name/value pairs, or one block per language.
                let has_langs = values.iter().any(|(_, v)| v.as_obj().is_some());
                let chosen = if has_langs { values.obj(language).or_else(|| values.obj("english")) } else { Some(values) };
                let Some(chosen) = chosen else { continue };
                for (name, value) in chosen.iter() {
                    let Some(value) = value.as_str() else { continue };
                    let value = match kind.to_ascii_lowercase().as_str() {
                        "dword" => RegValue::Dword(value.trim().parse().unwrap_or(0)),
                        _ => RegValue::Str(vars.expand(value)),
                    };
                    let name = if name.eq_ignore_ascii_case("(Default)") { String::new() } else { name.to_string() };
                    steps.push(Step {
                        label: key.rsplit('\\').next().unwrap_or(key).to_string(),
                        id: format!("{script_id}|reg|{key}|{name}"),
                        action: Action::Registry { key: key.to_string(), name, value },
                    });
                }
            }
        }
    }
    if let Some(fw) = root.obj("Firewall") {
        for (name, program) in fw.iter() {
            let Some(program) = program.as_str() else { continue };
            steps.push(Step { label: name.to_string(), id: format!("{script_id}|fw|{name}"), action: Action::Firewall { name: name.to_string(), program: vars.expand(program) } });
        }
    }
    if let Some(run) = root.obj("Run Process") {
        for (name, v) in run.iter() {
            let Some(e) = v.as_obj() else { continue };
            if let Some(req) = e.obj("Requirement_OS") {
                if let Some(want) = req.str("Is64BitWindows") {
                    if (want == "1") != win64 {
                        continue;
                    }
                }
            }
            let has_run = e.str("HasRunKey").map(|k| (k.to_string(), name.to_string(), e.u64("MinimumHasRunValue").unwrap_or(1) as u32));
            // "process 1"/"command 1", and more numbered pairs if present.
            for n in 1..=9 {
                let Some(exe) = e.str(&format!("Process {n}")) else { break };
                let args = e.str(&format!("Command {n}")).map(|c| vars.expand(c)).unwrap_or_default();
                steps.push(Step {
                    label: name.to_string(),
                    id: format!("{script_id}|run|{name}|{n}"),
                    action: Action::Run { exe: vars.expand(exe), args, as_user: e.str("AsCurrentUser") == Some("1"), has_run: has_run.clone() },
                });
            }
        }
    }
    steps
}

/// A game's install scripts: its own (listed in its app manifest under
/// `InstallScripts`) and those of the shared redistributables it uses
/// (`SharedDepots`, whose scripts the redistributables' app manifest lists).
/// Returns (script file, folder `%INSTALLDIR%` stands for).
pub fn scripts(library: &Path, app_id: AppId) -> Vec<(PathBuf, PathBuf)> {
    let apps = library.join("steamapps");
    let read = |id: AppId| std::fs::read_to_string(apps.join(format!("appmanifest_{id}.acf"))).ok().and_then(|t| vdf::parse(&t).ok());
    let Some(doc) = read(app_id) else { return Vec::new() };
    let Some(state) = doc.obj("AppState") else { return Vec::new() };
    let dir_of = |s: &vdf::Obj| apps.join("common").join(s.str("installdir").unwrap_or_default());
    let mut out = Vec::new();
    let game_dir = dir_of(state);
    for (_, rel) in state.obj("InstallScripts").map(|o| o.iter().collect::<Vec<_>>()).unwrap_or_default() {
        if let Some(rel) = rel.as_str() {
            out.push((game_dir.join(rel.replace('\\', "/")), game_dir.clone()));
        }
    }
    // depot -> the app it comes from (228980 for Steam's redistributables).
    let mut by_app: std::collections::BTreeMap<AppId, Vec<String>> = Default::default();
    for (depot, from) in state.obj("SharedDepots").map(|o| o.iter().collect::<Vec<_>>()).unwrap_or_default() {
        if let Some(from) = from.as_str().and_then(|f| f.parse().ok()) {
            by_app.entry(from).or_default().push(depot.to_string());
        }
    }
    for (from, depots) in by_app {
        let Some(doc) = read(from) else { continue };
        let Some(shared) = doc.obj("AppState") else { continue };
        let dir = dir_of(shared);
        let Some(list) = shared.obj("InstallScripts") else { continue };
        for d in depots {
            if let Some(rel) = list.str(&d) {
                out.push((dir.join(rel.replace('\\', "/")), dir.clone()));
            }
        }
    }
    out.retain(|(p, _)| p.is_file());
    out
}

/// Steps of a game's install scripts not done on this PC yet. Empty off
/// Windows (Steam doesn't run install scripts there either).
pub fn pending(library: &Path, app_id: AppId, steam_path: Option<PathBuf>) -> Vec<Step> {
    if !cfg!(windows) {
        return Vec::new();
    }
    // The game's language picks per-language registry values (Steam's default: english).
    let language = std::fs::read_to_string(library.join("steamapps").join(format!("appmanifest_{app_id}.acf")))
        .ok()
        .and_then(|t| vdf::parse(&t).ok())
        .and_then(|d| d.path(&["AppState", "UserConfig"]).and_then(|u| u.str("language")).map(String::from))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "english".into());
    let language = language.as_str();
    let done = load_done();
    let win64 = cfg!(target_pointer_width = "64") || std::env::var_os("PROCESSOR_ARCHITEW6432").is_some();
    let mut out = Vec::new();
    for (script, dir) in scripts(library, app_id) {
        let Ok(text) = std::fs::read_to_string(&script) else { continue };
        let id = format!("{}#{:x}", script.to_string_lossy().to_lowercase(), fnv(&text));
        let vars = Vars { install_dir: dir, steam_path: steam_path.clone() };
        for step in parse(&text, &vars, language, win64, &id) {
            if !is_done(&step, &done) {
                out.push(step);
            }
        }
    }
    out
}

/// Steps for the Windows version of a game on a Mac (Steam Play): paths as
/// Wine sees them (`Z:\...`), 64-bit Windows, and `done` = the step ids
/// that already ran in the game's Wine prefix.
pub fn pending_windows(library: &Path, app_id: AppId, done: &std::collections::HashSet<String>) -> Vec<Step> {
    let mut out = Vec::new();
    for (script, dir) in scripts(library, app_id) {
        let Ok(text) = std::fs::read_to_string(&script) else { continue };
        let id = format!("{}#{:x}", script.to_string_lossy().to_lowercase(), fnv(&text));
        let vars = Vars { install_dir: PathBuf::from(crate::compat::windows_path(&dir)), steam_path: None };
        out.extend(parse(&text, &vars, "english", true, &id).into_iter().filter(|s| !done.contains(&s.id)));
    }
    out
}

fn is_done(step: &Step, done: &std::collections::HashSet<String>) -> bool {
    match &step.action {
        Action::Run { has_run: Some((key, name, min)), .. } => reg::dword(key, name).is_some_and(|v| v >= *min),
        Action::Registry { key, name, value } => match (value, reg::dword(key, name), reg::string(key, name)) {
            (RegValue::Dword(want), Some(have), _) => *want == have,
            (RegValue::Str(want), _, Some(have)) => *want == have,
            _ => false,
        },
        _ => done.contains(&step.id),
    }
}

/// A small, stable hash: a changed script runs again.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn done_path() -> PathBuf {
    crate::store::config_dir().join("installscripts.json")
}

fn load_done() -> std::collections::HashSet<String> {
    std::fs::read_to_string(done_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn mark_done(ids: impl IntoIterator<Item = String>) {
    let mut done = load_done();
    done.extend(ids);
    if let Ok(json) = serde_json::to_string(&done) {
        let _ = std::fs::create_dir_all(crate::store::config_dir());
        let _ = std::fs::write(done_path(), json);
    }
}

/// Run the steps. Those needing an administrator run in one elevated copy of
/// this program (a single UAC prompt), the rest here. `progress(i, label)`
/// is called as each step starts.
pub fn run(steps: &[Step], progress: &(dyn Fn(usize, &str) + Sync)) -> Result<(), String> {
    let (admin, user): (Vec<(usize, &Step)>, Vec<(usize, &Step)>) = steps.iter().enumerate().partition(|(_, s)| s.needs_admin());
    let mut errors = Vec::new();
    for (i, s) in &user {
        progress(*i, &s.label);
        if let Err(e) = run_step(s) {
            errors.push(format!("{}: {e}", s.label));
        } else if s.needs_record() {
            mark_done([s.id.clone()]);
        }
    }
    if !admin.is_empty() {
        let list: Vec<Step> = admin.iter().map(|(_, s)| (*s).clone()).collect();
        let firsts: Vec<usize> = admin.iter().map(|(i, _)| *i).collect();
        match elevated(&list, &|n, label| progress(firsts.get(n).copied().unwrap_or(0), label)) {
            Ok(failed) => {
                mark_done(list.iter().filter(|s| s.needs_record() && !failed.iter().any(|(id, _)| id == &s.id)).map(|s| s.id.clone()));
                errors.extend(failed.into_iter().map(|(_, e)| e));
            }
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

/// The elevated copy: `steam-rs --install-script-steps <file>`. Runs the
/// steps in the file, writing progress next to it. Returns the exit code.
pub fn helper_main(file: &Path) -> i32 {
    let Ok(text) = std::fs::read_to_string(file) else { return 2 };
    let Ok(steps) = serde_json::from_str::<Vec<Step>>(&text) else { return 2 };
    let mut failed: Vec<(String, String)> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let _ = std::fs::write(file.with_extension("progress"), i.to_string());
        if let Err(e) = run_step(s) {
            failed.push((s.id.clone(), format!("{}: {e}", s.label)));
        }
    }
    let _ = std::fs::write(file.with_extension("result"), serde_json::to_string(&failed).unwrap_or_default());
    0
}

fn run_step(s: &Step) -> Result<(), String> {
    match &s.action {
        Action::Registry { key, name, value } => reg::set(key, name, value),
        Action::Firewall { name, program } => {
            let status = std::process::Command::new("netsh")
                .args(["advfirewall", "firewall", "add", "rule", &format!("name={name}"), "dir=in", "action=allow", &format!("program={program}"), "enable=yes"])
                .status()
                .map_err(|e| e.to_string())?;
            if status.success() { Ok(()) } else { Err(format!("netsh exited with {status}")) }
        }
        Action::Run { exe, args, has_run, .. } => {
            let code = run_process(exe, args)?;
            // 0: done; 3010/1641: done, restart later; 1638: a newer one is
            // already installed (Visual C++ installers).
            if ![0, 1638, 1641, 3010].contains(&code) {
                return Err(format!("exited with code {code}"));
            }
            if let Some((key, name, _)) = has_run {
                reg::set(key, name, &RegValue::Dword(1))?;
            }
            Ok(())
        }
    }
}

fn run_process(exe: &str, args: &str) -> Result<i32, String> {
    let path = Path::new(exe);
    if !path.exists() {
        return Err(format!("{exe} not found"));
    }
    let lower = exe.to_ascii_lowercase();
    let mut cmd = if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(exe);
        c
    } else if lower.ends_with(".msi") {
        let mut c = std::process::Command::new("msiexec");
        c.arg("/i").arg(exe);
        c
    } else {
        std::process::Command::new(exe)
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if !args.is_empty() {
            cmd.raw_arg(args);
        }
    }
    #[cfg(not(windows))]
    if !args.is_empty() {
        cmd.args(args.split_whitespace());
    }
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    let status = cmd.status().map_err(|e| e.to_string())?;
    Ok(status.code().unwrap_or(-1))
}

/// Run steps in an elevated copy of this program and wait. Returns the
/// steps that failed (id, message).
#[cfg(windows)]
fn elevated(steps: &[Step], progress: &dyn Fn(usize, &str)) -> Result<Vec<(String, String)>, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    let dir = std::env::temp_dir().join(format!("SteamInRust-setup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join("steps.json");
    std::fs::write(&file, serde_json::to_string(steps).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, file_w, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(&format!("--install-script-steps \"{}\"", file.display())));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file_w.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = 0; // hidden
    if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess.is_null() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("Administrator permission wasn't given".into());
    }
    let mut shown = usize::MAX;
    loop {
        let r = unsafe { WaitForSingleObject(info.hProcess, 300) };
        if let Some(i) = std::fs::read_to_string(file.with_extension("progress")).ok().and_then(|t| t.trim().parse::<usize>().ok()) {
            if i != shown {
                shown = i;
                progress(i, steps.get(i).map(|s| s.label.as_str()).unwrap_or_default());
            }
        }
        if r != WAIT_TIMEOUT {
            break;
        }
    }
    let mut code = 0u32;
    unsafe {
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
    }
    let failed = std::fs::read_to_string(file.with_extension("result")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let _ = std::fs::remove_dir_all(&dir);
    match failed {
        Some(f) => Ok(f),
        None => Err(format!("first-time setup stopped (code {code})")),
    }
}

#[cfg(not(windows))]
fn elevated(_steps: &[Step], _progress: &dyn Fn(usize, &str)) -> Result<Vec<(String, String)>, String> {
    Err("install scripts only run on Windows".into())
}

/// The registry, in the 32-bit view Steam (a 32-bit program) uses, so we
/// read and write the same `HasRunKey` values it does.
#[cfg(windows)]
mod reg {
    use super::RegValue;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::*;

    fn split(key: &str) -> Option<(HKEY, Vec<u16>)> {
        let (hive, rest) = key.split_once('\\').unwrap_or((key, ""));
        let root = match hive.to_ascii_uppercase().as_str() {
            "HKEY_LOCAL_MACHINE" | "HKLM" => HKEY_LOCAL_MACHINE,
            "HKEY_CURRENT_USER" | "HKCU" => HKEY_CURRENT_USER,
            "HKEY_CLASSES_ROOT" | "HKCR" => HKEY_CLASSES_ROOT,
            "HKEY_USERS" | "HKU" => HKEY_USERS,
            _ => return None,
        };
        Some((root, rest.encode_utf16().chain(std::iter::once(0)).collect()))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn query(key: &str, name: &str) -> Option<(u32, Vec<u8>)> {
        let (root, sub) = split(key)?;
        let mut h: HKEY = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(root, sub.as_ptr(), 0, KEY_READ | KEY_WOW64_32KEY, &mut h) } != ERROR_SUCCESS {
            return None;
        }
        let name = wide(name);
        let (mut kind, mut len) = (0u32, 0u32);
        let ok = unsafe { RegQueryValueExW(h, name.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut len) } == ERROR_SUCCESS;
        let mut buf = vec![0u8; len as usize];
        let ok = ok && unsafe { RegQueryValueExW(h, name.as_ptr(), std::ptr::null(), &mut kind, buf.as_mut_ptr(), &mut len) } == ERROR_SUCCESS;
        unsafe { RegCloseKey(h) };
        ok.then_some((kind, buf))
    }

    pub fn dword(key: &str, name: &str) -> Option<u32> {
        let (kind, buf) = query(key, name)?;
        (kind == REG_DWORD && buf.len() >= 4).then(|| u32::from_le_bytes(buf[..4].try_into().unwrap()))
    }

    pub fn string(key: &str, name: &str) -> Option<String> {
        let (kind, buf) = query(key, name)?;
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return None;
        }
        let units: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
    }

    pub fn set(key: &str, name: &str, value: &RegValue) -> Result<(), String> {
        let (root, sub) = split(key).ok_or_else(|| format!("unknown registry key {key}"))?;
        let mut h: HKEY = std::ptr::null_mut();
        let r = unsafe { RegCreateKeyExW(root, sub.as_ptr(), 0, std::ptr::null(), 0, KEY_WRITE | KEY_WOW64_32KEY, std::ptr::null(), &mut h, std::ptr::null_mut()) };
        if r != ERROR_SUCCESS {
            return Err(format!("couldn't open {key} (error {r})"));
        }
        let name = wide(name);
        let r = match value {
            RegValue::Dword(v) => unsafe { RegSetValueExW(h, name.as_ptr(), 0, REG_DWORD, v.to_le_bytes().as_ptr(), 4) },
            RegValue::Str(s) => {
                let data = wide(s);
                unsafe { RegSetValueExW(h, name.as_ptr(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32) }
            }
        };
        unsafe { RegCloseKey(h) };
        if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("couldn't write {key} (error {r})")) }
    }
}

#[cfg(not(windows))]
mod reg {
    use super::RegValue;
    pub fn dword(_: &str, _: &str) -> Option<u32> {
        None
    }
    pub fn string(_: &str, _: &str) -> Option<String> {
        None
    }
    pub fn set(_: &str, _: &str, _: &RegValue) -> Result<(), String> {
        Err("no registry here".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        Vars { install_dir: PathBuf::from(r"D:\Games\steamapps\common\Steamworks Shared"), steam_path: None }
    }

    #[test]
    fn reads_steams_redistributable_scripts() {
        let vc = r#""installscript"
{
	"Run Process"
	{
		"x86 14.51.36247.0"
		{
			"hasrunkey"		"HKEY_LOCAL_MACHINE\\Software\\Valve\\Steam\\Apps\\CommonRedist\\vcredist\\2022"
			"process 1"		"%INSTALLDIR%\\_CommonRedist\\vcredist\\2022\\Microsoft Visual C++ 2022 x86.cmd"
			"nocleanup"		"1"
		}
		"x64 14.51.36247.0"
		{
			"hasrunkey"		"HKEY_LOCAL_MACHINE\\Software\\Valve\\Steam\\Apps\\CommonRedist\\vcredist\\2022"
			"process 1"		"%INSTALLDIR%\\_CommonRedist\\vcredist\\2022\\Microsoft Visual C++ 2022 x64.cmd"
			"nocleanup"		"1"
			"Requirement_OS"
			{
				"Is64BitWindows"		"1"
			}
		}
	}
}
"kvsignatures"
{
	"installscript"		"37b9"
}"#;
        let steps = parse(vc, &vars(), "english", true, "vc");
        assert_eq!(steps.len(), 2);
        let Action::Run { exe, args, as_user, has_run } = &steps[1].action else { panic!() };
        assert_eq!(exe, r"D:\Games\steamapps\common\Steamworks Shared\_CommonRedist\vcredist\2022\Microsoft Visual C++ 2022 x64.cmd");
        assert!(args.is_empty() && !as_user);
        assert_eq!(has_run.as_ref().unwrap().1, "x64 14.51.36247.0");
        assert!(steps[1].needs_admin());
        // 32-bit Windows skips the x64 one.
        assert_eq!(parse(vc, &vars(), "english", false, "vc").len(), 1);
    }

    #[test]
    fn reads_registry_and_firewall() {
        let s = r#""InstallScript"
{
	"Registry"
	{
		"HKEY_LOCAL_MACHINE\\Software\\Studio\\Game"
		{
			"string" { "english" { "Language" "English" "(Default)" "%INSTALLDIR%" } "french" { "Language" "Français" } }
			"dword" { "Version" "3" }
		}
		"HKEY_CURRENT_USER\\Software\\Studio\\Game"
		{
			"string" { "Path" "%INSTALLDIR%\\bin" }
		}
	}
	"Firewall" { "Space War" "%INSTALLDIR%\\SpaceWar.exe" }
	"Run Process" { "setup" { "process 1" "%INSTALLDIR%\\setup.exe" "command 1" "/quiet /dir=\"%INSTALLDIR%\"" "AsCurrentUser" "1" } }
}"#;
        let steps = parse(s, &vars(), "french", true, "g");
        let names: Vec<String> = steps.iter().map(|s| match &s.action {
            Action::Registry { name, value, .. } => format!("reg {name}={value:?}"),
            Action::Firewall { program, .. } => format!("fw {program}"),
            Action::Run { args, .. } => format!("run {args}"),
        }).collect();
        assert_eq!(names, [
            "reg Language=Str(\"Français\")",
            "reg Version=Dword(3)",
            r#"reg Path=Str("D:\\Games\\steamapps\\common\\Steamworks Shared\\bin")"#,
            r#"fw D:\Games\steamapps\common\Steamworks Shared\SpaceWar.exe"#,
            r#"run /quiet /dir="D:\Games\steamapps\common\Steamworks Shared""#,
        ]);
        // English picks the english block, including the default value.
        assert!(parse(s, &vars(), "english", true, "g").iter().any(|s| matches!(&s.action, Action::Registry { name, .. } if name.is_empty())));
        // HKCU registry and a run-as-user installer don't need elevation.
        assert!(!steps[2].needs_admin() && !steps[4].needs_admin());
        assert!(steps[0].needs_admin() && steps[3].needs_admin());
    }

    #[test]
    fn finds_scripts_through_app_manifests() {
        let lib = std::env::temp_dir().join(format!("sir-is-{}", std::process::id()));
        let apps = lib.join("steamapps");
        let shared = apps.join("common/Steamworks Shared/_CommonRedist/DirectX/Jun2010");
        let game = apps.join("common/Game");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(shared.join("installscript.vdf"), "\"installscript\" {}").unwrap();
        std::fs::write(game.join("installscript.vdf"), "\"InstallScript\" {}").unwrap();
        std::fs::write(apps.join("appmanifest_10.acf"), "\"AppState\" { \"appid\" \"10\" \"installdir\" \"Game\" \"InstallScripts\" { \"11\" \"installscript.vdf\" } \"SharedDepots\" { \"228990\" \"228980\" } }").unwrap();
        std::fs::write(apps.join("appmanifest_228980.acf"), "\"AppState\" { \"appid\" \"228980\" \"installdir\" \"Steamworks Shared\" \"InstallScripts\" { \"228990\" \"_CommonRedist\\\\DirectX\\\\Jun2010\\\\installscript.vdf\" \"228983\" \"_CommonRedist\\\\vcredist\\\\2010\\\\installscript.vdf\" } }").unwrap();
        let found = scripts(&lib, 10);
        let _ = std::fs::remove_dir_all(&lib);
        assert_eq!(found.len(), 2);
        assert!(found[0].0.ends_with("Game/installscript.vdf"));
        assert!(found[1].0.ends_with("_CommonRedist/DirectX/Jun2010/installscript.vdf"));
        assert!(found[1].1.ends_with("Steamworks Shared"));
    }

    /// Runs a harmless script for real: a per-user registry value and a
    /// per-user installer (cmd) with a per-user has-run key. Windows only.
    #[cfg(windows)]
    #[test]
    fn runs_a_script_and_skips_it_after() {
        let lib = std::env::temp_dir().join(format!("sir-is-run-{}", std::process::id()));
        let apps = lib.join("steamapps");
        let game = apps.join("common").join("Game");
        std::fs::create_dir_all(&game).unwrap();
        let key = format!(r"HKEY_CURRENT_USER\Software\SteamInRustTest{}", std::process::id());
        // In the file every backslash is written twice, like Steam's.
        let k = key.replace('\\', r"\\");
        let script = format!(
            r#""InstallScript" {{ "Registry" {{ "{k}" {{ "string" {{ "Path" "%INSTALLDIR%" }} }} }} "Run Process" {{ "hello" {{ "process 1" "%WinDir%\\System32\\cmd.exe" "command 1" "/c exit 0" "HasRunKey" "{k}\\Ran" "AsCurrentUser" "1" }} }} }}"#
        );
        std::fs::write(game.join("installscript.vdf"), script).unwrap();
        std::fs::write(apps.join("appmanifest_10.acf"), "\"AppState\" { \"appid\" \"10\" \"installdir\" \"Game\" \"InstallScripts\" { \"11\" \"installscript.vdf\" } }").unwrap();
        let steps = pending(&lib, 10, None);
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert!(steps.iter().all(|s| !s.needs_admin()));
        run(&steps, &|_, _| {}).unwrap();
        assert_eq!(reg::string(&key, "Path").as_deref(), Some(game.to_string_lossy().trim_end_matches(['\\', '/'])));
        assert_eq!(reg::dword(&format!(r"{key}\Ran"), "hello"), Some(1));
        // Done: nothing left to run.
        assert!(pending(&lib, 10, None).is_empty());
        let _ = std::process::Command::new("reg").args(["delete", &key.replace("HKEY_CURRENT_USER", "HKCU"), "/f", "/reg:32"]).output();
        let _ = std::fs::remove_dir_all(&lib);
    }
}

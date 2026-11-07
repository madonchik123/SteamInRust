//! Everything that touches the local Steam installation on disk.
//!
//! We never modify Steam's files. Actions such as launching or installing are
//! handed to the official client through `steam://` URLs.

pub mod manage;

use model::{AppId, InstallState, InstalledGame, LocalAccount};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Steam installation not found")]
    NotFound,
    #[error("io error reading {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("could not parse {0}: {1}")]
    Parse(PathBuf, vdf::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Snapshot of everything we know about the local install.
#[derive(Debug, Clone, Default)]
pub struct LocalState {
    pub steam_root: PathBuf,
    pub libraries: Vec<PathBuf>,
    pub games: Vec<InstalledGame>,
    pub accounts: Vec<LocalAccount>,
}

/// Find the Steam install folder for this machine.
pub fn find_steam_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STEAM_ROOT") {
        return Some(PathBuf::from(p));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();

    #[cfg(windows)]
    {
        if let Some(p) = windows_registry_path() {
            candidates.push(p);
        }
        candidates.push(r"C:\Program Files (x86)\Steam".into());
        candidates.push(r"C:\Program Files\Steam".into());
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        candidates.push(home.join(".local/share/Steam"));
        candidates.push(home.join(".steam/steam"));
        candidates.push(home.join("Library/Application Support/Steam"));
        candidates.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }

    candidates
        .into_iter()
        .find(|p| p.join("steamapps").is_dir())
}

#[cfg(windows)]
fn windows_registry_path() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("SteamPath"))?;
    let value = line.split("REG_SZ").nth(1)?.trim();
    Some(PathBuf::from(value.replace('/', "\\")))
}

fn read_vdf(path: &Path) -> Result<vdf::Obj> {
    let text = fs::read_to_string(path).map_err(|e| Error::Io(path.into(), e))?;
    vdf::parse(&text).map_err(|e| Error::Parse(path.into(), e))
}

/// Load the full local state. Missing optional files are skipped, not errors.
pub fn load(steam_root: Option<PathBuf>) -> Result<LocalState> {
    let root = steam_root.or_else(find_steam_root).ok_or(Error::NotFound)?;
    let libraries = library_folders(&root)?;
    let accounts = accounts(&root).unwrap_or_else(|e| {
        tracing::warn!("could not read accounts: {e}");
        Vec::new()
    });

    let usage = accounts
        .iter()
        .find(|a| a.most_recent)
        .or(accounts.first())
        .map(|a| playtime(&root, model::account_id(a.steam_id)))
        .unwrap_or_default();

    let mut games: Vec<InstalledGame> = libraries
        .iter()
        .flat_map(|lib| scan_library(lib))
        .collect();
    for g in &mut games {
        if let Some(u) = usage.get(&g.app_id) {
            g.playtime_minutes = u.playtime_minutes;
            g.last_played = u.last_played.or(g.last_played);
        }
    }
    games.sort_by_key(|g| g.name.to_lowercase());
    games.dedup_by_key(|g| g.app_id);

    Ok(LocalState { steam_root: root, libraries, games, accounts })
}

/// All library folders listed in `steamapps/libraryfolders.vdf`.
pub fn library_folders(root: &Path) -> Result<Vec<PathBuf>> {
    let path = root.join("steamapps").join("libraryfolders.vdf");
    let mut libs = vec![root.to_path_buf()];
    if let Ok(doc) = read_vdf(&path) {
        if let Some(lf) = doc.obj("libraryfolders") {
            for (_, v) in lf.iter() {
                // Old format: "1" "D:\\Games". New format: "1" { "path" "..." }
                let p = v.as_str().or_else(|| v.get("path").and_then(|p| p.as_str()));
                if let Some(p) = p {
                    libs.push(PathBuf::from(p));
                }
            }
        }
    }
    libs.sort();
    libs.dedup_by(|a, b| same_path(a, b));
    libs.retain(|p| p.join("steamapps").is_dir());
    Ok(libs)
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().to_lowercase().replace('/', "\\");
    norm(a).trim_end_matches('\\') == norm(b).trim_end_matches('\\')
}

/// Read every `appmanifest_*.acf` in one library.
pub fn scan_library(lib: &Path) -> Vec<InstalledGame> {
    let apps = lib.join("steamapps");
    let Ok(entries) = fs::read_dir(&apps) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("appmanifest_") && name.ends_with(".acf")
        })
        .filter_map(|e| match parse_manifest(&e.path(), lib) {
            Ok(g) => g,
            Err(err) => {
                tracing::warn!("{err}");
                None
            }
        })
        .collect()
}

/// Tools and runtimes that show up as "apps" but aren't games.
fn is_tool(app_id: AppId, name: &str) -> bool {
    const TOOLS: &[AppId] = &[228980, 1070560, 1391110, 1628350, 1493710, 1826330, 2180100];
    TOOLS.contains(&app_id)
        || name.starts_with("Proton ")
        || name.starts_with("Steam Linux Runtime")
        || name == "Steamworks Common Redistributables"
}

fn parse_manifest(path: &Path, lib: &Path) -> Result<Option<InstalledGame>> {
    let doc = read_vdf(path)?;
    let Some(app) = doc.obj("AppState") else { return Ok(None) };
    let Some(app_id) = app.u64("appid").map(|v| v as AppId) else { return Ok(None) };
    let name = app.str("name").unwrap_or("Unknown").to_string();
    if is_tool(app_id, &name) {
        return Ok(None);
    }
    let install_dir = lib
        .join("steamapps")
        .join("common")
        .join(app.str("installdir").unwrap_or_default());
    Ok(Some(InstalledGame {
        app_id,
        name,
        install_dir,
        library: lib.to_path_buf(),
        size_on_disk: app.u64("SizeOnDisk").unwrap_or(0),
        last_updated: app.u64("LastUpdated").filter(|&t| t > 0),
        last_update_bytes: app.u64("BytesDownloaded").filter(|&b| b > 0),
        last_played: app.u64("LastPlayed").filter(|&t| t > 0),
        playtime_minutes: None,
        state: InstallState::from_flags(app.u64("StateFlags").unwrap_or(0)),
    }))
}

/// Accounts from `config/loginusers.vdf`, most recent first.
pub fn accounts(root: &Path) -> Result<Vec<LocalAccount>> {
    let doc = read_vdf(&root.join("config").join("loginusers.vdf"))?;
    let mut out: Vec<LocalAccount> = doc
        .obj("users")
        .map(|users| {
            users
                .iter()
                .filter_map(|(id, v)| {
                    let o = v.as_obj()?;
                    Some(LocalAccount {
                        steam_id: id.parse().ok()?,
                        account_name: o.str("AccountName").unwrap_or_default().into(),
                        persona_name: o.str("PersonaName").unwrap_or_default().into(),
                        most_recent: o.str("MostRecent") == Some("1"),
                        last_login: o.u64("Timestamp"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|a| (!a.most_recent, std::cmp::Reverse(a.last_login)));
    Ok(out)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Usage {
    pub playtime_minutes: Option<u64>,
    pub last_played: Option<u64>,
}

/// Per-app playtime from `userdata/<account>/config/localconfig.vdf`.
pub fn playtime(root: &Path, account_id: u32) -> HashMap<AppId, Usage> {
    let path = root
        .join("userdata")
        .join(account_id.to_string())
        .join("config")
        .join("localconfig.vdf");
    let Ok(doc) = read_vdf(&path) else { return HashMap::new() };
    let Some(apps) = doc.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"])
    else {
        return HashMap::new();
    };
    apps.iter()
        .filter_map(|(id, v)| {
            let o = v.as_obj()?;
            Some((
                id.parse().ok()?,
                Usage {
                    playtime_minutes: o.u64("Playtime"),
                    last_played: o.u64("LastPlayed").filter(|&t| t > 0),
                },
            ))
        })
        .collect()
}

/// Open a folder in the system file browser.
pub fn open_folder(path: &Path) {
    if let Err(e) = open::that_detached(path) {
        tracing::error!("failed to open {}: {e}", path.display());
    }
}

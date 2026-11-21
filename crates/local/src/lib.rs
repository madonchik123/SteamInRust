//! Everything that touches the local Steam installation on disk.
//!
//! We never modify Steam's files. Actions such as launching or installing are
//! handed to the official client through `steam://` URLs.

pub mod manage;
pub mod relocate;

use model::{AppId, InstallState, InstalledGame, LocalAccount};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

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

/// Where the official client installs itself on this system, used as the
/// first game library when Steam isn't installed.
pub fn default_steam_root() -> Option<PathBuf> {
    if cfg!(windows) {
        return Some(r"C:\Program Files (x86)\Steam".into());
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") { home.join("Library/Application Support/Steam") } else { home.join(".local/share/Steam") })
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
    load_with(steam_root, &[])
}

/// [`load`], also scanning `extra` library folders: ones games were
/// installed into that Steam's `libraryfolders.vdf` doesn't list (a folder
/// picked with "Choose another folder…", or any folder when Steam itself
/// isn't installed). An empty `steam_root` means "find it".
pub fn load_with(steam_root: Option<PathBuf>, extra: &[PathBuf]) -> Result<LocalState> {
    let root = steam_root.filter(|r| !r.as_os_str().is_empty()).or_else(find_steam_root);
    let mut libraries = match &root {
        Some(r) => library_folders(r)?,
        None => Vec::new(),
    };
    for lib in extra {
        if lib.join("steamapps").is_dir() && !libraries.iter().any(|l| same_path(l, lib)) {
            libraries.push(lib.clone());
        }
    }
    if root.is_none() && libraries.is_empty() {
        return Err(Error::NotFound);
    }
    let accounts = root.as_deref().map(accounts).unwrap_or(Ok(Vec::new())).unwrap_or_else(|e| {
        tracing::warn!("could not read accounts: {e}");
        Vec::new()
    });

    let usage = root
        .as_deref()
        .and_then(|root| accounts.iter().find(|a| a.most_recent).or(accounts.first()).map(|a| playtime(root, model::account_id(a.steam_id))))
        .unwrap_or_default();
    let root = root.unwrap_or_default();

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
    // Not just neighbours: "D:\Games" and "d:\games\" sort apart.
    let mut unique: Vec<PathBuf> = Vec::new();
    for lib in libs {
        if !unique.iter().any(|u| same_path(u, &lib)) {
            unique.push(lib);
        }
    }
    unique.retain(|p| p.join("steamapps").is_dir());
    Ok(unique)
}

/// True when `a` and `b` name the same folder (case, slashes and a
/// trailing separator aside).
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().to_lowercase().replace('/', "\\");
    norm(a).trim_end_matches('\\') == norm(b).trim_end_matches('\\')
}

/// Library folders listed in our own list file (one folder per line): the
/// ones games were installed into besides Steam's own libraries.
pub fn read_library_list(file: &Path) -> Vec<PathBuf> {
    fs::read_to_string(file)
        .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

/// Add `lib` to the list file unless it's there already. True when added.
pub fn add_to_library_list(file: &Path, lib: &Path) -> std::io::Result<bool> {
    if lib.as_os_str().is_empty() || lib.to_string_lossy().contains(['\n', '\r']) {
        return Ok(false);
    }
    let mut list = read_library_list(file);
    if list.iter().any(|l| same_path(l, lib)) {
        return Ok(false);
    }
    list.push(lib.to_path_buf());
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let text: String = list.iter().map(|l| format!("{}\n", l.display())).collect();
    fs::write(file, text)?;
    Ok(true)
}

/// The library folder for a folder the player picked: the picked folder
/// itself, or, when they picked something inside a library (its
/// `steamapps`, `steamapps\common` or a game's folder), that library.
pub fn library_root_of(picked: &Path) -> PathBuf {
    picked
        .ancestors()
        .find(|a| a.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("steamapps")))
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(picked)
        .to_path_buf()
}

/// The game `app_id` as recorded in `lib` (its `appmanifest_<id>.acf`), if
/// that library has it.
pub fn game_in_library(lib: &Path, app_id: AppId) -> Option<InstalledGame> {
    let path = lib.join("steamapps").join(format!("appmanifest_{app_id}.acf"));
    parse_manifest(&path, lib).ok().flatten().filter(|g| g.app_id == app_id)
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

fn is_windows_reserved(seg: &str) -> bool {
    let stem = seg.split('.').next().unwrap_or("");
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "CONIN$"
            | "CONOUT$"
            | "COM0"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT0"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return true;
    }
    if stem.len() > 3
        && stem.is_char_boundary(3)
        && (stem[..3].eq_ignore_ascii_case("COM") || stem[..3].eq_ignore_ascii_case("LPT"))
        && matches!(&stem[3..], "\u{00b9}" | "\u{00b2}" | "\u{00b3}")
    {
        return true;
    }
    false
}

/// True when `name` is a single safe directory entry name (never empty, `.`,
/// `..`, a path containing `/` or `\`, a drive/ADS `:`, control characters, or
/// a Windows reserved device name).
pub fn is_valid_folder_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains(['/', '\\', '\0']) || name.chars().any(|c| (c as u32) < 0x20) {
        return false;
    }
    // Names only Windows can't hold (a trailing dot or space, a drive colon,
    // CON/NUL/COM1...): fine as Mac and Linux folder names.
    if cfg!(windows) && (name.ends_with(' ') || name.ends_with('.') || name.contains(':') || is_windows_reserved(name)) {
        return false;
    }
    let mut comps = Path::new(name).components();
    matches!((comps.next(), comps.next()), (Some(Component::Normal(s)), None) if !s.is_empty())
}

/// True when `meta` describes a symlink or Windows reparse point / junction.
pub fn is_symlink_or_reparse(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Remove only the symlink or NTFS junction at `path` without recursing into
/// its target directory.
pub fn unlink_symlink_or_junction(path: &Path, meta: &fs::Metadata) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        if meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return fs::remove_dir(path);
        }
    }
    let _ = meta;
    fs::remove_file(path)
}

pub(crate) fn has_only_normal_components(path: &Path) -> bool {
    let raw = path.as_os_str().to_string_lossy();
    if raw.is_empty() || raw.split(['/', '\\']).any(|seg| seg == "." || seg == "..") {
        return false;
    }
    path.components().all(|c| matches!(c, Component::Prefix(_) | Component::RootDir | Component::Normal(_)))
}

/// Verify lexically that `dir` is a direct single-folder child of `common`
/// with no `.` / `..` / non-`Normal` path segments.
pub fn validate_common_child(common: &Path, dir: &Path) -> std::result::Result<(), String> {
    if !has_only_normal_components(common) || !has_only_normal_components(dir) {
        return Err(format!("refusing to delete {}: unsafe path components", dir.display()));
    }
    let Some(fname) = dir.file_name().and_then(|n| n.to_str()) else {
        return Err(format!("refusing to delete {}: missing valid folder name", dir.display()));
    };
    if !is_valid_folder_name(fname) {
        return Err(format!("refusing to delete {}: unsafe folder name {fname:?}", dir.display()));
    }
    if dir == common || dir.parent() != Some(common) {
        return Err(format!("refusing to delete {}: not a game folder in {}", dir.display(), common.display()));
    }
    let Ok(rel) = dir.strip_prefix(common) else {
        return Err(format!("refusing to delete {}: not inside {}", dir.display(), common.display()));
    };
    let mut comps = rel.components();
    if !matches!((comps.next(), comps.next()), (Some(Component::Normal(s)), None) if s == fname) {
        return Err(format!("refusing to delete {}: not a direct child of {}", dir.display(), common.display()));
    }
    Ok(())
}

fn parse_manifest(path: &Path, lib: &Path) -> Result<Option<InstalledGame>> {
    let doc = read_vdf(path)?;
    let Some(app) = doc.obj("AppState") else { return Ok(None) };
    let Some(app_id) = app.u64("appid").map(|v| v as AppId) else { return Ok(None) };
    let name = app.str("name").unwrap_or("Unknown").to_string();
    if is_tool(app_id, &name) {
        return Ok(None);
    }
    let Some(raw_install_dir) = app.str("installdir").filter(|s| is_valid_folder_name(s)) else {
        tracing::warn!("ignoring {}: unsafe installdir {:?}", path.display(), app.str("installdir"));
        return Ok(None);
    };
    let install_dir = lib
        .join("steamapps")
        .join("common")
        .join(raw_install_dir);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_installdir_in_appmanifest() {
        let lib = std::env::temp_dir().join(format!("sir-local-manifest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&lib);
        let apps = lib.join("steamapps");
        fs::create_dir_all(apps.join("common")).unwrap();

        let mut bad_names = vec!["..", ".", "", "../outside", "a/b", r"a\b", "/etc"];
        // Only Windows can't hold these (fine as Mac and Linux folders).
        if cfg!(windows) {
            bad_names.extend(["C:evil", "CON", "trailing."]);
        }
        for (idx, bad) in bad_names.iter().enumerate() {
            assert!(!is_valid_folder_name(bad), "accepted {bad:?}");
            let id = 1000 + idx as u32;
            let state = vdf::Obj::default()
                .with("appid", id.to_string())
                .with("name", format!("Bad {idx}"))
                .with("installdir", *bad)
                .with("StateFlags", "4");
            let doc = vdf::Obj::default().with_obj("AppState", state);
            fs::write(apps.join(format!("appmanifest_{id}.acf")), vdf::write(&doc)).unwrap();
        }
        let good = "\"AppState\"\n{\n\t\"appid\"\t\"2000\"\n\t\"name\"\t\"Good Game\"\n\t\"installdir\"\t\"Good Game\"\n\t\"StateFlags\"\t\"4\"\n}\n";
        fs::write(apps.join("appmanifest_2000.acf"), good).unwrap();

        let games = scan_library(&lib);
        assert_eq!(games.len(), 1, "unsafe installdir entries were not rejected: {games:?}");
        assert_eq!(games[0].app_id, 2000);
        let _ = fs::remove_dir_all(&lib);
    }

    /// Issue #103: a game downloaded into a folder picked with "Choose
    /// another folder…" (here a second library, like D:\GAMES\STEAM) has its
    /// files and its appmanifest there, but Steam's libraryfolders.vdf
    /// doesn't list that folder, so it was never scanned: "Install" forever.
    #[test]
    fn finds_games_in_libraries_steam_does_not_list() {
        let base = std::env::temp_dir().join(format!("sir-local-extra-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("Steam");
        let second = base.join("D").join("GAMES").join("STEAM");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::write(
            root.join("steamapps").join("libraryfolders.vdf"),
            vdf::write(&vdf::Obj::default().with_obj("libraryfolders", vdf::Obj::default().with_obj("0", vdf::Obj::default().with("path", root.to_string_lossy())))),
        )
        .unwrap();
        let game = second.join("steamapps").join("common").join("Megabonk");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("Megabonk.exe"), b"MZ").unwrap();
        let manifest = "\"AppState\"\n{\n\t\"appid\"\t\"3405340\"\n\t\"name\"\t\"Megabonk\"\n\t\"StateFlags\"\t\"4\"\n\t\"installdir\"\t\"Megabonk\"\n}\n";
        fs::write(second.join("steamapps").join("appmanifest_3405340.acf"), manifest).unwrap();

        // Only Steam's libraries: the game isn't seen (the bug).
        let state = load_with(Some(root.clone()), &[]).unwrap();
        assert!(state.games.iter().all(|g| g.app_id != 3405340));

        // With the folder remembered it's installed, and Play finds its files.
        let list = base.join("config").join("libraries.txt");
        assert!(add_to_library_list(&list, &second).unwrap());
        assert!(!add_to_library_list(&list, &second).unwrap(), "added twice");
        let state = load_with(Some(root.clone()), &read_library_list(&list)).unwrap();
        assert_eq!(state.libraries.len(), 2);
        let g = state.games.iter().find(|g| g.app_id == 3405340).expect("Megabonk found");
        assert_eq!(g.state, InstallState::Installed);
        assert_eq!(g.library, second);
        assert!(g.install_dir.join("Megabonk.exe").exists());
        assert_eq!(game_in_library(&second, 3405340).map(|g| g.name), Some("Megabonk".into()));

        // A folder with the files but no manifest isn't counted as installed
        // (installing there checks the files and writes the manifest).
        fs::remove_file(second.join("steamapps").join("appmanifest_3405340.acf")).unwrap();
        let state = load_with(Some(root.clone()), &read_library_list(&list)).unwrap();
        assert!(state.games.iter().all(|g| g.app_id != 3405340));
        assert!(game_in_library(&second, 3405340).is_none());

        // Picking a folder inside the library means the library.
        for inside in [second.join("steamapps"), second.join("steamapps").join("common"), game.clone(), second.join("STEAMAPPS")] {
            assert_eq!(library_root_of(&inside), second, "{inside:?}");
        }
        assert_eq!(library_root_of(&second), second);

        // No Steam install at all: the remembered library still counts.
        let no_steam = base.join("NoSteam");
        let state = load_with(Some(no_steam), std::slice::from_ref(&second)).unwrap();
        assert_eq!(state.libraries, vec![second.clone()]);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn refuses_dotdot_traversal_in_validate_common_child() {
        let common = PathBuf::from("/tmp/steam/steamapps/common");
        for bad in [common.join(".."), common.join("."), common.join("Game").join("..")] {
            assert!(validate_common_child(&common, &bad).is_err(), "accepted {bad:?}");
        }
        assert!(validate_common_child(&common, &common.join("ValidGame")).is_ok());
    }
}

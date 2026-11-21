//! Managing installed games: installed depots / DLC, file verification, uninstall.
//!
//! Verification uses the depot manifests Steam keeps in `<steam>/depotcache`
//! (`<depot>_<manifest>.manifest`). Each lists every file with its size and
//! SHA-1, so we can check an install without any network access.

use model::{AppId, InstalledGame};
use prost::Message;
use sha1::{Digest, Sha1};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A depot listed under `InstalledDepots` in an app manifest.
#[derive(Debug, Clone)]
pub struct InstalledDepot {
    pub depot: u32,
    pub manifest: u64,
    pub size: u64,
    /// Set when the depot belongs to a DLC.
    pub dlc_app: Option<AppId>,
}

fn manifest_file(game: &InstalledGame) -> PathBuf {
    game.library.join("steamapps").join(format!("appmanifest_{}.acf", game.app_id))
}

pub fn installed_depots(game: &InstalledGame) -> Vec<InstalledDepot> {
    let Ok(text) = fs::read_to_string(manifest_file(game)) else { return Vec::new() };
    let Ok(doc) = vdf::parse(&text) else { return Vec::new() };
    let Some(depots) = doc.path(&["AppState", "InstalledDepots"]) else { return Vec::new() };
    depots
        .iter()
        .filter_map(|(id, v)| {
            let o = v.as_obj()?;
            Some(InstalledDepot {
                depot: id.parse().ok()?,
                manifest: o.u64("manifest")?,
                size: o.u64("size").unwrap_or(0),
                dlc_app: o.u64("dlcappid").map(|v| v as AppId),
            })
        })
        .collect()
}

/// DLC app ids that have content installed for this game.
pub fn installed_dlc(game: &InstalledGame) -> Vec<AppId> {
    let mut v: Vec<AppId> = installed_depots(game).into_iter().filter_map(|d| d.dlc_app).collect();
    v.sort();
    v.dedup();
    v
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub sha1: Vec<u8>,
}

const PAYLOAD_MAGIC: u32 = 0x71F6_17D0;
const METADATA_MAGIC: u32 = 0x1F48_12BE;
const SIGNATURE_MAGIC: u32 = 0x1B81_B817;
const END_MAGIC: u32 = 0x32C4_15AB;

/// Parse a depot manifest. Returns `None` if filenames are encrypted
/// (Steam stores those decrypted, so this is rare).
pub fn read_manifest(data: &[u8]) -> Result<Option<Vec<FileEntry>>, String> {
    let mut pos = 0;
    let mut payload = None;
    let mut encrypted = false;
    let u32_at = |p: usize| data.get(p..p + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    while let Some(magic) = u32_at(pos) {
        pos += 4;
        if magic == END_MAGIC {
            break;
        }
        let len = u32_at(pos).ok_or("truncated manifest")? as usize;
        pos += 4;
        let body = data.get(pos..pos + len).ok_or("truncated manifest section")?;
        pos += len;
        match magic {
            PAYLOAD_MAGIC => payload = Some(proto::ContentManifestPayload::decode(body).map_err(|e| e.to_string())?),
            METADATA_MAGIC => {
                encrypted = proto::ContentManifestMetadata::decode(body).map(|m| m.filenames_encrypted()).unwrap_or(false);
            }
            SIGNATURE_MAGIC => {}
            other => return Err(format!("unknown manifest section {other:#x}")),
        }
    }
    if encrypted {
        return Ok(None);
    }
    let payload = payload.ok_or("manifest has no file list")?;
    const DIRECTORY: u32 = 64;
    Ok(Some(
        payload
            .mappings
            .into_iter()
            .filter(|m| m.flags() & DIRECTORY == 0 && m.linktarget().is_empty())
            .map(|m| FileEntry { path: m.filename().replace('\\', "/"), size: m.size(), sha1: m.sha_content().to_vec() })
            .collect(),
    ))
}

/// Shared progress for a running verification.
#[derive(Debug, Default)]
pub struct Progress {
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        let total = self.total_bytes.load(Ordering::Relaxed).max(1);
        self.done_bytes.load(Ordering::Relaxed) as f32 / total as f32
    }
}

#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    pub files: usize,
    pub ok: usize,
    pub missing: Vec<String>,
    pub corrupt: Vec<String>,
    /// Depots whose manifest isn't on disk (needs the downloader to fetch it).
    pub unchecked_depots: Vec<u32>,
    pub cancelled: bool,
}

impl VerifyReport {
    pub fn healthy(&self) -> bool {
        self.missing.is_empty() && self.corrupt.is_empty()
    }
}

/// Hash every file of every installed depot and compare with the manifests.
pub fn verify(steam_root: &Path, game: &InstalledGame, progress: &Progress) -> VerifyReport {
    let mut report = VerifyReport::default();
    let mut work: Vec<FileEntry> = Vec::new();
    for d in installed_depots(game) {
        let path = steam_root.join("depotcache").join(format!("{}_{}.manifest", d.depot, d.manifest));
        match fs::read(&path).map_err(|e| e.to_string()).and_then(|b| read_manifest(&b)) {
            Ok(Some(files)) => work.extend(files),
            Ok(None) | Err(_) => report.unchecked_depots.push(d.depot),
        }
    }
    work.sort_by(|a, b| a.path.cmp(&b.path));
    work.dedup_by(|a, b| a.path == b.path);
    progress.total_bytes.store(work.iter().map(|f| f.size).sum(), Ordering::Relaxed);
    report.files = work.len();

    let mut buf = vec![0u8; 1 << 20];
    for f in work {
        if progress.cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let full = game.install_dir.join(&f.path);
        let Ok(mut file) = fs::File::open(&full) else {
            report.missing.push(f.path);
            progress.done_bytes.fetch_add(f.size, Ordering::Relaxed);
            continue;
        };
        if file.metadata().map(|m| m.len()).unwrap_or(0) != f.size {
            report.corrupt.push(f.path);
            progress.done_bytes.fetch_add(f.size, Ordering::Relaxed);
            continue;
        }
        let mut hasher = Sha1::new();
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    hasher.update(&buf[..n]);
                    progress.done_bytes.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
        if f.sha1.is_empty() || hasher.finalize().as_slice() == f.sha1.as_slice() {
            report.ok += 1;
        } else {
            report.corrupt.push(f.path);
        }
    }
    report
}

/// Delete a game's files and its app manifest, like Steam: its Workshop
/// items and their record, and the download manifests only it used, go
/// too. Refuses anything that isn't a folder inside `<library>/steamapps/common`.
pub fn uninstall(game: &InstalledGame) -> Result<(), String> {
    uninstall_with(game, &Progress::default())
}

fn collect_contained_entries(parent: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64, bool)>) -> Result<bool, String> {
    if !crate::has_only_normal_components(parent) || !crate::has_only_normal_components(dir) {
        return Err(format!("refusing to delete {}: unsafe path components", dir.display()));
    }
    match fs::symlink_metadata(parent) {
        Ok(pm) => {
            if crate::is_symlink_or_reparse(&pm) || !pm.is_dir() {
                return Err(format!("refusing to delete {}: {} is not a real directory", dir.display(), parent.display()));
            }
            match fs::symlink_metadata(dir) {
                Ok(dm) if crate::is_symlink_or_reparse(&dm) => {
                    crate::unlink_symlink_or_junction(dir, &dm)
                        .map_err(|e| format!("could not unlink {}: {e}", dir.display()))?;
                    Ok(false)
                }
                Ok(dm) if dm.is_dir() => {
                    let canon_parent = parent
                        .canonicalize()
                        .map_err(|e| format!("could not resolve {}: {e}", parent.display()))?;
                    let canon_dir = dir
                        .canonicalize()
                        .map_err(|e| format!("could not resolve {}: {e}", dir.display()))?;
                    if canon_dir == canon_parent || canon_dir.parent() != Some(canon_parent.as_path()) {
                        return Err(format!(
                            "refusing to delete {}: canonical path escapes {}",
                            dir.display(),
                            parent.display()
                        ));
                    }
                    let mut stack = vec![dir.to_path_buf()];
                    while let Some(d) = stack.pop() {
                        let Ok(m) = fs::symlink_metadata(&d) else { continue };
                        if crate::is_symlink_or_reparse(&m) || !m.is_dir() {
                            continue;
                        }
                        let Ok(entries) = fs::read_dir(&d) else { continue };
                        for e in entries.flatten() {
                            let p = e.path();
                            let Ok(em) = fs::symlink_metadata(&p) else { continue };
                            if crate::is_symlink_or_reparse(&em) {
                                out.push((p, 0, true));
                            } else if em.is_dir() {
                                stack.push(p);
                            } else {
                                out.push((p, em.len(), false));
                            }
                        }
                    }
                    Ok(true)
                }
                Ok(_) => Err(format!("refusing to delete {}: not a directory", dir.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(format!("could not inspect {}: {e}", dir.display())),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("could not inspect {}: {e}", parent.display())),
    }
}

/// Uninstall file by file, counting deleted bytes in `progress` (run it
/// off the UI thread: a big game takes a while).
pub fn uninstall_with(game: &InstalledGame, progress: &Progress) -> Result<(), String> {
    let common = game.library.join("steamapps").join("common");
    let dir = &game.install_dir;
    crate::validate_common_child(&common, dir)?;

    // Read before the app manifest goes: which download manifests are its own.
    let own_manifests = depot_manifests_only_for(game);
    let workshop = workshop_dir(game);
    let ws_content = workshop.parent().ok_or("unexpected workshop directory")?.to_path_buf();

    let mut files = Vec::new();
    let rm_dir = collect_contained_entries(&common, dir, &mut files)?;
    let rm_ws = collect_contained_entries(&ws_content, &workshop, &mut files)?;

    delete_entries(files, progress);
    if rm_dir && dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("could not delete {}: {e}", dir.display()))?;
    }
    if rm_ws && workshop.exists() {
        fs::remove_dir_all(&workshop).map_err(|e| format!("could not delete {}: {e}", workshop.display()))?;
    }
    let _ = fs::remove_file(game.library.join("steamapps").join("workshop").join(format!("appworkshop_{}.acf", game.app_id)));
    for m in own_manifests {
        let _ = fs::remove_file(m);
    }
    let manifest = manifest_file(game);
    if fs::symlink_metadata(&manifest).is_ok() {
        fs::remove_file(&manifest).map_err(|e| format!("could not delete {}: {e}", manifest.display()))?;
    }
    Ok(())
}

/// Delete listed files and links, counting bytes in `progress`.
fn delete_entries(files: Vec<(PathBuf, u64, bool)>, progress: &Progress) {
    if files.is_empty() {
        return;
    }
    progress.total_bytes.store(files.iter().map(|f| f.1).sum::<u64>().max(1), Ordering::Relaxed);
    for (path, size, is_link) in files {
        if is_link {
            if let Ok(m) = fs::symlink_metadata(&path)
                && crate::is_symlink_or_reparse(&m)
            {
                let _ = crate::unlink_symlink_or_junction(&path, &m);
            }
        } else if fs::remove_file(&path).is_err() {
            // Read-only files: clear the flag (only if not a symlink) and try again.
            if let Ok(m) = fs::symlink_metadata(&path)
                && !crate::is_symlink_or_reparse(&m)
            {
                let mut perms = m.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                let _ = fs::set_permissions(&path, perms);
                let _ = fs::remove_file(&path);
            }
        }
        progress.done_bytes.fetch_add(size, Ordering::Relaxed);
    }
}

/// Delete the folder `dir`, a direct child of `parent`, with the same
/// checks as uninstalling (nothing outside it, links aren't followed).
pub(crate) fn remove_contained_dir(parent: &Path, dir: &Path, progress: &Progress) -> Result<(), String> {
    let mut files = Vec::new();
    let is_dir = collect_contained_entries(parent, dir, &mut files)?;
    delete_entries(files, progress);
    if is_dir && dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("could not delete {}: {e}", dir.display()))?;
    }
    Ok(())
}

/// `<library>/steamapps/workshop/content/<app>`: the game's Workshop items.
fn workshop_dir(game: &InstalledGame) -> PathBuf {
    game.library.join("steamapps").join("workshop").join("content").join(game.app_id.to_string())
}

/// Bytes of the game's Workshop items (what Steam counts with the game).
pub fn workshop_size(game: &InstalledGame) -> u64 {
    workshop_items(game).iter().map(|i| i.size).sum()
}

/// `depotcache/<depot>_<manifest>.manifest` files of this game's installed
/// depots that no other installed game in the library lists.
fn depot_manifests_only_for(game: &InstalledGame) -> Vec<PathBuf> {
    let installed = |path: &Path| -> Vec<(String, String)> {
        let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
        let Ok(doc) = vdf::parse(&text) else { return Vec::new() };
        doc.path(&["AppState", "InstalledDepots"])
            .map(|d| d.iter().filter_map(|(id, v)| Some((id.to_string(), v.as_obj()?.str("manifest")?.to_string()))).collect())
            .unwrap_or_default()
    };
    let mine = installed(&manifest_file(game));
    let steamapps = game.library.join("steamapps");
    let others: std::collections::HashSet<String> = fs::read_dir(&steamapps)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("appmanifest_")) && *p != manifest_file(game))
                .flat_map(|p| installed(&p).into_iter().map(|(id, _)| id))
                .collect()
        })
        .unwrap_or_default();
    let cache = game.library.join("depotcache");
    mine.into_iter().filter(|(id, _)| !others.contains(id)).map(|(id, gid)| cache.join(format!("{id}_{gid}.manifest"))).filter(|p| p.is_file()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_manifest() {
        let payload = proto::ContentManifestPayload {
            mappings: vec![
                proto::content_manifest_payload::FileMapping { filename: Some("bin\\game.exe".into()), size: Some(3), sha_content: Some(vec![1; 20]), ..Default::default() },
                proto::content_manifest_payload::FileMapping { filename: Some("bin".into()), flags: Some(64), ..Default::default() },
            ],
        };
        let body = payload.encode_to_vec();
        let mut data = PAYLOAD_MAGIC.to_le_bytes().to_vec();
        data.extend((body.len() as u32).to_le_bytes());
        data.extend(body);
        data.extend(END_MAGIC.to_le_bytes());
        let files = read_manifest(&data).unwrap().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "bin/game.exe");
    }

    #[test]
    fn uninstall_takes_workshop_items_and_own_manifests() {
        let lib = std::env::temp_dir().join(format!("sir-uninstall-{}", std::process::id()));
        let _ = fs::remove_dir_all(&lib);
        let apps = lib.join("steamapps");
        let game_dir = apps.join("common").join("Game");
        fs::create_dir_all(game_dir.join("bin")).unwrap();
        fs::write(game_dir.join("bin").join("game"), b"x").unwrap();
        let ws = apps.join("workshop");
        fs::create_dir_all(ws.join("content").join("10").join("555")).unwrap();
        fs::write(ws.join("content").join("10").join("555").join("level.dat"), b"lvl").unwrap();
        fs::create_dir_all(ws.join("content").join("20")).unwrap();
        fs::write(ws.join("appworkshop_10.acf"), "\"AppWorkshop\" { \"WorkshopItemsInstalled\" { \"555\" { \"size\" \"3\" } } }").unwrap();
        fs::write(apps.join("appmanifest_10.acf"), "\"AppState\" { \"InstalledDepots\" { \"11\" { \"manifest\" \"1\" } \"99\" { \"manifest\" \"2\" } } }").unwrap();
        // Another game shares depot 99.
        fs::write(apps.join("appmanifest_20.acf"), "\"AppState\" { \"InstalledDepots\" { \"99\" { \"manifest\" \"2\" } } }").unwrap();
        fs::create_dir_all(lib.join("depotcache")).unwrap();
        fs::write(lib.join("depotcache").join("11_1.manifest"), b"m").unwrap();
        fs::write(lib.join("depotcache").join("99_2.manifest"), b"m").unwrap();
        let g = InstalledGame {
            app_id: 10,
            name: "Game".into(),
            install_dir: game_dir.clone(),
            library: lib.clone(),
            size_on_disk: 1,
            last_updated: None,
            last_played: None,
            playtime_minutes: None,
            state: model::InstallState::Installed,
            last_update_bytes: None,
        };
        assert_eq!(workshop_size(&g), 3);
        uninstall(&g).unwrap();
        assert!(!game_dir.exists());
        assert!(!ws.join("content").join("10").exists());
        assert!(!ws.join("appworkshop_10.acf").exists());
        assert!(!apps.join("appmanifest_10.acf").exists());
        assert!(!lib.join("depotcache").join("11_1.manifest").exists());
        // Still used by the other game.
        assert!(lib.join("depotcache").join("99_2.manifest").exists());
        assert!(ws.join("content").join("20").exists());
        let _ = fs::remove_dir_all(&lib);
    }

    #[test]
    fn refuses_to_uninstall_outside_common() {
        let g = InstalledGame {
            app_id: 1,
            name: "x".into(),
            install_dir: PathBuf::from("C:/Windows"),
            library: PathBuf::from("C:/Games"),
            size_on_disk: 0,
            last_updated: None,
            last_played: None,
            playtime_minutes: None,
            state: model::InstallState::Installed,
            last_update_bytes: None,
        };
        assert!(uninstall(&g).is_err());
    }

    #[test]
    fn refuses_dotdot_traversal_and_unlinks_symlink_without_deleting_target() {
        let lib = std::env::temp_dir().join(format!("sir-uninstall-sec-{}", std::process::id()));
        let _ = fs::remove_dir_all(&lib);
        let apps = lib.join("steamapps");
        let common = apps.join("common");
        let compat = apps.join("compatdata");
        fs::create_dir_all(&common).unwrap();
        fs::create_dir_all(&compat).unwrap();
        let save = compat.join("precious.sav");
        fs::write(&save, b"important save").unwrap();

        for bad_dir in [common.join(".."), common.join("."), common.join("sub").join("..")] {
            let g = InstalledGame {
                app_id: 99,
                name: "Evil".into(),
                install_dir: bad_dir.clone(),
                library: lib.clone(),
                size_on_disk: 0,
                last_updated: None,
                last_played: None,
                playtime_minutes: None,
                state: model::InstallState::Installed,
                last_update_bytes: None,
            };
            assert!(uninstall(&g).is_err(), "accepted {bad_dir:?}");
            assert!(save.exists(), "deleted steamapps/compatdata via {bad_dir:?}!");
            assert!(common.exists(), "deleted steamapps/common via {bad_dir:?}!");
        }

        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join(format!("sir-uninstall-ext-{}", std::process::id()));
            let _ = fs::remove_dir_all(&outside);
            fs::create_dir_all(&outside).unwrap();
            let victim = outside.join("victim.txt");
            fs::write(&victim, b"do not delete").unwrap();
            let sym_game = common.join("SymGame");
            std::os::unix::fs::symlink(&outside, &sym_game).unwrap();
            let ws_content = apps.join("workshop").join("content");
            fs::create_dir_all(&ws_content).unwrap();
            let sym_ws = ws_content.join("77");
            std::os::unix::fs::symlink(&outside, &sym_ws).unwrap();
            fs::write(apps.join("appmanifest_77.acf"), "\"AppState\" {}").unwrap();

            let g = InstalledGame {
                app_id: 77,
                name: "SymGame".into(),
                install_dir: sym_game.clone(),
                library: lib.clone(),
                size_on_disk: 0,
                last_updated: None,
                last_played: None,
                playtime_minutes: None,
                state: model::InstallState::Installed,
                last_update_bytes: None,
            };
            uninstall(&g).unwrap();
            assert!(victim.exists(), "uninstall followed symlink and deleted target!");
            assert!(fs::symlink_metadata(&sym_game).is_err(), "game symlink should be unlinked");
            assert!(fs::symlink_metadata(&sym_ws).is_err(), "workshop symlink should be unlinked");
            assert!(!apps.join("appmanifest_77.acf").exists());
            let _ = fs::remove_dir_all(&outside);
        }

        let _ = fs::remove_dir_all(&lib);
    }
}

/// A workshop item installed for a game (from `appworkshop_<app>.acf`).
#[derive(Debug, Clone)]
pub struct WorkshopItem {
    pub id: u64,
    pub size: u64,
    /// Unix seconds.
    pub updated: u64,
    pub folder: PathBuf,
}

/// A value parsed from a file, re-read only when the file changes (its size
/// or modification time): pages call these every frame, and a workshop
/// manifest can list thousands of mods.
fn cached_read<T: Clone + Send + 'static>(cache: &'static std::sync::Mutex<Option<std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, u64, T)>>>, path: &Path, parse: impl FnOnce(Option<String>) -> T) -> T {
    let stamp = fs::metadata(path).ok().map(|m| (m.modified().ok(), m.len()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(Default::default);
    if let (Some((modified, len)), Some((m, l, v))) = (stamp, map.get(path)) {
        if *m == modified && *l == len {
            return v.clone();
        }
    }
    let value = parse(fs::read_to_string(path).ok());
    match stamp {
        Some((modified, len)) => {
            map.insert(path.to_path_buf(), (modified, len, value.clone()));
        }
        None => {
            map.remove(path);
        }
    }
    value
}

pub fn workshop_items(game: &InstalledGame) -> Vec<WorkshopItem> {
    static CACHE: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, u64, Vec<WorkshopItem>)>>> = std::sync::Mutex::new(None);
    let ws = game.library.join("steamapps").join("workshop");
    let acf = ws.join(format!("appworkshop_{}.acf", game.app_id));
    cached_read(&CACHE, &acf, |text| parse_workshop_items(game, &ws, text))
}

fn parse_workshop_items(game: &InstalledGame, ws: &Path, text: Option<String>) -> Vec<WorkshopItem> {
    let Some(text) = text else { return Vec::new() };
    let Ok(doc) = vdf::parse(&text) else { return Vec::new() };
    let content = ws.join("content").join(game.app_id.to_string());
    let mut items: Vec<WorkshopItem> = doc
        .path(&["AppWorkshop", "WorkshopItemsInstalled"])
        .map(|o| {
            o.iter()
                .filter_map(|(id, v)| {
                    let o = v.as_obj()?;
                    let id: u64 = id.parse().ok()?;
                    Some(WorkshopItem { id, size: o.u64("size").unwrap_or(0), updated: o.u64("timeupdated").unwrap_or(0), folder: content.join(id.to_string()) })
                })
                .collect()
        })
        .unwrap_or_default();
    items.sort_by_key(|i| std::cmp::Reverse(i.updated));
    items
}

/// Delete a workshop item's downloaded files (Steam re-downloads it unless
/// you also unsubscribe).
pub fn delete_workshop_files(item: &WorkshopItem) -> Result<(), String> {
    // Only ever delete <library>/steamapps/workshop/content/<app>/<item>.
    if !crate::has_only_normal_components(&item.folder) {
        return Err("unexpected workshop folder".into());
    }
    let named_right = item.folder.file_name().is_some_and(|n| n.to_string_lossy() == item.id.to_string());
    let Some(app_dir) = item.folder.parent() else {
        return Err("unexpected workshop folder".into());
    };
    let in_content = app_dir
        .parent()
        .is_some_and(|p| p.file_name().is_some_and(|n| n == "content") && p.parent().is_some_and(|w| w.file_name().is_some_and(|n| n == "workshop")));
    if !named_right || !in_content {
        return Err("unexpected workshop folder".into());
    }
    match fs::symlink_metadata(&item.folder) {
        Ok(m) if crate::is_symlink_or_reparse(&m) => {
            crate::unlink_symlink_or_junction(&item.folder, &m).map_err(|e| e.to_string())?;
        }
        Ok(m) if m.is_dir() => {
            let canon_parent = app_dir.canonicalize().map_err(|e| e.to_string())?;
            let canon_item = item.folder.canonicalize().map_err(|e| e.to_string())?;
            if canon_item == canon_parent || canon_item.parent() != Some(canon_parent.as_path()) {
                return Err("workshop folder escapes content directory".into());
            }
            fs::remove_dir_all(&item.folder).map_err(|e| e.to_string())?;
        }
        Ok(_) => return Err("unexpected workshop folder".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// Build id and branch Steam has installed (from the app manifest).
pub fn installed_build(game: &InstalledGame) -> (u64, String) {
    static CACHE: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, u64, (u64, String))>>> = std::sync::Mutex::new(None);
    cached_read(&CACHE, &manifest_file(game), parse_installed_build)
}

fn parse_installed_build(text: Option<String>) -> (u64, String) {
    let Some(text) = text else { return (0, "public".into()) };
    let Ok(doc) = vdf::parse(&text) else { return (0, "public".into()) };
    let app = doc.obj("AppState");
    let build = app.and_then(|a| a.u64("buildid")).unwrap_or(0);
    let branch = app.and_then(|a| a.path(&["UserConfig"])).and_then(|u| u.str("BetaKey")).filter(|b| !b.is_empty()).unwrap_or("public").to_string();
    (build, branch)
}

#[cfg(test)]
mod cached_read_tests {
    use super::*;

    #[test]
    fn rereads_only_when_the_file_changes() {
        static CACHE: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, u64, String)>>> = std::sync::Mutex::new(None);
        let dir = std::env::temp_dir().join(format!("sir-cached-read-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.acf");
        fs::write(&path, "one").unwrap();
        let read = |calls: &mut u32| cached_read(&CACHE, &path, |t| {
            *calls += 1;
            t.unwrap_or_default()
        });
        let mut calls = 0;
        assert_eq!(read(&mut calls), "one");
        assert_eq!(read(&mut calls), "one");
        assert_eq!(calls, 1, "unchanged file: parsed once");
        fs::write(&path, "two!").unwrap();
        assert_eq!(read(&mut calls), "two!");
        assert_eq!(calls, 2);
        fs::remove_file(&path).unwrap();
        assert_eq!(read(&mut calls), "");
        let _ = fs::remove_dir_all(&dir);
    }
}

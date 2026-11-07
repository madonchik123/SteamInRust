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

/// Delete a game's files and its app manifest. Refuses anything that isn't
/// a folder inside `<library>/steamapps/common`.
pub fn uninstall(game: &InstalledGame) -> Result<(), String> {
    let common = game.library.join("steamapps").join("common");
    let dir = &game.install_dir;
    let inside = dir.starts_with(&common) && dir != &common && dir.parent() == Some(common.as_path());
    if !inside {
        return Err(format!("refusing to delete {}: not a game folder in {}", dir.display(), common.display()));
    }
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("could not delete {}: {e}", dir.display()))?;
    }
    let manifest = manifest_file(game);
    if manifest.exists() {
        fs::remove_file(&manifest).map_err(|e| format!("could not delete {}: {e}", manifest.display()))?;
    }
    Ok(())
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

pub fn workshop_items(game: &InstalledGame) -> Vec<WorkshopItem> {
    let ws = game.library.join("steamapps").join("workshop");
    let Ok(text) = fs::read_to_string(ws.join(format!("appworkshop_{}.acf", game.app_id))) else { return Vec::new() };
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
    let named_right = item.folder.file_name().is_some_and(|n| n.to_string_lossy() == item.id.to_string());
    let in_content = item
        .folder
        .parent()
        .and_then(|p| p.parent())
        .is_some_and(|p| p.file_name().is_some_and(|n| n == "content") && p.parent().is_some_and(|w| w.file_name().is_some_and(|n| n == "workshop")));
    if !named_right || !in_content {
        return Err("unexpected workshop folder".into());
    }
    if item.folder.exists() {
        fs::remove_dir_all(&item.folder).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Build id and branch Steam has installed (from the app manifest).
pub fn installed_build(game: &InstalledGame) -> (u64, String) {
    let Ok(text) = fs::read_to_string(manifest_file(game)) else { return (0, "public".into()) };
    let Ok(doc) = vdf::parse(&text) else { return (0, "public".into()) };
    let app = doc.obj("AppState");
    let build = app.and_then(|a| a.u64("buildid")).unwrap_or(0);
    let branch = app.and_then(|a| a.path(&["UserConfig"])).and_then(|u| u.str("BetaKey")).filter(|b| !b.is_empty()).unwrap_or("public").to_string();
    (build, branch)
}

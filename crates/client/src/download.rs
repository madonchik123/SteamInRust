//! Installing and updating games: picks depots, gets keys and manifests
//! through the CM connection, then hands the transfer to the `depot` crate.

use model::{AppId, DownloadPhase, DownloadStatus, OwnedApp};
use net::{Connection, emsg, eresult};
use proto::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstallRequest {
    pub app_id: AppId,
    /// Library folder (the one that contains `steamapps`).
    pub library: PathBuf,
    /// Steam install root (for `depotcache`).
    pub steam_root: PathBuf,
    pub branch: String,
    /// Steam language name, e.g. "english", "russian".
    pub language: String,
    pub skip_dlc: Vec<AppId>,
    /// DLC the user turned on after installing: added on this update.
    #[serde(default)]
    pub add_dlc: Vec<AppId>,
}

/// Choose the depots an install on this system needs (see [`pick_depots_for`]).
pub fn pick_depots<'a>(app: &'a OwnedApp, owned: &HashSet<AppId>, req: &InstallRequest, installed: &HashSet<u32>) -> Vec<&'a model::DepotInfo> {
    pick_depots_for(model::THIS_OS, app, owned, req, installed)
}

/// Choose the depots a 64-bit install for `os` needs. `installed` = the
/// depots on disk now (empty for a new install). Like Steam, a new install
/// gets the owned DLC, and an update only keeps the DLC already installed
/// (plus any the user turned on), so owned-but-uninstalled DLC never turns
/// an update into a multi-gigabyte download.
pub fn pick_depots_for<'a>(os: &str, app: &'a OwnedApp, owned: &HashSet<AppId>, req: &InstallRequest, installed: &HashSet<u32>) -> Vec<&'a model::DepotInfo> {
    let os_ok = |d: &&model::DepotInfo| model::os_matches(&d.os, os);
    let lang_ok = |d: &&model::DepotInfo| d.language.is_empty() || d.language == req.language;
    // An update keeps every depot already installed (its language, 32 or
    // 64-bit), like Steam; the OS/language/arch choice is for new content.
    let was = |d: &&model::DepotInfo| installed.contains(&d.id);
    let mut out: Vec<&model::DepotInfo> = app
        .depots
        .iter()
        .filter(|d| was(d) || (os_ok(d) && lang_ok(d)))
        .filter(|d| !d.low_violence && !d.shared_install && !d.manifests.is_empty())
        .filter(|d| match d.dlc_app {
            Some(dlc) => owned.contains(&dlc) && !req.skip_dlc.contains(&dlc) && (installed.is_empty() || was(d) || req.add_dlc.contains(&dlc)),
            None => true,
        })
        .collect();
    // Prefer 64-bit depots when both exist (but never drop an installed one).
    let has64 = out.iter().any(|d| d.arch == "64");
    if has64 {
        out.retain(|d| d.arch != "32" || was(d));
    }
    // Fall back to English content if the chosen language has none.
    if out.iter().all(|d| d.language.is_empty()) && req.language != "english" {
        let mut en = req.clone();
        en.language = "english".into();
        let extra: Vec<&model::DepotInfo> = app.depots.iter().filter(os_ok).filter(|d| d.language == "english").collect();
        out.extend(extra);
    }
    out
}

/// How the player knows a system.
pub fn os_name(os: &str) -> &'static str {
    match os {
        "macos" => "macOS",
        "linux" => "Linux",
        _ => "Windows",
    }
}

pub async fn content_servers(conn: &Connection, cell_id: u32) -> Result<Vec<depot::Server>, String> {
    let r: CContentServerDirectoryGetServersForSteamPipeResponse = conn
        .call(
            "ContentServerDirectory.GetServersForSteamPipe#1",
            &CContentServerDirectoryGetServersForSteamPipeRequest { cell_id: Some(cell_id), max_servers: Some(20), ..Default::default() },
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(r.servers
        .into_iter()
        .filter(|s| matches!(s.r#type(), "SteamCache" | "CDN"))
        .map(|s| depot::Server {
            host: if s.vhost().is_empty() { s.host().to_string() } else { s.vhost().to_string() },
            https: !s.https_support().is_empty() && !s.https_support().eq_ignore_ascii_case("none"),
            load: s.weighted_load(),
        })
        .filter(|s| s.https && depot::cdn::valid_host(&s.host))
        .collect())
}

pub(crate) async fn depot_key(conn: &Connection, app: AppId, depot: u32) -> Result<Vec<u8>, String> {
    let reply = conn
        .request(emsg::CLIENT_GET_DEPOT_DECRYPTION_KEY, &CMsgClientGetDepotDecryptionKey { depot_id: Some(depot), app_id: Some(app) })
        .await
        .map_err(|e| e.to_string())?;
    let r: CMsgClientGetDepotDecryptionKeyResponse = reply.body().map_err(|e| e.to_string())?;
    if r.eresult() == eresult::ACCESS_DENIED {
        // For content this means the license doesn't include this depot
        // (optional content, another region's files, a DLC you don't own).
        return Err(format!("your license doesn't include depot {depot}"));
    }
    if r.eresult() != eresult::OK {
        return Err(eresult::describe(r.eresult(), &format!("depot {depot}")));
    }
    Ok(r.depot_encryption_key().to_vec())
}

pub(crate) async fn request_code(conn: &Connection, app: AppId, depot: u32, gid: u64, branch: &str) -> u64 {
    conn.call::<_, CContentServerDirectoryGetManifestRequestCodeResponse>(
        "ContentServerDirectory.GetManifestRequestCode#1",
        &CContentServerDirectoryGetManifestRequestCodeRequest {
            app_id: Some(app),
            depot_id: Some(depot),
            manifest_id: Some(gid),
            app_branch: Some(branch.to_string()),
            branch_password_hash: None,
        },
    )
    .await
    .map(|r| r.manifest_request_code())
    .unwrap_or(0)
}

/// Read what's installed now: depot -> manifest id (from the app manifest).
fn installed_manifests(req: &InstallRequest) -> Vec<(u32, u64)> {
    let path = req.library.join("steamapps").join(format!("appmanifest_{}.acf", req.app_id));
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let Ok(doc) = vdf::parse(&text) else { return Vec::new() };
    doc.path(&["AppState", "InstalledDepots"])
        .map(|d| d.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_obj()?.u64("manifest")?))).collect())
        .unwrap_or_default()
}

pub fn status(app_id: AppId, p: &depot::Progress, speed: u64) -> DownloadStatus {
    DownloadStatus {
        app_id,
        phase: match p.phase() {
            depot::Phase::Preparing => DownloadPhase::Preparing,
            depot::Phase::Checking => DownloadPhase::Checking,
            depot::Phase::Downloading => DownloadPhase::Downloading,
            depot::Phase::Finishing => DownloadPhase::Finishing,
            depot::Phase::Done => DownloadPhase::Done,
        },
        downloaded: p.downloaded.load(Ordering::Relaxed),
        download_total: p.download_total.load(Ordering::Relaxed),
        disk_done: p.disk_done.load(Ordering::Relaxed),
        disk_total: p.disk_total.load(Ordering::Relaxed),
        speed,
        paused: p.paused.load(Ordering::Relaxed),
        error: None,
        reused: 0,
        notes: p.notes.lock().map(|n| n.clone()).unwrap_or_default(),
        install_dir: None,
        disk_speed: 0,
    }
}

pub(crate) fn install_folder_name(app: &OwnedApp) -> Result<String, String> {
    if !app.install_dir.is_empty() {
        return depot::install::validate_folder_name(&app.install_dir).map_err(|e| e.to_string());
    }
    if app.name.is_empty() || app.name == "." || app.name == ".." || app.name.contains(['/', '\\', '\0']) {
        return Err(format!("unsafe app folder name: {:?}", app.name));
    }
    let sanitized: String = app
        .name
        .chars()
        .map(|c| if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 0x20 { '_' } else { c })
        .collect();
    let trimmed = sanitized.trim_end_matches([' ', '.']);
    depot::install::validate_folder_name(trimmed).map_err(|e| e.to_string())
}

/// Run one install/update to completion.
pub async fn run(
    conn: Connection,
    app: OwnedApp,
    owned: HashSet<AppId>,
    req: InstallRequest,
    cell_id: u32,
    preferred_host: Option<String>,
    owner: u64,
    progress: Arc<depot::Progress>,
) -> Result<depot::Report, String> {
    progress.set_phase(depot::Phase::Preparing);
    let folder = install_folder_name(&app)?;
    let common = req.library.join("steamapps").join("common");
    let dir = depot::install::resolve_inside(&common, &folder).map_err(|e| e.to_string())?;
    let old = installed_manifests(&req);
    let on_disk: HashSet<u32> = old.iter().map(|(d, _)| *d).collect();
    let depots = pick_depots(&app, &owned, &req, &on_disk);
    if depots.is_empty() {
        return Err(format!("{} has no downloadable content for {}", app.name, os_name(model::THIS_OS)));
    }
    let mut servers = content_servers(&conn, cell_id).await?;
    // A host the player picked goes first (the others stay as fallbacks).
    if let Some(host) = preferred_host.filter(|h| depot::cdn::valid_host(h)) {
        servers.retain(|s| s.host != host);
        servers.insert(0, depot::Server { host, https: true, load: 0.0 });
    }
    let cdn = Arc::new(depot::Cdn::new(servers));
    if cdn.server_count() == 0 {
        return Err("Steam returned no HTTPS content servers".into());
    }
    cdn.rank().await;

    let mut jobs = Vec::new();
    let mut installed = Vec::new();
    // Installed depots left untouched this time (couldn't get their key).
    let mut kept: Vec<(u32, u64, u64, Option<AppId>)> = Vec::new();
    let picked: HashSet<u32> = depots.iter().map(|d| d.id).collect();
    for d in depots {
        let Some((gid, size)) = d.manifest(&req.branch) else { continue };
        let key = match depot_key(&conn, app.app_id, d.id).await {
            Ok(k) => k,
            Err(e) => {
                // Usually a depot this account doesn't own (optional content);
                // Steam's client skips these too. Shown on the Downloads page.
                tracing::info!("skipping depot {}: {e}", d.id);
                if let Ok(mut n) = progress.notes.lock() {
                    n.push(e);
                }
                // Already installed: keep it (and its record) as it is.
                if let Some((_, m)) = old.iter().find(|(dep, _)| *dep == d.id) {
                    kept.push((d.id, *m, 0, d.dlc_app));
                }
                continue;
            }
        };
        let code = request_code(&conn, app.app_id, d.id, gid, &req.branch).await;
        let raw = cdn.manifest(d.id, gid, code).await.map_err(|e| e.to_string())?;
        let mut manifest = depot::Manifest::from_cdn(&raw).map_err(|e| e.to_string())?;
        manifest.decrypt_names(&key).map_err(|e| e.to_string())?;
        let old_manifest = old
            .iter()
            .find(|(dep, _)| *dep == d.id)
            .and_then(|(dep, m)| std::fs::read(req.steam_root.join("depotcache").join(format!("{dep}_{m}.manifest"))).ok())
            .and_then(|b| depot::Manifest::parse(&b).ok());
        installed.push((d.id, gid, size.max(manifest.original_size), d.dlc_app));
        jobs.push(depot::DepotJob { depot: d.id, manifest, key, old: old_manifest });
    }
    if jobs.is_empty() {
        return Err("This account can't download any of this game's content".into());
    }

    // Make sure it fits (a rough upper bound: the full new size).
    let need: u64 = jobs.iter().map(|j| j.manifest.original_size).sum();
    if let Ok(free) = fs4::available_space(&req.library) {
        let have: u64 = dir_size(&dir);
        if need > free + have {
            return Err(format!("Not enough disk space: needs {} MB more", (need - free - have) / 1_048_576));
        }
    }

    let manifests: Vec<depot::Manifest> = jobs.iter().map(|j| j.manifest.clone()).collect();
    // Depots installed before but not wanted now: only DLC the user turned
    // off. Anything else not picked (unknown to Steam's app info now, say)
    // is left alone rather than deleted.
    let dropped: Vec<(u32, u64)> = old
        .iter()
        .filter(|(d, _)| !picked.contains(d))
        .filter(|(d, _)| app.depots.iter().find(|x| x.id == *d).and_then(|x| x.dlc_app).is_some_and(|dlc| req.skip_dlc.contains(&dlc) || !owned.contains(&dlc)))
        .copied()
        .collect();
    // Installed depots neither updated nor removed keep their record.
    for (d, m) in &old {
        if !picked.contains(d) && !dropped.iter().any(|(x, _)| x == d) {
            kept.push((*d, *m, 0, app.depots.iter().find(|x| x.id == *d).and_then(|x| x.dlc_app)));
        }
    }
    installed.extend(kept.iter().copied());
    // A new install creates its folder; an update writes into the game's.
    // The folder is marked as new until the game is complete, so a new
    // install resumed after a restart is still cleaned up fully if cancelled.
    let fresh = is_new_install(&old, &dir);
    let marker = dir.join(NEW_INSTALL);
    if fresh {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let _ = std::fs::write(&marker, b"");
    }
    // Steam's shared redistributables (DirectX, Visual C++...) the game uses,
    // into their own folder (`Steamworks Shared`), like Steam.
    let shared = match install_shared(&conn, &cdn, &app, &req, &progress).await {
        Ok(s) => s,
        Err(e) if e == depot::Error::Cancelled.to_string() || e == depot::Error::Stopped.to_string() => return Err(e),
        Err(e) => {
            tracing::warn!("shared redistributables of {}: {e}", app.name);
            Vec::new()
        }
    };
    let report = match depot::install(cdn, &dir, jobs, progress).await {
        Ok(r) => r,
        Err(depot::Error::Cancelled) => {
            // Unfinished files are reserved at full size: give the space back.
            remove_unfinished(&req, &dir, fresh);
            // An update that was adding depots (e.g. DLC): undo those too,
            // finished files included, so the game is as it was.
            let added: Vec<&depot::Manifest> = manifests.iter().filter(|m| !on_disk.contains(&m.depot)).collect();
            if !fresh && !added.is_empty() {
                remove_added(&req, &dir, &added, &old);
            }
            return Err(depot::Error::Cancelled.to_string());
        }
        Err(e) => return Err(e.to_string()),
    };

    // Leftovers of earlier interrupted attempts are junk once it's complete.
    remove_unfinished(&req, &dir, false);
    if !dropped.is_empty() {
        remove_dropped(&req, &dir, &dropped, &manifests);
    }
    // Leave the same records Steam does, so both clients agree.
    let cache = req.steam_root.join("depotcache");
    let _ = std::fs::create_dir_all(&cache);
    for m in &manifests {
        let _ = std::fs::write(cache.join(format!("{}_{}.manifest", m.depot, m.gid)), m.to_bytes());
    }
    let scripts = install_scripts(&manifests, &req, app.app_id);
    write_app_manifest(&req, &app, &folder, &installed, need, owner, &scripts, &shared).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&marker);
    Ok(report)
}

/// The install scripts (`installscript.vdf`, flagged in the manifests) per
/// depot, for the app manifest's `InstallScripts` like Steam writes. Depots
/// not downloaded this time keep what the old app manifest said.
fn install_scripts(manifests: &[depot::Manifest], req: &InstallRequest, app_id: AppId) -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = manifests
        .iter()
        .filter_map(|m| m.files.iter().find(|f| f.install_script).map(|f| (m.depot, f.name.replace('/', "\\"))))
        .collect();
    let old = std::fs::read_to_string(req.library.join("steamapps").join(format!("appmanifest_{app_id}.acf"))).ok().and_then(|t| vdf::parse(&t).ok());
    if let Some(list) = old.as_ref().and_then(|d| d.path(&["AppState", "InstallScripts"])) {
        for (depot, rel) in list.iter() {
            if let (Ok(d), Some(rel)) = (depot.parse::<u32>(), rel.as_str()) {
                if !manifests.iter().any(|m| m.depot == d) && !out.iter().any(|(x, _)| *x == d) {
                    out.push((d, rel.to_string()));
                }
            }
        }
    }
    out
}

/// Download the shared redistributable depots a game lists (another app's
/// depots, e.g. 228980 "Steamworks Common Redistributables") into that
/// app's folder, and keep its app manifest (depots and install scripts) up
/// to date like Steam does. Already-current depots are skipped. Returns the
/// game's shared depots with the app they come from.
async fn install_shared(conn: &Connection, cdn: &Arc<depot::Cdn>, app: &OwnedApp, req: &InstallRequest, progress: &Arc<depot::Progress>) -> Result<Vec<(u32, AppId)>, String> {
    let wanted: Vec<(u32, AppId)> = app
        .depots
        .iter()
        .filter(|d| d.shared_install && model::os_matches(&d.os, model::THIS_OS))
        .filter_map(|d| Some((d.id, d.from_app?)))
        .collect();
    let mut by_app: std::collections::BTreeMap<AppId, Vec<u32>> = Default::default();
    for (depot, from) in &wanted {
        by_app.entry(*from).or_default().push(*depot);
    }
    for (from, depots) in by_app {
        let Some(info) = crate::pics::raw_app_info(conn, from).await else { continue };
        let source = crate::pics::parse_depots(&info);
        let install_dir = depot::install::validate_folder_name(info.path(&["config"]).and_then(|c| c.str("installdir")).unwrap_or("Steamworks Shared"))
            .map_err(|e| e.to_string())?;
        let name = info.path(&["common"]).and_then(|c| c.str("name")).unwrap_or("Steamworks Common Redistributables").to_string();
        let common = req.library.join("steamapps").join("common");
        let dir = depot::install::resolve_inside(&common, &install_dir).map_err(|e| e.to_string())?;
        let acf_path = req.library.join("steamapps").join(format!("appmanifest_{from}.acf"));
        let acf = std::fs::read_to_string(&acf_path).ok().and_then(|t| vdf::parse(&t).ok());
        let have = |depot: u32| acf.as_ref().and_then(|d| d.path(&["AppState", "InstalledDepots", &depot.to_string()])).and_then(|o| o.u64("manifest"));
        let mut jobs = Vec::new();
        for depot in depots {
            let Some((gid, _)) = source.iter().find(|d| d.id == depot).and_then(|d| d.manifest("public")) else { continue };
            if have(depot) == Some(gid) && dir.exists() {
                continue;
            }
            let key = match depot_key(conn, from, depot).await {
                Ok(k) => k,
                Err(_) => match depot_key(conn, app.app_id, depot).await {
                    Ok(k) => k,
                    Err(e) => {
                        tracing::info!("shared depot {depot}: {e}");
                        continue;
                    }
                },
            };
            let code = request_code(conn, from, depot, gid, "public").await;
            let raw = cdn.manifest(depot, gid, code).await.map_err(|e| e.to_string())?;
            let mut manifest = depot::Manifest::from_cdn(&raw).map_err(|e| e.to_string())?;
            manifest.decrypt_names(&key).map_err(|e| e.to_string())?;
            let old = have(depot)
                .and_then(|m| std::fs::read(req.steam_root.join("depotcache").join(format!("{depot}_{m}.manifest"))).ok())
                .and_then(|b| depot::Manifest::parse(&b).ok());
            jobs.push(depot::DepotJob { depot, manifest, key, old });
        }
        if jobs.is_empty() {
            continue;
        }
        tracing::info!("installing {} shared depot(s) of {name} for {}", jobs.len(), app.name);
        let manifests: Vec<depot::Manifest> = jobs.iter().map(|j| j.manifest.clone()).collect();
        depot::install(cdn.clone(), &dir, jobs, progress.clone()).await.map_err(|e| e.to_string())?;
        let cache = req.steam_root.join("depotcache");
        let _ = std::fs::create_dir_all(&cache);
        for m in &manifests {
            let _ = std::fs::write(cache.join(format!("{}_{}.manifest", m.depot, m.gid)), m.to_bytes());
        }
        write_shared_manifest(&acf_path, acf.as_ref(), from, &name, &install_dir, &manifests).map_err(|e| e.to_string())?;
    }
    Ok(wanted)
}

/// The shared app's manifest (`appmanifest_228980.acf`): its depots and
/// their install scripts, merged with what was there.
fn write_shared_manifest(path: &std::path::Path, old: Option<&vdf::Obj>, app_id: AppId, name: &str, install_dir: &str, new: &[depot::Manifest]) -> std::io::Result<()> {
    let old_state = old.and_then(|d| d.obj("AppState"));
    let mut depots: Vec<(String, u64, u64)> = old_state
        .and_then(|s| s.obj("InstalledDepots"))
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.to_string(), v.as_obj()?.u64("manifest")?, v.as_obj()?.u64("size").unwrap_or(0)))).collect())
        .unwrap_or_default();
    let mut scripts: Vec<(String, String)> = old_state
        .and_then(|s| s.obj("InstallScripts"))
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    for m in new {
        let id = m.depot.to_string();
        depots.retain(|(d, _, _)| *d != id);
        depots.push((id.clone(), m.gid, m.original_size));
        scripts.retain(|(d, _)| *d != id);
        if let Some(f) = m.files.iter().find(|f| f.install_script) {
            scripts.push((id, f.name.replace('/', "\\")));
        }
    }
    depots.sort();
    scripts.sort();
    let mut installed = vdf::Obj::default();
    for (d, gid, size) in &depots {
        installed = installed.with_obj(d, vdf::Obj::default().with("manifest", gid.to_string()).with("size", size.to_string()));
    }
    let mut list = vdf::Obj::default();
    for (d, rel) in &scripts {
        list = list.with(d, rel);
    }
    let size: u64 = depots.iter().map(|(_, _, s)| s).sum();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let state = vdf::Obj::default()
        .with("appid", app_id.to_string())
        .with("universe", "1")
        .with("name", name)
        .with("StateFlags", "4")
        .with("installdir", install_dir)
        .with("LastUpdated", now.to_string())
        .with("SizeOnDisk", size.to_string())
        .with_obj("InstalledDepots", installed)
        .with_obj("InstallScripts", list);
    std::fs::write(path, vdf::write(&vdf::Obj::default().with_obj("AppState", state)))
}

/// Marks a game folder this client created for a download that hasn't
/// finished yet.
const NEW_INSTALL: &str = ".sirnew";

/// A first install (not an update): Steam has no record of the game being
/// installed, and the folder doesn't exist yet or was created by a download
/// of ours that never finished.
fn is_new_install(installed: &[(u32, u64)], dir: &Path) -> bool {
    installed.is_empty() && (!dir.exists() || dir.join(NEW_INSTALL).exists())
}

pub fn game_dir(req: &InstallRequest, app: &OwnedApp) -> PathBuf {
    req.library.join("steamapps").join("common").join(if app.install_dir.is_empty() { &app.name } else { &app.install_dir })
}

/// A download taken off the queue before it ran again (one left from the
/// last session, say): remove what its earlier attempts left on disk, as
/// cancelling a running download does. An update keeps the installed game.
pub fn discard(req: &InstallRequest, app: &OwnedApp) {
    let Ok(folder) = install_folder_name(app) else {
        return;
    };
    let common = req.library.join("steamapps").join("common");
    let Ok(dir) = depot::install::resolve_inside(&common, &folder) else {
        return;
    };
    if !dir.exists() {
        return;
    }
    let fresh = is_new_install(&installed_manifests(req), &dir);
    remove_unfinished(req, &dir, fresh);
}

/// Delete the files of depots that are no longer installed (a DLC turned
/// off), except files another installed depot still has.
fn remove_dropped(req: &InstallRequest, dir: &Path, dropped: &[(u32, u64)], kept: &[depot::Manifest]) {
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();
    let keep: HashSet<String> = kept.iter().flat_map(|m| m.files.iter().map(|f| norm(&f.name))).collect();
    let cache = req.steam_root.join("depotcache");
    for (depot, gid) in dropped {
        let Some(m) = std::fs::read(cache.join(format!("{depot}_{gid}.manifest"))).ok().and_then(|b| depot::Manifest::parse(&b).ok()) else {
            tracing::warn!("depot {depot} dropped but its manifest isn't cached; its files stay");
            continue;
        };
        let mut removed = 0;
        for f in m.files.iter().filter(|f| !f.directory) {
            if keep.contains(&norm(&f.name)) {
                continue;
            }
            if let Ok(path) = depot::install::resolve_inside(dir, &f.name)
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
        tracing::info!("depot {depot} no longer installed: removed {removed} files");
    }
}

/// After a cancelled update: delete the files of depots it was adding,
/// except files the depots installed before also have.
fn remove_added(req: &InstallRequest, dir: &Path, added: &[&depot::Manifest], before: &[(u32, u64)]) {
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();
    let cache = req.steam_root.join("depotcache");
    let keep: HashSet<String> = before
        .iter()
        .filter_map(|(d, g)| std::fs::read(cache.join(format!("{d}_{g}.manifest"))).ok().and_then(|b| depot::Manifest::parse(&b).ok()))
        .flat_map(|m| m.files.into_iter().map(|f| norm(&f.name)))
        .collect();
    let mut removed = 0;
    for m in added {
        for f in m.files.iter().filter(|f| !f.directory && !keep.contains(&norm(&f.name))) {
            if let Ok(path) = depot::install::resolve_inside(dir, &f.name)
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
    }
    tracing::info!("cancelled: removed {removed} files of depots this update was adding");
}

/// After a cancelled download: delete the folder of a new install, or just
/// the unfinished `*.sirpart` files of an update (the game stays playable).
fn remove_unfinished(req: &InstallRequest, dir: &Path, fresh: bool) {
    // Only ever inside <library>/steamapps/common/<game>.
    let common = req.library.join("steamapps").join("common");
    if let Err(e) = local::validate_common_child(&common, dir) {
        tracing::warn!("cancelled: {e}");
        return;
    }
    let Ok(common_meta) = std::fs::symlink_metadata(&common) else {
        return;
    };
    if local::is_symlink_or_reparse(&common_meta) || !common_meta.is_dir() {
        tracing::warn!("cancelled: refusing symlink or non-directory {}", common.display());
        return;
    }
    let dir_meta = match std::fs::symlink_metadata(dir) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!("cancelled: couldn't inspect {}: {e}", dir.display());
            return;
        }
    };
    if local::is_symlink_or_reparse(&dir_meta) {
        if fresh {
            match local::unlink_symlink_or_junction(dir, &dir_meta) {
                Ok(()) => tracing::info!("cancelled: unlinked symlink {}", dir.display()),
                Err(e) => tracing::warn!("cancelled: couldn't unlink {}: {e}", dir.display()),
            }
        } else {
            tracing::warn!("cancelled: refusing to walk symlink {}", dir.display());
        }
        return;
    }
    if !dir_meta.is_dir() {
        return;
    }
    let (Ok(canon_common), Ok(canon_dir)) = (common.canonicalize(), dir.canonicalize()) else {
        tracing::warn!("cancelled: couldn't canonicalize {}", dir.display());
        return;
    };
    if canon_dir == canon_common || canon_dir.parent() != Some(canon_common.as_path()) {
        tracing::warn!("cancelled: refusing {} outside {}", dir.display(), common.display());
        return;
    }
    if fresh {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => tracing::info!("cancelled: removed {}", dir.display()),
            Err(e) => tracing::warn!("cancelled: couldn't remove {}: {e}", dir.display()),
        }
        return;
    }
    // The note of what those files held goes with them.
    depot::resume::Record::remove(dir);
    let mut stack = vec![dir.to_path_buf()];
    let mut freed = 0u64;
    while let Some(d) = stack.pop() {
        let Ok(dm) = std::fs::symlink_metadata(&d) else { continue };
        if local::is_symlink_or_reparse(&dm) || !dm.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(em) = std::fs::symlink_metadata(&path) else { continue };
            if local::is_symlink_or_reparse(&em) {
                continue;
            }
            if em.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "sirpart") {
                freed += em.len();
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    tracing::info!("cancelled: removed {} MB of unfinished files in {}", freed / 1_048_576, dir.display());
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

fn write_app_manifest(req: &InstallRequest, app: &OwnedApp, install_dir: &str, depots: &[(u32, u64, u64, Option<AppId>)], size: u64, owner: u64, scripts: &[(u32, String)], shared: &[(u32, AppId)]) -> std::io::Result<()> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let build = app.branches.iter().find(|b| b.name == req.branch).map(|b| b.build_id).unwrap_or(0);
    let mut installed = vdf::Obj::default();
    for (depot, gid, size, dlc) in depots {
        let mut d = vdf::Obj::default().with("manifest", gid.to_string()).with("size", size.to_string());
        if let Some(dlc) = dlc {
            d = d.with("dlcappid", dlc.to_string());
        }
        installed = installed.with_obj(&depot.to_string(), d);
    }
    let mut user = vdf::Obj::default().with("language", &req.language);
    if req.branch != "public" {
        user = user.with("BetaKey", &req.branch);
    }
    let mut state = vdf::Obj::default()
        .with("appid", app.app_id.to_string())
        .with("universe", "1")
        .with("name", &app.name)
        .with("StateFlags", "4")
        .with("installdir", install_dir)
        .with("LastUpdated", now.to_string())
        .with("SizeOnDisk", size.to_string())
        .with("buildid", build.to_string())
        .with("LastOwner", owner.to_string())
        .with("AutoUpdateBehavior", "0")
        .with_obj("InstalledDepots", installed);
    // Like Steam: the install scripts per depot, and the shared
    // redistributables (depot -> the app they come from).
    if !scripts.is_empty() {
        state = state.with_obj("InstallScripts", scripts.iter().fold(vdf::Obj::default(), |o, (d, rel)| o.with(&d.to_string(), rel)));
    }
    if !shared.is_empty() {
        state = state.with_obj("SharedDepots", shared.iter().fold(vdf::Obj::default(), |o, (d, from)| o.with(&d.to_string(), from.to_string())));
    }
    let state = state.with_obj("UserConfig", user.clone()).with_obj("MountedConfig", user);
    let doc = vdf::Obj::default().with_obj("AppState", state);
    std::fs::write(req.library.join("steamapps").join(format!("appmanifest_{}.acf", app.app_id)), vdf::write(&doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_windows_64bit_language_depots() {
        let d = |id, os: &str, arch: &str, lang: &str, dlc: Option<u32>| model::DepotInfo {
            id,
            os: os.into(),
            arch: arch.into(),
            language: lang.into(),
            dlc_app: dlc,
            manifests: vec![("public".into(), 1, 1)],
            ..Default::default()
        };
        let app = OwnedApp {
            app_id: 10,
            depots: vec![d(11, "windows", "64", "", None), d(12, "windows", "32", "", None), d(13, "linux", "", "", None), d(14, "", "", "german", None), d(15, "windows", "", "", Some(99))],
            ..Default::default()
        };
        let req = InstallRequest { app_id: 10, library: ".".into(), steam_root: ".".into(), branch: "public".into(), language: "english".into(), skip_dlc: vec![], add_dlc: vec![] };
        let fresh = HashSet::new();
        let ids: Vec<u32> = pick_depots_for("windows", &app, &HashSet::new(), &req, &fresh).iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![11]);
        let owned: HashSet<u32> = [99].into_iter().collect();
        // A new install gets the owned DLC.
        let ids: Vec<u32> = pick_depots_for("windows", &app, &owned, &req, &fresh).iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![11, 15]);
        // An update of an install without it doesn't add it...
        let installed: HashSet<u32> = [11].into_iter().collect();
        let ids: Vec<u32> = pick_depots_for("windows", &app, &owned, &req, &installed).iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![11]);
        // ...unless the user turned it on.
        let on = InstallRequest { add_dlc: vec![99], ..req.clone() };
        let ids: Vec<u32> = pick_depots_for("windows", &app, &owned, &on, &installed).iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![11, 15]);
        // Installed DLC stays, and goes when turned off.
        let both: HashSet<u32> = [11, 15].into_iter().collect();
        assert_eq!(pick_depots_for("windows", &app, &owned, &req, &both).len(), 2);
        let off = InstallRequest { skip_dlc: vec![99], ..req.clone() };
        assert_eq!(pick_depots_for("windows", &app, &owned, &off, &both).iter().map(|d| d.id).collect::<Vec<_>>(), vec![11]);
        // An update keeps an installed depot in another language, or 32-bit.
        let ru = InstallRequest { language: "russian".into(), ..req.clone() };
        let with_lang: HashSet<u32> = [11, 14].into_iter().collect();
        let ids: Vec<u32> = pick_depots_for("windows", &app, &HashSet::new(), &ru, &with_lang).iter().map(|d| d.id).collect();
        assert!(ids.contains(&14), "kept the installed german depot: {ids:?}");
        let with32: HashSet<u32> = [11, 12].into_iter().collect();
        let ids: Vec<u32> = pick_depots_for("windows", &app, &HashSet::new(), &req, &with32).iter().map(|d| d.id).collect();
        assert!(ids.contains(&12), "kept the installed 32-bit depot: {ids:?}");
    }
}

#[test]
fn picks_mac_depots() {
    let d = |id, os: &str| model::DepotInfo { id, os: os.into(), manifests: vec![("public".into(), 1, 1)], ..Default::default() };
    let app = OwnedApp { app_id: 10, depots: vec![d(11, "windows"), d(12, "macos"), d(13, ""), d(14, "windows,macos")], ..Default::default() };
    let req = InstallRequest { app_id: 10, library: ".".into(), steam_root: ".".into(), branch: "public".into(), language: "english".into(), skip_dlc: vec![], add_dlc: vec![] };
    let ids: Vec<u32> = pick_depots_for("macos", &app, &HashSet::new(), &req, &HashSet::new()).iter().map(|d| d.id).collect();
    assert_eq!(ids, vec![12, 13, 14]);
}

#[cfg(test)]
mod cancel_tests {
    use super::*;

    #[test]
    fn cancel_removes_only_unfinished_files() {
        let lib = std::env::temp_dir().join(format!("sir-cancel-{}", std::process::id()));
        let game = lib.join("steamapps").join("common").join("Game");
        std::fs::create_dir_all(game.join("data")).unwrap();
        std::fs::write(game.join("game.exe"), b"done").unwrap();
        std::fs::write(game.join("data").join("big.pak.sirpart"), b"half").unwrap();
        std::fs::write(game.join(".sirresume"), b"sirresume 1\n").unwrap();
        let req = InstallRequest { app_id: 1, library: lib.clone(), steam_root: lib.clone(), branch: "public".into(), language: "english".into(), skip_dlc: vec![], add_dlc: vec![] };
        // An update: the finished file stays, the unfinished one goes.
        remove_unfinished(&req, &game, false);
        assert!(game.join("game.exe").exists());
        assert!(!game.join("data").join("big.pak.sirpart").exists());
        assert!(!game.join(".sirresume").exists());
        // A new install: the whole folder goes.
        remove_unfinished(&req, &game, true);
        assert!(!game.exists());
        // Never outside steamapps/common.
        let other = lib.join("elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        remove_unfinished(&req, &other, true);
        assert!(other.exists());
        let _ = std::fs::remove_dir_all(lib);
    }

    #[test]
    fn remove_unfinished_rejects_dotdot_and_unlinks_only_symlink() {
        let lib = std::env::temp_dir().join(format!("sir-cancel-sec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&lib);
        let apps = lib.join("steamapps");
        let common = apps.join("common");
        let compat = apps.join("compatdata");
        std::fs::create_dir_all(&common).unwrap();
        std::fs::create_dir_all(&compat).unwrap();
        let keep = compat.join("keep.sirpart");
        std::fs::write(&keep, b"keep").unwrap();

        let req = InstallRequest {
            app_id: 1,
            library: lib.clone(),
            steam_root: lib.clone(),
            branch: "public".into(),
            language: "english".into(),
            skip_dlc: vec![],
            add_dlc: vec![],
        };

        for bad_dir in [common.join(".."), common.join("."), common.join("sub").join("..")] {
            remove_unfinished(&req, &bad_dir, true);
            remove_unfinished(&req, &bad_dir, false);
            assert!(keep.exists(), "remove_unfinished deleted outside common via {bad_dir:?}!");
            assert!(common.exists(), "remove_unfinished deleted common via {bad_dir:?}!");
        }

        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join(format!("sir-cancel-ext-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&outside);
            std::fs::create_dir_all(&outside).unwrap();
            let ext_part = outside.join("ext.sirpart");
            let ext_file = outside.join("ext.exe");
            std::fs::write(&ext_part, b"part").unwrap();
            std::fs::write(&ext_file, b"exe").unwrap();
            let sym = common.join("SymGame");
            std::os::unix::fs::symlink(&outside, &sym).unwrap();

            remove_unfinished(&req, &sym, false);
            assert!(ext_part.exists(), "remove_unfinished(fresh=false) walked into symlink!");
            assert!(ext_file.exists());

            remove_unfinished(&req, &sym, true);
            assert!(ext_part.exists(), "remove_unfinished(fresh=true) recursed into symlink target!");
            assert!(ext_file.exists(), "remove_unfinished(fresh=true) deleted symlink target!");
            assert!(std::fs::symlink_metadata(&sym).is_err(), "symlink itself was not unlinked!");
            let _ = std::fs::remove_dir_all(&outside);
        }

        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn cancelling_a_queued_download_cleans_up_after_a_restart() {
        let lib = std::env::temp_dir().join(format!("sir-discard-{}", std::process::id()));
        let game = lib.join("steamapps").join("common").join("Game");
        let app = OwnedApp { app_id: 7, name: "Game".into(), ..Default::default() };
        let req = InstallRequest { app_id: 7, library: lib.clone(), steam_root: lib.clone(), branch: "public".into(), language: "english".into(), skip_dlc: vec![], add_dlc: vec![] };
        let leave = |game: &Path| {
            std::fs::create_dir_all(game).unwrap();
            std::fs::write(game.join("done.pak"), b"done").unwrap();
            std::fs::write(game.join("big.pak.sirpart"), b"half").unwrap();
            std::fs::write(game.join(".sirresume"), b"sirresume 1\n").unwrap();
        };
        // A new install left by the last session: the whole folder goes.
        leave(&game);
        std::fs::write(game.join(NEW_INSTALL), b"").unwrap();
        assert!(is_new_install(&[], &game));
        discard(&req, &app);
        assert!(!game.exists());
        // An update of an installed game: only the unfinished parts go.
        leave(&game);
        std::fs::write(lib.join("steamapps").join("appmanifest_7.acf"), "\"AppState\" { \"InstalledDepots\" { \"8\" { \"manifest\" \"1\" } } }").unwrap();
        discard(&req, &app);
        assert!(game.join("done.pak").exists());
        assert!(!game.join("big.pak.sirpart").exists() && !game.join(".sirresume").exists());
        // A folder we didn't create is never treated as a new install.
        std::fs::remove_file(lib.join("steamapps").join("appmanifest_7.acf")).unwrap();
        assert!(!is_new_install(&[], &game));
        let _ = std::fs::remove_dir_all(lib);
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn dropped_and_added_cleanup_rejects_traversal() {
        let root = std::env::temp_dir().join(format!("sir-dl-aur02-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("steamapps").join("common").join("Game");
        let cache = root.join("depotcache");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let victim = root.join("steamapps").join("common").join("victim.txt");
        std::fs::write(&victim, b"precious").unwrap();
        std::fs::write(game.join("dlc.pak"), b"dlc").unwrap();

        let bad_manifest = depot::Manifest {
            depot: 77,
            gid: 1,
            files: vec![
                depot::manifest::FileEntry {
                    name: "../victim.txt".into(),
                    size: 8,
                    sha: vec![0; 20],
                    chunks: vec![],
                    directory: false,
                    executable: false,
                    install_script: false,
                    link: None,
                },
                depot::manifest::FileEntry {
                    name: "dlc.pak".into(),
                    size: 3,
                    sha: vec![0; 20],
                    chunks: vec![],
                    directory: false,
                    executable: false,
                    install_script: false,
                    link: None,
                },
            ],
            ..Default::default()
        };
        std::fs::write(cache.join("77_1.manifest"), bad_manifest.to_bytes()).unwrap();
        let req = InstallRequest {
            app_id: 1,
            library: root.clone(),
            steam_root: root.clone(),
            branch: "public".into(),
            language: "english".into(),
            skip_dlc: vec![],
            add_dlc: vec![],
        };
        remove_dropped(&req, &game, &[(77, 1)], &[]);
        assert!(victim.exists(), "remove_dropped deleted outside install dir!");
        assert!(!game.join("dlc.pak").exists());

        std::fs::write(game.join("dlc.pak"), b"dlc").unwrap();
        remove_added(&req, &game, &[&bad_manifest], &[]);
        assert!(victim.exists(), "remove_added deleted outside install dir!");
        assert!(!game.join("dlc.pak").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_folder_name_rejects_traversal() {
        let mut bad_names = vec!["..", ".", "../evil", "a/b", r"a\b"];
        // Only Windows can't hold these (fine as Mac and Linux folders).
        if cfg!(windows) {
            bad_names.extend(["CON", "evil."]);
        }
        for bad in bad_names {
            let app = OwnedApp { app_id: 1, install_dir: bad.into(), name: "Game".into(), ..Default::default() };
            assert!(install_folder_name(&app).is_err(), "expected {bad:?} rejected");
        }
    }
}

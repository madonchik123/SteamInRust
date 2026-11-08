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
}

/// Choose the depots a Windows 64-bit install needs.
pub fn pick_depots<'a>(app: &'a OwnedApp, owned: &HashSet<AppId>, req: &InstallRequest) -> Vec<&'a model::DepotInfo> {
    let os_ok = |d: &&model::DepotInfo| d.os.is_empty() || d.os.split(',').any(|o| o.trim() == "windows");
    let lang_ok = |d: &&model::DepotInfo| d.language.is_empty() || d.language == req.language;
    let mut out: Vec<&model::DepotInfo> = app
        .depots
        .iter()
        .filter(os_ok)
        .filter(lang_ok)
        .filter(|d| !d.low_violence && !d.shared_install && !d.manifests.is_empty())
        .filter(|d| match d.dlc_app {
            Some(dlc) => owned.contains(&dlc) && !req.skip_dlc.contains(&dlc),
            None => true,
        })
        .collect();
    // Prefer 64-bit depots when both exist.
    let has64 = out.iter().any(|d| d.arch == "64");
    if has64 {
        out.retain(|d| d.arch != "32");
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
            https: s.https_support() != "none" && !s.https_support().is_empty(),
            load: s.weighted_load(),
        })
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
    let depots = pick_depots(&app, &owned, &req);
    if depots.is_empty() {
        return Err(format!("{} has no downloadable content for Windows", app.name));
    }
    let mut servers = content_servers(&conn, cell_id).await?;
    // A host the player picked goes first (the others stay as fallbacks).
    if let Some(host) = preferred_host {
        servers.retain(|s| s.host != host);
        servers.insert(0, depot::Server { host, https: true, load: 0.0 });
    }
    if servers.is_empty() {
        return Err("Steam returned no content servers".into());
    }
    let cdn = Arc::new(depot::Cdn::new(servers));
    cdn.rank().await;

    let old = installed_manifests(&req);
    let mut jobs = Vec::new();
    let mut installed = Vec::new();
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

    let dir = req.library.join("steamapps").join("common").join(if app.install_dir.is_empty() { &app.name } else { &app.install_dir });
    // Make sure it fits (a rough upper bound: the full new size).
    let need: u64 = jobs.iter().map(|j| j.manifest.original_size).sum();
    if let Ok(free) = fs4::available_space(&req.library) {
        let have: u64 = dir_size(&dir);
        if need > free + have {
            return Err(format!("Not enough disk space: needs {} MB more", (need - free - have) / 1_048_576));
        }
    }

    let manifests: Vec<depot::Manifest> = jobs.iter().map(|j| j.manifest.clone()).collect();
    let report = depot::install(cdn, &dir, jobs, progress).await.map_err(|e| e.to_string())?;

    // Leave the same records Steam does, so both clients agree.
    let cache = req.steam_root.join("depotcache");
    let _ = std::fs::create_dir_all(&cache);
    for m in &manifests {
        let _ = std::fs::write(cache.join(format!("{}_{}.manifest", m.depot, m.gid)), m.to_bytes());
    }
    write_app_manifest(&req, &app, &installed, need, owner).map_err(|e| e.to_string())?;
    Ok(report)
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

fn write_app_manifest(req: &InstallRequest, app: &OwnedApp, depots: &[(u32, u64, u64, Option<AppId>)], size: u64, owner: u64) -> std::io::Result<()> {
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
    let state = vdf::Obj::default()
        .with("appid", app.app_id.to_string())
        .with("universe", "1")
        .with("name", &app.name)
        .with("StateFlags", "4")
        .with("installdir", if app.install_dir.is_empty() { &app.name } else { &app.install_dir })
        .with("LastUpdated", now.to_string())
        .with("SizeOnDisk", size.to_string())
        .with("buildid", build.to_string())
        .with("LastOwner", owner.to_string())
        .with("AutoUpdateBehavior", "0")
        .with_obj("InstalledDepots", installed)
        .with_obj("UserConfig", user.clone())
        .with_obj("MountedConfig", user);
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
        let req = InstallRequest { app_id: 10, library: ".".into(), steam_root: ".".into(), branch: "public".into(), language: "english".into(), skip_dlc: vec![] };
        let ids: Vec<u32> = pick_depots(&app, &HashSet::new(), &req).iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![11]);
        let owned: HashSet<u32> = [99].into_iter().collect();
        let ids: Vec<u32> = pick_depots(&app, &owned, &req).iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![11, 15]);
    }
}

//! Downloading subscribed Workshop items, the way Steam does: into
//! `<library>/steamapps/workshop/content/<app>/<item>`, recorded in
//! `steamapps/workshop/appworkshop_<app>.acf`. Games find them there through
//! ISteamUGC (our steamclient DLL reads the same folder).
//!
//! Most items are depot content: the item's `hcontent_file` is a manifest in
//! the game's own depot (depot id = app id), fetched and installed like a
//! game. Old ("legacy") items are a single file at `file_url`.

use model::{AppId, SteamId};
use net::Connection;
use proto::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What a sync did.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Items downloaded or updated: (id, title).
    pub downloaded: Vec<(u64, String)>,
    /// Items that failed: (id, title, why).
    pub failed: Vec<(u64, String, String)>,
}

/// The library folder (the one with `steamapps`) for a game's install folder
/// `<library>/steamapps/common/<dir>`.
pub fn library_of(install_dir: &Path) -> Option<PathBuf> {
    let common = install_dir.parent()?;
    let steamapps = common.parent()?;
    let named = |p: &Path, n: &str| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n));
    (named(common, "common") && named(steamapps, "steamapps")).then(|| steamapps.parent().map(Path::to_path_buf)).flatten()
}

/// Every item this account subscribes to for `app`.
pub async fn subscribed(conn: &Connection, me: SteamId, app: AppId) -> Result<Vec<u64>, String> {
    let mut ids = Vec::new();
    for page in 1..=20 {
        let req = CPublishedFileGetUserFilesRequest {
            steamid: Some(me),
            appid: Some(app),
            page: Some(page),
            numperpage: Some(100),
            r#type: Some("mysubscriptions".into()),
            ids_only: Some(true),
            return_vote_data: Some(false),
            ..Default::default()
        };
        let r: CPublishedFileGetUserFilesResponse = conn.call("PublishedFile.GetUserFiles#1", &req).await.map_err(|e| e.to_string())?;
        let got = r.publishedfiledetails.len();
        ids.extend(r.publishedfiledetails.iter().map(|d| d.publishedfileid()));
        if got < 100 || ids.len() as u32 >= r.total() {
            break;
        }
    }
    Ok(ids)
}

/// Download (or update) `items`, or every subscribed item when `items` is
/// empty. Items already installed at the current version are skipped.
pub async fn sync(conn: &Connection, me: SteamId, app: AppId, library: &Path, cell_id: u32, items: Vec<u64>) -> Result<Report, String> {
    let ids = if items.is_empty() { subscribed(conn, me, app).await? } else { items };
    let mut report = Report::default();
    if ids.is_empty() {
        return Ok(report);
    }
    let req = CPublishedFileGetDetailsRequest { publishedfileids: ids, includechildren: Some(true), ..Default::default() };
    let details: CPublishedFileGetDetailsResponse = conn.call("PublishedFile.GetDetails#1", &req).await.map_err(|e| e.to_string())?;
    let ws = library.join("steamapps").join("workshop");
    let mut record = Record::load(&ws, app);
    let mut cdn: Option<Arc<depot::Cdn>> = None;
    for d in details.publishedfiledetails.iter().filter(|d| d.result() == 1) {
        let id = d.publishedfileid();
        let title = d.title().to_string();
        let manifest = d.hcontent_file();
        let dir = ws.join("content").join(app.to_string()).join(id.to_string());
        if record.installed.get(&id).is_some_and(|i| i.manifest == manifest && i.updated >= d.time_updated()) && dir.is_dir() {
            continue;
        }
        let result = if manifest != 0 {
            if cdn.is_none() {
                let servers = crate::download::content_servers(conn, cell_id).await?;
                let c = Arc::new(depot::Cdn::new(servers));
                c.rank().await;
                cdn = Some(c);
            }
            depot_item(conn, cdn.clone().unwrap(), app, manifest, &dir).await
        } else if !d.file_url().is_empty() {
            legacy_item(d.file_url(), d.filename(), &dir).await
        } else {
            Err("Steam has no files for this item".into())
        };
        match result {
            Ok(size) => {
                tracing::info!("workshop {app}/{id} ({title}): {size} bytes");
                record.installed.insert(id, Installed { size, updated: d.time_updated(), manifest, subscribed_by: me });
                // Saved after each item, so an interrupted sync keeps what's done.
                record.save(&ws, app).map_err(|e| e.to_string())?;
                report.downloaded.push((id, title));
            }
            Err(e) => {
                tracing::warn!("workshop {app}/{id}: {e}");
                report.failed.push((id, title, e));
            }
        }
    }
    Ok(report)
}

/// An item stored as a manifest in the game's depot (depot id = app id).
async fn depot_item(conn: &Connection, cdn: Arc<depot::Cdn>, app: AppId, gid: u64, dir: &Path) -> Result<u64, String> {
    let key = crate::download::depot_key(conn, app, app).await?;
    let code = crate::download::request_code(conn, app, app, gid, "public").await;
    let raw = cdn.manifest(app, gid, code).await.map_err(|e| e.to_string())?;
    let mut manifest = depot::Manifest::from_cdn(&raw).map_err(|e| e.to_string())?;
    manifest.decrypt_names(&key).map_err(|e| e.to_string())?;
    let size = manifest.original_size;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let job = depot::DepotJob { depot: app, manifest, key, old: None };
    depot::install(cdn, dir, vec![job], Arc::new(depot::Progress::default())).await.map_err(|e| e.to_string())?;
    Ok(size)
}

/// An old-style item: one file on Steam's UGC servers.
async fn legacy_item(url: &str, filename: &str, dir: &Path) -> Result<u64, String> {
    let name = Path::new(filename).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "item".into());
    let bytes = reqwest::get(url).await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(name), &bytes).map_err(|e| e.to_string())?;
    Ok(bytes.len() as u64)
}

struct Installed {
    size: u64,
    updated: u32,
    manifest: u64,
    subscribed_by: SteamId,
}

/// `appworkshop_<app>.acf`, the record Steam keeps of installed items.
struct Record {
    installed: BTreeMap<u64, Installed>,
}

impl Record {
    fn load(ws: &Path, app: AppId) -> Record {
        let mut installed = BTreeMap::new();
        let text = std::fs::read_to_string(ws.join(format!("appworkshop_{app}.acf"))).unwrap_or_default();
        if let Ok(doc) = vdf::parse(&text) {
            let details = doc.path(&["AppWorkshop", "WorkshopItemDetails"]);
            if let Some(items) = doc.path(&["AppWorkshop", "WorkshopItemsInstalled"]) {
                for (id, v) in items.iter() {
                    let (Ok(id), Some(o)) = (id.parse::<u64>(), v.as_obj()) else { continue };
                    let by = details.and_then(|d| d.obj(&id.to_string())).and_then(|d| d.u64("subscribedby")).unwrap_or(0);
                    installed.insert(
                        id,
                        Installed { size: o.u64("size").unwrap_or(0), updated: o.u64("timeupdated").unwrap_or(0) as u32, manifest: o.u64("manifest").unwrap_or(0), subscribed_by: by },
                    );
                }
            }
        }
        Record { installed }
    }

    fn save(&self, ws: &Path, app: AppId) -> std::io::Result<()> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut installed = vdf::Obj::default();
        let mut details = vdf::Obj::default();
        for (id, i) in &self.installed {
            installed = installed.with_obj(
                &id.to_string(),
                vdf::Obj::default().with("size", i.size.to_string()).with("timeupdated", i.updated.to_string()).with("manifest", i.manifest.to_string()),
            );
            details = details.with_obj(
                &id.to_string(),
                vdf::Obj::default()
                    .with("manifest", i.manifest.to_string())
                    .with("timeupdated", i.updated.to_string())
                    .with("timetouched", now.to_string())
                    .with("subscribedby", i.subscribed_by.to_string()),
            );
        }
        let total: u64 = self.installed.values().map(|i| i.size).sum();
        let doc = vdf::Obj::default().with_obj(
            "AppWorkshop",
            vdf::Obj::default()
                .with("appid", app.to_string())
                .with("SizeOnDisk", total.to_string())
                .with("NeedsUpdate", "0")
                .with("NeedsDownload", "0")
                .with("TimeLastUpdated", now.to_string())
                .with("TimeLastAppRan", now.to_string())
                .with_obj("WorkshopItemsInstalled", installed)
                .with_obj("WorkshopItemDetails", details),
        );
        std::fs::create_dir_all(ws)?;
        std::fs::write(ws.join(format!("appworkshop_{app}.acf")), vdf::write(&doc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_library_of_an_install() {
        let p = Path::new(r"D:\Games\SteamLibrary\steamapps\common\Human Fall Flat");
        assert_eq!(library_of(p), Some(PathBuf::from(r"D:\Games\SteamLibrary")));
        assert_eq!(library_of(Path::new(r"C:\Games\Somewhere")), None);
    }

    #[test]
    fn record_round_trips() {
        let dir = std::env::temp_dir().join(format!("sir-ws-{}", std::process::id()));
        let mut r = Record { installed: BTreeMap::new() };
        r.installed.insert(42, Installed { size: 10, updated: 5, manifest: 7, subscribed_by: 9 });
        r.save(&dir, 477160).unwrap();
        let back = Record::load(&dir, 477160);
        let i = &back.installed[&42];
        assert_eq!((i.size, i.updated, i.manifest, i.subscribed_by), (10, 5, 7, 9));
        let _ = std::fs::remove_dir_all(dir);
    }
}

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
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const LEGACY_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LEGACY_ITEM_BYTES: usize = 512 * 1024 * 1024;

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
    sync_with_progress(conn, me, app, library, cell_id, items, |_, _, _| {}).await
}

/// [`sync`], telling `progress(done, total, title)` before each item that
/// needs downloading (for "Updating Workshop items… 3 of 12").
pub async fn sync_with_progress(conn: &Connection, me: SteamId, app: AppId, library: &Path, cell_id: u32, items: Vec<u64>, progress: impl Fn(u32, u32, &str)) -> Result<Report, String> {
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
    let dir_of = |id: u64| ws.join("content").join(app.to_string()).join(id.to_string());
    let current = |d: &PublishedFileDetails| record.installed.get(&d.publishedfileid()).is_some_and(|i| i.manifest == d.hcontent_file() && i.updated >= d.time_updated()) && dir_of(d.publishedfileid()).is_dir();
    let todo: Vec<&PublishedFileDetails> = details.publishedfiledetails.iter().filter(|d| d.result() == 1 && !current(d)).collect();
    let total = todo.len() as u32;
    for (n, d) in todo.into_iter().enumerate() {
        let id = d.publishedfileid();
        let title = d.title().to_string();
        let manifest = d.hcontent_file();
        let dir = dir_of(id);
        progress(n as u32, total, &title);
        let result = if manifest != 0 {
            if cdn.is_none() {
                let servers = crate::download::content_servers(conn, cell_id).await?;
                let c = Arc::new(depot::Cdn::new(servers));
                c.rank().await;
                cdn = Some(c);
            }
            depot_item(conn, cdn.clone().unwrap(), app, manifest, &dir).await
        } else if !d.file_url().is_empty() {
            if d.file_size() > MAX_LEGACY_ITEM_BYTES as u64 {
                Err("Workshop item exceeds 512 MiB limit".into())
            } else {
                let file_sha = ugc_file_sha(conn, app, d.hcontent_file()).await;
                legacy_item(d.file_url(), d.filename(), file_sha.as_bytes(), &dir).await
            }
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

async fn ugc_file_sha(conn: &Connection, app: AppId, ugcid: u64) -> String {
    if ugcid == 0 {
        return String::new();
    }
    conn.call::<_, CCloudGetFileDetailsResponse>(
        "Cloud.GetFileDetails#1",
        &CCloudGetFileDetailsRequest { ugcid: Some(ugcid), appid: Some(app) },
    )
    .await
    .ok()
    .and_then(|r| r.details)
    .and_then(|d| d.file_sha)
    .unwrap_or_default()
}

fn legacy_http() -> &'static reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(LEGACY_TIMEOUT)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many redirects")
                } else if attempt.url().scheme() != "https" {
                    attempt.error("refusing non-HTTPS redirect")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .unwrap_or_default()
    })
}

fn validate_https_url(url: &str) -> Result<reqwest::Url, String> {
    if url.is_empty() || url.contains('\\') || url.chars().any(|c| c.is_control()) {
        return Err("invalid Workshop item URL".into());
    }
    let mut parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid Workshop item URL: {e}"))?;
    // Steam still hands out http:// addresses for older items: upgrade them
    // (its file hosts serve HTTPS) instead of refusing the download.
    if parsed.scheme() == "http" {
        parsed.set_scheme("https").map_err(|_| "invalid Workshop item URL".to_string())?;
    }
    if parsed.scheme() != "https" {
        return Err("refusing non-HTTPS Workshop item URL".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.host_str().is_none_or(|h| h.is_empty()) {
        return Err("invalid Workshop item URL authority".into());
    }
    Ok(parsed)
}

fn is_safe_filename(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.ends_with(' ') || name.ends_with('.') {
        return false;
    }
    if name.contains(['/', '\\', ':', '\0']) || name.chars().any(|c| (c as u32) < 0x20) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("");
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            | "COM0" | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT0" | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    ) {
        return false;
    }
    let mut comps = Path::new(name).components();
    matches!((comps.next(), comps.next()), (Some(std::path::Component::Normal(s)), None) if !s.is_empty())
}

fn is_symlink_or_reparse(meta: &std::fs::Metadata) -> bool {
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

fn safe_legacy_filename(filename: &str) -> String {
    let raw = filename.rsplit(['/', '\\']).next().unwrap_or("");
    if is_safe_filename(raw) { raw.to_string() } else { "item".to_string() }
}

fn decode_hex_sha1(hex_str: &str) -> Option<[u8; 20]> {
    if hex_str.len() != 40 || !hex_str.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 20];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex_str[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn parse_expected_sha1(file_sha: &[u8]) -> Result<Option<[u8; 20]>, String> {
    if file_sha.is_empty() {
        return Ok(None);
    }
    if file_sha.len() == 20 {
        let mut out = [0u8; 20];
        out.copy_from_slice(file_sha);
        return Ok(Some(out));
    }
    let text = std::str::from_utf8(file_sha).map_err(|_| "invalid Workshop SHA-1 checksum".to_string())?.trim();
    if text.is_empty() {
        return Ok(None);
    }
    decode_hex_sha1(text).map(Some).ok_or_else(|| "invalid Workshop SHA-1 checksum".into())
}

fn verify_legacy_sha1(bytes: &[u8], file_sha: &[u8]) -> Result<(), String> {
    if let Some(expected) = parse_expected_sha1(file_sha)? {
        let actual: [u8; 20] = Sha1::digest(bytes).into();
        if actual != expected {
            return Err("Workshop item failed SHA-1 checksum verification".into());
        }
    }
    Ok(())
}

fn append_capped(buf: &mut Vec<u8>, chunk: &[u8], max_bytes: usize) -> Result<(), String> {
    let new_len = buf.len().checked_add(chunk.len()).ok_or("Workshop item size overflow")?;
    if new_len > max_bytes {
        return Err(format!("Workshop item exceeds {} MiB limit", max_bytes / (1024 * 1024)));
    }
    buf.extend_from_slice(chunk);
    Ok(())
}

/// An old-style item: one file on Steam's UGC servers.
async fn legacy_item(url: &str, filename: &str, file_sha: &[u8], dir: &Path) -> Result<u64, String> {
    let parsed_url = validate_https_url(url)?;
    let _ = parse_expected_sha1(file_sha)?;
    let name = safe_legacy_filename(filename);

    let mut resp = legacy_http()
        .get(parsed_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    if resp.url().scheme() != "https" {
        return Err("refusing non-HTTPS Workshop redirect".into());
    }
    if resp.content_length().is_some_and(|len| len > MAX_LEGACY_ITEM_BYTES as u64) {
        return Err("Workshop item exceeds 512 MiB limit".into());
    }
    let init_cap = resp.content_length().unwrap_or(0).min(1024 * 1024) as usize;
    let mut bytes = Vec::with_capacity(init_cap);
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        append_capped(&mut bytes, &chunk, MAX_LEGACY_ITEM_BYTES)?;
    }
    verify_legacy_sha1(&bytes, file_sha)?;

    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let dir_meta = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    if is_symlink_or_reparse(&dir_meta) || !dir_meta.is_dir() {
        return Err("refusing symlink Workshop directory".into());
    }
    let target = dir.join(&name);
    if let Ok(tm) = std::fs::symlink_metadata(&target)
        && is_symlink_or_reparse(&tm)
    {
        return Err("refusing symlink Workshop file".into());
    }
    std::fs::write(target, &bytes).map_err(|e| e.to_string())?;
    Ok(bytes.len() as u64)
}

#[cfg(test)]
mod legacy_tests {
    use super::*;

    #[test]
    fn legacy_item_rejects_plaintext_http() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let hit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let hit_srv = hit.clone();
            let srv = tokio::spawn(async move {
                if let Ok((mut sock, _)) = tokio::time::timeout(std::time::Duration::from_millis(250), listener.accept()).await.unwrap_or(Err(std::io::Error::other("timeout"))) {
                    // Upgraded to HTTPS: only a TLS handshake may arrive, never a plain GET.
                    let mut buf = vec![0u8; 1024];
                    let n = tokio::time::timeout(std::time::Duration::from_millis(250), tokio::io::AsyncReadExt::read(&mut sock, &mut buf)).await.ok().and_then(|r| r.ok()).unwrap_or(0);
                    if buf[..n].starts_with(b"GET") {
                        hit_srv.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut sock, b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntest").await;
                }
            });

            let dir = std::env::temp_dir().join(format!("sir-ws-http-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let url = format!("http://{addr}/item.bin");
            let res = legacy_item(&url, r"sub\mod.pak", &[], &dir).await;
            let _ = srv.await;
            let _ = std::fs::remove_dir_all(&dir);

            assert!(res.is_err(), "a plain-HTTP server served an item that should come over HTTPS");
            assert!(!hit.load(std::sync::atomic::Ordering::SeqCst), "legacy_item sent a plaintext HTTP request!");
        });
    }

    #[test]
    fn legacy_filename_cap_and_sha1_verification() {
        assert_eq!(safe_legacy_filename(r"nested\folder\cool_map.bsp"), "cool_map.bsp");
        assert_eq!(safe_legacy_filename("nested/folder/cool_map.bsp"), "cool_map.bsp");
        assert_eq!(safe_legacy_filename(r"..\.."), "item");
        assert_eq!(safe_legacy_filename(".."), "item");
        assert_eq!(safe_legacy_filename("."), "item");
        assert_eq!(safe_legacy_filename(""), "item");
        assert_eq!(safe_legacy_filename("CON.zip"), "item");

        assert!(validate_https_url("https://steamuserimages-a.akamaihd.net/ugc/123/mod.zip").is_ok());
        // Older items' http:// addresses are upgraded, not refused.
        assert_eq!(validate_https_url("http://steamuserimages-a.akamaihd.net/ugc/123/mod.zip").unwrap().scheme(), "https");
        assert!(validate_https_url("file:///etc/passwd").is_err());
        assert!(validate_https_url("https://user:pass@example.com/mod.zip").is_err());

        let payload = b"workshop-legacy-payload";
        let digest: [u8; 20] = Sha1::digest(payload).into();
        let hex_upper: String = digest.iter().map(|b| format!("{b:02X}")).collect();
        let hex_lower: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert!(verify_legacy_sha1(payload, &[]).is_ok());
        assert!(verify_legacy_sha1(payload, &digest).is_ok());
        assert!(verify_legacy_sha1(payload, hex_upper.as_bytes()).is_ok());
        assert!(verify_legacy_sha1(payload, hex_lower.as_bytes()).is_ok());
        assert!(verify_legacy_sha1(b"tampered", &digest).is_err());
        assert!(verify_legacy_sha1(b"tampered", hex_lower.as_bytes()).is_err());
        assert!(verify_legacy_sha1(payload, b"not-a-valid-sha1").is_err());

        let mut buf = Vec::new();
        assert!(append_capped(&mut buf, &[0u8; 1024], 2048).is_ok());
        assert!(append_capped(&mut buf, &[0u8; 1024], 2048).is_ok());
        assert!(append_capped(&mut buf, &[0u8; 1], 2048).is_err());
    }
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
        let lib = std::env::temp_dir().join("SteamLibrary");
        let p = lib.join("steamapps").join("common").join("Human Fall Flat");
        assert_eq!(library_of(&p), Some(lib.clone()));
        assert_eq!(library_of(&lib.join("Games").join("Somewhere")), None);
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

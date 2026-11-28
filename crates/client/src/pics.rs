//! Resolving owned licenses into named apps via PICS (Steam's product info service).
//!
//! licenses -> package info (binary KV, lists app ids) -> app access tokens
//! -> app info (text KV with name, type, launch options).

use crate::Error;
use model::{AppId, LaunchOption, OwnedApp};
use net::{Connection, emsg};
use proto::*;
use sha1::{Digest, Sha1};
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::time::Duration;

type Result<T> = std::result::Result<T, Error>;

/// App types shown in the library.
const LIBRARY_TYPES: &[&str] = &["game", "demo", "application", "tool", "music", "video", "dlc"];

pub async fn resolve(conn: &Connection, licenses: &[(u32, u64)]) -> Result<Vec<OwnedApp>> {
    let started = std::time::Instant::now();
    let (app_ids, master_subs) = package_apps(conn, licenses).await?;
    tracing::info!("{} licenses -> {} app ids", licenses.len(), app_ids.len());
    let ids: Vec<AppId> = app_ids.into_iter().collect();
    let tokens = app_tokens(conn, &ids).await?;
    let mut apps = app_info(conn, &ids, &tokens).await?;
    apps.retain(|a| LIBRARY_TYPES.contains(&a.kind.as_str()));
    for a in &mut apps {
        a.master_sub = master_subs.get(&a.app_id).copied().unwrap_or(0);
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    tracing::info!("library: {} apps in {:.1} s", apps.len(), started.elapsed().as_secs_f32());
    Ok(apps)
}

fn pending(p: &net::Packet) -> bool {
    p.body::<CMsgClientPicsProductInfoResponse>()
        .map(|r| r.response_pending())
        .unwrap_or(false)
}

async fn product_info(
    conn: &Connection,
    req: &CMsgClientPicsProductInfoRequest,
) -> Result<Vec<CMsgClientPicsProductInfoResponse>> {
    let replies = conn
        .request_many(emsg::CLIENT_PICS_PRODUCT_INFO_REQUEST, req, |p| !pending(p))
        .await?;
    replies.iter().map(|p| p.body().map_err(Error::from)).collect()
}

/// The apps in these licenses, and which of them come with a subscription
/// (like EA Play), by the subscription's app.
async fn package_apps(conn: &Connection, licenses: &[(u32, u64)]) -> Result<(BTreeSet<AppId>, HashMap<AppId, u32>)> {
    let mut out = BTreeSet::new();
    let mut subs = HashMap::new();
    for chunk in licenses.chunks(500) {
        let req = CMsgClientPicsProductInfoRequest {
            packages: chunk
                .iter()
                .map(|&(id, token)| c_msg_client_pics_product_info_request::PackageInfo {
                    packageid: Some(id),
                    access_token: Some(token),
                })
                .collect(),
            ..Default::default()
        };
        for resp in product_info(conn, &req).await? {
            for pkg in resp.packages {
                // Buffer is a u32 package id followed by binary KV.
                let Some(buf) = pkg.buffer.filter(|b| b.len() > 4) else { continue };
                let Ok(kv) = vdf::binary::parse(&buf[4..]) else { continue };
                for (_, v) in kv.iter() {
                    let Some(apps) = v.get("appids").and_then(|a| a.as_obj()) else { continue };
                    let ids: Vec<AppId> = apps.iter().filter_map(|(_, a)| a.as_str()?.parse::<AppId>().ok()).collect();
                    let sub = v.as_obj().and_then(|p| p.obj("extended")).and_then(|e| e.u64("mastersubscriptionappid")).unwrap_or(0) as u32;
                    if sub != 0 {
                        subs.extend(ids.iter().map(|id| (*id, sub)));
                    }
                    out.extend(ids);
                }
            }
        }
    }
    Ok((out, subs))
}

async fn app_tokens(conn: &Connection, ids: &[AppId]) -> Result<HashMap<AppId, u64>> {
    let mut out = HashMap::new();
    for chunk in ids.chunks(500) {
        let reply = conn
            .request(
                emsg::CLIENT_PICS_ACCESS_TOKEN_REQUEST,
                &CMsgClientPicsAccessTokenRequest { appids: chunk.to_vec(), packageids: vec![] },
            )
            .await?;
        let resp: CMsgClientPicsAccessTokenResponse = reply.body()?;
        out.extend(resp.app_access_tokens.iter().map(|t| (t.appid(), t.access_token())));
    }
    Ok(out)
}

async fn app_info(
    conn: &Connection,
    ids: &[AppId],
    tokens: &HashMap<AppId, u64>,
) -> Result<Vec<OwnedApp>> {
    use futures_util::stream::{self, StreamExt, TryStreamExt};
    let http = reqwest::Client::new();
    // A few batches at once instead of one after another.
    let requests: Vec<CMsgClientPicsProductInfoRequest> = ids
        .chunks(200)
        .map(|chunk| CMsgClientPicsProductInfoRequest {
            apps: chunk
                .iter()
                .map(|&id| c_msg_client_pics_product_info_request::AppInfo {
                    appid: Some(id),
                    access_token: tokens.get(&id).copied(),
                    only_public_obsolete: None,
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    let replies: Vec<Vec<CMsgClientPicsProductInfoResponse>> =
        stream::iter(requests.into_iter().map(|req| {
            let conn = conn.clone();
            async move { product_info(&conn, &req).await }
        }))
        .buffer_unordered(4)
        .try_collect()
        .await?;
    let mut out = Vec::new();
    let mut remote = Vec::new();
    for resp in replies.into_iter().flatten() {
        let host = resp.http_host.clone().filter(|h| !h.is_empty());
        for app in resp.apps {
            let id = app.appid();
            match app.buffer.as_deref().filter(|b| !b.is_empty()) {
                Some(buf) => {
                    if let Some(a) = parse_app(id, String::from_utf8_lossy(buf).trim_end_matches('\0')) {
                        out.push(a);
                    }
                }
                // Large entries are served over HTTP instead of inline.
                None => {
                    if let (Some(h), Some(sha)) = (host.clone(), app.sha.clone()) {
                        remote.push((id, h, sha));
                    }
                }
            }
        }
    }
    // Those downloads run 16 at a time instead of one by one.
    let fetched: Vec<Option<OwnedApp>> = stream::iter(remote.into_iter().map(|(id, h, sha)| {
        let http = http.clone();
        async move { fetch_http(&http, &h, id, &sha).await.and_then(|t| parse_app(id, t.trim_end_matches('\0'))) }
    }))
    .buffer_unordered(16)
    .collect()
    .await;
    out.extend(fetched.into_iter().flatten());
    Ok(out)
}

/// One app's whole PICS app info (the `appinfo` object), with its access token.
pub async fn raw_app_info(conn: &Connection, id: AppId) -> Option<vdf::Obj> {
    let tokens = app_tokens(conn, &[id]).await.unwrap_or_default();
    let req = CMsgClientPicsProductInfoRequest {
        apps: vec![c_msg_client_pics_product_info_request::AppInfo { appid: Some(id), access_token: tokens.get(&id).copied(), only_public_obsolete: None }],
        ..Default::default()
    };
    let http = reqwest::Client::new();
    for resp in product_info(conn, &req).await.ok()? {
        let host = resp.http_host.clone().filter(|h| !h.is_empty());
        for app in resp.apps.into_iter().filter(|a| a.appid() == id) {
            let text = match app.buffer.as_deref().filter(|b| !b.is_empty()) {
                Some(buf) => String::from_utf8_lossy(buf).into_owned(),
                None => fetch_http(&http, host.as_deref()?, id, app.sha.as_deref()?).await?,
            };
            return vdf::parse(text.trim_end_matches('\0')).ok()?.obj("appinfo").cloned();
        }
    }
    None
}

/// Names of any apps, from their public info (no ownership needed).
pub async fn names(conn: &Connection, ids: &[AppId]) -> Result<Vec<(AppId, String)>> {
    let apps = app_info(conn, ids, &HashMap::new()).await?;
    Ok(apps.into_iter().filter(|a| a.name != "Unknown").map(|a| (a.app_id, a.name)).collect())
}

const MAX_PICS_COMPRESSED_BYTES: usize = 32 * 1024 * 1024;
const MAX_PICS_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

fn is_valid_http_host(host: &str) -> bool {
    if host.is_empty() || host.len() > 255 {
        return false;
    }
    if !host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'-')) {
        return false;
    }
    let name = match host.split_once(':') {
        Some((h, port)) => {
            if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            if port.parse::<u16>().ok().is_none_or(|p| p == 0) {
                return false;
            }
            h
        }
        None => host,
    };
    if name.is_empty() || name.starts_with('.') || name.ends_with('.') || name.contains("..") {
        return false;
    }
    name.split('.').all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-'))
}

fn decode_and_verify_pics(bytes: &[u8], expected_sha: &[u8]) -> Option<String> {
    if expected_sha.len() != 20 {
        return None;
    }
    let mut decoded = Vec::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut gz = flate2::read::GzDecoder::new(bytes).take((MAX_PICS_DECOMPRESSED_BYTES + 1) as u64);
        gz.read_to_end(&mut decoded).ok()?;
        if decoded.len() > MAX_PICS_DECOMPRESSED_BYTES {
            return None;
        }
    } else {
        if bytes.len() > MAX_PICS_DECOMPRESSED_BYTES {
            return None;
        }
        decoded.extend_from_slice(bytes);
    }

    let without_nul = decoded.strip_suffix(b"\0").unwrap_or(&decoded);
    let matches_sha = Sha1::digest(without_nul).as_slice() == expected_sha
        || Sha1::digest(&decoded).as_slice() == expected_sha
        || Sha1::digest(bytes).as_slice() == expected_sha;
    if !matches_sha {
        return None;
    }
    Some(String::from_utf8_lossy(&decoded).into_owned())
}

async fn fetch_http(http: &reqwest::Client, host: &str, id: AppId, sha: &[u8]) -> Option<String> {
    if sha.len() != 20 || !is_valid_http_host(host) {
        return None;
    }
    let sha_hex: String = sha.iter().map(|b| format!("{b:02x}")).collect();
    let url = format!("https://{host}/appinfo/{id}/sha/{sha_hex}.txt.gz");
    let parsed = reqwest::Url::parse(&url).ok()?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    let mut resp = tokio::time::timeout(Duration::from_secs(30), http.get(parsed).send())
        .await
        .ok()?
        .ok()?
        .error_for_status()
        .ok()?;
    if resp.url().scheme() != "https" {
        return None;
    }
    if resp.content_length().is_some_and(|len| len > MAX_PICS_COMPRESSED_BYTES as u64) {
        return None;
    }
    let mut bytes = Vec::with_capacity(resp.content_length().unwrap_or(0).min(256 * 1024) as usize);
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(30), resp.chunk()).await.ok()?.ok()? {
        if bytes.len().checked_add(chunk.len()).is_none_or(|n| n > MAX_PICS_COMPRESSED_BYTES) {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    decode_and_verify_pics(&bytes, sha)
}

fn parse_app(id: AppId, text: &str) -> Option<OwnedApp> {
    let kv = vdf::parse(text).ok()?;
    let info = kv.obj("appinfo")?;
    let common = info.obj("common")?;
    let launch = info
        .path(&["config", "launch"])
        .map(|l| {
            l.iter()
                .filter_map(|(_, v)| {
                    let o = v.as_obj()?;
                    Some(LaunchOption {
                        executable: o.str("executable").unwrap_or_default().into(),
                        arguments: o.str("arguments").unwrap_or_default().into(),
                        working_dir: o.str("workingdir").unwrap_or_default().into(),
                        description: o.str("description").unwrap_or_default().into(),
                        os_list: o
                            .obj("config")
                            .and_then(|c| c.str("oslist"))
                            .unwrap_or_default()
                            .into(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(OwnedApp {
        app_id: id,
        name: common.str("name").unwrap_or("Unknown").to_string(),
        kind: common.str("type").unwrap_or_default().to_lowercase(),
        launch,
        // "icon" is the .jpg the store and library show; "clienticon" is a
        // Windows .ico, only used when there is no .jpg.
        icon: common.str("icon").filter(|s| !s.is_empty()).or(common.str("clienticon")).filter(|s| !s.is_empty()).map(String::from),
        parent: common.u64("parent").map(|p| p as AppId).filter(|p| *p != 0),
        install_dir: info.path(&["config"]).and_then(|c| c.str("installdir")).unwrap_or_default().to_string(),
        depots: parse_depots(info),
        branches: parse_branches(info),
        cloud: parse_cloud(info),
        store_assets: common.obj("header_image").is_some() || common.str("store_asset_mtime").is_some() || common.obj("library_assets").is_some(),
        art: art_paths(common),
        categories: common.obj("category").map(|c| c.iter().filter_map(|(k, _)| k.strip_prefix("category_")?.parse().ok()).collect()).unwrap_or_default(),
        genres: common.obj("genres").map(|g| g.iter().filter_map(|(_, v)| v.as_str()?.parse().ok()).collect()).unwrap_or_default(),
        tags: common.obj("store_tags").map(|g| g.iter().filter_map(|(_, v)| v.as_str()?.parse().ok()).collect()).unwrap_or_default(),
        deck: common.obj("steam_deck_compatibility").and_then(|d| d.u64("category")).unwrap_or(0) as u8,
        machine: common.obj("steam_machine_compatibility").and_then(|d| d.u64("category")).unwrap_or(0) as u8,
        frame: common.obj("steam_frame_compatibility").and_then(|d| d.u64("category")).unwrap_or(0) as u8,
        languages: common
            .obj("supported_languages")
            .map(|l| {
                crate::collections::language_bits(
                    l.iter()
                        .filter(|(_, v)| v.as_obj().and_then(|o| o.str("supported")).is_some_and(|s| s == "true" || s == "1"))
                        .map(|(name, _)| name),
                )
            })
            .unwrap_or(0),
        master_sub: 0,
        os_list: common.str("oslist").unwrap_or_default().to_string(),
    })
}

/// Art in Steam's newer layout ("<hash>/header.jpg"): the header, and the
/// library capsule, hero and logo from `library_assets_full`. English first.
fn art_paths(common: &vdf::Obj) -> Vec<String> {
    fn walk(o: &vdf::Obj, english: bool, out: &mut Vec<(bool, String)>) {
        for (k, v) in o.iter() {
            match v {
                vdf::Value::Str(s) if s.contains('/') => out.push((english || k == "english", s.clone())),
                vdf::Value::Obj(inner) => walk(inner, english || k == "english", out),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    for key in ["header_image", "library_assets_full"] {
        if let Some(o) = common.obj(key) {
            walk(o, false, &mut found);
        }
    }
    // English versions win when a file has several languages.
    found.sort_by_key(|(english, _)| !*english);
    let mut seen = std::collections::HashSet::new();
    found.into_iter().filter(|(_, p)| seen.insert(p.rsplit('/').next().unwrap_or("").to_string())).map(|(_, p)| p).collect()
}

/// The `ufs` section: Steam Cloud quota and Auto-Cloud save folders.
fn parse_cloud(info: &vdf::Obj) -> Option<model::CloudConfig> {
    let ufs = info.obj("ufs")?;
    let quota = ufs.u64("quota").unwrap_or(0);
    let save_files: Vec<model::SaveRule> = ufs
        .obj("savefiles")
        .map(|s| {
            s.iter()
                .filter_map(|(_, v)| {
                    let o = v.as_obj()?;
                    // Skip rules for other systems.
                    let platforms: Vec<String> = o.obj("platforms").map(|p| p.iter().filter_map(|(_, v)| v.as_str().map(|s| s.to_lowercase())).collect()).unwrap_or_default();
                    if !platforms.is_empty() && !platforms.iter().any(|p| p == "windows" || p == "all") {
                        return None;
                    }
                    Some(model::SaveRule {
                        root: o.str("root").unwrap_or_default().to_string(),
                        path: o.str("path").unwrap_or_default().to_string(),
                        pattern: o.str("pattern").unwrap_or("*").to_string(),
                        recursive: o.str("recursive") == Some("1"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let root_overrides = ufs
        .obj("rootoverrides")
        .map(|r| {
            r.iter()
                .filter_map(|(_, v)| {
                    let o = v.as_obj()?;
                    let os = match o.str("os")?.to_lowercase().as_str() {
                        "macos" | "osx" | "mac" => "macos",
                        "linux" => "linux",
                        "windows" => "windows",
                        _ => return None,
                    };
                    let transforms = o
                        .obj("pathtransforms")
                        .map(|t| t.iter().filter_map(|(_, v)| v.as_obj()).filter_map(|t| Some((t.str("find")?.to_string(), t.str("replace").unwrap_or_default().to_string()))).collect())
                        .unwrap_or_default();
                    Some(model::RootOverride {
                        root: o.str("root")?.to_string(),
                        os: os.to_string(),
                        not_os: o.str("oscompare") == Some("!="),
                        use_instead: o.str("useinstead").unwrap_or_default().to_string(),
                        add_path: o.str("addpath").unwrap_or_default().to_string(),
                        transforms,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (quota > 0 || !save_files.is_empty()).then_some(model::CloudConfig { quota, max_files: ufs.u64("maxnumfiles").unwrap_or(0) as u32, save_files, root_overrides })
}

pub(crate) fn parse_depots(info: &vdf::Obj) -> Vec<model::DepotInfo> {
    let Some(depots) = info.obj("depots") else { return Vec::new() };
    depots
        .iter()
        .filter_map(|(id, v)| {
            let id: u32 = id.parse().ok()?;
            let o = v.as_obj()?;
            let config = o.obj("config");
            let c = |k: &str| config.and_then(|c| c.str(k)).unwrap_or_default().to_string();
            let mut manifests = Vec::new();
            if let Some(m) = o.obj("manifests") {
                for (branch, val) in m.iter() {
                    // New format: { "gid": .., "size": .. }; old: plain gid.
                    let (gid, size) = match val {
                        vdf::Value::Obj(x) => (x.u64("gid").unwrap_or(0), x.u64("size").unwrap_or(0)),
                        vdf::Value::Str(s) => (s.parse().unwrap_or(0), 0),
                    };
                    if gid != 0 {
                        manifests.push((branch.to_string(), gid, size));
                    }
                }
            }
            Some(model::DepotInfo {
                id,
                os: c("oslist"),
                arch: c("osarch"),
                language: c("language"),
                low_violence: c("lowviolence") == "1",
                dlc_app: o.u64("dlcappid").map(|d| d as AppId),
                shared_install: o.str("sharedinstall") == Some("1") || o.u64("depotfromapp").is_some(),
                from_app: o.u64("depotfromapp").map(|a| a as AppId),
                manifests,
            })
        })
        .collect()
}

fn parse_branches(info: &vdf::Obj) -> Vec<model::Branch> {
    let Some(branches) = info.path(&["depots", "branches"]) else { return Vec::new() };
    let mut out: Vec<model::Branch> = branches
        .iter()
        .filter_map(|(name, v)| {
            let o = v.as_obj()?;
            Some(model::Branch {
                name: name.to_string(),
                description: o.str("description").unwrap_or_default().to_string(),
                build_id: o.u64("buildid").unwrap_or(0),
                updated: o.u64("timeupdated").unwrap_or(0),
                password: o.str("pwdrequired") == Some("1"),
            })
        })
        .collect();
    out.sort_by_key(|b| (b.name != "public", std::cmp::Reverse(b.updated)));
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_new_style_art() {
        // Like Rust's Warhammer Pack: art only under hashed folders.
        let text = r#""appinfo" { "common" { "name" "Pack" "type" "DLC"
            "header_image" { "english" "3833/header.jpg" "russian" "77aa/header.jpg" }
            "library_assets_full" { "library_capsule" { "image" { "english" "ac2f/library_600x900.jpg" } }
                "library_logo" { "image" { "english" "logo.png" } } } } }"#;
        let app = super::parse_app(3891560, text).unwrap();
        assert_eq!(app.art, vec!["3833/header.jpg".to_string(), "ac2f/library_600x900.jpg".to_string()]);
        for p in &app.art {
            model::art::remember(3891560, p);
        }
        assert!(model::art::header(3891560).ends_with("/store_item_assets/steam/apps/3891560/3833/header.jpg"));
        // Old-style apps keep the classic address.
        assert!(model::art::logo(3891560).ends_with("/steam/apps/3891560/logo.png"));
    }

    #[test]
    fn parses_appinfo_text() {
        let text = r#""appinfo" { "appid" "440" "common" { "name" "Team Fortress 2" "type" "Game" }
            "config" { "launch" { "0" { "executable" "tf_win64.exe" "arguments" "-game tf"
            "config" { "oslist" "windows" } } } } }"#;
        let app = super::parse_app(440, text).unwrap();
        assert_eq!(app.name, "Team Fortress 2");
        assert_eq!(app.kind, "game");
        assert_eq!(app.launch[0].executable, "tf_win64.exe");
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn validates_pics_http_host_and_verifies_appinfo_sha1() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;

        assert!(is_valid_http_host("clientconfig.akamai.steamstatic.com"));
        assert!(is_valid_http_host("steamcache.local:8443"));
        assert!(is_valid_http_host("127.0.0.1:443"));
        assert!(!is_valid_http_host(""));
        assert!(!is_valid_http_host("evil.com/../../etc"));
        assert!(!is_valid_http_host("user:pass@evil.com"));
        assert!(!is_valid_http_host("evil.com?query=1"));
        assert!(!is_valid_http_host("evil.com#frag"));
        assert!(!is_valid_http_host("evil..com"));
        assert!(!is_valid_http_host("evil.com:0"));
        assert!(!is_valid_http_host("evil.com:99999"));
        assert!(!is_valid_http_host("evil.com:80:80"));

        let vdf_body = b"\"appinfo\"\n{\n\t\"appid\"\t\"440\"\n}\n";
        let mut with_nul = vdf_body.to_vec();
        with_nul.push(0);

        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&with_nul).unwrap();
        let gz_bytes = enc.finish().unwrap();

        let pics_sha: [u8; 20] = Sha1::digest(vdf_body).into();
        let gz_sha: [u8; 20] = Sha1::digest(&gz_bytes).into();

        let decoded = decode_and_verify_pics(&gz_bytes, &pics_sha).expect("valid Valve PICS .txt.gz should verify");
        assert_eq!(decoded.trim_end_matches('\0').as_bytes(), vdf_body);

        let decoded_gz_sha = decode_and_verify_pics(&gz_bytes, &gz_sha).expect("direct .txt.gz SHA-1 should also verify");
        assert_eq!(decoded_gz_sha.trim_end_matches('\0').as_bytes(), vdf_body);

        assert!(decode_and_verify_pics(&gz_bytes, &[]).is_none());
        assert!(decode_and_verify_pics(&gz_bytes, &pics_sha[..19]).is_none());
        let mut wrong_sha = pics_sha;
        wrong_sha[0] ^= 0x55;
        assert!(decode_and_verify_pics(&gz_bytes, &wrong_sha).is_none());

        let mut tampered_enc = GzEncoder::new(Vec::new(), Compression::default());
        tampered_enc.write_all(b"\"appinfo\"\n{\n\t\"appid\"\t\"666\"\n}\n\0").unwrap();
        let tampered_gz = tampered_enc.finish().unwrap();
        assert!(decode_and_verify_pics(&tampered_gz, &pics_sha).is_none());

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let client = reqwest::Client::new();
            assert!(fetch_http(&client, "evil.com/escape", 440, &pics_sha).await.is_none());
            assert!(fetch_http(&client, "127.0.0.1:1", 440, &[]).await.is_none());
        });
    }
}

//! Resolving owned licenses into named apps via PICS (Steam's product info service).
//!
//! licenses -> package info (binary KV, lists app ids) -> app access tokens
//! -> app info (text KV with name, type, launch options).

use crate::Error;
use model::{AppId, LaunchOption, OwnedApp};
use net::{Connection, emsg};
use proto::*;
use std::collections::{BTreeSet, HashMap};
use std::io::Read;

type Result<T> = std::result::Result<T, Error>;

/// App types shown in the library.
const LIBRARY_TYPES: &[&str] = &["game", "demo", "application", "tool", "music", "video", "dlc"];

pub async fn resolve(conn: &Connection, licenses: &[(u32, u64)]) -> Result<Vec<OwnedApp>> {
    let app_ids = package_apps(conn, licenses).await?;
    tracing::info!("{} licenses -> {} app ids", licenses.len(), app_ids.len());
    let ids: Vec<AppId> = app_ids.into_iter().collect();
    let tokens = app_tokens(conn, &ids).await?;
    let mut apps = app_info(conn, &ids, &tokens).await?;
    apps.retain(|a| LIBRARY_TYPES.contains(&a.kind.as_str()));
    apps.sort_by_key(|a| a.name.to_lowercase());
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

async fn package_apps(conn: &Connection, licenses: &[(u32, u64)]) -> Result<BTreeSet<AppId>> {
    let mut out = BTreeSet::new();
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
                    out.extend(apps.iter().filter_map(|(_, a)| a.as_str()?.parse::<AppId>().ok()));
                }
            }
        }
    }
    Ok(out)
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
    let http = reqwest::Client::new();
    let mut out = Vec::new();
    for chunk in ids.chunks(200) {
        let req = CMsgClientPicsProductInfoRequest {
            apps: chunk
                .iter()
                .map(|&id| c_msg_client_pics_product_info_request::AppInfo {
                    appid: Some(id),
                    access_token: tokens.get(&id).copied(),
                    only_public_obsolete: None,
                })
                .collect(),
            ..Default::default()
        };
        for resp in product_info(conn, &req).await? {
            let host = resp.http_host.clone().filter(|h| !h.is_empty());
            for app in resp.apps {
                let id = app.appid();
                let text = match app.buffer.as_deref().filter(|b| !b.is_empty()) {
                    Some(buf) => String::from_utf8_lossy(buf).into_owned(),
                    // Large entries are served over HTTP instead of inline.
                    None => match (&host, app.sha.as_deref()) {
                        (Some(h), Some(sha)) => match fetch_http(&http, h, id, sha).await {
                            Some(t) => t,
                            None => continue,
                        },
                        _ => continue,
                    },
                };
                if let Some(a) = parse_app(id, text.trim_end_matches('\0')) {
                    out.push(a);
                }
            }
        }
    }
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

async fn fetch_http(http: &reqwest::Client, host: &str, id: AppId, sha: &[u8]) -> Option<String> {
    let sha: String = sha.iter().map(|b| format!("{b:02x}")).collect();
    let url = format!("https://{host}/appinfo/{id}/sha/{sha}.txt.gz");
    let bytes = http.get(&url).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?;
    let mut text = String::new();
    if flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut text).is_err() {
        text = String::from_utf8_lossy(&bytes).into_owned();
    }
    Some(text)
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
        icon: common.str("clienticon").or(common.str("icon")).filter(|s| !s.is_empty()).map(String::from),
        parent: common.u64("parent").map(|p| p as AppId).filter(|p| *p != 0),
        install_dir: info.path(&["config"]).and_then(|c| c.str("installdir")).unwrap_or_default().to_string(),
        depots: parse_depots(info),
        branches: parse_branches(info),
        cloud: parse_cloud(info),
        store_assets: common.obj("header_image").is_some() || common.str("store_asset_mtime").is_some() || common.obj("library_assets").is_some(),
        art: art_paths(common),
        categories: common.obj("category").map(|c| c.iter().filter_map(|(k, _)| k.strip_prefix("category_")?.parse().ok()).collect()).unwrap_or_default(),
        genres: common.obj("genres").map(|g| g.iter().filter_map(|(_, v)| v.as_str()?.parse().ok()).collect()).unwrap_or_default(),
        deck: common.obj("steam_deck_compatibility").and_then(|d| d.u64("category")).unwrap_or(0) as u8,
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
    (quota > 0 || !save_files.is_empty()).then_some(model::CloudConfig { quota, max_files: ufs.u64("maxnumfiles").unwrap_or(0) as u32, save_files })
}

fn parse_depots(info: &vdf::Obj) -> Vec<model::DepotInfo> {
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

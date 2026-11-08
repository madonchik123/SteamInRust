//! Steam Input bindings for a game (ISteamInput in our steamclient): the
//! controller_mappings VDF that maps an Xbox-style pad to the game's actions.
//! Looked up like Steam's client does: the player's own configuration
//! first, then the configuration the game ships, then the game's official
//! configurations (Workshop files listed in its app info).

use crate::ipc::protocol::Reply;
use model::AppId;
use net::Connection;
use proto::*;
use std::path::{Path, PathBuf};

/// Pads we read through XInput, best match first.
const TYPES: &[&str] = &["controller_xboxone", "controller_xbox360", "controller_xboxelite", "controller_generic", "controller_ps4", "controller_ps5"];

fn is_mappings(text: &str) -> bool {
    text.contains("controller_mappings") && text.contains("game_action")
}

fn rank(text: &str) -> usize {
    TYPES.iter().position(|t| text.contains(&format!("\"{t}\""))).unwrap_or(TYPES.len())
}

/// The player's configurations: `<Steam>/steamapps/common/Steam Controller Configs/<account>/config/<app>/`.
fn player_config(steam_root: &Path, account: u32, app_id: AppId) -> Option<(String, PathBuf)> {
    let dir = steam_root.join("steamapps").join("common").join("Steam Controller Configs").join(account.to_string()).join("config").join(app_id.to_string());
    best_in(&dir, 1)
}

/// The best Xbox-style mappings file under `dir`.
fn best_in(dir: &Path, depth: u32) -> Option<(String, PathBuf)> {
    let mut found: Vec<(usize, String, PathBuf)> = Vec::new();
    fn walk(dir: &Path, depth: u32, found: &mut Vec<(usize, String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth > 0 {
                    walk(&p, depth - 1, found);
                }
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vdf")) && e.metadata().map(|m| m.len() < 2_000_000).unwrap_or(false) {
                if let Ok(t) = std::fs::read_to_string(&p) {
                    if is_mappings(&t) {
                        found.push((rank(&t), t, p));
                    }
                }
            }
        }
    }
    walk(dir, depth, &mut found);
    found.sort_by_key(|f| f.0);
    found.into_iter().next().map(|(_, t, p)| (t, p))
}

/// Workshop configurations the player picked per controller type
/// (`config/configset_<vid>-<pid>-….vdf`: "controller_config" { "<app>" { "workshop" "<id>" } }),
/// Xbox pads (Microsoft, 045e) first.
fn player_workshop_choices(steam_root: &Path, account: u32, app_id: AppId) -> Vec<u64> {
    let dir = steam_root.join("steamapps").join("common").join("Steam Controller Configs").join(account.to_string()).join("config");
    let mut sets: Vec<(u8, PathBuf)> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter_map(|p| {
                    let n = p.file_name()?.to_str()?.to_ascii_lowercase();
                    let order = if n.starts_with("configset_45e-") { 0 } else if n.starts_with("configset_controller_generic") || n.starts_with("configset_controller_xbox") { 1 } else { return None };
                    Some((order, p))
                })
                .collect()
        })
        .unwrap_or_default();
    sets.sort();
    sets.iter()
        .filter_map(|(_, p)| vdf::parse(&std::fs::read_to_string(p).ok()?).ok())
        .filter_map(|v| v.obj("controller_config")?.obj(&app_id.to_string())?.u64("workshop"))
        .collect()
}

/// A configuration from the Workshop (they belong to Steam's controller
/// configs app, 241100, not to the game).
async fn workshop_file(conn: &Connection, _app_id: AppId, id: u64) -> Option<String> {
    let req = CPublishedFileGetDetailsRequest { publishedfileids: vec![id], ..Default::default() };
    let r = conn.call::<_, CPublishedFileGetDetailsResponse>("PublishedFile.GetDetails#1", &req).await.ok()?;
    let d = r.publishedfiledetails.first()?;
    let url = Some(d.file_url().to_string()).filter(|u| !u.is_empty())?;
    let text = reqwest::get(&url).await.ok()?.text().await.ok()?;
    is_mappings(&text).then_some(text)
}

/// Official configurations from the app info:
/// `config/steamcontrollerconfigdetails/<file id>/controller_type`.
async fn official(conn: &Connection, app_id: AppId) -> Option<(String, u64)> {
    let info = crate::pics::raw_app_info(conn, app_id).await?;
    let details = info.path(&["config", "steamcontrollerconfigdetails"])?;
    let mut files: Vec<(usize, u64)> = details
        .iter()
        .filter_map(|(id, v)| {
            let kind = v.as_obj()?.str("controller_type").unwrap_or_default();
            let branches = v.as_obj()?.str("enabled_branches").unwrap_or("public");
            (branches.is_empty() || branches.split(',').any(|b| b.trim() == "public")).then_some(())?;
            Some((TYPES.iter().position(|t| *t == kind).unwrap_or(TYPES.len()), id.parse().ok()?))
        })
        .collect();
    files.sort_by_key(|f| f.0);
    for (_, id) in files {
        if let Some(text) = workshop_file(conn, app_id, id).await {
            return Some((text, id));
        }
    }
    None
}

pub async fn config(conn: &Connection, app_id: AppId, me: u64, steam_root: &Path, install_dir: &Path) -> Reply {
    let found = if let Some((t, p)) = player_config(steam_root, (me & 0xFFFF_FFFF) as u32, app_id) {
        Some((t, format!("your configuration ({})", p.display())))
    } else if let Some((t, id)) = async {
        for id in player_workshop_choices(steam_root, (me & 0xFFFF_FFFF) as u32, app_id) {
            if let Some(t) = workshop_file(conn, app_id, id).await {
                return Some((t, id));
            }
        }
        None
    }
    .await
    {
        Some((t, format!("your Workshop configuration ({id})")))
    } else if let Some((t, p)) = (!install_dir.as_os_str().is_empty()).then(|| best_in(install_dir, 3)).flatten() {
        Some((t, format!("the game's files ({})", p.display())))
    } else {
        official(conn, app_id).await.map(|(t, id)| (t, format!("official configuration (Workshop {id})")))
    };
    match found {
        Some((bindings, source)) => {
            tracing::info!("Steam Input for {app_id}: {source}");
            Reply::InputConfig { bindings, source }
        }
        None => {
            tracing::info!("Steam Input for {app_id}: no bindings for an Xbox-style pad");
            Reply::InputConfig { bindings: String::new(), source: String::new() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_xbox_mappings() {
        let dir = std::env::temp_dir().join("sir-input-config-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cfg")).unwrap();
        std::fs::write(dir.join("cfg").join("ps4.vdf"), "\"controller_mappings\" { \"controller_type\" \"controller_ps4\" \"binding\" \"game_action set a\" }").unwrap();
        std::fs::write(dir.join("cfg").join("xbox.vdf"), "\"controller_mappings\" { \"controller_type\" \"controller_xboxone\" \"binding\" \"game_action set a\" }").unwrap();
        std::fs::write(dir.join("other.vdf"), "\"AppState\" {}").unwrap();
        let (_, p) = best_in(&dir, 2).unwrap();
        assert!(p.ends_with("xbox.vdf"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

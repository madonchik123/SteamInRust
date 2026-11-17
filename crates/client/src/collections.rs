//! Library collections, kept in step with Steam.
//!
//! Steam stores Favorites, Hidden and your own collections in a per-account
//! key-value store (`CloudConfigStore`, namespace 1), one JSON entry per
//! collection under `user-collections.<id>`. The official client reads and
//! writes that store, so edits made here show up in Steam and on your other
//! computers, and the other way round.

use net::Connection;
use proto::webui::{
    CCloudConfigStoreDownloadRequest, CCloudConfigStoreDownloadResponse, CCloudConfigStoreEntry, CCloudConfigStoreNamespaceData,
    CCloudConfigStoreNamespaceVersion, CCloudConfigStoreUploadRequest, CCloudConfigStoreUploadResponse,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The library's namespace in Steam's store.
const NAMESPACE: u32 = 1;
const PREFIX: &str = "user-collections.";

/// What we last read from Steam: the namespace's version and each entry's
/// value and version (sent back with an edit, so Steam knows which copy it
/// replaces).
#[derive(Default)]
pub struct Store {
    version: u64,
    entries: HashMap<String, (String, u64)>,
}

pub type Shared = Arc<Mutex<Store>>;

/// Read every collection from Steam and remember the raw entries.
pub async fn download(conn: &Connection, store: &Shared) -> Result<Vec<model::SteamCollection>, String> {
    let req = CCloudConfigStoreDownloadRequest { versions: vec![CCloudConfigStoreNamespaceVersion { enamespace: Some(NAMESPACE), version: Some(0) }] };
    let r: CCloudConfigStoreDownloadResponse = conn.call("CloudConfigStore.Download#1", &req).await.map_err(|e| e.to_string())?;
    let mut s = store.lock().expect("collections store");
    s.entries.clear();
    for d in r.data.iter().filter(|d| d.enamespace() == NAMESPACE) {
        s.version = d.version();
        for e in &d.entries {
            if !e.is_deleted() {
                s.entries.insert(e.key().to_string(), (e.value().to_string(), e.version()));
            }
        }
    }
    let list = crate::social::collections(r.data.iter().flat_map(|d| d.entries.iter()).map(|e| (e.key(), e.is_deleted(), e.value())));
    Ok(list)
}

/// Send changed or deleted collections to Steam.
pub async fn save(conn: &Connection, store: &Shared, edits: &[model::CollectionEdit]) -> Result<(), String> {
    if edits.is_empty() {
        return Ok(());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
    let data = {
        let s = store.lock().expect("collections store");
        let entries = edits
            .iter()
            .map(|e| {
                let key = format!("{PREFIX}{}", e.id);
                let old = s.entries.get(&key);
                CCloudConfigStoreEntry {
                    is_deleted: Some(e.deleted),
                    value: Some(if e.deleted { String::new() } else { entry_value(old.map(|(v, _)| v.as_str()), e) }),
                    timestamp: Some(now),
                    version: Some(old.map(|(_, v)| *v).unwrap_or(0)),
                    key: Some(key),
                }
            })
            .collect();
        CCloudConfigStoreNamespaceData { enamespace: Some(NAMESPACE), version: Some(s.version), entries, horizon: None }
    };
    let sent: Vec<(String, Option<String>)> = data.entries.iter().map(|e| (e.key().to_string(), (!e.is_deleted()).then(|| e.value().to_string()))).collect();
    let r: CCloudConfigStoreUploadResponse =
        conn.call("CloudConfigStore.Upload#1", &CCloudConfigStoreUploadRequest { data: vec![data] }).await.map_err(|e| e.to_string())?;
    let mut s = store.lock().expect("collections store");
    if let Some(v) = r.versions.iter().find(|v| v.enamespace() == NAMESPACE) {
        s.version = v.version();
        // Each changed entry now carries the namespace's new version.
        for (key, value) in sent {
            match value {
                Some(value) => drop(s.entries.insert(key, (value, v.version()))),
                None => drop(s.entries.remove(&key)),
            }
        }
    }
    Ok(())
}

/// The JSON Steam keeps for a collection. Fields we don't know about (a
/// dynamic collection's filters, anything Steam adds later) are kept.
pub fn entry_value(old: Option<&str>, e: &model::CollectionEdit) -> String {
    let mut v: Map<String, Value> = old.and_then(|o| serde_json::from_str(o).ok()).unwrap_or_default();
    let mut games = e.games.clone();
    games.sort_unstable();
    games.dedup();
    v.insert("id".into(), Value::from(e.id.clone()));
    v.insert("name".into(), Value::from(e.name.clone()));
    v.insert("added".into(), Value::from(games));
    v.insert("removed".into(), Value::Array(Vec::new()));
    Value::Object(v).to_string()
}

/// A new id for a collection made here, in Steam's own form (`uc-` and 12
/// letters and digits).
pub fn new_id() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64;
    n ^= (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    n ^= COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    n |= 1;
    let mut s = String::from("uc-");
    for _ in 0..12 {
        // xorshift, so ids made in the same instant still differ.
        n ^= n << 13;
        n ^= n >> 7;
        n ^= n << 17;
        s.push(CHARS[(n % CHARS.len() as u64) as usize] as char);
    }
    s
}

/// A dynamic collection's filters (Steam's `filterSpec`, format 2), so its
/// games can be worked out here the way Steam's library does.
///
/// Steam keeps nine filter groups; a game must pass every group, and a group
/// passes when it's empty, or when any (union) / all of its options match.
/// Groups this client can't check (subscriptions, friends who own the game,
/// languages, app type) are left out, so such a collection may show a few
/// more games than in Steam.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DynamicFilter {
    search: String,
    groups: Vec<(Vec<u64>, bool)>,
}

/// What the filters look at for one game.
#[derive(Debug, Clone, Default)]
pub struct GameFacts<'a> {
    pub name: &'a str,
    pub installed: bool,
    pub played: bool,
    /// Runs on this computer's system.
    pub runs_here: bool,
    /// Steam's store categories (2 single-player, 23 cloud, 28 controller…).
    pub categories: &'a [u32],
    /// Steam's store tags (user tags).
    pub tags: &'a [u32],
    /// Steam Deck: 0 unknown, 1 unsupported, 2 playable, 3 verified.
    pub deck: u8,
}

/// Steam's filter groups (indexes into `filterGroups`).
const PLAY_STATE: usize = 1;
const FEATURES: usize = 2;
const TAGS: usize = 4;
const CATEGORIES: usize = 8;

impl DynamicFilter {
    pub fn parse(spec: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(spec).ok()?;
        let groups = v
            .get("filterGroups")?
            .as_array()?
            .iter()
            .map(|g| {
                let options = g.get("rgOptions").and_then(|o| o.as_array()).map(|o| o.iter().filter_map(|x| x.as_u64()).collect()).unwrap_or_default();
                (options, g.get("bAcceptUnion").and_then(|u| u.as_bool()).unwrap_or(false))
            })
            .collect();
        Some(Self { search: v.get("strSearchText").and_then(|s| s.as_str()).unwrap_or_default().to_lowercase(), groups })
    }

    pub fn matches(&self, g: &GameFacts) -> bool {
        let group = |i: usize, test: &dyn Fn(u64) -> bool| match self.groups.get(i) {
            None => true,
            Some((options, _)) if options.is_empty() => true,
            Some((options, true)) => options.iter().any(|&o| test(o)),
            Some((options, false)) => options.iter().all(|&o| test(o)),
        };
        let cat = |c: u32| g.categories.contains(&c);
        let ok = group(PLAY_STATE, &|o| match o {
            1 | 2 => g.installed,
            3 => g.played,
            4 => !g.played,
            5 => g.runs_here,
            _ => false,
        }) && group(FEATURES, &|o| match o {
            1 => cat(28),
            2 => cat(28) || cat(18),
            3 => cat(31),
            4 => cat(29),
            5 => cat(30),
            6 => cat(22),
            7 => cat(2),
            8 => [1, 36, 37, 27, 20, 24].into_iter().any(cat),
            9 => [9, 38, 39].into_iter().any(cat),
            10 => cat(23),
            11 => cat(44),
            12 => g.deck >= 3,
            13 => g.deck >= 2,
            14 => g.deck != 0,
            15 => true,
            16 => cat(55) || cat(56),
            17 => cat(56),
            18 => cat(57) || cat(58),
            19 => cat(58),
            20 => cat(59),
            21 => cat(60),
            22 => cat(61),
            23 => cat(62),
            // Not VR-only (that isn't known here: most games aren't).
            26 => true,
            27 => [24, 37, 39].into_iter().any(cat),
            _ => false,
        }) && group(TAGS, &|o| g.tags.contains(&(o as u32)))
            && group(CATEGORIES, &|o| cat(o as u32));
        ok && (self.search.is_empty() || g.name.to_lowercase().contains(&self.search))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_unknown_fields_and_writes_the_games() {
        let old = r#"{"id":"uc-abc","name":"Old","added":[5,1],"removed":[1],"filterSpec":{"x":1}}"#;
        let e = model::CollectionEdit { id: "uc-abc".into(), name: "Co-op".into(), games: vec![730, 570, 730], deleted: false };
        let v: Value = serde_json::from_str(&entry_value(Some(old), &e)).unwrap();
        assert_eq!(v["name"], "Co-op");
        assert_eq!(v["added"], serde_json::json!([570, 730]));
        assert_eq!(v["removed"], serde_json::json!([]));
        assert_eq!(v["filterSpec"]["x"], 1);
    }

    #[test]
    fn new_ids_look_like_steams() {
        let (a, b) = (new_id(), new_id());
        assert!(a.starts_with("uc-") && a.len() == 15);
        assert_ne!(a, b);
    }

    #[test]
    fn dynamic_filters_follow_steam() {
        use super::{DynamicFilter, GameFacts};
        let blank = r#"{"nFormatVersion":2,"strSearchText":"","filterGroups":[{"rgOptions":[],"bAcceptUnion":true},{"rgOptions":[1],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false}],"setSuggestions":{}}"#;
        let installed = DynamicFilter::parse(blank).unwrap();
        let game = GameFacts { name: "Portal 2", installed: true, categories: &[2, 28], tags: &[1664], deck: 3, ..Default::default() };
        assert!(installed.matches(&game));
        assert!(!installed.matches(&GameFacts { installed: false, ..game.clone() }));

        // Features, all of (controller and single-player), and a tag.
        let spec = blank.replace(r#"{"rgOptions":[1],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false}"#, r#"{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[1,7],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[1664],"bAcceptUnion":true}"#);
        let f = DynamicFilter::parse(&spec).unwrap();
        assert!(f.matches(&GameFacts { installed: false, ..game.clone() }));
        assert!(!f.matches(&GameFacts { categories: &[2], ..game.clone() }));
        assert!(!f.matches(&GameFacts { tags: &[19], ..game.clone() }));

        // Search text.
        let named = DynamicFilter::parse(&blank.replace(r#""strSearchText":"""#, r#""strSearchText":"PORTAL""#)).unwrap();
        assert!(named.matches(&game));
        assert!(!named.matches(&GameFacts { name: "Dota 2", ..game.clone() }));
        assert!(DynamicFilter::parse("not json").is_none());
    }
}

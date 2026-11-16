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
}

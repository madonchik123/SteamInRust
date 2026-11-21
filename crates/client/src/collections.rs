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
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::Value;
use serde_json::value::RawValue;
use std::collections::{HashMap, HashSet};
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
        if edits.iter().any(|e| changed_elsewhere(s.entries.get(&format!("{PREFIX}{}", e.id)).map(|(v, _)| v.as_str()), e)) {
            return Err(CHANGED_IN_STEAM.to_string());
        }
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

/// Why a dynamic collection's edit wasn't saved.
pub const CHANGED_IN_STEAM: &str = "This collection was changed in Steam while you were editing it. Open it again to see the new filters.";

/// Whether Steam's copy of these filters is no longer the one the edit
/// started from, so saving would overwrite a change made in the official
/// client or on another computer.
pub fn changed_elsewhere(current: Option<&str>, e: &model::CollectionEdit) -> bool {
    !e.deleted && e.filter.is_some() && current.and_then(|c| raw_field(c, "filterSpec")).as_deref() != e.based_on.as_deref()
}

/// A JSON object kept exactly as it was written: its keys in order and each
/// value's own text, so changing one field leaves the rest byte for byte.
#[derive(Default)]
struct RawObject(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for RawObject {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = RawObject;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawObject, A::Error> {
                let mut out = Vec::new();
                while let Some(field) = map.next_entry::<String, Box<RawValue>>()? {
                    out.push(field);
                }
                Ok(RawObject(out))
            }
        }
        d.deserialize_map(Fields)
    }
}

impl RawObject {
    fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    fn raw(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.get())
    }

    /// Set a field to this JSON text. Says whether anything changed.
    fn put(&mut self, key: &str, json: String) -> bool {
        if self.raw(key) == Some(json.as_str()) {
            return false;
        }
        let Ok(value) = RawValue::from_string(json) else { return false };
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((key.to_string(), value)),
        }
        true
    }

    /// Set a text field, unless it already says this (however it's escaped).
    fn put_text(&mut self, key: &str, text: &str) -> bool {
        if self.raw(key).and_then(|r| serde_json::from_str::<String>(r).ok()).as_deref() == Some(text) {
            return false;
        }
        self.put(key, serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into()))
    }

    fn games(&self, key: &str) -> Vec<model::AppId> {
        self.raw(key).and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default()
    }

    fn write(&self) -> String {
        let fields: Vec<String> = self.0.iter().map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap_or_default(), v.get())).collect();
        format!("{{{}}}", fields.join(","))
    }
}

/// One field of a stored entry, as the exact text Steam wrote.
pub(crate) fn raw_field(entry: &str, key: &str) -> Option<String> {
    RawObject::parse(entry)?.raw(key).map(str::to_string)
}

/// The JSON Steam keeps for a collection, in Steam's key order for a new one.
/// Only what this edit changes is rewritten: everything else (fields Steam
/// adds later included) stays byte for byte, and an edit that changes nothing
/// gives back the very same text.
///
/// A dynamic collection's `added` and `removed` are the games you put in or
/// took out by hand on top of its filters; they're never touched here.
pub fn entry_value(old: Option<&str>, e: &model::CollectionEdit) -> String {
    let parsed = old.and_then(RawObject::parse);
    let existed = parsed.is_some();
    let mut v = parsed.unwrap_or_default();
    let mut changed = v.put_text("id", &e.id);
    changed |= v.put_text("name", &e.name);
    let dynamic = e.filter.is_some() || v.raw("filterSpec").is_some_and(|f| f != "null");
    if dynamic {
        for key in ["added", "removed"] {
            if v.raw(key).is_none() {
                changed |= v.put(key, "[]".into());
            }
        }
        if let Some(spec) = &e.filter {
            changed |= v.put("filterSpec", spec.clone());
        }
    } else {
        let removed = v.games("removed");
        let mut was: Vec<model::AppId> = v.games("added").into_iter().filter(|g| !removed.contains(g)).collect();
        was.sort_unstable();
        was.dedup();
        let mut games = e.games.clone();
        games.sort_unstable();
        games.dedup();
        if !existed || was != games {
            changed |= v.put("added", serde_json::to_string(&games).unwrap_or_else(|_| "[]".into()));
            changed |= v.put("removed", "[]".into());
        }
    }
    match old {
        Some(text) if existed && !changed => text.to_string(),
        _ => v.write(),
    }
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

/// How many filter groups Steam keeps.
pub const GROUP_COUNT: usize = 9;
/// Steam's filter groups (indexes into `filterGroups`). Group 0 (app type)
/// and group 3 are saved but Steam's collections don't check them.
pub const PLAY_STATE: usize = 1;
pub const FEATURES: usize = 2;
pub const TAGS: usize = 4;
pub const SUBSCRIPTIONS: usize = 5;
pub const FRIENDS: usize = 6;
pub const LANGUAGES: usize = 7;
pub const CATEGORIES: usize = 8;

/// The subscription option Steam offers: EA Play, by its app.
pub const EA_PLAY: u64 = 4000;
pub const EA_PLAY_APP: u32 = 1_289_670;

/// Steam's language ids, by the name app info uses for each.
pub const LANGUAGE_IDS: &[(&str, u64)] = &[
    ("english", 0), ("german", 1), ("french", 2), ("italian", 3), ("koreana", 4), ("spanish", 5), ("schinese", 6), ("tchinese", 7),
    ("russian", 8), ("thai", 9), ("japanese", 10), ("portuguese", 11), ("polish", 12), ("danish", 13), ("dutch", 14), ("finnish", 15),
    ("norwegian", 16), ("swedish", 17), ("hungarian", 18), ("czech", 19), ("romanian", 20), ("turkish", 21), ("brazilian", 22),
    ("bulgarian", 23), ("greek", 24), ("arabic", 25), ("ukrainian", 26), ("latam", 27), ("vietnamese", 28), ("sc_schinese", 29),
    ("indonesian", 30), ("malay", 31),
];

/// The bit Steam uses for each of these languages.
pub fn language_bits<'a>(names: impl Iterator<Item = &'a str>) -> u64 {
    names.filter_map(|n| LANGUAGE_IDS.iter().find(|(k, _)| *k == n)).fold(0, |bits, (_, id)| bits | (1 << id))
}

/// Each friend's games, by account id. A friend who isn't here (not loaded
/// yet, or a private profile) can't be checked.
pub type FriendGames = HashMap<u32, HashSet<model::AppId>>;

/// One of Steam's filter groups.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterGroup {
    pub options: Vec<u64>,
    /// A game needs any of the options (`bAcceptUnion`), or else all of them.
    pub any: bool,
}

/// A dynamic collection's filters (Steam's `filterSpec`, format 2), so its
/// games can be worked out here the way Steam's library does.
///
/// Steam keeps nine filter groups; a game must pass every group, and a group
/// passes when it's empty, or when any (union) / all of its options match.
/// Where this client has no data for an option (private games, Steam OS
/// compatibility, a friend whose games it can't see), the option counts as
/// passing, so such a collection may show a few more games than in Steam but
/// never hides one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicFilter {
    pub search: String,
    pub groups: [FilterGroup; GROUP_COUNT],
}

impl Default for DynamicFilter {
    /// Steam's blank filter: the first group accepts any option.
    fn default() -> Self {
        Self { search: String::new(), groups: std::array::from_fn(|i| FilterGroup { options: Vec::new(), any: i == 0 }) }
    }
}

/// What the filters look at for one game.
#[derive(Debug, Clone, Default)]
pub struct GameFacts<'a> {
    pub app_id: model::AppId,
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
    /// Steam Machine compatibility, on the same scale.
    pub machine: u8,
    /// Steam Frame compatibility, on the same scale.
    pub frame: u8,
    /// Supported languages, one bit per Steam language id (0 = not known).
    pub languages: u64,
    /// The subscription app this game comes with (0 = none).
    pub master_sub: u32,
    /// What friends own, for the "friends who own it" filter.
    pub friends: Option<&'a FriendGames>,
    /// The account whose copy this is, when it's borrowed (0 = your own).
    pub owner: u32,
}

impl DynamicFilter {
    /// Read Steam's stored filters. Like Steam, only format 2 is understood.
    pub fn parse(spec: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(spec).ok()?;
        if v.get("nFormatVersion")?.as_u64()? != 2 {
            return None;
        }
        let saved = v.get("filterGroups")?.as_array()?;
        let mut f = Self { search: v.get("strSearchText").and_then(|s| s.as_str()).unwrap_or_default().to_string(), ..Self::default() };
        for (group, s) in f.groups.iter_mut().zip(saved) {
            if let Some(o) = s.get("rgOptions").and_then(|o| o.as_array()) {
                group.options = o.iter().filter_map(Value::as_u64).collect();
            }
            if let Some(any) = s.get("bAcceptUnion").and_then(Value::as_bool) {
                group.any = any;
            }
        }
        Some(f)
    }

    /// The filters as Steam stores them, byte for byte.
    pub fn to_json(&self) -> String {
        let groups: Vec<String> = self
            .groups
            .iter()
            .map(|g| {
                let options: Vec<String> = g.options.iter().map(u64::to_string).collect();
                format!("{{\"rgOptions\":[{}],\"bAcceptUnion\":{}}}", options.join(","), g.any)
            })
            .collect();
        let search = serde_json::to_string(&self.search).unwrap_or_else(|_| "\"\"".into());
        format!("{{\"nFormatVersion\":2,\"strSearchText\":{search},\"filterGroups\":[{}],\"setSuggestions\":{{}}}}", groups.join(","))
    }

    /// Whether these stored filters can be edited here without losing
    /// anything: reading and writing them gives back the very same text.
    /// Anything else stays as it is and is changed in Steam.
    pub fn is_exact(spec: &str) -> bool {
        Self::parse(spec).is_some_and(|f| f.to_json() == spec)
    }

    /// No search and no options (Steam then lists only the games added by hand).
    pub fn is_empty(&self) -> bool {
        self.search.is_empty() && self.groups.iter().all(|g| g.options.is_empty())
    }

    /// Whether the game belongs in a collection with these filters, counting
    /// the games added and removed by hand, in the order Steam applies them.
    pub fn contains(&self, g: &GameFacts, added: &[model::AppId], removed: &[model::AppId]) -> bool {
        ((!self.is_empty() && self.matches(g)) || added.contains(&g.app_id)) && !removed.contains(&g.app_id)
    }

    pub fn matches(&self, g: &GameFacts) -> bool {
        let group = |i: usize, test: &dyn Fn(u64) -> bool| {
            let f = &self.groups[i];
            f.options.is_empty() || if f.any { f.options.iter().any(|&o| test(o)) } else { f.options.iter().all(|&o| test(o)) }
        };
        let cat = |c: u32| g.categories.contains(&c);
        let ok = group(PLAY_STATE, &|o| match o {
            1 | 2 => g.installed,
            3 => g.played,
            4 => !g.played,
            5 => g.runs_here,
            // Private games aren't known here.
            6 => true,
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
            // Steam OS compatibility isn't in app info.
            24 | 25 => true,
            // Not VR-only (that isn't known here: most games aren't).
            26 => true,
            27 => [24, 37, 39].into_iter().any(cat),
            29 => g.machine >= 3,
            30 => g.machine >= 2,
            31 => g.machine != 0,
            32 => g.frame >= 3,
            33 => g.frame >= 2,
            34 => g.frame != 0,
            _ => false,
        }) && group(TAGS, &|o| g.tags.contains(&(o as u32)))
            && group(SUBSCRIPTIONS, &|o| o == EA_PLAY && g.master_sub == EA_PLAY_APP)
            && group(FRIENDS, &|o| match g.friends.and_then(|f| f.get(&(o as u32))) {
                Some(games) => games.contains(&g.app_id) && g.owner != o as u32,
                None => true,
            })
            && group(LANGUAGES, &|o| g.languages == 0 || (o < 64 && g.languages >> o & 1 == 1))
            && group(CATEGORIES, &|o| cat(o as u32));
        if !ok {
            return false;
        }
        if self.search.is_empty() || g.app_id.to_string() == self.search {
            return true;
        }
        g.name.to_lowercase().contains(&self.search.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_unknown_fields_and_writes_the_games() {
        let old = r#"{"id":"uc-abc","name":"Old","added":[5,1],"removed":[1],"future":{"z":1,"a":2}}"#;
        let e = model::CollectionEdit { id: "uc-abc".into(), name: "Co-op".into(), games: vec![730, 570, 730], ..Default::default() };
        assert_eq!(entry_value(Some(old), &e), r#"{"id":"uc-abc","name":"Co-op","added":[570,730],"removed":[],"future":{"z":1,"a":2}}"#);
    }

    /// Entries in the exact form Steam stores them, one per line.
    const SAMPLES: &str = include_str!("../testdata/steam-collections.txt");

    fn samples() -> Vec<&'static str> {
        SAMPLES.lines().filter(|l| !l.trim().is_empty()).collect()
    }

    fn read(entry: &str) -> model::SteamCollection {
        crate::social::collections([("user-collections.x", false, entry)].into_iter()).remove(0)
    }

    /// The edit that says "keep it as it is".
    fn same(c: &model::SteamCollection) -> model::CollectionEdit {
        model::CollectionEdit { id: c.id.clone(), name: c.name.clone(), games: c.games.clone(), filter: c.filter.clone(), based_on: c.filter.clone(), deleted: false }
    }

    #[test]
    fn wont_overwrite_filters_changed_in_steam() {
        let entry = samples().into_iter().find(|e| e.contains("Ready on Deck")).unwrap();
        let c = read(entry);
        let edit = same(&c);
        assert!(!changed_elsewhere(Some(entry), &edit));
        // Steam's copy moved on while the editor was open.
        let moved = entry.replace(r#""rgOptions":[13]"#, r#""rgOptions":[12]"#);
        assert!(changed_elsewhere(Some(&moved), &edit));
        // Deleted in Steam meanwhile.
        assert!(changed_elsewhere(None, &edit));
        // A new one has nothing to clash with; nor has a plain edit or a delete.
        let new = model::CollectionEdit { filter: c.filter.clone(), based_on: None, ..edit.clone() };
        assert!(!changed_elsewhere(None, &new));
        assert!(!changed_elsewhere(Some(&moved), &model::CollectionEdit { filter: None, ..edit.clone() }));
        assert!(!changed_elsewhere(Some(&moved), &model::CollectionEdit { deleted: true, ..edit }));
    }

    #[test]
    fn saved_samples_round_trip_byte_for_byte() {
        let list = samples();
        assert!(list.len() >= 7);
        for entry in list {
            let c = read(entry);
            assert_eq!(entry_value(Some(entry), &same(&c)), entry);
            if let Some(spec) = &c.filter {
                assert!(entry.contains(spec.as_str()), "the filters are read exactly as stored: {entry}");
                assert!(DynamicFilter::is_exact(spec), "{spec}");
                assert_eq!(&DynamicFilter::parse(spec).unwrap().to_json(), spec);
            }
        }
    }

    #[test]
    fn renaming_a_dynamic_collection_changes_only_its_name() {
        for entry in samples().into_iter().filter(|e| e.contains("filterSpec")) {
            let c = read(entry);
            let quoted = serde_json::to_string(&c.name).unwrap();
            let edit = model::CollectionEdit { name: "Новое \"имя\"".into(), ..same(&c) };
            let want = entry.replacen(&format!("\"name\":{quoted}"), r#""name":"Новое \"имя\"""#, 1);
            assert_eq!(entry_value(Some(entry), &edit), want);
        }
    }

    #[test]
    fn editing_filters_keeps_the_games_added_by_hand_and_unknown_fields() {
        let entry = samples().into_iter().find(|e| e.contains("futureField")).unwrap();
        let c = read(entry);
        let mut f = DynamicFilter::parse(c.filter.as_deref().unwrap()).unwrap();
        f.groups[LANGUAGES].options.push(8);
        let spec = f.to_json();
        let out = entry_value(Some(entry), &model::CollectionEdit { filter: Some(spec.clone()), ..same(&c) });
        assert_eq!(out, entry.replace(c.filter.as_deref().unwrap(), &spec));
    }

    #[test]
    fn a_dynamic_entry_keeps_its_lists_even_without_new_filters() {
        let entry = samples().into_iter().find(|e| e.contains("filterSpec") && !e.contains(r#""added":[]"#)).unwrap();
        let c = read(entry);
        // An edit that doesn't carry the filters must not rewrite the games.
        let edit = model::CollectionEdit { id: c.id.clone(), name: c.name.clone(), games: vec![1, 2, 3], ..Default::default() };
        assert_eq!(entry_value(Some(entry), &edit), entry);
    }

    #[test]
    fn a_new_dynamic_collection_is_in_steams_shape() {
        let mut f = DynamicFilter::default();
        f.groups[FEATURES].options = vec![7, 9];
        f.groups[TAGS] = FilterGroup { options: vec![19, 492], any: true };
        let out = entry_value(None, &model::CollectionEdit { id: "uc-aaaaaaaaaaaa".into(), name: "Co-op".into(), filter: Some(f.to_json()), ..Default::default() });
        assert_eq!(
            out,
            concat!(
                r#"{"id":"uc-aaaaaaaaaaaa","name":"Co-op","added":[],"removed":[],"filterSpec":{"nFormatVersion":2,"strSearchText":"","filterGroups":["#,
                r#"{"rgOptions":[],"bAcceptUnion":true},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[7,9],"bAcceptUnion":false},"#,
                r#"{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[19,492],"bAcceptUnion":true},{"rgOptions":[],"bAcceptUnion":false},"#,
                r#"{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false}],"setSuggestions":{}}}"#
            )
        );
    }

    #[test]
    fn filters_it_cant_write_back_exactly_arent_editable() {
        let good = DynamicFilter::default().to_json();
        assert!(DynamicFilter::is_exact(&good));
        for bad in [
            good.replace("\"setSuggestions\":{}", "\"setSuggestions\":{},\"future\":1"),
            good.replace("\"nFormatVersion\":2,\"strSearchText\":\"\"", "\"strSearchText\":\"\",\"nFormatVersion\":2"),
            good.replacen("{\"rgOptions\":[],\"bAcceptUnion\":true},", "", 1),
            good.replacen("\"rgOptions\":[]", "\"rgOptions\":[\"x\"]", 1),
            good.replace("\"nFormatVersion\":2", "\"nFormatVersion\":3"),
        ] {
            assert!(!DynamicFilter::is_exact(&bad), "{bad}");
        }
        assert!(DynamicFilter::parse(&good.replace("\"nFormatVersion\":2", "\"nFormatVersion\":3")).is_none());
    }

    /// Point `STEAM_COLLECTIONS_COPY` at a **copy** of Steam's
    /// `cloud-storage-namespace-1.json`. Reads it only and prints no names.
    #[test]
    #[ignore = "needs a copy of a real Steam cache"]
    fn a_copy_of_real_collections_round_trips() {
        let path = std::env::var("STEAM_COLLECTIONS_COPY").expect("STEAM_COLLECTIONS_COPY");
        let all: Vec<(String, Value)> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let (mut count, mut dynamic) = (0, 0);
        for (key, e) in &all {
            let Some(value) = e.get("value").and_then(Value::as_str).filter(|_| key.starts_with(PREFIX)) else { continue };
            let c = read(value);
            assert!(entry_value(Some(value), &same(&c)) == value, "entry {count} changed");
            if let Some(spec) = &c.filter {
                assert!(DynamicFilter::is_exact(spec), "filters of entry {count} aren't exact");
                dynamic += 1;
            }
            count += 1;
        }
        assert!(count > 0);
        println!("{count} collections round-tripped, {dynamic} dynamic");
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

    fn with(group: usize, options: &[u64], any: bool) -> DynamicFilter {
        let mut f = DynamicFilter::default();
        f.groups[group] = FilterGroup { options: options.to_vec(), any };
        f
    }

    #[test]
    fn checks_languages() {
        let game = GameFacts { languages: language_bits(["english", "russian"].into_iter()), ..Default::default() };
        assert!(with(LANGUAGES, &[8], true).matches(&game));
        assert!(!with(LANGUAGES, &[10], true).matches(&game));
        assert!(with(LANGUAGES, &[0, 10], true).matches(&game));
        assert!(!with(LANGUAGES, &[0, 10], false).matches(&game));
        // No language data: shown rather than hidden.
        assert!(with(LANGUAGES, &[10], true).matches(&GameFacts::default()));
    }

    #[test]
    fn checks_subscriptions() {
        let f = with(SUBSCRIPTIONS, &[EA_PLAY], false);
        assert!(f.matches(&GameFacts { master_sub: EA_PLAY_APP, ..Default::default() }));
        assert!(!f.matches(&GameFacts { master_sub: 5, ..Default::default() }));
        assert!(!f.matches(&GameFacts::default()));
    }

    #[test]
    fn checks_friends_who_own_a_game() {
        let friends: FriendGames = [(11, HashSet::from([440])), (22, HashSet::from([570]))].into();
        let game = GameFacts { app_id: 440, friends: Some(&friends), ..Default::default() };
        assert!(with(FRIENDS, &[11], false).matches(&game));
        assert!(!with(FRIENDS, &[22], false).matches(&game));
        assert!(with(FRIENDS, &[11, 22], true).matches(&game));
        assert!(!with(FRIENDS, &[11, 22], false).matches(&game));
        // A copy borrowed from that friend doesn't count as them owning it.
        assert!(!with(FRIENDS, &[11], false).matches(&GameFacts { owner: 11, ..game.clone() }));
        // A private or unloaded friend can't be checked: shown, not hidden.
        assert!(with(FRIENDS, &[33], false).matches(&game));
    }

    #[test]
    fn games_added_and_removed_by_hand_win_over_the_filters() {
        let f = with(FEATURES, &[7], false);
        let single = GameFacts { app_id: 1, categories: &[2], ..Default::default() };
        let other = GameFacts { app_id: 2, ..Default::default() };
        assert!(f.contains(&single, &[], &[]));
        assert!(!f.contains(&other, &[], &[]));
        assert!(f.contains(&other, &[2], &[]));
        assert!(!f.contains(&single, &[], &[1]));
        assert!(!f.contains(&other, &[2], &[2]));
        // With no filters at all, only the games added by hand.
        let blank = DynamicFilter::default();
        assert!(!blank.contains(&single, &[], &[]));
        assert!(blank.contains(&single, &[1], &[]));
    }
}

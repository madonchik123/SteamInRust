//! Storage helpers (`storage`): `storage_has`, `storage_delete`,
//! `storage_keys`, `storage_clear`, `storage_size`, and JSON values with
//! `storage_set_json` / `storage_get_json`. Everything a plugin saves is
//! capped at [`MAX_KEYS`] keys and [`MAX_TOTAL`] bytes.

use super::{Host, Permission, from_json, to_json};
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use std::sync::{Arc, Mutex};

pub const MAX_KEYS: usize = 200;
/// Longest value, in characters (`storage_set` cuts longer ones; JSON
/// values are refused, since a cut one wouldn't parse).
pub const MAX_VALUE: usize = 16 * 1024;
/// All keys and values together, in bytes.
pub const MAX_TOTAL: usize = 1024 * 1024;

impl Host {
    fn storage_bytes(&self) -> usize {
        self.storage.iter().map(|(k, v)| k.len() + v.len()).sum()
    }

    /// Save a value, within the limits.
    pub(crate) fn store(&mut self, key: &str, value: String) -> Result<(), Box<EvalAltResult>> {
        let key: String = key.chars().take(100).collect();
        if self.storage.len() >= MAX_KEYS && !self.storage.contains_key(&key) {
            return Err(format!("storage is limited to {MAX_KEYS} keys").into());
        }
        let old = self.storage.get(&key).map(|v| key.len() + v.len()).unwrap_or(0);
        if self.storage_bytes() - old + key.len() + value.len() > MAX_TOTAL {
            return Err("storage is full (1 MB)".into());
        }
        self.storage.insert(key, value);
        self.storage_dirty = true;
        Ok(())
    }
}

pub(crate) fn register(e: &mut Engine, host: &Arc<Mutex<Host>>) {
    let h = host.clone();
    e.register_fn("storage_has", move |k: &str| -> Result<bool, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        Ok(h.storage.contains_key(k))
    });

    let h = host.clone();
    e.register_fn("storage_delete", move |k: &str| -> Result<bool, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        let gone = h.storage.remove(k).is_some();
        h.storage_dirty |= gone;
        Ok(gone)
    });

    let h = host.clone();
    e.register_fn("storage_keys", move || -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        Ok(h.storage.keys().map(|k| Dynamic::from(k.clone())).collect())
    });

    let h = host.clone();
    e.register_fn("storage_clear", move || -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        if !h.storage.is_empty() {
            h.storage.clear();
            h.storage_dirty = true;
        }
        Ok(())
    });

    let h = host.clone();
    e.register_fn("storage_size", move || -> Result<i64, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        Ok(h.storage_bytes() as i64)
    });

    let h = host.clone();
    e.register_fn("storage_set_json", move |k: &str, v: Dynamic| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        let text = to_json(&v).to_string();
        if text.chars().count() > MAX_VALUE {
            return Err(format!("that value is too big to save (at most {MAX_VALUE} characters as JSON)").into());
        }
        h.store(k, text)
    });

    let h = host.clone();
    e.register_fn("storage_get_json", move |k: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        // Not JSON (saved with storage_set): `()`, like a missing key.
        Ok(h.storage.get(k).and_then(|v| serde_json::from_str(v).ok()).map(from_json).unwrap_or(Dynamic::UNIT))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::plugin;
    use std::collections::{BTreeMap, BTreeSet};

    fn run(script: &str, saved: BTreeMap<String, String>) -> crate::Plugin {
        let mut p = plugin("id = \"sto\"\nname = \"S\"\npermissions = [\"storage\"]\n", script);
        p.start(&BTreeSet::from([Permission::Storage]), saved);
        p
    }

    #[test]
    fn has_delete_keys_clear_and_json() {
        let p = run(
            r#"fn on_load() {
                storage_set_json("cfg", #{games: [440, 620], on: true, name: "x"});
                let c = storage_get_json("cfg");
                storage_set("seen", c.games[1] + " " + c.on + " " + c.name + " " + storage_has("cfg") + " " + storage_has("nope"));
                storage_set("plain", "hello");
                storage_set("json_of_plain", "" + (storage_get_json("plain") == ()));
                storage_set("deleted", "" + storage_delete("plain") + " " + storage_delete("plain"));
                storage_set("keys", storage_keys().to_string());
                storage_set("size", "" + (storage_size() > 0));
            }"#,
            BTreeMap::from([("old".to_string(), "1".to_string())]),
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["seen"], "620 true x true false");
        assert_eq!(s["json_of_plain"], "true");
        assert_eq!(s["deleted"], "true false");
        assert_eq!(s["keys"], "[\"cfg\", \"deleted\", \"json_of_plain\", \"old\", \"seen\"]");
        assert_eq!(s["size"], "true");

        let p = run("fn on_load() { storage_clear(); }", BTreeMap::from([("a".to_string(), "1".to_string())]));
        assert!(p.take_storage().unwrap().is_empty());
    }

    #[test]
    fn storage_is_capped() {
        let p = run("fn on_load() { let s = \"x\"; for i in 0..15 { s += s; } storage_set_json(\"big\", s); }", BTreeMap::new());
        assert!(p.error.as_deref().is_some_and(|e| e.contains("too big")), "{:?}", p.error);

        // 1 MB holds about 69 values of 15 000 characters, so 80 don't fit.
        let p = run("fn on_load() { let s = \"x\"; for i in 0..14 { s += s; } s.truncate(15000); for i in 0..80 { storage_set(\"k\" + i, s); } }", BTreeMap::new());
        assert!(p.error.as_deref().is_some_and(|e| e.contains("full")), "{:?}", p.error);

        let p = run("fn on_load() { storage_keys(); }", BTreeMap::new());
        assert!(p.error.is_none());
        let mut q = plugin("id = \"sto2\"\nname = \"S\"\n", "fn on_load() { storage_keys(); }");
        q.start(&BTreeSet::new(), BTreeMap::new());
        assert!(q.error.as_deref().is_some_and(|e| e.contains("'storage'")), "{:?}", q.error);
    }
}

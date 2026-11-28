//! Store data (`store.read`): `app_details`, `store_price`, `store_search`,
//! `wishlist_items`, and the `wishlist_changed`, `app_details_loaded` and
//! `store_search_loaded` events.
//!
//! Store prices are what the game costs in the store, in the store's
//! currency. They are never mixed with Market prices (`market_price`).

use super::{Host, HostEvent, Permission, Request, map};
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use std::sync::{Arc, Mutex};

/// New store lookups a plugin may start in a minute (repeats of one that's
/// already loading are free).
pub const STORE_LOOKUPS_PER_MINUTE: usize = 30;

/// A store price.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StorePrice {
    /// Cents (the store's smallest unit) before any discount.
    pub initial: Option<u64>,
    /// Cents now.
    pub final_: Option<u64>,
    /// Percent off.
    pub discount: u8,
    /// "USD", "EUR"...
    pub currency: String,
    /// As the store shows it ("$9.99"), when it says.
    pub text: Option<String>,
}

/// A game's store page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoreApp {
    pub app_id: u32,
    pub name: String,
    pub kind: String,
    pub short_description: String,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub genres: Vec<String>,
    pub categories: Vec<String>,
    pub release_date: String,
    pub coming_soon: bool,
    pub is_free: bool,
    pub price: Option<StorePrice>,
    pub metacritic: Option<u32>,
    pub recommendations: Option<u64>,
    /// (windows, macos, linux)
    pub platforms: (bool, bool, bool),
    pub website: Option<String>,
    pub header: String,
    pub background: String,
    pub dlc: Vec<u32>,
    pub demo: Option<u32>,
}

/// A store search result.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoreHit {
    pub app_id: u32,
    pub name: String,
    pub header: String,
    pub price: StorePrice,
}

fn opt_u(v: Option<u64>) -> Dynamic {
    v.map(|x| Dynamic::from(x as i64)).unwrap_or(Dynamic::UNIT)
}

fn strings(v: &[String]) -> Dynamic {
    Dynamic::from_array(v.iter().map(|s| Dynamic::from(s.clone())).collect())
}

fn price_map(p: &StorePrice, free: bool) -> Dynamic {
    Dynamic::from_map(map([
        ("initial", opt_u(p.initial)),
        ("final", opt_u(p.final_)),
        ("discount", (p.discount as i64).into()),
        ("currency", p.currency.clone().into()),
        ("text", p.text.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT)),
        ("free", free.into()),
    ]))
}

fn app_map(a: &StoreApp) -> Dynamic {
    let (win, mac, linux) = a.platforms;
    let platforms: Array = [(win, "windows"), (mac, "macos"), (linux, "linux")].into_iter().filter(|(on, _)| *on).map(|(_, s)| Dynamic::from(s)).collect();
    Dynamic::from_map(map([
        ("app_id", (a.app_id as i64).into()),
        ("name", a.name.clone().into()),
        ("type", a.kind.clone().into()),
        ("short_description", a.short_description.clone().into()),
        ("developers", strings(&a.developers)),
        ("publishers", strings(&a.publishers)),
        ("genres", strings(&a.genres)),
        ("categories", strings(&a.categories)),
        ("release_date", a.release_date.clone().into()),
        ("coming_soon", a.coming_soon.into()),
        ("free", a.is_free.into()),
        ("price", a.price.as_ref().map(|p| price_map(p, a.is_free)).unwrap_or(Dynamic::UNIT)),
        ("metacritic", a.metacritic.map(|m| Dynamic::from(m as i64)).unwrap_or(Dynamic::UNIT)),
        ("recommendations", opt_u(a.recommendations)),
        ("platforms", Dynamic::from_array(platforms)),
        ("website", a.website.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT)),
        ("header", a.header.clone().into()),
        ("background", a.background.clone().into()),
        ("dlc", Dynamic::from_array(a.dlc.iter().map(|d| Dynamic::from(*d as i64)).collect())),
        ("demo", a.demo.map(|d| Dynamic::from(d as i64)).unwrap_or(Dynamic::UNIT)),
        ("store_url", format!("https://store.steampowered.com/app/{}/", a.app_id).into()),
    ]))
}

fn error_map(e: &str) -> Dynamic {
    Dynamic::from_map(map([("error", e.to_string().into())]))
}

impl Host {
    /// Ask for a store lookup unless it's already on its way; at most
    /// [`STORE_LOOKUPS_PER_MINUTE`] new ones a minute (the rest wait for a
    /// later call).
    fn store_lookup(&mut self, key: String, r: Request) {
        let now = std::time::Instant::now();
        self.store_asked.retain(|_, t| now.duration_since(*t).as_secs() < 60);
        if self.store_asked.contains_key(&key) {
            return;
        }
        if self.store_asked.len() >= STORE_LOOKUPS_PER_MINUTE {
            if !self.store_limit_logged {
                self.store_limit_logged = true;
                self.log(format!("store lookups are limited to {STORE_LOOKUPS_PER_MINUTE} a minute; the rest come later"));
            }
            return;
        }
        self.store_limit_logged = false;
        self.store_asked.insert(key, now);
        self.push_load(r);
    }

    /// A store page: `Some(Ok)` once loaded, `Some(Err)` if it failed, `None`
    /// after asking for it.
    fn store_app(&mut self, app_id: u32) -> Option<Result<StoreApp, String>> {
        match self.snapshot.store.get(&app_id) {
            Some(r) => Some(r.clone()),
            None => {
                self.store_lookup(format!("app:{app_id}"), Request::StoreDetails(app_id));
                None
            }
        }
    }
}

/// The search key: trimmed, lower case, at most 100 characters.
pub fn search_key(term: &str) -> String {
    term.trim().to_lowercase().chars().take(100).collect()
}

pub(crate) fn register(e: &mut Engine, host: &Arc<Mutex<Host>>) {
    let h = host.clone();
    e.register_fn("app_details", move |app_id: i64| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::StoreRead)?;
        Ok(match h.store_app(app_id as u32) {
            Some(Ok(a)) => app_map(&a),
            Some(Err(e)) => error_map(&e),
            None => Dynamic::UNIT,
        })
    });

    let h = host.clone();
    e.register_fn("store_price", move |app_id: i64| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::StoreRead)?;
        Ok(match h.store_app(app_id as u32) {
            Some(Ok(a)) if a.is_free => price_map(&StorePrice { initial: Some(0), final_: Some(0), ..a.price.clone().unwrap_or_default() }, true),
            Some(Ok(a)) => a.price.as_ref().map(|p| price_map(p, false)).unwrap_or_else(|| error_map("not for sale")),
            Some(Err(e)) => error_map(&e),
            None => Dynamic::UNIT,
        })
    });

    let h = host.clone();
    e.register_fn("store_search", move |term: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::StoreRead)?;
        let key = search_key(term);
        if key.is_empty() {
            return Ok(Dynamic::from_array(Array::new()));
        }
        Ok(match h.snapshot.searches.get(&key) {
            Some(Ok(hits)) => Dynamic::from_array(
                hits.iter()
                    .map(|x| {
                        Dynamic::from_map(map([
                            ("app_id", (x.app_id as i64).into()),
                            ("name", x.name.clone().into()),
                            ("header", x.header.clone().into()),
                            ("price", price_map(&x.price, x.price.final_ == Some(0))),
                        ]))
                    })
                    .collect(),
            ),
            Some(Err(e)) => error_map(e),
            None => {
                h.store_lookup(format!("search:{key}"), Request::StoreSearch(key.clone()));
                Dynamic::UNIT
            }
        })
    });

    let h = host.clone();
    e.register_fn("wishlist_items", move || -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::StoreRead)?;
        let name = |id: u32| h.snapshot.app_names.get(&id).cloned().unwrap_or_default();
        Ok(h.snapshot
            .wishlist
            .iter()
            .enumerate()
            .map(|(i, (id, added))| {
                Dynamic::from_map(map([
                    ("app_id", (*id as i64).into()),
                    ("name", name(*id).into()),
                    ("added", (*added as i64).into()),
                    ("rank", (i as i64 + 1).into()),
                ]))
            })
            .collect())
    });
}

/// Games added to and removed from the wishlist between two lists.
pub fn wishlist_changes(before: &[u32], after: &[u32]) -> Option<HostEvent> {
    let added: Vec<u32> = after.iter().copied().filter(|id| !before.contains(id)).collect();
    let removed: Vec<u32> = before.iter().copied().filter(|id| !after.contains(id)).collect();
    (!added.is_empty() || !removed.is_empty()).then_some(HostEvent::WishlistChanged { added, removed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Snapshot;
    use crate::tests::plugin;
    use std::collections::{BTreeSet, HashMap};

    fn price(cents: u64, discount: u8) -> StorePrice {
        StorePrice { initial: Some(cents * 100 / (100 - discount as u64)), final_: Some(cents), discount, currency: "USD".into(), text: Some(format!("${}.{:02}", cents / 100, cents % 100)) }
    }

    fn snap() -> Snapshot {
        Snapshot {
            store: Arc::new(HashMap::from([
                (440, Ok(StoreApp { app_id: 440, name: "TF2".into(), is_free: true, developers: vec!["Valve".into()], platforms: (true, false, true), ..Default::default() })),
                (620, Ok(StoreApp { app_id: 620, name: "Portal 2".into(), price: Some(price(199, 80)), release_date: "Apr 18, 2011".into(), genres: vec!["Action".into()], ..Default::default() })),
                (1, Err("not found".into())),
            ])),
            searches: Arc::new(HashMap::from([("portal".to_string(), Ok(vec![StoreHit { app_id: 620, name: "Portal 2".into(), price: price(199, 80), ..Default::default() }]))])),
            wishlist: vec![(620, 1_600_000_000), (70, 1_700_000_000)],
            app_names: Arc::new(HashMap::from([(620, "Portal 2".to_string())])),
            ..Default::default()
        }
    }

    fn run(script: &str, perms: &[Permission]) -> crate::Plugin {
        let keys: Vec<String> = perms.iter().map(|p| format!("\"{}\"", p.key())).collect();
        let mut p = plugin(&format!("id = \"st\"\nname = \"S\"\npermissions = [{}]\n", keys.join(", ")), script);
        p.set_snapshot(snap());
        p.start(&perms.iter().copied().collect::<BTreeSet<_>>(), Default::default());
        p
    }

    #[test]
    fn details_prices_and_search() {
        let p = run(
            r#"fn on_load() {
                let a = app_details(620);
                storage_set("a", a.name + " " + a.release_date + " " + a.genres + " " + a.price.final + " " + a.price.discount + " " + a.price.currency);
                let t = app_details(440);
                storage_set("t", t.developers[0] + " " + t.platforms + " " + t.free);
                storage_set("p", store_price(440).final + " " + store_price(440).free + " " + store_price(620).text);
                storage_set("e", app_details(1).error);
                let s = store_search("  Portal ");
                storage_set("s", s.len() + " " + s[0].name + " " + s[0].price.final);
                storage_set("blank", "" + store_search(" ").len());
            }"#,
            &[Permission::StoreRead, Permission::Storage],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["a"], "Portal 2 Apr 18, 2011 [\"Action\"] 199 80 USD");
        assert_eq!(s["t"], "Valve [\"windows\", \"linux\"] true");
        assert_eq!(s["p"], "0 true $1.99");
        assert_eq!(s["e"], "not found");
        assert_eq!(s["s"], "1 Portal 2 199");
        assert_eq!(s["blank"], "0");
        assert!(p.drain().is_empty());
    }

    #[test]
    fn unknown_pages_load_once_and_are_rate_limited() {
        let p = run(
            r#"fn on_load() {
                for i in 0..5 { app_details(10); store_price(10); store_search("half life"); }
                for i in 100..200 { app_details(i); }
            }"#,
            &[Permission::StoreRead],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let reqs = p.drain();
        assert_eq!(reqs.iter().filter(|r| matches!(r, Request::StoreDetails(10))).count(), 1);
        assert_eq!(reqs.iter().filter(|r| matches!(r, Request::StoreSearch(t) if t == "half life")).count(), 1);
        assert_eq!(reqs.len(), STORE_LOOKUPS_PER_MINUTE, "{reqs:?}");
        assert!(p.logs().iter().any(|l| l.contains("limited")));
    }

    #[test]
    fn wishlist_items_and_changes() {
        let p = run(
            r#"fn on_load() {
                let w = wishlist_items();
                storage_set("w", w[0].name + " " + w[0].added + " " + w[0].rank + " " + w[1].app_id + " " + w[1].rank);
                storage_set("ids", wishlist().to_string());
            }"#,
            &[Permission::StoreRead, Permission::Storage],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["w"], "Portal 2 1600000000 1 70 2");
        assert_eq!(s["ids"], "[620, 70]");

        assert!(wishlist_changes(&[1, 2], &[2, 1]).is_none(), "a new order isn't a change");
        let ev = wishlist_changes(&[1, 2], &[2, 3]).unwrap();
        assert!(matches!(&ev, HostEvent::WishlistChanged { added, removed } if added == &vec![3] && removed == &vec![1]));
        let mut p = run(
            "fn on_event(name, data) { if name == \"wishlist_changed\" { storage_set(name, data.added + \" \" + data.removed); } else { storage_set(name, \"\" + data.app_id); } }",
            &[Permission::StoreRead, Permission::Storage],
        );
        p.event(&ev);
        p.event(&HostEvent::AppDetailsLoaded { app_id: 3 });
        let s = p.take_storage().unwrap();
        assert_eq!(s["wishlist_changed"], "[3] [1]");
        assert!(s.contains_key("app_details_loaded"));
    }

    #[test]
    fn store_calls_need_permission() {
        for call in ["app_details(440)", "store_price(440)", "store_search(\"x\")", "wishlist_items()"] {
            let p = run(&format!("fn on_load() {{ {call}; }}"), &[Permission::LibraryRead]);
            assert!(p.error.as_deref().is_some_and(|e| e.contains("store.read")), "{call}: {:?}", p.error);
            assert!(p.drain().is_empty());
        }
    }
}

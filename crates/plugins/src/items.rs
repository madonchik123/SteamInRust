//! Inventory and trade helpers: `market_prices`, `item_info`,
//! `tradable_items`, `marketable_items`, `inventory_value`
//! (`inventory.read`), `trade_offer_info` and `evaluate_trade`
//! (`trades.read`).
//!
//! Prices here are Community Market prices, in cents of your wallet
//! currency. Store prices are separate (`store_price`).

use super::{Host, Permission, Request, item_map, map, offer_map};
use rhai::{Array, Dynamic, Engine, EvalAltResult};
use std::sync::{Arc, Mutex};

/// Most names one `market_prices` call looks up.
const MAX_PRICE_NAMES: usize = 500;

impl Host {
    fn price(&self, app_id: u32, hash_name: &str) -> Option<u64> {
        self.snapshot.prices.get(&(app_id, hash_name.to_string())).copied()
    }

    /// Look up the prices of these marketable items that aren't known yet.
    fn ask_prices<'a>(&mut self, items: impl Iterator<Item = (u32, &'a model::InventoryItem)>) {
        let mut by_app: std::collections::BTreeMap<u32, Vec<String>> = Default::default();
        for (app, i) in items {
            if i.marketable && !i.market_hash_name.is_empty() && self.price(app, &i.market_hash_name).is_none() {
                let names = by_app.entry(app).or_default();
                if !names.contains(&i.market_hash_name) && names.len() < MAX_PRICE_NAMES {
                    names.push(i.market_hash_name.clone());
                }
            }
        }
        for (app, names) in by_app {
            self.push_load(Request::Prices(app, names));
        }
    }

    /// One of your inventories (shared, not copied), or `None` after asking for it.
    fn items(&mut self, app_id: u32, context_id: u64) -> Option<Inventory> {
        let all = self.snapshot.inventory.clone();
        if all.contains_key(&(app_id, context_id)) {
            return Some(Inventory(all, (app_id, context_id)));
        }
        self.push_load(Request::LoadInventory(app_id, context_id));
        None
    }
}

/// A loaded inventory, read from the shared snapshot.
struct Inventory(Arc<std::collections::HashMap<(u32, u64), Vec<model::InventoryItem>>>, (u32, u64));

impl std::ops::Deref for Inventory {
    type Target = [model::InventoryItem];
    fn deref(&self) -> &Self::Target {
        self.0.get(&self.1).map(Vec::as_slice).unwrap_or_default()
    }
}

/// What a list of items is worth: (total cents, priced, unpriced).
fn worth<'a>(h: &Host, items: impl Iterator<Item = (u32, &'a model::InventoryItem)>) -> (u64, i64, i64) {
    let (mut total, mut priced, mut unpriced) = (0u64, 0, 0);
    for (app, i) in items.filter(|(_, i)| i.marketable) {
        match h.price(app, &i.market_hash_name) {
            Some(p) => {
                total += p * i.amount.max(1);
                priced += 1;
            }
            None => unpriced += 1,
        }
    }
    (total, priced, unpriced)
}

pub(crate) fn register(e: &mut Engine, host: &Arc<Mutex<Host>>) {
    let h = host.clone();
    e.register_fn("market_prices", move |app_id: i64, names: Array| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::InventoryRead)?;
        let app = app_id as u32;
        let mut out = rhai::Map::new();
        let mut missing = Vec::new();
        for n in names.into_iter().take(MAX_PRICE_NAMES).map(|n| n.to_string()) {
            let p = h.price(app, &n);
            if p.is_none() && !missing.contains(&n) {
                missing.push(n.clone());
            }
            out.insert(n.into(), p.map(|c| Dynamic::from(c as i64)).unwrap_or(Dynamic::UNIT));
        }
        if !missing.is_empty() {
            h.push_load(Request::Prices(app, missing));
        }
        Ok(Dynamic::from_map(out))
    });

    let h = host.clone();
    e.register_fn("item_info", move |app_id: i64, context_id: i64, asset_id: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::InventoryRead)?;
        let asset: u64 = asset_id.parse().map_err(|_| "asset_id must be a number in a string")?;
        let app = app_id as u32;
        Ok(h.items(app, context_id as u64)
            .and_then(|l| l.iter().find(|i| i.asset_id == asset).map(|i| item_map(app, i, h.price(app, &i.market_hash_name))))
            .unwrap_or(Dynamic::UNIT))
    });

    let h = host.clone();
    let filtered = move |app_id: i64, context_id: i64, keep: fn(&model::InventoryItem) -> bool| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::InventoryRead)?;
        let app = app_id as u32;
        Ok(match h.items(app, context_id as u64) {
            Some(l) => Dynamic::from_array(l.iter().filter(|i| keep(i)).map(|i| item_map(app, i, h.price(app, &i.market_hash_name))).collect()),
            None => Dynamic::UNIT,
        })
    };
    let f = filtered.clone();
    e.register_fn("tradable_items", move |app_id: i64, context_id: i64| f(app_id, context_id, |i| i.tradable));
    let f = filtered;
    e.register_fn("marketable_items", move |app_id: i64, context_id: i64| f(app_id, context_id, |i| i.marketable));

    let h = host.clone();
    e.register_fn("inventory_value", move |app_id: i64, context_id: i64| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::InventoryRead)?;
        let app = app_id as u32;
        let Some(list) = h.items(app, context_id as u64) else { return Ok(Dynamic::UNIT) };
        h.ask_prices(list.iter().map(|i| (app, i)));
        let (total, priced, unpriced) = worth(&h, list.iter().map(|i| (app, i)));
        Ok(Dynamic::from_map(map([
            ("total", (total as i64).into()),
            ("priced", priced.into()),
            ("unpriced", unpriced.into()),
            ("currency", h.snapshot.currency.clone().into()),
        ])))
    });

    let h = host.clone();
    e.register_fn("trade_offer_info", move |id: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::TradesRead)?;
        let id: u64 = id.parse().map_err(|_| "id must be a trade offer id")?;
        Ok(h.snapshot.trades.iter().find(|(o, _)| o.id == id).map(|(o, name)| offer_map(&h.snapshot, o, name)).unwrap_or(Dynamic::UNIT))
    });

    let h = host.clone();
    e.register_fn("evaluate_trade", move |id: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::TradesRead)?;
        let id: u64 = id.parse().map_err(|_| "id must be a trade offer id")?;
        let trades = h.snapshot.trades.clone();
        let Some((o, _)) = trades.iter().find(|(o, _)| o.id == id) else { return Ok(Dynamic::UNIT) };
        fn side(l: &[model::TradeItem]) -> Vec<(u32, &model::InventoryItem)> {
            l.iter().map(|x| (x.app_id, &x.item)).collect()
        }
        let (give, get) = (side(&o.give), side(&o.get));
        h.ask_prices(give.iter().chain(get.iter()).copied());
        let (give_total, _, give_unpriced) = worth(&h, give.iter().copied());
        let (get_total, _, get_unpriced) = worth(&h, get.iter().copied());
        // Items that can't be sold on the Market have no price at all.
        let unsellable = give.iter().chain(get.iter()).filter(|(_, i)| !i.marketable).count() as i64;
        Ok(Dynamic::from_map(map([
            ("give", (give_total as i64).into()),
            ("get", (get_total as i64).into()),
            ("difference", (get_total as i64 - give_total as i64).into()),
            ("give_unpriced", give_unpriced.into()),
            ("get_unpriced", get_unpriced.into()),
            ("not_marketable", unsellable.into()),
            ("complete", (give_unpriced == 0 && get_unpriced == 0).into()),
            ("currency", h.snapshot.currency.clone().into()),
        ])))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Snapshot;
    use crate::tests::plugin;
    use std::collections::{BTreeSet, HashMap};

    fn item(asset: u64, name: &str, tradable: bool, marketable: bool, amount: u64) -> model::InventoryItem {
        model::InventoryItem {
            asset_id: asset,
            class_id: 9,
            name: name.into(),
            market_hash_name: format!("753-{name}"),
            tradable,
            marketable,
            amount,
            context_id: 6,
            trade_hold_days: 7,
            commodity: true,
            description: vec![("Line one".into(), None)],
            ..Default::default()
        }
    }

    fn snap() -> Snapshot {
        let inv = vec![item(1, "Card", true, true, 2), item(2, "Gem", false, true, 1), item(3, "Badge", false, false, 1), item(4, "Bg", true, true, 1)];
        let offer = model::TradeOffer {
            id: 77,
            partner: 5,
            give: vec![model::TradeItem { app_id: 753, item: item(1, "Card", true, true, 1), missing: false }],
            get: vec![model::TradeItem { app_id: 753, item: item(4, "Bg", true, true, 1), missing: false }, model::TradeItem { app_id: 753, item: item(9, "Odd", true, false, 1), missing: false }],
            ..Default::default()
        };
        Snapshot {
            inventory: Arc::new(HashMap::from([((753, 6), inv)])),
            prices: Arc::new(HashMap::from([((753, "753-Card".to_string()), 10), ((753, "753-Gem".to_string()), 3)])),
            trades: Arc::new(vec![(offer, "Pal".into())]),
            currency: "USD".into(),
            ..Default::default()
        }
    }

    fn run(script: &str, perms: &[Permission]) -> crate::Plugin {
        let keys: Vec<String> = perms.iter().map(|p| format!("\"{}\"", p.key())).collect();
        let mut p = plugin(&format!("id = \"it\"\nname = \"I\"\npermissions = [{}]\n", keys.join(", ")), script);
        p.set_snapshot(snap());
        p.start(&perms.iter().copied().collect::<BTreeSet<_>>(), Default::default());
        p
    }

    #[test]
    fn prices_items_and_value() {
        let p = run(
            r#"fn on_load() {
                let m = market_prices(753, ["753-Card", "753-Bg", "753-Card"]);
                storage_set("m", m["753-Card"] + " " + (m["753-Bg"] == ()) + " " + m.len());
                let i = item_info(753, 6, "1");
                storage_set("i", i.name + " " + i.price + " " + i.class_id + " " + i.trade_hold_days + " " + i.commodity + " " + i.description);
                storage_set("none", "" + (item_info(753, 6, "99") == ()));
                storage_set("t", tradable_items(753, 6).map(|x| x.name).to_string());
                storage_set("k", marketable_items(753, 6).map(|x| x.name).to_string());
                let v = inventory_value(753, 6);
                storage_set("v", v.total + " " + v.priced + " " + v.unpriced + " " + v.currency);
            }"#,
            &[Permission::InventoryRead, Permission::Storage],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["m"], "10 true 2");
        assert_eq!(s["i"], "Card 10 9 7 true [\"Line one\"]");
        assert_eq!(s["none"], "true");
        assert_eq!(s["t"], "[\"Card\", \"Bg\"]");
        assert_eq!(s["k"], "[\"Card\", \"Gem\", \"Bg\"]");
        assert_eq!(s["v"], "23 2 1 USD");
        // The missing price is asked for once, in one request.
        let reqs = p.drain();
        assert!(matches!(reqs.as_slice(), [Request::Prices(753, names)] if names == &vec!["753-Bg".to_string()]), "{reqs:?}");
    }

    #[test]
    fn unloaded_inventory_is_asked_for() {
        let p = run(
            "fn on_load() { if tradable_items(730, 2) != () || inventory_value(730, 2) != () || item_info(730, 2, \"1\") != () { throw \"loaded?\"; } }",
            &[Permission::InventoryRead],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let reqs = p.drain();
        assert_eq!(reqs.iter().filter(|r| matches!(r, Request::LoadInventory(730, 2))).count(), 1, "{reqs:?}");
    }

    #[test]
    fn trades_are_read_and_valued() {
        let p = run(
            r#"fn on_load() {
                let o = trade_offer_info("77");
                storage_set("o", o.partner_name + " " + o.give.len() + " " + o.get.len());
                storage_set("none", "" + (trade_offer_info("1") == ()) + " " + (evaluate_trade("1") == ()));
                let v = evaluate_trade("77");
                storage_set("v", v.give + " " + v.get + " " + v.difference + " " + v.get_unpriced + " " + v.not_marketable + " " + v.complete);
            }"#,
            &[Permission::TradesRead, Permission::Storage],
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        assert_eq!(s["o"], "Pal 1 2");
        assert_eq!(s["none"], "true true");
        assert_eq!(s["v"], "10 0 -10 1 1 false");
        assert!(p.drain().iter().any(|r| matches!(r, Request::Prices(753, n) if n == &vec!["753-Bg".to_string()])));
    }

    #[test]
    fn need_permissions() {
        for (call, perm) in [
            ("market_prices(753, [])", "inventory.read"),
            ("item_info(753, 6, \"1\")", "inventory.read"),
            ("tradable_items(753, 6)", "inventory.read"),
            ("marketable_items(753, 6)", "inventory.read"),
            ("inventory_value(753, 6)", "inventory.read"),
            ("trade_offer_info(\"77\")", "trades.read"),
            ("evaluate_trade(\"77\")", "trades.read"),
        ] {
            let p = run(&format!("fn on_load() {{ {call}; }}"), &[Permission::Storage]);
            assert!(p.error.as_deref().is_some_and(|e| e.contains(perm)), "{call}: {:?}", p.error);
        }
    }
}

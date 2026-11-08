//! Game inventories (ISteamInventory in our steamclient): the player's items
//! for one game and its item definitions, through Steam's Inventory service
//! like Steam's client does. Changes (drops, consuming, crafting) happen only
//! when the game asks for them.

use crate::ipc::protocol::{InvItem, InventoryOp, PropValue, Reply};
use model::AppId;
use net::{Connection, eresult};
use proto::*;

fn code(e: &net::Error) -> i32 {
    match e {
        net::Error::EResult(c, _) => *c,
        _ => eresult::FAIL,
    }
}

fn text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn number<T: std::str::FromStr + Default>(o: &serde_json::Map<String, serde_json::Value>, key: &str) -> T {
    o.get(key).map(text).and_then(|s| s.parse().ok()).unwrap_or_default()
}

/// Items from the service's `item_json` (plus removed ones, flagged).
pub fn items(item_json: &str, removed: &[u64]) -> Vec<InvItem> {
    let list: Vec<serde_json::Value> = serde_json::from_str(item_json).unwrap_or_default();
    let mut out: Vec<InvItem> = list
        .iter()
        .filter_map(|v| v.as_object())
        .map(|o| {
            let quantity: u32 = number(o, "quantity");
            // "state" lists flags by name; tradability comes as a timestamp/flag.
            let state = o.get("state").map(text).unwrap_or_default();
            let mut flags = 0u16;
            if state.contains("notrade") || o.get("tradable").is_some_and(|t| t == &serde_json::Value::Bool(false)) {
                flags |= 1;
            }
            if quantity == 0 {
                flags |= 0x200;
            }
            InvItem {
                id: number(o, "itemid"),
                def: number(o, "itemdefid"),
                quantity: quantity.min(u16::MAX as u32) as u16,
                flags,
                props: o.iter().map(|(k, v)| (k.clone(), text(v))).collect(),
            }
        })
        .collect();
    for id in removed {
        if !out.iter().any(|i| i.id == *id) {
            out.push(InvItem { id: *id, flags: 0x100, props: vec![("itemid".into(), id.to_string())], ..Default::default() });
        }
    }
    out
}

fn reply(r: Result<CInventoryResponse, net::Error>) -> Reply {
    match r {
        Ok(r) => Reply::Inventory { eresult: eresult::OK, items: items(r.item_json(), &r.removeditemids), ticket: r.ticket().to_vec() },
        Err(e) => {
            tracing::info!("inventory: {e}");
            Reply::Inventory { eresult: code(&e), items: Vec::new(), ticket: Vec::new() }
        }
    }
}

fn now() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

pub async fn run(conn: &Connection, app_id: AppId, me: u64, op: InventoryOp) -> Reply {
    let all = || CInventoryGetInventoryRequest { appid: Some(app_id), steamid: Some(me) };
    match op {
        InventoryOp::All => reply(conn.call("Inventory.GetInventory#1", &all()).await),
        InventoryOp::ByIds(ids) => match conn.call::<_, CInventoryResponse>("Inventory.GetInventory#1", &all()).await {
            Ok(r) => {
                let mut list = items(r.item_json(), &[]);
                list.retain(|i| ids.contains(&i.id));
                Reply::Inventory { eresult: eresult::OK, items: list, ticket: r.ticket().to_vec() }
            }
            Err(e) => Reply::Inventory { eresult: code(&e), items: Vec::new(), ticket: Vec::new() },
        },
        InventoryOp::Consume { item, quantity } => reply(
            conn.call(
                "Inventory.ConsumeItem#1",
                &CInventoryConsumeItemRequest { appid: Some(app_id), itemid: Some(item), quantity: Some(quantity.max(1)), steamid: Some(me), ..Default::default() },
            )
            .await,
        ),
        InventoryOp::Exchange { generate, destroy } => reply(
            conn.call(
                "Inventory.ExchangeItem#1",
                &CInventoryExchangeItemRequest {
                    appid: Some(app_id),
                    steamid: Some(me),
                    materialsitemid: destroy.iter().map(|d| d.0).collect(),
                    materialsquantity: destroy.iter().map(|d| d.1).collect(),
                    outputitemdefid: Some(generate as u64),
                },
            )
            .await,
        ),
        InventoryOp::Transfer { source, quantity, dest } if dest == 0 || dest == u64::MAX => reply(
            conn.call("Inventory.SplitItemStack#1", &CInventorySplitItemStackRequest { appid: Some(app_id), itemid: Some(source), quantity: Some(quantity), steamid: Some(me) }).await,
        ),
        InventoryOp::Transfer { source, quantity, dest } => reply(
            conn.call(
                "Inventory.CombineItemStacks#1",
                &CInventoryCombineItemStacksRequest { appid: Some(app_id), fromitemid: Some(source), destitemid: Some(dest), quantity: Some(quantity), steamid: Some(me) },
            )
            .await,
        ),
        InventoryOp::Drop { def } => {
            reply(conn.call("Inventory.ConsumePlaytime#1", &CInventoryConsumePlaytimeRequest { appid: Some(app_id), itemdefid: Some(def as u64) }).await)
        }
        InventoryOp::Promo(defs) => {
            let defs: Vec<u64> = if defs.is_empty() {
                match promo(conn, app_id, me).await {
                    Ok(d) => d.into_iter().map(|d| d as u64).collect(),
                    Err(e) => return Reply::Inventory { eresult: e, items: Vec::new(), ticket: Vec::new() },
                }
            } else {
                defs.into_iter().map(|d| d as u64).collect()
            };
            if defs.is_empty() {
                return Reply::Inventory { eresult: eresult::OK, items: Vec::new(), ticket: Vec::new() };
            }
            reply(conn.call("Inventory.AddPromoItem#1", &CInventoryAddItemRequest { appid: Some(app_id), itemdefid: defs, steamid: Some(me), notify: Some(true), ..Default::default() }).await)
        }
        InventoryOp::Generate(list) => reply(
            conn.call(
                "Inventory.DevGenerateItem#1",
                &CInventoryAddItemRequest {
                    appid: Some(app_id),
                    itemdefid: list.iter().map(|l| l.0 as u64).collect(),
                    itemquantity: list.iter().map(|l| l.1).collect(),
                    steamid: Some(me),
                    notify: Some(true),
                    ..Default::default()
                },
            )
            .await,
        ),
        InventoryOp::Modify(changes) => {
            use c_inventory_modify_items_request::ItemPropertyUpdate;
            let updates = changes
                .into_iter()
                .map(|(item, name, value)| {
                    let mut u = ItemPropertyUpdate { itemid: Some(item), property_name: Some(name), remove_property: Some(value.is_none()), ..Default::default() };
                    match value {
                        Some(PropValue::Str(s)) => u.property_value_string = Some(s),
                        Some(PropValue::Bool(b)) => u.property_value_bool = Some(b),
                        Some(PropValue::Int(i)) => u.property_value_int = Some(i),
                        Some(PropValue::Float(f)) => u.property_value_float = Some(f),
                        None => {}
                    }
                    u
                })
                .collect();
            reply(conn.call("Inventory.SafeModifyItems#1", &CInventoryModifyItemsRequest { appid: Some(app_id), steamid: Some(me), updates, timestamp: Some(now()) }).await)
        }
    }
}

async fn promo(conn: &Connection, app_id: AppId, me: u64) -> Result<Vec<i32>, i32> {
    conn.call::<_, CInventoryGetEligiblePromoItemDefIDsResponse>("Inventory.GetEligiblePromoItemDefIDs#1", &CInventoryGetEligiblePromoItemDefIDsRequest { appid: Some(app_id), steamid: Some(me) })
        .await
        .map(|r| r.itemdefids.into_iter().map(|d| d as i32).collect())
        .map_err(|e| code(&e))
}

pub async fn promo_items(conn: &Connection, app_id: AppId, me: u64) -> Reply {
    match promo(conn, app_id, me).await {
        Ok(defs) => Reply::PromoItems { eresult: eresult::OK, defs },
        Err(eresult) => Reply::PromoItems { eresult, defs: Vec::new() },
    }
}

/// Item definitions: (id, properties), parsed from Steam's item-def archive.
pub fn defs(json: &str) -> Vec<(i32, Vec<(String, String)>)> {
    let json = json.trim_end_matches('\0');
    let list: Vec<serde_json::Value> = serde_json::from_str(json).unwrap_or_default();
    list.iter()
        .filter_map(|v| v.as_object())
        .filter_map(|o| Some((o.get("itemdefid").map(text)?.parse().ok()?, o.iter().map(|(k, v)| (k.clone(), text(v))).collect())))
        .collect()
}

/// The digest of the game's current item definitions. Steam's client asks
/// while the game runs; a session that isn't playing it gets "access denied".
async fn digest(conn: &Connection, app_id: AppId) -> Option<String> {
    let header = CMsgProtoBufHeader { target_job_name: Some("Inventory.GetItemDefMeta#1".into()), routing_appid: Some(app_id), ..Default::default() };
    match conn.request_with(net::emsg::SERVICE_METHOD_CALL_FROM_CLIENT, header, &CInventoryGetItemDefMetaRequest { appid: Some(app_id) }).await.and_then(|p| p.check()?.body::<CInventoryGetItemDefMetaResponse>()) {
        Ok(m) => Some(m.digest().to_string()).filter(|d| !d.is_empty()),
        Err(e) => {
            tracing::info!("item definitions of {app_id}: {e}");
            None
        }
    }
}

/// The game's item definitions (the archive is public, keyed by its digest).
pub async fn item_defs(conn: &Connection, app_id: AppId) -> Reply {
    let empty = Reply::ItemDefs { defs: Vec::new() };
    let Some(digest) = digest(conn, app_id).await else { return empty };
    let url = format!("https://api.steampowered.com/IGameInventory/GetItemDefArchive/v0001?appid={app_id}&digest={digest}");
    match reqwest::get(&url).await {
        Ok(r) if r.status().is_success() => match r.text().await {
            Ok(body) => Reply::ItemDefs { defs: defs(&body) },
            Err(_) => empty,
        },
        Ok(r) => {
            tracing::info!("item definitions of {app_id}: HTTP {}", r.status());
            empty
        }
        Err(e) => {
            tracing::info!("item definitions of {app_id}: {e}");
            empty
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_items_and_defs() {
        let j = r#"[{"accountid":"1","itemid":"1001","quantity":3,"originalitemid":"1001","itemdefid":"10","appid":480,"acquired":"20240101T000000Z","state":"","origin":"playtime"}]"#;
        let it = items(j, &[77]);
        assert_eq!((it[0].id, it[0].def, it[0].quantity, it[0].flags), (1001, 10, 3, 0));
        assert!(it[0].props.contains(&("origin".to_string(), "playtime".to_string())));
        assert_eq!((it[1].id, it[1].flags), (77, 0x100));
        let d = defs("[{\"itemdefid\":10,\"name\":\"Hat\",\"tradable\":true}]\0");
        assert_eq!(d[0].0, 10);
        assert!(d[0].1.contains(&("tradable".to_string(), "true".to_string())));
    }
}

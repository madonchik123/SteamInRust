//! Trade offers: list, answer and send them the way Steam's own pages do.
//!
//! The list comes from Steam's trade offer web API (with the signed-in web
//! session's access token); accepting, declining, cancelling and sending go
//! through the same steamcommunity.com calls as the trade offer pages. Giving
//! items away still needs your confirmation in the Steam Mobile app.

use crate::market::Web;
use model::{AppId, InventoryItem, SteamId, TradeAction, TradeAsset, TradeItem, TradeOffer, TradeOffers, TradeState};
use serde_json::{Value, json};
use std::collections::HashMap;

const COMMUNITY: &str = "https://steamcommunity.com";
/// Steam ids of individual accounts are this plus the account id.
const INDIVIDUAL: u64 = 76561197960265728;

fn num(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn field(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(num).unwrap_or(0)
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(|s| s.as_str()).unwrap_or_default().to_string()
}

/// Your active trade offers and the ones that changed in the last two weeks.
pub async fn offers(web: &Web, language: &str) -> Result<TradeOffers, String> {
    if web.access_token.is_empty() {
        return Err("Sign in again to see your trade offers.".into());
    }
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).saturating_sub(14 * 86400);
    let r = web
        .http
        .get("https://api.steampowered.com/IEconService/GetTradeOffers/v1/")
        .query(&[
            ("access_token", web.access_token.as_str()),
            ("get_sent_offers", "1"),
            ("get_received_offers", "1"),
            ("get_descriptions", "1"),
            ("active_only", "1"),
            ("language", language),
            ("time_historical_cutoff", &since.to_string()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("Steam answered {}", r.status()));
    }
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    Ok(parse(&v))
}

/// Read Steam's `GetTradeOffers` answer.
pub fn parse(v: &Value) -> TradeOffers {
    let r = v.get("response").unwrap_or(v);
    let mut info: HashMap<(u64, u64, u64), &Value> = HashMap::new();
    for d in r.get("descriptions").and_then(|d| d.as_array()).into_iter().flatten() {
        info.insert((field(d, "appid"), field(d, "classid"), field(d, "instanceid")), d);
    }
    let list = |k: &str| -> Vec<TradeOffer> {
        let mut out: Vec<TradeOffer> = r.get(k).and_then(|o| o.as_array()).into_iter().flatten().map(|o| offer(o, &info)).collect();
        out.sort_by(|a, b| b.state.open().cmp(&a.state.open()).then(b.updated.max(b.created).cmp(&a.updated.max(a.created))));
        out
    };
    TradeOffers { received: list("trade_offers_received"), sent: list("trade_offers_sent") }
}

fn offer(o: &Value, info: &HashMap<(u64, u64, u64), &Value>) -> TradeOffer {
    let items = |k: &str| -> Vec<TradeItem> { o.get(k).and_then(|i| i.as_array()).into_iter().flatten().map(|a| item(a, info)).collect() };
    TradeOffer {
        id: field(o, "tradeofferid"),
        partner: INDIVIDUAL + field(o, "accountid_other"),
        message: text(o, "message"),
        created: field(o, "time_created"),
        updated: field(o, "time_updated"),
        expires: field(o, "expiration_time"),
        escrow_end: field(o, "escrow_end_date"),
        state: TradeState::from_steam(field(o, "trade_offer_state")),
        ours: o.get("is_our_offer").and_then(|b| b.as_bool()).unwrap_or(false),
        give: items("items_to_give"),
        get: items("items_to_receive"),
    }
}

fn item(a: &Value, info: &HashMap<(u64, u64, u64), &Value>) -> TradeItem {
    let app_id = field(a, "appid") as AppId;
    let class_id = field(a, "classid");
    let color = |d: &Value, k: &str| Some(text(d, k).trim_start_matches('#').to_string()).filter(|c| !c.is_empty());
    let mut item = InventoryItem {
        asset_id: field(a, "assetid"),
        class_id,
        context_id: field(a, "contextid"),
        amount: field(a, "amount").max(1),
        ..Default::default()
    };
    if let Some(d) = info.get(&(app_id as u64, class_id, field(a, "instanceid"))) {
        let name = text(d, "name");
        item.name = if name.is_empty() { text(d, "market_name") } else { name };
        item.kind = text(d, "type");
        item.name_color = color(d, "name_color");
        item.background = color(d, "background_color");
        item.icon = text(d, "icon_url");
        item.icon_large = Some(text(d, "icon_url_large")).filter(|s| !s.is_empty());
        item.tradable = d.get("tradable").and_then(|b| b.as_bool().or_else(|| b.as_u64().map(|n| n == 1))).unwrap_or(false);
        item.marketable = d.get("marketable").and_then(|b| b.as_bool().or_else(|| b.as_u64().map(|n| n == 1))).unwrap_or(false);
        item.market_hash_name = text(d, "market_hash_name");
        item.description = d
            .get("descriptions")
            .and_then(|l| l.as_array())
            .into_iter()
            .flatten()
            .filter(|l| !text(l, "value").trim().is_empty())
            .map(|l| (text(l, "value"), color(l, "color")))
            .collect();
        item.tags = d
            .get("tags")
            .and_then(|l| l.as_array())
            .into_iter()
            .flatten()
            .map(|t| (text(t, "localized_category_name"), text(t, "localized_tag_name"), color(t, "color")))
            .collect();
    }
    TradeItem { app_id, item, missing: a.get("missing").and_then(|b| b.as_bool()).unwrap_or(false) }
}

/// The error Steam put in its answer, or a plain one.
fn steam_error(v: &Value, fallback: &str) -> String {
    let e = text(v, "strError");
    if e.is_empty() { fallback.to_string() } else { e.trim().to_string() }
}

/// Answer an offer. `Ok(true)` = confirm it in the Steam Mobile app.
pub async fn respond(web: &Web, id: u64, partner: SteamId, action: TradeAction) -> Result<bool, String> {
    let (path, mut form) = match action {
        TradeAction::Accept => (
            "accept",
            vec![("serverid", "1".to_string()), ("tradeofferid", id.to_string()), ("partner", partner.to_string()), ("captcha", String::new())],
        ),
        TradeAction::Decline => ("decline", vec![]),
        TradeAction::Cancel => ("cancel", vec![]),
    };
    form.insert(0, ("sessionid", web.session_id.clone()));
    let r = web
        .http
        .post(format!("{COMMUNITY}/tradeoffer/{id}/{path}"))
        .header("Referer", format!("{COMMUNITY}/tradeoffer/{id}/"))
        .form(&form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if v.get("needs_mobile_confirmation").and_then(|b| b.as_bool()) == Some(true) || v.get("needs_email_confirmation").and_then(|b| b.as_bool()) == Some(true) {
        return Ok(true);
    }
    if status.is_success() && v.get("strError").is_none() {
        return Ok(false);
    }
    Err(steam_error(&v, &format!("Steam answered {status}")))
}

/// The `json_tradeoffer` Steam's trade page sends.
pub fn offer_json(give: &[TradeAsset], get: &[TradeAsset]) -> Value {
    let assets = |l: &[TradeAsset]| -> Vec<Value> {
        l.iter()
            .map(|a| json!({ "appid": a.app_id, "contextid": a.context_id.to_string(), "amount": a.amount.max(1), "assetid": a.asset_id.to_string() }))
            .collect()
    };
    json!({
        "newversion": true,
        "version": give.len() + get.len() + 1,
        "me": { "assets": assets(give), "currency": [], "ready": false },
        "them": { "assets": assets(get), "currency": [], "ready": false },
    })
}

/// Send a new offer (or a counter offer). `token` is the partner's trade
/// link token, needed when they aren't your friend. Returns the new offer's
/// id and whether to confirm it in the Steam Mobile app.
pub async fn send(web: &Web, partner: SteamId, give: &[TradeAsset], get: &[TradeAsset], message: &str, counter: Option<u64>, token: Option<&str>) -> Result<(u64, bool), String> {
    if give.is_empty() && get.is_empty() {
        return Err("Add at least one item to the offer.".into());
    }
    let account = partner.saturating_sub(INDIVIDUAL);
    let params = match token.filter(|t| !t.is_empty()) {
        Some(t) => json!({ "trade_offer_access_token": t }),
        None => json!({}),
    };
    let mut form = vec![
        ("sessionid", web.session_id.clone()),
        ("serverid", "1".to_string()),
        ("partner", partner.to_string()),
        ("tradeoffermessage", message.to_string()),
        ("json_tradeoffer", offer_json(give, get).to_string()),
        ("captcha", String::new()),
        ("trade_offer_create_params", params.to_string()),
    ];
    if let Some(c) = counter {
        form.push(("tradeofferid_countered", c.to_string()));
    }
    let referer = match (counter, token) {
        (Some(c), _) => format!("{COMMUNITY}/tradeoffer/{c}/"),
        (None, Some(t)) => format!("{COMMUNITY}/tradeoffer/new/?partner={account}&token={t}"),
        (None, None) => format!("{COMMUNITY}/tradeoffer/new/?partner={account}"),
    };
    let r = web.http.post(format!("{COMMUNITY}/tradeoffer/new/send")).header("Referer", referer).form(&form).send().await.map_err(|e| e.to_string())?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    match v.get("tradeofferid").and_then(num) {
        Some(id) => {
            let confirm = v.get("needs_mobile_confirmation").and_then(|b| b.as_bool()) == Some(true) || v.get("needs_email_confirmation").and_then(|b| b.as_bool()) == Some(true);
            Ok((id, confirm))
        }
        None => Err(steam_error(&v, &format!("Steam answered {status}"))),
    }
}

/// The partner and token from a trade link
/// (`https://steamcommunity.com/tradeoffer/new/?partner=123&token=abc`).
pub fn parse_link(link: &str) -> Option<(SteamId, Option<String>)> {
    let q = link.split_once('?')?.1;
    let mut partner = None;
    let mut token = None;
    for kv in q.split('&') {
        match kv.split_once('=') {
            Some(("partner", v)) => partner = v.trim().parse::<u64>().ok(),
            Some(("token", v)) => token = Some(v.trim().to_string()).filter(|t| !t.is_empty()),
            _ => {}
        }
    }
    Some((INDIVIDUAL + partner.filter(|p| *p > 0 && *p < 1 << 32)?, token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_offers_with_their_items() {
        let v: Value = serde_json::from_str(
            r##"{"response":{
                "trade_offers_received":[{"tradeofferid":"7001","accountid_other":42,"message":"hi","expiration_time":1800000000,
                  "trade_offer_state":2,"items_to_receive":[{"appid":753,"contextid":"6","assetid":"55","classid":"9","instanceid":"0","amount":"1","missing":false}],
                  "is_our_offer":false,"time_created":1700000000,"time_updated":1700000001,"escrow_end_date":0}],
                "trade_offers_sent":[{"tradeofferid":"7002","accountid_other":43,"message":"","trade_offer_state":9,"is_our_offer":true,
                  "items_to_give":[{"appid":730,"contextid":"2","assetid":"66","classid":"10","instanceid":"3","amount":"1"}],"time_created":1700000100}],
                "descriptions":[{"appid":753,"classid":"9","instanceid":"0","name":"Card","type":"Trading Card","icon_url":"abc","tradable":1,"name_color":"#CF6A32",
                  "tags":[{"localized_category_name":"Rarity","localized_tag_name":"Rare","color":"4b69ff"}]},
                  {"appid":730,"classid":"10","instanceid":"3","name":"Case","icon_url":"def","tradable":true}]
            }}"##,
        )
        .unwrap();
        let o = parse(&v);
        assert_eq!(o.received.len(), 1);
        let r = &o.received[0];
        assert_eq!((r.id, r.partner, r.state, r.ours), (7001, INDIVIDUAL + 42, TradeState::Active, false));
        assert_eq!(r.get[0].item.name, "Card");
        assert_eq!(r.get[0].item.name_color.as_deref(), Some("CF6A32"));
        assert!(r.get[0].item.tradable);
        assert_eq!(r.get[0].item.tags[0].1, "Rare");
        assert_eq!((r.get[0].app_id, r.get[0].item.context_id, r.get[0].item.asset_id), (753, 6, 55));
        let s = &o.sent[0];
        assert_eq!((s.state, s.ours, s.give[0].item.name.as_str()), (TradeState::NeedsConfirmation, true, "Case"));
    }

    #[test]
    fn builds_the_offer_steam_expects() {
        let a = TradeAsset { app_id: 730, context_id: 2, asset_id: 66, amount: 1 };
        let v = offer_json(&[a], &[]);
        assert_eq!(v["version"], 2);
        assert_eq!(v["me"]["assets"][0]["assetid"], "66");
        assert_eq!(v["me"]["assets"][0]["contextid"], "2");
        assert_eq!(v["them"]["assets"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn reads_trade_links() {
        assert_eq!(parse_link("https://steamcommunity.com/tradeoffer/new/?partner=42&token=AbC_1"), Some((INDIVIDUAL + 42, Some("AbC_1".into()))));
        assert_eq!(parse_link("https://steamcommunity.com/tradeoffer/new/?partner=42"), Some((INDIVIDUAL + 42, None)));
        assert_eq!(parse_link("https://steamcommunity.com/id/someone"), None);
    }
}

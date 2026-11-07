//! Signed-in steamcommunity.com session and the Community Market.
//!
//! The web session is made the same way the official client makes it: a
//! short-lived access token generated from your saved sign-in, sent as the
//! `steamLoginSecure` cookie. It lives in memory only. Listing an item still
//! needs your confirmation in the Steam Mobile app, exactly like on Steam.

use model::{AppId, SteamId};
use net::Connection;
use proto::*;
use serde_json::Value;

use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Web {
    pub http: reqwest::Client,
    pub me: SteamId,
    pub session_id: String,
}

impl Web {
    /// A signed-out session, for public pages.
    pub fn anonymous() -> Web {
        Web { http: reqwest::Client::new(), me: 0, session_id: String::new() }
    }
}

pub type WebCache = Arc<tokio::sync::Mutex<Option<(Arc<Web>, Instant)>>>;

const COMMUNITY: &str = "https://steamcommunity.com";

/// The signed-in web session, made on first use and renewed every 20 minutes.
pub async fn web(conn: &Connection, me: SteamId, refresh: &str, cache: &WebCache) -> Result<Arc<Web>, String> {
    let mut slot = cache.lock().await;
    if let Some((w, at)) = slot.as_ref() {
        if at.elapsed() < Duration::from_secs(20 * 60) {
            return Ok(w.clone());
        }
    }
    let req = CAuthenticationAccessTokenGenerateForAppRequest { refresh_token: Some(refresh.to_string()), steamid: Some(me), ..Default::default() };
    let r: CAuthenticationAccessTokenGenerateForAppResponse =
        conn.call("Authentication.GenerateAccessTokenForApp#1", &req).await.map_err(|e| e.to_string())?;
    if r.access_token().is_empty() {
        return Err("Steam didn't give a web sign-in".into());
    }
    let session_id: String = (0..12).map(|_| format!("{:02x}", rand_byte())).collect();
    let jar = Arc::new(reqwest::cookie::Jar::default());
    for site in ["https://steamcommunity.com", "https://store.steampowered.com", "https://help.steampowered.com"] {
        let url: reqwest::Url = site.parse().unwrap();
        let host = url.host_str().unwrap_or_default().to_string();
        jar.add_cookie_str(&format!("steamLoginSecure={me}%7C%7C{}; Domain={host}; Path=/; Secure; HttpOnly", r.access_token()), &url);
        jar.add_cookie_str(&format!("sessionid={session_id}; Domain={host}; Path=/; Secure"), &url);
    }
    let http = reqwest::Client::builder()
        .cookie_provider(jar)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let w = Arc::new(Web { http, me, session_id });
    *slot = Some((w.clone(), Instant::now()));
    Ok(w)
}

fn rand_byte() -> u8 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish() as u8
}

/// Every game with items in an account's inventory, read from the
/// `g_rgAppContextData` list that Steam's inventory page is built from.
pub async fn inventory_apps(web: Option<&Web>, steam_id: SteamId) -> (Result<Vec<model::InventoryApp>, String>, Option<model::fees::Rules>) {
    let url = format!("{COMMUNITY}/profiles/{steam_id}/inventory/");
    let resp = match web {
        Some(w) => w.http.get(&url).send().await,
        None => reqwest::Client::new().get(&url).send().await,
    };
    let body = match resp {
        Ok(r) => r.text().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match body {
        Ok(b) => (parse_app_contexts(&b), wallet_rules(&b)),
        Err(e) => (Err(e), None),
    }
}

/// Fee rules for the signed-in wallet, from a page's `g_rgWalletInfo`.
pub fn wallet_rules(body: &str) -> Option<model::fees::Rules> {
    let at = body.find("g_rgWalletInfo")?;
    let rest = &body[at..];
    let start = rest.find('{')?;
    let v: Value = serde_json::Deserializer::from_str(&rest[start..]).into_iter::<Value>().next()?.ok()?;
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str()?.parse().ok()));
    let d = model::fees::Rules::default();
    Some(model::fees::Rules {
        minimum: f("wallet_fee_minimum").map(|x| x as u64).unwrap_or(d.minimum).max(1),
        percent: f("wallet_fee_percent").unwrap_or(d.percent),
        base: f("wallet_fee_base").map(|x| x as u64).unwrap_or(d.base),
        game_default: f("wallet_publisher_fee_percent_default").unwrap_or(d.game_default),
    })
}

pub fn parse_app_contexts(body: &str) -> Result<Vec<model::InventoryApp>, String> {
    let Some(at) = body.find("g_rgAppContextData") else {
        return Err(if body.contains("profile_private_info") || body.contains("private_profile") { "private".into() } else { "unavailable".into() });
    };
    let rest = &body[at..];
    let start = rest.find('{').ok_or("unavailable")?;
    let value = serde_json::Deserializer::from_str(&rest[start..]).into_iter::<Value>().next().ok_or("unavailable")?.map_err(|e| e.to_string())?;
    let mut apps: Vec<model::InventoryApp> = value
        .as_object()
        .map(|o| o.values().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| {
            let app_id = num(a.get("appid")?)? as AppId;
            let mut contexts: Vec<(u64, String, u32)> = a
                .get("rgContexts")
                .and_then(|c| c.as_object())
                .map(|c| {
                    c.values()
                        .filter_map(|x| {
                            let id = num(x.get("id")?)?;
                            Some((id, str_of(x, "name"), x.get("asset_count").and_then(num).unwrap_or(0) as u32))
                        })
                        .collect()
                })
                .unwrap_or_default();
            contexts.sort_by_key(|c| c.0);
            Some(model::InventoryApp { app_id, name: str_of(a, "name"), icon: Some(str_of(a, "icon")).filter(|s| !s.is_empty()), contexts })
        })
        .collect();
    let count = |a: &model::InventoryApp| a.contexts.iter().map(|c| c.2 as u64).sum::<u64>();
    apps.sort_by(|a, b| count(b).cmp(&count(a)).then_with(|| a.name.cmp(&b.name)));
    Ok(apps)
}

fn num(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.replace(',', "").parse().ok())
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(|n| n.as_str()).unwrap_or_default().to_string()
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Market listing page of an item.
pub fn listing_url(app: AppId, hash_name: &str) -> String {
    format!("{COMMUNITY}/market/listings/{app}/{}", encode(hash_name))
}

/// What an item's market page tells us: order book, price history, fee rules.
#[derive(Default)]
struct Page {
    name_id: Option<u64>,
    rules: Option<model::fees::Rules>,
    book: Option<Value>,
    history: Option<Value>,
}

async fn listing_page(http: &reqwest::Client, app: AppId, hash_name: &str) -> Page {
    let Ok(r) = http.get(listing_url(app, hash_name)).send().await else { return Page::default() };
    if !r.status().is_success() {
        return Page::default();
    }
    let body = r.text().await.unwrap_or_default();
    Page {
        // Older market pages load the order book with a separate request.
        name_id: body.find("Market_LoadOrderSpread(").and_then(|at| {
            let rest = &body[at + "Market_LoadOrderSpread(".len()..];
            rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
        }),
        rules: wallet_rules(&body),
        // Current market pages embed their data as an escaped JSON string.
        book: embedded(&body, r#"{\"amtMaxBuyOrder\""#),
        history: embedded(&body, r#"{\"ecurrency\":"#).filter(|v| v.get("prices").is_some()),
    }
}

/// Parse the JSON object starting at `marker` inside a JS string literal.
fn embedded(body: &str, marker: &str) -> Option<Value> {
    let at = body.find(marker)?;
    let mut text = String::new();
    let mut chars = body[at..].chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                'n' => text.push('\n'),
                't' => text.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    text.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?).unwrap_or('?'));
                }
                other => text.push(other),
            }
        } else if c == '"' {
            break; // end of the JS string
        } else {
            text.push(c);
        }
    }
    serde_json::Deserializer::from_str(&text).into_iter::<Value>().next()?.ok()
}

/// [price, count, price, count, ...] -> pairs, best first as Steam sends them.
fn ladder(v: Option<&Value>) -> Vec<(u64, u64)> {
    let nums: Vec<u64> = v.and_then(|x| x.as_array()).map(|a| a.iter().filter_map(num).collect()).unwrap_or_default();
    nums.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect()
}

pub async fn price(web: Option<&Web>, app: AppId, hash_name: &str, currency: i32, country: &str, full: bool) -> Result<model::MarketPrice, String> {
    let fallback = reqwest::Client::new();
    let http = web.map(|w| &w.http).unwrap_or(&fallback);
    let currency = if currency > 0 { currency } else { 1 };
    let mut out = model::MarketPrice { currency, ..Default::default() };
    let r = http
        .get(format!("{COMMUNITY}/market/priceoverview/"))
        .query(&[("appid", app.to_string()), ("currency", currency.to_string()), ("market_hash_name", hash_name.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().as_u16() == 429 {
        return Err("Steam is limiting price requests. Try again in a minute.".into());
    }
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if v.get("success").and_then(|s| s.as_bool()) == Some(true) {
        out.lowest = v.get("lowest_price").and_then(|s| s.as_str()).map(String::from);
        out.median = v.get("median_price").and_then(|s| s.as_str()).map(String::from);
        out.volume = v.get("volume").and_then(num);
    }
    if !full {
        return Ok(out);
    }
    let page = listing_page(http, app, hash_name).await;
    out.fee_rules = page.rules;
    if let Some(book) = &page.book {
        let book_currency = book.get("eCurrency").and_then(num).map(|c| c as i32);
        // Only use the embedded book if it's in your wallet currency.
        if book_currency.is_none_or(|c| c == currency) {
            out.highest_buy = book.get("amtMaxBuyOrder").and_then(num);
            out.lowest_sell = book.get("amtMinSellOrder").and_then(num);
            out.buy_count = book.get("cBuyOrders").and_then(num);
            out.sell_count = book.get("cSellOrders").and_then(num);
            out.buy_ladder = ladder(book.get("rgCompactBuyOrders")).into_iter().take(8).collect();
            out.sell_ladder = ladder(book.get("rgCompactSellOrders")).into_iter().take(8).collect();
        }
    }
    if let Some(h) = &page.history {
        let same = h.get("ecurrency").and_then(num).map(|c| c as i32).is_none_or(|c| c == currency);
        if same {
            out.history = h
                .get("prices")
                .and_then(|p| p.as_array())
                .map(|p| {
                    p.iter()
                        .filter_map(|x| Some((num(x.get("time")?)?, x.get("price_median")?.as_f64()?, x.get("purchases").and_then(num).unwrap_or(0) as u32)))
                        .collect()
                })
                .unwrap_or_default();
        }
    }
    if out.lowest_sell.is_none() {
        if let Some(id) = page.name_id {
            let h: Value = http
                .get(format!("{COMMUNITY}/market/itemordershistogram"))
                .query(&[
                    ("country", country.to_uppercase()),
                    ("language", "english".into()),
                    ("currency", currency.to_string()),
                    ("item_nameid", id.to_string()),
                    ("two_factor", "0".into()),
                ])
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .unwrap_or(Value::Null);
            out.highest_buy = h.get("highest_buy_order").and_then(num);
            out.lowest_sell = h.get("lowest_sell_order").and_then(num);
            out.buy_count = h.get("buy_order_count").and_then(num);
            out.sell_count = h.get("sell_order_count").and_then(num);
        }
    }
    if out.lowest.is_none() && out.lowest_sell.is_none() && out.volume.is_none() {
        return Err("Nobody is selling this item right now".into());
    }
    Ok(out)
}

/// List an item. Returns whether Steam wants a mobile/email confirmation.
pub async fn sell(web: &Web, app: AppId, context: u64, asset: u64, amount: u64, receive: u64) -> Result<bool, String> {
    let v: Value = web
        .http
        .post(format!("{COMMUNITY}/market/sellitem/"))
        .header("Referer", format!("{COMMUNITY}/profiles/{}/inventory/", web.me))
        .form(&[
            ("sessionid", web.session_id.clone()),
            ("appid", app.to_string()),
            ("contextid", context.to_string()),
            ("assetid", asset.to_string()),
            ("amount", amount.max(1).to_string()),
            ("price", receive.to_string()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if v.get("success").and_then(|s| s.as_bool()) == Some(true) {
        Ok(v.get("requires_confirmation").and_then(num).unwrap_or(0) == 1 || v.get("needs_mobile_confirmation").and_then(|b| b.as_bool()).unwrap_or(false))
    } else {
        Err(Some(str_of(&v, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| "Steam didn't accept the listing".into()))
    }
}

pub async fn cancel_listing(web: &Web, listing: u64) -> Result<(), String> {
    let r = web
        .http
        .post(format!("{COMMUNITY}/market/removelisting/{listing}"))
        .header("Referer", format!("{COMMUNITY}/market/"))
        .form(&[("sessionid", web.session_id.clone())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().is_success() { Ok(()) } else { Err(format!("Steam answered {}", r.status())) }
}

pub async fn cancel_buy_order(web: &Web, order: u64) -> Result<(), String> {
    let v: Value = web
        .http
        .post(format!("{COMMUNITY}/market/cancelbuyorder/"))
        .header("Referer", format!("{COMMUNITY}/market/"))
        .form(&[("sessionid", web.session_id.clone()), ("buy_orderid", order.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if num(v.get("success").unwrap_or(&Value::Null)) == Some(1) { Ok(()) } else { Err("Steam didn't cancel the order".into()) }
}

/// Your active listings, ones waiting for confirmation, and buy orders.
pub async fn my_listings(web: &Web) -> Result<model::MyListings, String> {
    let mut out = model::MyListings::default();
    let mut start = 0u64;
    loop {
        let v: Value = web
            .http
            .get(format!("{COMMUNITY}/market/mylistings"))
            .query(&[("norender", "1".to_string()), ("start", start.to_string()), ("count", "100".to_string())])
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|_| "Couldn't read your listings. Is the market available on your account?".to_string())?;
        if v.get("success").and_then(|s| s.as_bool()) != Some(true) {
            return Err("Steam didn't return your listings".into());
        }
        let page = |key: &str, status: model::ListingStatus| -> Vec<model::MyListing> {
            v.get(key).and_then(|l| l.as_array()).map(|l| l.iter().filter_map(|x| listing(x, status)).collect()).unwrap_or_default()
        };
        let active = page("listings", model::ListingStatus::Active);
        let n = active.len() as u64;
        out.listings.extend(active);
        if start == 0 {
            out.listings.extend(page("listings_to_confirm", model::ListingStatus::NeedsConfirmation));
            out.listings.extend(page("listings_on_hold", model::ListingStatus::OnHold));
            out.buy_orders = v.get("buy_orders").and_then(|l| l.as_array()).map(|l| l.iter().filter_map(buy_order).collect()).unwrap_or_default();
        }
        let total = v.get("total_count").and_then(num).unwrap_or(0);
        start += 100;
        if n < 100 || start >= total || start >= 2000 {
            break;
        }
    }
    Ok(out)
}

fn listing(x: &Value, status: model::ListingStatus) -> Option<model::MyListing> {
    let a = x.get("asset")?;
    if tracing::enabled!(tracing::Level::DEBUG) {
        let keys = ["price", "original_price", "fee", "steam_fee", "publisher_fee", "publisher_fee_percent", "converted_price", "converted_fee", "currencyid", "converted_currencyid", "original_amount_listed"];
        tracing::debug!("listing fields: {:?}", keys.iter().map(|k| (*k, x.get(*k))).collect::<Vec<_>>());
    }
    let receive = x.get("original_price").or_else(|| x.get("price")).and_then(num).unwrap_or(0);
    let fee = x.get("fee").and_then(num).unwrap_or(0);
    Some(model::MyListing {
        listing_id: num(x.get("listingid")?)?,
        app_id: a.get("appid").and_then(num).unwrap_or(0) as AppId,
        name: str_of(a, "name"),
        game: str_of(a, "type"),
        icon: str_of(a, "icon_url"),
        name_color: Some(str_of(a, "name_color")).filter(|c| !c.is_empty()),
        you_receive: receive,
        buyer_pays: receive + fee,
        currency: x.get("currencyid").and_then(num).map(|c| (c as i32) - 2000).unwrap_or(1),
        amount: a.get("amount").and_then(num).unwrap_or(1).max(1),
        created: x.get("time_created").and_then(num).unwrap_or(0),
        status,
        market_hash_name: str_of(a, "market_hash_name"),
    })
}

fn buy_order(x: &Value) -> Option<model::BuyOrder> {
    let d = x.get("description");
    Some(model::BuyOrder {
        order_id: num(x.get("buy_orderid")?)?,
        app_id: x.get("appid").and_then(num).unwrap_or(0) as AppId,
        name: d.map(|d| str_of(d, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| str_of(x, "hash_name")),
        icon: d.map(|d| str_of(d, "icon_url")).unwrap_or_default(),
        price: x.get("price").and_then(num).unwrap_or(0),
        currency: x.get("wallet_currency").and_then(num).unwrap_or(1) as i32,
        quantity: x.get("quantity").and_then(num).unwrap_or(0),
        remaining: x.get("quantity_remaining").and_then(num).unwrap_or(0),
        market_hash_name: str_of(x, "hash_name"),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_embedded_order_book() {
        let page = r#"<script>x = JSON.parse("{\"queries\":[{\"state\":{\"data\":{\"amtMaxBuyOrder\":null,\"amtMinSellOrder\":3,\"eCurrency\":1,\"cBuyOrders\":0,\"cSellOrders\":51830,\"rgCompactBuyOrders\":[],\"rgCompactSellOrders\":[3,11059,4,25755]},\"x\":1}]}");</script>"#;
        let book = super::embedded(page, r#"{\"amtMaxBuyOrder\""#).unwrap();
        assert_eq!(book["cSellOrders"], 51830);
        assert_eq!(super::ladder(book.get("rgCompactSellOrders")), vec![(3, 11059), (4, 25755)]);
    }

    #[test]
    fn reads_app_contexts() {
        let page = r#"<script>var g_rgAppContextData = {"753":{"appid":753,"name":"Steam","icon":"https://x/753.jpg","asset_count":12,"rgContexts":{"6":{"asset_count":12,"id":"6","name":"Community"}}},"730":{"appid":730,"name":"Counter-Strike 2","asset_count":40,"rgContexts":{"2":{"asset_count":40,"id":"2","name":"Backpack"}}}};
var g_strInventoryLoadURL = "x";</script>"#;
        let apps = super::parse_app_contexts(page).unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].app_id, 730);
        assert_eq!(apps[0].contexts, vec![(2, "Backpack".to_string(), 40)]);
        assert_eq!(apps[1].contexts[0].0, 6);
    }
}

/// Fetch a signed-in Steam web page (for features without an API).
pub async fn page(web: &Web, url: &str) -> Result<String, String> {
    let r = web.http.get(url).send().await.map_err(|e| e.to_string())?;
    let status = r.status();
    let body = r.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Steam answered {status}"));
    }
    Ok(body)
}

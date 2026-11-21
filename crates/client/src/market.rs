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
    /// The web sign-in's access token, for Steam's web API.
    pub access_token: String,
}

impl Web {
    /// A signed-out session, for public pages.
    pub fn anonymous() -> Web {
        Web { http: reqwest::Client::new(), me: 0, session_id: String::new(), access_token: String::new() }
    }
}

pub type WebCache = Arc<tokio::sync::Mutex<Option<(Arc<Web>, Instant)>>>;

const COMMUNITY: &str = "https://steamcommunity.com";
const STEAM_WEB_HOSTS: [&str; 3] = ["steamcommunity.com", "store.steampowered.com", "help.steampowered.com"];

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
    let session_id = new_session_id();
    let jar = build_cookie_jar(me, r.access_token(), &session_id, utc_offset());
    let http = build_web_http(jar)?;
    let w = Arc::new(Web { http, me, session_id, access_token: r.access_token().to_string() });
    *slot = Some((w.clone(), Instant::now()));
    Ok(w)
}

fn build_cookie_jar(me: SteamId, access_token: &str, session_id: &str, tz_offset: i64) -> Arc<reqwest::cookie::Jar> {
    let jar = Arc::new(reqwest::cookie::Jar::default());
    for host in STEAM_WEB_HOSTS {
        let url: reqwest::Url = format!("https://{host}").parse().unwrap();
        // Omit Domain= so cookies stay host-only and cannot be sent to subdomains.
        jar.add_cookie_str(&format!("steamLoginSecure={me}%7C%7C{access_token}; Path=/; Secure; HttpOnly"), &url);
        jar.add_cookie_str(&format!("sessionid={session_id}; Path=/; Secure"), &url);
        // Times on Steam's pages in your time zone, like a browser gets them (also host-only).
        jar.add_cookie_str(&format!("timezoneOffset={tz_offset},0; Path=/; Secure"), &url);
    }
    jar
}

fn build_web_http(jar: Arc<reqwest::cookie::Jar>) -> Result<reqwest::Client, String> {
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        if check_steam_url_parts(attempt.url(), &STEAM_WEB_HOSTS).is_ok() {
            attempt.follow()
        } else {
            attempt.error("redirect leaves allowed Steam origins")
        }
    });
    reqwest::Client::builder()
        .cookie_provider(jar)
        .redirect(redirect)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

fn decode_percent_segment(seg: &str) -> Result<String, String> {
    let bytes = seg.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|s| std::str::from_utf8(s).ok()).ok_or_else(|| "invalid percent-encoding in URL path".to_string())?;
            let b = u8::from_str_radix(hex, 16).map_err(|_| "invalid percent-encoding in URL path".to_string())?;
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    if out.iter().any(|&b| b == b'/' || b == b'\\' || b < 0x20 || b == 0x7f) {
        return Err("forbidden character in URL path".into());
    }
    String::from_utf8(out).map_err(|_| "non-UTF-8 URL path".into())
}

fn check_segment(seg: &str) -> Result<(), String> {
    if seg == "." || seg == ".." {
        return Err("path traversal in URL".into());
    }
    let decoded = decode_percent_segment(seg)?;
    if decoded == "." || decoded == ".." {
        return Err("encoded path traversal in URL".into());
    }
    Ok(())
}

fn check_steam_url_parts(url: &reqwest::Url, allowed_hosts: &[&str]) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err("Steam URL must use https".into());
    }
    let Some(host) = url.host_str() else {
        return Err("Steam URL missing host".into());
    };
    if !allowed_hosts.contains(&host) {
        return Err("not a Steam URL".into());
    }
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return Err("unexpected authority components in Steam URL".into());
    }
    if let Some(segments) = url.path_segments() {
        for seg in segments {
            check_segment(seg)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_steam_url_hosts(raw: &str, allowed_hosts: &[&str]) -> Result<reqwest::Url, String> {
    if raw.bytes().any(|b| b < 0x20 || b == 0x7f) || raw.contains('\\') {
        return Err("forbidden character in URL".into());
    }
    let url = reqwest::Url::parse(raw).map_err(|e| e.to_string())?;
    check_steam_url_parts(&url, allowed_hosts)?;
    // Check raw pre-normalization segments so literal `/../` is rejected before Url::parse collapses it.
    if let Some((_, after_scheme)) = raw.split_once("://") {
        let before_q = after_scheme.split(['?', '#']).next().unwrap_or("");
        if let Some((_, raw_path)) = before_q.split_once('/') {
            for seg in raw_path.split('/') {
                check_segment(seg)?;
            }
        }
    }
    Ok(url)
}

pub(crate) fn validate_steam_url(raw: &str, expected_host: &str) -> Result<reqwest::Url, String> {
    validate_steam_url_hosts(raw, &[expected_host])
}

fn new_session_id() -> String {
    auth::random_hex(12)
}

/// Your time zone's offset from UTC right now, in seconds (east positive).
pub fn utc_offset() -> i64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
        let (mut local, mut utc) = unsafe { (std::mem::zeroed::<SYSTEMTIME>(), std::mem::zeroed::<SYSTEMTIME>()) };
        unsafe {
            GetLocalTime(&mut local);
            GetSystemTime(&mut utc);
        }
        let secs = |t: &SYSTEMTIME| model::days_from_civil(t.wYear as i64, t.wMonth as i64, t.wDay as i64) * 86_400 + t.wHour as i64 * 3600 + t.wMinute as i64 * 60;
        // Whole quarter hours (the two clocks are read a moment apart).
        ((secs(&local) - secs(&utc)) as f64 / 900.0).round() as i64 * 900
    }
    #[cfg(not(windows))]
    {
        // struct tm up to tm_gmtoff (the same layout on macOS and Linux).
        #[repr(C)]
        struct Tm {
            fields: [i32; 9],
            gmtoff: std::ffi::c_long,
            zone: *const std::ffi::c_char,
        }
        unsafe extern "C" {
            fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let mut tm = Tm { fields: [0; 9], gmtoff: 0, zone: std::ptr::null() };
        if unsafe { localtime_r(&now, &mut tm) }.is_null() { 0 } else { tm.gmtoff as i64 }
    }
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

pub(crate) fn num(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.replace(',', "").parse().ok())
}

pub(crate) fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(|n| n.as_str()).unwrap_or_default().to_string()
}

pub(crate) fn encode(s: &str) -> String {
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
pub(crate) fn embedded(body: &str, marker: &str) -> Option<Value> {
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

/// A page of your Community Market history (newest first) and how many
/// rows there are in all.
pub async fn history(web: &Web, start: u32, language: &str) -> Result<(Vec<model::HistoryRow>, u32), String> {
    let v: Value = web
        .http
        .get(format!("{COMMUNITY}/market/myhistory/render/"))
        .query(&[("query", String::new()), ("start", start.to_string()), ("count", "50".to_string()), ("l", language.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|_| "Couldn't read your market history. Is the market available on your account?".to_string())?;
    if v.get("success").and_then(|s| s.as_bool()) != Some(true) {
        return Err("Steam didn't return your market history".into());
    }
    let html = v.get("results_html").and_then(|h| h.as_str()).unwrap_or_default();
    let total = v.get("total_count").and_then(num).unwrap_or(0) as u32;
    Ok((parse_history(html), total))
}

/// The rows of Steam's market history table.
pub fn parse_history(html: &str) -> Vec<model::HistoryRow> {
    fn between<'a>(s: &'a str, a: &str, b: &str) -> Option<&'a str> {
        let i = s.find(a)? + a.len();
        let j = s[i..].find(b)? + i;
        Some(&s[i..j])
    }
    /// The inside of the element whose opening tag contains `marker`.
    fn inner<'a>(s: &'a str, marker: &str) -> Option<&'a str> {
        let at = s.find(marker)?;
        let open = s[at..].find('>')? + at + 1;
        let tag_start = if s[at..].starts_with('<') { at } else { s[..at].rfind('<')? };
        let tag: String = s[tag_start + 1..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        // Count nested tags of the same name to find the matching close.
        let (open_tag, close_tag) = (format!("<{tag}"), format!("</{tag}>"));
        let mut depth = 1;
        let mut i = open;
        while depth > 0 {
            let next_close = s[i..].find(&close_tag)? + i;
            match s[i..].find(&open_tag).map(|o| o + i).filter(|o| *o < next_close) {
                Some(o) => {
                    depth += 1;
                    i = o + open_tag.len();
                }
                None => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&s[open..next_close]);
                    }
                    i = next_close + close_tag.len();
                }
            }
        }
        None
    }
    let text = |s: Option<&str>| s.map(|x| crate::hub::decode(&crate::hub::strip_tags(x)).split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
    html.split("<div class=\"market_listing_row market_recent_listing_row")
        .skip(1)
        .filter_map(|row| {
            let id = between(row, "id=\"", "\"")?.trim_start_matches("history_row_").to_string();
            let sign = text(inner(row, "market_listing_gainorloss"));
            let who = inner(row, "market_listing_whoactedwith").unwrap_or_default();
            // The name block: "Seller: <a>name</a>", or just "Listing canceled".
            let block = inner(who, "market_listing_whoactedwith_name_block").unwrap_or(who);
            let other = text(block.rfind("<a ").and_then(|i| inner(&block[i..], "<a ")));
            let note = if other.is_empty() { text(Some(block)) } else { String::new() };
            let lower = note.to_lowercase();
            let kind = match sign.as_str() {
                "+" => model::HistoryKind::Bought,
                "-" | "\u{2212}" => model::HistoryKind::Sold,
                _ if lower.contains("cancel") => model::HistoryKind::Canceled,
                _ if lower.contains("creat") || lower.contains("listed") => model::HistoryKind::Listed,
                _ => model::HistoryKind::Other,
            };
            let dates: Vec<String> = row.match_indices("market_listing_listed_date can_combine").filter_map(|(i, _)| inner(&row[row[..i].rfind('<').unwrap_or(0)..], "market_listing_listed_date can_combine")).map(|d| text(Some(d))).collect();
            let name_tag = row.find("class=\"market_listing_item_name\"").map(|i| &row[row[..i].rfind('<').unwrap_or(i)..]);
            let name_color = name_tag.and_then(|t| between(t, "color: #", ";")).map(|c| c.trim().to_string()).filter(|c| c.len() == 6 && c.bytes().all(|b| b.is_ascii_hexdigit()));
            let image = between(row, "_image\" src=\"", "\"").map(|s| s.trim().to_string()).unwrap_or_default();
            Some(model::HistoryRow {
                id,
                kind,
                name: text(inner(row, "class=\"market_listing_item_name\"")),
                name_color,
                game: text(inner(row, "class=\"market_listing_game_name\"")),
                image: if image.starts_with("https://") { image } else { String::new() },
                price: text(inner(row, "class=\"market_listing_price\"")),
                acted: dates.first().cloned().unwrap_or_default(),
                listed: dates.get(1).cloned().unwrap_or_default(),
                other,
                note,
            })
        })
        .collect()
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
    fn reads_market_history() {
        let html = r##"<div class="market_listing_table_header"></div>
<div class="market_listing_row market_recent_listing_row" id="history_row_1_2">
	<div class="market_listing_left_cell market_listing_gainorloss">
		+			</div>
	<img id="history_row_1_2_image" src="https://community.akamai.steamstatic.com/economy/image/abc/62fx62f" style="border-color: #D2D2D2;" class="market_listing_item_img" alt="" />
	<div class="market_listing_right_cell market_listing_their_price">
		<span class="market_table_value">
			<span class="market_listing_price">
				0,03&#8364;			</span>
			<br/>
		</span>
	</div>
	<div class="market_listing_right_cell market_listing_whoactedwith">
		<span class="market_listing_owner_avatar"><span class="playerAvatar offline"><a href="https://steamcommunity.com/profiles/1"><img src="x.jpg"></a></span></span>
		<div class="market_listing_whoactedwith_name_block">
			Seller:<br/>
			<a href="https://steamcommunity.com/profiles/1" class="market_whoactedwith_name">Some  Seller</a>
		</div>
	</div>
	<div class="market_listing_right_cell market_listing_listed_date can_combine">
		17 Nov	</div>
	<div class="market_listing_right_cell market_listing_listed_date can_combine">
		15 Nov	</div>
	<div class="market_listing_item_name_block">
		<span id="history_row_1_2_name" class="market_listing_item_name" style="color: #CF6A32;">Chroma Case</span>
		<br/>
		<span class="market_listing_game_name">Counter-Strike 2</span>
	</div>
	<div style="clear: both"></div>
</div>
<div class="market_listing_row market_recent_listing_row" id="history_row_3_4">
	<div class="market_listing_left_cell market_listing_gainorloss">
					</div>
	<img id="history_row_3_4_image" src="https://community.akamai.steamstatic.com/economy/image/def/62fx62f" class="market_listing_item_img" alt="" />
	<div class="market_listing_right_cell market_listing_their_price"><span class="market_table_value"><span class="market_listing_price">0,05&#8364;</span></span></div>
	<div class="market_listing_right_cell market_listing_whoactedwith">
		<div class="market_listing_whoactedwith_name_block">Listing canceled</div>
	</div>
	<div class="market_listing_right_cell market_listing_listed_date can_combine">12 Nov</div>
	<div class="market_listing_right_cell market_listing_listed_date can_combine">10 Nov</div>
	<div class="market_listing_item_name_block"><span class="market_listing_item_name">Card</span><br/><span class="market_listing_game_name">Dota 2 Trading Card</span></div>
</div>"##;
        let rows = super::parse_history(html);
        assert_eq!(rows.len(), 2);
        let r = &rows[0];
        assert_eq!(r.id, "1_2");
        assert_eq!(r.kind, model::HistoryKind::Bought);
        assert_eq!((r.name.as_str(), r.game.as_str(), r.price.as_str()), ("Chroma Case", "Counter-Strike 2", "0,03\u{20ac}"));
        assert_eq!(r.name_color.as_deref(), Some("CF6A32"));
        assert_eq!((r.acted.as_str(), r.listed.as_str()), ("17 Nov", "15 Nov"));
        assert_eq!(r.other, "Some Seller");
        assert!(r.image.ends_with("abc/62fx62f"));
        let r = &rows[1];
        assert_eq!(r.kind, model::HistoryKind::Canceled);
        assert_eq!(r.note, "Listing canceled");
        assert_eq!(r.other, "");
        assert_eq!((r.acted.as_str(), r.listed.as_str()), ("12 Nov", "10 Nov"));
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
    let url = validate_steam_url_hosts(url, &STEAM_WEB_HOSTS)?;
    let r = web.http.get(url).send().await.map_err(|e| e.to_string())?;
    let status = r.status();
    let body = r.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Steam answered {status}"));
    }
    Ok(body)
}

/// Search the Community Market (Steam's market search, most popular first).
/// `app` limits it to one game. Returns the results and how many there are.
pub async fn search(web: Option<&Web>, query: &str, app: Option<model::AppId>, start: u32) -> Result<(Vec<model::MarketResult>, u32), String> {
    // "cards:<appid>": a game's trading cards, filtered the way Steam's
    // Market page does it (matching names finds the wrong things).
    let cards = query.strip_prefix("cards:").and_then(|g| g.trim().parse::<model::AppId>().ok());
    let (query, app) = match cards {
        Some(_) => ("", Some(753)),
        None => (query, app),
    };
    let mut url = reqwest::Url::parse(&format!("{COMMUNITY}/market/search/render/")).map_err(|e| e.to_string())?;
    if let Some(game) = cards {
        url.query_pairs_mut().append_pair("category_753_Game[]", &format!("tag_app_{game}")).append_pair("category_753_item_class[]", "tag_item_class_2");
    }
    url.query_pairs_mut()
        .append_pair("query", query)
        .append_pair("start", &start.to_string())
        .append_pair("count", "30")
        .append_pair("search_descriptions", "0")
        .append_pair("sort_column", "popular")
        .append_pair("sort_dir", "desc")
        .append_pair("norender", "1");
    if let Some(app) = app {
        url.query_pairs_mut().append_pair("appid", &app.to_string());
    }
    let client = web.map(|w| w.http.clone()).unwrap_or_default();
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("The Market is busy. Try again in a minute.".into());
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    parse_search(&v).ok_or_else(|| "The Market is busy. Try again in a minute.".into())
}

/// Many prices at once, the way browser extensions get them: the market
/// search lists up to 100 items per request with their cheapest price. Steam
/// items (trading cards, backgrounds, emoticons) are searched per game, and
/// other games by their most-listed items. `found` gets each page's prices;
/// names it never sees are left for the one-at-a-time lookup.
pub async fn price_pages(web: Option<&Web>, app: AppId, names: &[String], currency: i32, mut found: impl FnMut(Vec<(String, model::MarketPrice)>)) -> Result<(), String> {
    use std::collections::{BTreeMap, HashSet};
    let mut left: HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
    // (extra query parameters, pages at most)
    let mut queries: Vec<(Vec<(String, String)>, u32)> = Vec::new();
    if app == 753 {
        let mut games: BTreeMap<&str, ()> = BTreeMap::new();
        for n in names {
            if let Some((g, _)) = n.split_once('-').filter(|(g, _)| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit())) {
                games.insert(g, ());
            }
        }
        for g in games.keys() {
            queries.push((vec![("category_753_Game[]".into(), format!("tag_app_{g}")), ("sort_column".into(), "name".into()), ("sort_dir".into(), "asc".into())], 4));
        }
    } else {
        queries.push((vec![("sort_column".into(), "quantity".into()), ("sort_dir".into(), "desc".into())], 10));
    }
    let client = web.map(|w| w.http.clone()).unwrap_or_default();
    let mut first = true;
    for (extra, pages) in queries {
        let mut start = 0u32;
        for _ in 0..pages {
            if left.is_empty() {
                return Ok(());
            }
            if !first {
                tokio::time::sleep(Duration::from_millis(1300)).await;
            }
            first = false;
            let mut url = reqwest::Url::parse(&format!("{COMMUNITY}/market/search/render/")).map_err(|e| e.to_string())?;
            url.query_pairs_mut()
                .append_pair("norender", "1")
                .append_pair("appid", &app.to_string())
                .append_pair("count", "100")
                .append_pair("start", &start.to_string())
                .append_pair("search_descriptions", "0");
            for (k, v) in &extra {
                url.query_pairs_mut().append_pair(k, v);
            }
            let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err("The Market is busy. Try again in a minute.".into());
            }
            let v: Value = resp.json().await.map_err(|e| e.to_string())?;
            let list = v.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
            let total = v.get("total_count").and_then(|n| n.as_u64()).unwrap_or(0) as u32;
            let page: Vec<(String, model::MarketPrice)> = list.iter().filter_map(|r| search_price(r, currency)).collect();
            for (n, _) in &page {
                left.remove(n.as_str());
            }
            if !page.is_empty() {
                found(page);
            }
            start += list.len() as u32;
            if list.is_empty() || start >= total {
                break;
            }
        }
    }
    Ok(())
}

/// One market search result as a price.
fn search_price(r: &Value, currency: i32) -> Option<(String, model::MarketPrice)> {
    let name = r.get("hash_name")?.as_str()?.to_string();
    let text = r.get("sell_price_text").and_then(|s| s.as_str()).unwrap_or_default().to_string();
    let listings = r.get("sell_listings").and_then(|n| n.as_u64()).unwrap_or(0);
    if listings == 0 || text.is_empty() {
        return None;
    }
    Some((name, model::MarketPrice { lowest_sell: model::parse_money(&text), lowest: Some(text), sell_count: Some(listings), currency, ..Default::default() }))
}

pub fn parse_search(v: &serde_json::Value) -> Option<(Vec<model::MarketResult>, u32)> {
    let total = v.get("total_count")?.as_u64()? as u32;
    let list = v
        .get("results")?
        .as_array()?
        .iter()
        .filter_map(|r| {
            let d = r.get("asset_description");
            let s = |o: Option<&serde_json::Value>, k: &str| o.and_then(|o| o.get(k)).and_then(|x| x.as_str()).unwrap_or_default().to_string();
            Some(model::MarketResult {
                app_id: d.and_then(|d| d.get("appid")).and_then(|a| a.as_u64()).unwrap_or(0) as model::AppId,
                app_name: s(Some(r), "app_name"),
                name: s(Some(r), "name"),
                hash_name: Some(s(Some(r), "hash_name")).filter(|h| !h.is_empty())?,
                icon: Some(s(d, "icon_url")).filter(|i| !i.is_empty()).map(|i| format!("https://community.fastly.steamstatic.com/economy/image/{i}/120x120")).unwrap_or_default(),
                color: s(d, "name_color"),
                listings: r.get("sell_listings").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
                price: s(Some(r), "sell_price_text"),
            })
        })
        .collect();
    Some((list, total))
}

#[cfg(test)]
mod search_tests {
    #[test]
    fn reads_market_search() {
        let v: serde_json::Value = serde_json::from_str(r#"{"success":true,"start":0,"pagesize":30,"total_count":1234,"results":[
            {"name":"Revolution Case","hash_name":"Revolution Case","sell_listings":98765,"sell_price":45,"sell_price_text":"$0.45","app_name":"Counter-Strike 2",
             "asset_description":{"appid":730,"icon_url":"abc","name_color":"D2D2D2","type":"Base Grade Container"},"sale_price_text":"$0.43"}]}"#).unwrap();
        let (list, total) = super::parse_search(&v).unwrap();
        assert_eq!(total, 1234);
        assert_eq!((list[0].app_id, list[0].hash_name.as_str(), list[0].listings, list[0].price.as_str()), (730, "Revolution Case", 98765, "$0.45"));
        assert!(list[0].icon.ends_with("/abc/120x120"));
        assert!(super::parse_search(&serde_json::Value::Null).is_none());
    }
}

#[cfg(test)]
mod security_tests {
    #[tokio::test]
    async fn cookies_are_host_only_and_redirects_stay_on_steam() {
        use reqwest::cookie::CookieStore;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let jar = super::build_cookie_jar(76561198000000001, "tok", "deadbeef", 3600);
        for host in ["steamcommunity.com", "store.steampowered.com", "help.steampowered.com"] {
            let root: reqwest::Url = format!("https://{host}/market/").parse().unwrap();
            let sub: reqwest::Url = format!("https://evil.{host}/market/").parse().unwrap();
            let cookies = jar.cookies(&root).expect("expected cookies on root host");
            let s = cookies.to_str().unwrap();
            assert!(s.contains("steamLoginSecure="));
            assert!(s.contains("sessionid="));
            assert!(s.contains("timezoneOffset="));
            assert!(jar.cookies(&sub).is_none(), "cookie must be host-only, leaked to {sub}");
        }

        for bad in [
            "https://steamcommunity.com.evil.tld/discussions/",
            "https://user@steamcommunity.com/discussions/",
            "http://steamcommunity.com/discussions/",
            "https://steamcommunity.com/app/440/../secret",
            "https://steamcommunity.com/app/440/%2e%2e/secret",
            "https://steamcommunity.com/app/440/%2E./secret",
        ] {
            assert!(super::validate_steam_url(bad, "steamcommunity.com").is_err(), "must reject {bad}");
        }
        assert!(super::validate_steam_url("https://steamcommunity.com/app/440/discussions/", "steamcommunity.com").is_ok());

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf);
            let _ = sock.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: https://steamcommunity.com.evil.tld/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let http = super::build_web_http(jar).unwrap();
        let err = http.get(format!("http://{addr}/start")).send().await.unwrap_err();
        assert!(err.is_redirect(), "expected redirect policy error, got {err}");
        srv.join().unwrap();
    }
}

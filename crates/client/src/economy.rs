//! Spending on Steam: buying from the Community Market, buy orders, gems,
//! booster packs, and answering pending gifts.
//!
//! Every request here matches what Steam's own web pages send. Anything that
//! spends money or gems, or destroys an item, runs only from a [`Request`]
//! the app sends after you've confirmed the exact amounts. Nothing retries on
//! its own, except repeating a purchase's request while Steam waits for your
//! approval in the Steam Mobile app, which is how Steam's own page does it.

use crate::Event;
use crate::market::{Web, embedded, listing_url, num, str_of};
use model::economy::*;
use model::{AppId, SteamId};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const COMMUNITY: &str = "https://steamcommunity.com";
/// How long to wait for an approval in the Steam Mobile app.
const PHONE_WAIT: Duration = Duration::from_secs(120);

/// Said when a request that spends something got no clear answer: it may
/// have gone through, so it must not simply be tried again.
pub const UNKNOWN_RESULT: &str = "Steam didn't answer, so this may or may not have gone through. Check your Market history and wallet before trying again.";

#[derive(Clone)]
pub enum Request {
    LoadPurchaseInfo,
    /// The cheapest listing of an item someone else is selling.
    LoadListing { app_id: AppId, hash_name: String, name: String },
    BuyListing { listing: MarketListing, info: PurchaseInfo },
    PlaceBuyOrder { app_id: AppId, hash_name: String, price: u64, quantity: u64, info: PurchaseInfo },
    /// How many gems an item is worth. `game` comes from the item's action.
    LoadGemValue { game: AppId, context_id: u64, asset_id: u64 },
    /// Destroy an item for gems. `expected` is the amount you were shown.
    Grind { game: AppId, context_id: u64, asset_id: u64, expected: u64 },
    /// Gems to sacks (1 -> 1000) or sacks to gems (1000 -> 1).
    ExchangeGems { game: AppId, asset_id: u64, from_size: u64, to_size: u64, amount_in: u64 },
    LoadBoosters,
    MakeBooster { app_id: AppId, series: u32, preference: u8 },
    OpenBooster { game: AppId, asset_id: u64 },
    LoadGifts,
    /// `to_library`: add it to your library now; otherwise keep it in your inventory.
    AcceptGift { id: u64, to_library: bool },
    DeclineGift { id: u64, sender: SteamId, note: String },
}

// Written by hand so a billing address can never reach a log.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Request::LoadPurchaseInfo => "LoadPurchaseInfo",
            Request::LoadListing { .. } => "LoadListing",
            Request::BuyListing { .. } => "BuyListing",
            Request::PlaceBuyOrder { .. } => "PlaceBuyOrder",
            Request::LoadGemValue { .. } => "LoadGemValue",
            Request::Grind { .. } => "Grind",
            Request::ExchangeGems { .. } => "ExchangeGems",
            Request::LoadBoosters => "LoadBoosters",
            Request::MakeBooster { .. } => "MakeBooster",
            Request::OpenBooster { .. } => "OpenBooster",
            Request::LoadGifts => "LoadGifts",
            Request::AcceptGift { .. } => "AcceptGift",
            Request::DeclineGift { .. } => "DeclineGift",
        })
    }
}

#[derive(Debug, Clone)]
pub enum Reply {
    PurchaseInfo(Result<PurchaseInfo, String>),
    Listing { app_id: AppId, hash_name: String, result: Result<MarketListing, String> },
    /// Steam is waiting for you to approve this in the Steam Mobile app.
    WaitingForPhone,
    /// Bought. Holds your new wallet balance when Steam sent it.
    Bought { name: String, result: Result<Option<(i64, i32)>, String> },
    /// `Ok(true)`: an item was already bought at that price.
    OrderPlaced(Result<bool, String>),
    GemValue { asset_id: u64, result: Result<u64, String> },
    Ground { asset_id: u64, result: Result<u64, String> },
    Exchanged { from_size: u64, amount_in: u64, amount_out: u64, result: Result<(), String> },
    Boosters(Result<BoosterCatalog, String>),
    BoosterMade { app_id: AppId, result: Result<(u64, u64, u64), String> },
    BoosterOpened { asset_id: u64, result: Result<Vec<BoosterCard>, String> },
    Gifts(Result<Vec<PendingGift>, String>),
    GiftAnswered { id: u64, accepted: bool, to_library: bool, result: Result<(), String> },
}

impl Reply {
    /// Your wallet after this, if it changed.
    pub fn new_wallet(&self) -> Option<(i64, i32)> {
        match self {
            Reply::Bought { result: Ok(w), .. } => *w,
            _ => None,
        }
    }
}

/// What's been loaded for these features.
#[derive(Debug, Clone, Default)]
pub struct Book {
    pub purchase_info: Option<Result<PurchaseInfo, String>>,
    pub listings: HashMap<(AppId, String), Result<MarketListing, String>>,
    pub gem_values: HashMap<u64, Result<u64, String>>,
    pub boosters: Option<Result<BoosterCatalog, String>>,
    pub gifts: Option<Result<Vec<PendingGift>, String>>,
}

impl Book {
    pub fn apply(&mut self, r: &Reply) {
        match r {
            Reply::PurchaseInfo(i) => self.purchase_info = Some(i.clone()),
            Reply::Listing { app_id, hash_name, result } => {
                self.listings.insert((*app_id, hash_name.clone()), result.clone());
            }
            // A listing can only be bought once, and the price may have moved.
            Reply::Bought { result: Ok(_), .. } => self.listings.clear(),
            Reply::GemValue { asset_id, result } => {
                self.gem_values.insert(*asset_id, result.clone());
            }
            Reply::Ground { asset_id, result: Ok(_) } => {
                self.gem_values.remove(asset_id);
            }
            Reply::Boosters(b) => self.boosters = Some(b.clone()),
            Reply::BoosterMade { app_id, result: Ok((gems, tradable, untradable)) } => {
                if let Some(Ok(c)) = &mut self.boosters {
                    (c.gems, c.tradable_gems, c.untradable_gems) = (*gems, *tradable, *untradable);
                    if let Some(g) = c.games.iter_mut().find(|g| g.app_id == *app_id) {
                        g.available = false;
                    }
                }
            }
            Reply::Gifts(g) => self.gifts = Some(g.clone()),
            Reply::GiftAnswered { id, result: Ok(()), .. } => {
                if let Some(Ok(g)) = &mut self.gifts {
                    g.retain(|x| x.id != *id);
                }
            }
            _ => {}
        }
    }
}

/// Run one request with the signed-in web session.
pub async fn run(web: Result<std::sync::Arc<Web>, String>, request: Request, sink: crate::Sink) {
    let send = |r: Reply| sink(Event::Economy(r));
    let web = match web {
        Ok(w) => w,
        Err(e) => return send(failed(&request, e)),
    };
    let w = &*web;
    let reply = match request {
        Request::LoadPurchaseInfo => Reply::PurchaseInfo(purchase_info(w).await),
        Request::LoadListing { app_id, hash_name, name } => {
            let result = cheapest_listing(w, app_id, &hash_name, &name).await;
            Reply::Listing { app_id, hash_name, result }
        }
        Request::BuyListing { listing, info } => {
            let name = listing.name.clone();
            Reply::Bought { name, result: buy_listing(w, &listing, &info, &send).await }
        }
        Request::PlaceBuyOrder { app_id, hash_name, price, quantity, info } => Reply::OrderPlaced(place_order(w, app_id, &hash_name, price, quantity, &info, &send).await),
        Request::LoadGemValue { game, context_id, asset_id } => Reply::GemValue { asset_id, result: gem_value(w, game, context_id, asset_id).await },
        Request::Grind { game, context_id, asset_id, expected } => Reply::Ground { asset_id, result: grind(w, game, context_id, asset_id, expected).await },
        Request::ExchangeGems { game, asset_id, from_size, to_size, amount_in } => {
            let amount_out = exchange_out(amount_in, from_size, to_size).unwrap_or(0);
            Reply::Exchanged { from_size, amount_in, amount_out, result: exchange(w, game, asset_id, from_size, to_size, amount_in).await }
        }
        Request::LoadBoosters => Reply::Boosters(boosters(w).await),
        Request::MakeBooster { app_id, series, preference } => Reply::BoosterMade { app_id, result: make_booster(w, app_id, series, preference).await },
        Request::OpenBooster { game, asset_id } => Reply::BoosterOpened { asset_id, result: open_booster(w, game, asset_id).await },
        Request::LoadGifts => Reply::Gifts(gifts(w).await),
        Request::AcceptGift { id, to_library } => Reply::GiftAnswered { id, accepted: true, to_library, result: accept_gift(w, id, to_library).await },
        Request::DeclineGift { id, sender, note } => Reply::GiftAnswered { id, accepted: false, to_library: false, result: decline_gift(w, id, sender, &note).await },
    };
    send(reply);
}

/// The reply for a request that couldn't start (no web session).
fn failed(request: &Request, e: String) -> Reply {
    match request {
        Request::LoadPurchaseInfo => Reply::PurchaseInfo(Err(e)),
        Request::LoadListing { app_id, hash_name, .. } => Reply::Listing { app_id: *app_id, hash_name: hash_name.clone(), result: Err(e) },
        Request::BuyListing { listing, .. } => Reply::Bought { name: listing.name.clone(), result: Err(e) },
        Request::PlaceBuyOrder { .. } => Reply::OrderPlaced(Err(e)),
        Request::LoadGemValue { asset_id, .. } => Reply::GemValue { asset_id: *asset_id, result: Err(e) },
        Request::Grind { asset_id, .. } => Reply::Ground { asset_id: *asset_id, result: Err(e) },
        Request::ExchangeGems { from_size, to_size, amount_in, .. } => Reply::Exchanged { from_size: *from_size, amount_in: *amount_in, amount_out: exchange_out(*amount_in, *from_size, *to_size).unwrap_or(0), result: Err(e) },
        Request::LoadBoosters => Reply::Boosters(Err(e)),
        Request::MakeBooster { app_id, .. } => Reply::BoosterMade { app_id: *app_id, result: Err(e) },
        Request::OpenBooster { asset_id, .. } => Reply::BoosterOpened { asset_id: *asset_id, result: Err(e) },
        Request::LoadGifts => Reply::Gifts(Err(e)),
        Request::AcceptGift { id, to_library } => Reply::GiftAnswered { id: *id, accepted: true, to_library: *to_library, result: Err(e) },
        Request::DeclineGift { id, .. } => Reply::GiftAnswered { id: *id, accepted: false, to_library: false, result: Err(e) },
    }
}

type Form = Vec<(&'static str, String)>;

/// POST a form. Gives whether the status was OK, and the JSON answer (Steam
/// puts error details in the body of failed requests too).
async fn post(web: &Web, url: &str, referer: &str, form: &Form) -> Result<(bool, Value), String> {
    let r = web.http.post(url).header("Referer", referer).form(form).send().await.map_err(|e| e.to_string())?;
    let ok = r.status().is_success();
    let text = r.text().await.map_err(|e| e.to_string())?;
    Ok((ok, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn message(v: &Value) -> Option<String> {
    ["message", "error", "strError"].iter().map(|k| str_of(v, k)).find(|m| !m.trim().is_empty()).map(|m| crate::hub::decode(&crate::hub::strip_tags(&m)).trim().to_string())
}

fn profile(web: &Web) -> String {
    format!("{COMMUNITY}/profiles/{}", web.me)
}

// ---- Buying ----

pub async fn purchase_info(web: &Web) -> Result<PurchaseInfo, String> {
    let v: Value = web.http.get(format!("{COMMUNITY}/market/userbillinginfo")).send().await.map_err(|e| e.to_string())?.json().await.map_err(|_| NO_WALLET.to_string())?;
    parse_purchase_info(&v)
}

const NO_WALLET: &str = "Steam didn't share your wallet details. Try again, or buy on Steam's website.";

pub fn parse_purchase_info(v: &Value) -> Result<PurchaseInfo, String> {
    let w = v.get("wallet_info").filter(|w| w.is_object()).ok_or(NO_WALLET)?;
    let wallet_currency = w.get("wallet_currency").and_then(num).filter(|c| *c > 0).ok_or(NO_WALLET)? as i32;
    let tax = v.get("tax_rate");
    let adds_tax = tax.and_then(|t| t.get("tradefee_addtax")).is_some_and(|b| b.as_bool().unwrap_or(false) || b.as_u64() == Some(1));
    let rate = tax.and_then(|t| t.get("tradefee_taxrate")).and_then(|r| r.as_f64().or_else(|| r.as_str()?.parse().ok())).unwrap_or(0.0);
    let a = v.get("billing_address");
    let field = |k: &str| a.map(|a| str_of(a, k)).unwrap_or_default();
    Ok(PurchaseInfo {
        wallet_balance: w.get("wallet_balance").and_then(num).unwrap_or(0) as i64,
        wallet_currency,
        market_minimum: w.get("wallet_market_minimum").and_then(num).unwrap_or(1).max(1),
        requires_billing: v.get("require_billing_info").is_some_and(|b| b.as_bool().unwrap_or(false) || b.as_u64() == Some(1)),
        tax_percent: if adds_tax { rate.max(0.0) } else { 0.0 },
        country: str_of(v, "country_code"),
        billing: BillingAddress {
            first_name: field("firstname"),
            last_name: field("lastname"),
            address: field("address1"),
            address_two: field("address2"),
            city: field("city"),
            state: field("state"),
            postal_code: field("postcode"),
        },
    })
}

async fn cheapest_listing(web: &Web, app: AppId, hash_name: &str, name: &str) -> Result<MarketListing, String> {
    let r = web.http.get(listing_url(app, hash_name)).send().await.map_err(|e| e.to_string())?;
    if r.status().as_u16() == 429 {
        return Err("The Market is busy. Try again in a minute.".into());
    }
    let body = r.text().await.map_err(|e| e.to_string())?;
    parse_cheapest_listing(&body, app, hash_name, name).ok_or_else(|| "Nobody is selling this item right now".into())
}

/// The cheapest listing on an item's market page that isn't your own.
pub fn parse_cheapest_listing(body: &str, app: AppId, hash_name: &str, name: &str) -> Option<MarketListing> {
    let list = embedded(body, r#"[{\"listingid\""#)?;
    list.as_array()?
        .iter()
        .filter(|l| l.get("bMine").and_then(|b| b.as_bool()) != Some(true))
        .filter_map(|l| {
            let description = l.get("description");
            Some(MarketListing {
                listing_id: num(l.get("listingid")?)?,
                app_id: app,
                hash_name: hash_name.to_string(),
                name: description.map(|d| str_of(d, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| name.to_string()),
                icon: description.map(|d| str_of(d, "icon_url")).unwrap_or_default(),
                currency: l.get("eCurrency").and_then(num)? as i32,
                price: l.get("unPricePerUnit").and_then(num).filter(|p| *p > 0)?,
                fee: l.get("unFeePerUnit").and_then(num).unwrap_or(0),
                steam_fee: l.get("unSteamFeePerUnit").and_then(num).unwrap_or(0),
                game_fee: l.get("unPublisherFeePerUnit").and_then(num).unwrap_or(0),
            })
        })
        .min_by_key(|l| l.price + l.fee)
}

fn billing_fields(info: &PurchaseInfo) -> Form {
    let b = &info.billing;
    vec![
        ("first_name", b.first_name.clone()),
        ("last_name", b.last_name.clone()),
        ("billing_address", b.address.clone()),
        ("billing_address_two", b.address_two.clone()),
        ("billing_country", info.country.clone()),
        ("billing_city", b.city.clone()),
        ("billing_state", b.state.clone()),
        ("billing_postal_code", b.postal_code.clone()),
        ("save_my_address", "0".into()),
    ]
}

/// The form for buying one item from a listing.
pub fn buy_listing_form(session: &str, l: &MarketListing, info: &PurchaseInfo) -> Form {
    let c = l.cost(info.tax_percent);
    let mut f: Form = vec![
        ("sessionid", session.to_string()),
        ("currency", info.wallet_currency.to_string()),
        ("subtotal", c.subtotal.to_string()),
        ("fee", c.fee.to_string()),
        ("total", c.total.to_string()),
        ("tradefee_tax", c.tax.to_string()),
        ("quantity", "1".into()),
    ];
    f.extend(billing_fields(info));
    f
}

/// The form for a buy order. `price` is per item, with fees.
pub fn buy_order_form(session: &str, app: AppId, hash_name: &str, price: u64, quantity: u64, info: &PurchaseInfo) -> Form {
    let mut f: Form = vec![
        ("sessionid", session.to_string()),
        ("currency", info.wallet_currency.to_string()),
        ("appid", app.to_string()),
        ("market_hash_name", hash_name.to_string()),
        ("price_total", (price * quantity).to_string()),
        ("tradefee_tax", order_tax(price, quantity, info.tax_percent).to_string()),
        ("quantity", quantity.to_string()),
    ];
    f.extend(billing_fields(info));
    f
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Done,
    /// Steam wants approval in the Steam Mobile app: ask again with this id.
    Confirm(String),
    Failed(String),
}

/// What an answer to a purchase means. A buy order must also say `success: 1`.
pub fn purchase_step(ok: bool, v: &Value, is_order: bool, waiting: bool) -> Step {
    if !v.is_object() {
        return Step::Failed(UNKNOWN_RESULT.into());
    }
    let done = if is_order { ok && v.get("success").and_then(num) == Some(1) } else { ok };
    if done {
        return match message(v).filter(|_| is_order) {
            Some(m) => Step::Failed(m),
            None => Step::Done,
        };
    }
    if v.get("need_confirmation").is_some_and(|b| b.as_bool().unwrap_or(false) || b.as_u64() == Some(1)) {
        let id = v.get("confirmation").and_then(|c| c.get("confirmation_id")).and_then(|c| c.as_str().map(String::from).or_else(|| c.as_u64().map(|n| n.to_string())));
        return match id {
            Some(id) if !id.is_empty() && id != "0" => Step::Confirm(id),
            _ => Step::Failed(UNKNOWN_RESULT.into()),
        };
    }
    if waiting {
        return Step::Failed("The approval in the Steam Mobile app was declined or ran out of time.".into());
    }
    Step::Failed(message(v).unwrap_or_else(|| if is_order { "Steam didn't accept the buy order.".into() } else { "Steam couldn't complete the purchase. The item may already be sold.".into() }))
}

/// Send a purchase, repeating it while Steam waits for your phone.
async fn submit_purchase(web: &Web, url: &str, referer: &str, form: Form, is_order: bool, send: &(dyn Fn(Reply) + Send + Sync)) -> Result<Value, String> {
    let started = Instant::now();
    let mut confirmation: Option<String> = None;
    loop {
        let mut f = form.clone();
        f.push(("confirmation", confirmation.clone().unwrap_or_else(|| "0".into())));
        let (ok, v) = post(web, url, referer, &f).await.map_err(|e| {
            tracing::warn!("purchase request: {e}");
            UNKNOWN_RESULT.to_string()
        })?;
        match purchase_step(ok, &v, is_order, confirmation.is_some()) {
            Step::Done => return Ok(v),
            Step::Failed(e) => return Err(e),
            Step::Confirm(id) => {
                if confirmation.is_none() {
                    send(Reply::WaitingForPhone);
                }
                confirmation = Some(id);
                if started.elapsed() > PHONE_WAIT {
                    return Err("Still waiting for your approval. If you approve it in the Steam Mobile app it can still go through, so deny it there if you changed your mind.".into());
                }
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
        }
    }
}

async fn buy_listing(web: &Web, l: &MarketListing, info: &PurchaseInfo, send: &(dyn Fn(Reply) + Send + Sync)) -> Result<Option<(i64, i32)>, String> {
    if l.currency != info.wallet_currency {
        return Err("This listing's price isn't in your wallet currency. Open it again to get a fresh price.".into());
    }
    let url = format!("{COMMUNITY}/market/buylisting/{}", l.listing_id);
    let v = submit_purchase(web, &url, &listing_url(l.app_id, &l.hash_name), buy_listing_form(&web.session_id, l, info), false, send).await?;
    Ok(new_wallet(&v))
}

fn new_wallet(v: &Value) -> Option<(i64, i32)> {
    let w = v.get("wallet_info")?;
    Some((w.get("wallet_balance").and_then(num)? as i64, w.get("wallet_currency").and_then(num)? as i32))
}

async fn place_order(web: &Web, app: AppId, hash_name: &str, price: u64, quantity: u64, info: &PurchaseInfo, send: &(dyn Fn(Reply) + Send + Sync)) -> Result<bool, String> {
    let url = format!("{COMMUNITY}/market/createbuyorder/");
    let v = submit_purchase(web, &url, &listing_url(app, hash_name), buy_order_form(&web.session_id, app, hash_name, price, quantity, info), true, send).await?;
    let Some(order) = v.get("buy_orderid").and_then(num) else { return Ok(false) };
    // Like Steam's dialog: look for a few seconds in case it filled at once.
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let status = web.http.get(format!("{COMMUNITY}/market/getbuyorderstatus/")).query(&[("sessionid", web.session_id.clone()), ("buy_orderid", order.to_string())]).send().await;
        let Ok(r) = status else { break };
        let Ok(s) = r.json::<Value>().await else { break };
        if s.get("success").and_then(num) == Some(1) && s.get("purchased").and_then(num).unwrap_or(0) > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

// ---- Gems ----

async fn gem_value(web: &Web, game: AppId, context: u64, asset: u64) -> Result<u64, String> {
    let r = web
        .http
        .get(format!("{}/ajaxgetgoovalue/", profile(web)))
        .query(&[("sessionid", web.session_id.clone()), ("appid", game.to_string()), ("assetid", asset.to_string()), ("contextid", context.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = r.json().await.unwrap_or(Value::Null);
    v.get("goo_value").and_then(num).filter(|n| *n > 0).ok_or_else(|| message(&v).unwrap_or_else(|| "Steam didn't say what this item is worth in gems.".into()))
}

pub fn grind_form(session: &str, game: AppId, context: u64, asset: u64, expected: u64) -> Form {
    vec![("sessionid", session.into()), ("appid", game.to_string()), ("assetid", asset.to_string()), ("contextid", context.to_string()), ("goo_value_expected", expected.to_string())]
}

async fn grind(web: &Web, game: AppId, context: u64, asset: u64, expected: u64) -> Result<u64, String> {
    let referer = format!("{}/inventory/", profile(web));
    let (ok, v) = post(web, &format!("{}/ajaxgrindintogoo/", profile(web)), &referer, &grind_form(&web.session_id, game, context, asset, expected)).await.map_err(|_| UNKNOWN_RESULT.to_string())?;
    parse_grind(ok, &v, expected)
}

pub fn parse_grind(ok: bool, v: &Value, expected: u64) -> Result<u64, String> {
    if !v.is_object() {
        return Err(UNKNOWN_RESULT.into());
    }
    let success = v.get("success").and_then(num);
    if !ok || success.is_some_and(|s| s != 1) {
        return Err(message(v).unwrap_or_else(|| "Steam didn't turn this item into gems.".into()));
    }
    // Steam's key has had a stray space in it.
    Ok(["goo_value_received", "goo_value_received "].iter().find_map(|k| v.get(*k).and_then(num)).unwrap_or(expected))
}

/// How much comes out of an exchange, if the amount divides evenly.
pub fn exchange_out(amount_in: u64, from_size: u64, to_size: u64) -> Option<u64> {
    if amount_in == 0 || from_size == 0 || to_size == 0 {
        return None;
    }
    let value = amount_in.checked_mul(from_size)?;
    value.is_multiple_of(to_size).then_some(value / to_size)
}

pub fn exchange_form(session: &str, game: AppId, asset: u64, from_size: u64, to_size: u64, amount_in: u64) -> Option<Form> {
    let out = exchange_out(amount_in, from_size, to_size)?;
    Some(vec![
        ("sessionid", session.into()),
        ("appid", game.to_string()),
        ("assetid", asset.to_string()),
        ("goo_denomination_in", from_size.to_string()),
        ("goo_amount_in", amount_in.to_string()),
        ("goo_denomination_out", to_size.to_string()),
        ("goo_amount_out_expected", out.to_string()),
    ])
}

async fn exchange(web: &Web, game: AppId, asset: u64, from_size: u64, to_size: u64, amount_in: u64) -> Result<(), String> {
    let form = exchange_form(&web.session_id, game, asset, from_size, to_size, amount_in).ok_or("That amount doesn't divide into whole sacks.")?;
    let referer = format!("{}/inventory/", profile(web));
    let (ok, v) = post(web, &format!("{}/ajaxexchangegoo/", profile(web)), &referer, &form).await.map_err(|_| UNKNOWN_RESULT.to_string())?;
    if !v.is_object() {
        return Err(UNKNOWN_RESULT.into());
    }
    match v.get("success").and_then(num) {
        Some(1) if ok => Ok(()),
        Some(78) => Err("You don't have enough gems for that.".into()),
        _ => Err(message(&v).unwrap_or_else(|| "Steam didn't exchange your gems.".into())),
    }
}

// ---- Booster packs ----

async fn boosters(web: &Web) -> Result<BoosterCatalog, String> {
    let body = crate::market::page(web, &format!("{COMMUNITY}/tradingcards/boostercreator/")).await?;
    parse_boosters(&body).ok_or_else(|| "Couldn't read Steam's booster pack page.".into())
}

/// The arguments of `CBoosterCreatorPage.Init( [...], gems, tradable, untradable, ...)`.
pub fn parse_boosters(body: &str) -> Option<BoosterCatalog> {
    let at = body.find("CBoosterCreatorPage.Init(")? + "CBoosterCreatorPage.Init(".len();
    let mut rest = body[at..].trim_start();
    let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
    let list = stream.next()?.ok()?;
    rest = rest[stream.byte_offset()..].trim_start();
    let mut amounts = [0u64; 3];
    for slot in amounts.iter_mut() {
        rest = rest.strip_prefix(',')?.trim_start();
        let mut s = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
        let v = s.next()?.ok()?;
        rest = rest[s.byte_offset()..].trim_start();
        *slot = v.as_f64().or_else(|| v.as_str()?.replace(',', "").parse().ok())?.max(0.0) as u64;
    }
    let mut games: Vec<BoosterGame> = list
        .as_array()?
        .iter()
        .filter_map(|g| {
            let unavailable = g.get("unavailable").is_some_and(|b| b.as_bool().unwrap_or(false) || b.as_u64() == Some(1));
            Some(BoosterGame {
                app_id: g.get("appid").and_then(num)? as AppId,
                name: crate::hub::decode(&str_of(g, "name")),
                series: g.get("series").and_then(num).unwrap_or(1) as u32,
                gems: g.get("price").and_then(num)?,
                available: !unavailable,
                available_at: Some(str_of(g, "available_at_time")).filter(|s| !s.is_empty()),
            })
        })
        .collect();
    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Some(BoosterCatalog { games, gems: amounts[0], tradable_gems: amounts[1], untradable_gems: amounts[2] })
}

pub fn make_booster_form(session: &str, app: AppId, series: u32, preference: u8) -> Form {
    vec![("sessionid", session.into()), ("appid", app.to_string()), ("series", series.to_string()), ("tradability_preference", preference.to_string())]
}

async fn make_booster(web: &Web, app: AppId, series: u32, preference: u8) -> Result<(u64, u64, u64), String> {
    let (ok, v) = post(web, &format!("{COMMUNITY}/tradingcards/ajaxcreatebooster/"), &format!("{COMMUNITY}/tradingcards/boostercreator/"), &make_booster_form(&web.session_id, app, series, preference)).await.map_err(|_| UNKNOWN_RESULT.to_string())?;
    parse_made_booster(ok, &v)
}

/// Your gems after making a pack: (all, tradable, untradable).
pub fn parse_made_booster(ok: bool, v: &Value) -> Result<(u64, u64, u64), String> {
    if !v.is_object() {
        return Err(UNKNOWN_RESULT.into());
    }
    if !ok || v.get("purchase_result").is_none() {
        return Err(message(v).unwrap_or_else(|| "Steam couldn't make this booster pack right now.".into()));
    }
    let n = |k: &str| v.get(k).and_then(num).unwrap_or(0);
    Ok((n("goo_amount"), n("tradable_goo_amount"), n("untradable_goo_amount")))
}

pub fn open_booster_form(session: &str, game: AppId, asset: u64) -> Form {
    vec![("appid", game.to_string()), ("communityitemid", asset.to_string()), ("sessionid", session.into())]
}

async fn open_booster(web: &Web, game: AppId, asset: u64) -> Result<Vec<BoosterCard>, String> {
    let referer = format!("{}/inventory/", profile(web));
    let (_, v) = post(web, &format!("{}/ajaxunpackbooster/", profile(web)), &referer, &open_booster_form(&web.session_id, game, asset)).await.map_err(|_| UNKNOWN_RESULT.to_string())?;
    parse_opened_booster(&v)
}

pub fn parse_opened_booster(v: &Value) -> Result<Vec<BoosterCard>, String> {
    if !v.is_object() {
        return Err(UNKNOWN_RESULT.into());
    }
    let items = v.get("rgItems").and_then(|i| i.as_array()).filter(|i| !i.is_empty()).ok_or_else(|| message(v).unwrap_or_else(|| "Steam couldn't open this booster pack. It may already be open.".into()))?;
    Ok(items
        .iter()
        .map(|c| BoosterCard {
            name: str_of(c, "name"),
            image: str_of(c, "image"),
            series: c.get("series").and_then(num).unwrap_or(1) as u32,
            foil: c.get("foil").is_some_and(|f| f.as_bool().unwrap_or(false) || f.as_u64() == Some(1)),
        })
        .collect())
}

// ---- Pending gifts ----

async fn gifts(web: &Web) -> Result<Vec<PendingGift>, String> {
    let body = crate::market::page(web, &format!("{}/inventory/", profile(web))).await?;
    Ok(parse_gifts(&body))
}

/// Pending gifts on your inventory page. Steam's scripts name each gift's
/// block `pending_gift_<id>` and its buttons `DoAcceptGift` and
/// `ShowDeclineGiftOptions`, so those are what's relied on here.
pub fn parse_gifts(html: &str) -> Vec<PendingGift> {
    fn call_args<'a>(s: &'a str, name: &str) -> Vec<Vec<&'a str>> {
        s.match_indices(name)
            .filter_map(|(i, _)| {
                let rest = s[i + name.len()..].trim_start().strip_prefix('(')?;
                let inner = &rest[..rest.find(')')?];
                Some(inner.split(',').map(|a| a.trim().trim_matches(['\'', '"'])).collect())
            })
            .collect()
    }
    /// The text of the first element whose class contains one of `classes`.
    fn text_of(block: &str, classes: &[&str]) -> String {
        for class in classes {
            let Some(at) = block.find(class) else { continue };
            let Some(open) = block[at..].find('>') else { continue };
            let start = at + open + 1;
            let end = block[start..].find("</").map_or(block.len(), |e| start + e);
            let text = crate::hub::decode(&crate::hub::strip_tags(&block[start..end])).split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() {
                return text;
            }
        }
        String::new()
    }
    let mut out: Vec<PendingGift> = Vec::new();
    for part in html.split("id=\"pending_gift_").skip(1) {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        let Ok(id) = digits.parse::<u64>() else { continue };
        if out.iter().any(|g| g.id == id) {
            continue;
        }
        let block = part.replace("&#39;", "'").replace("&quot;", "\"");
        let of_this = |args: &Vec<&str>| args.first().is_some_and(|a| *a == digits);
        let sender = call_args(&block, "ShowDeclineGiftOptions").iter().find(|a| of_this(a)).and_then(|a| a.get(1)?.parse().ok()).unwrap_or(0);
        let accepts = call_args(&block, "DoAcceptGift");
        let accept = |wanted: [&str; 2]| accepts.iter().any(|a| of_this(a) && a.get(1).is_some_and(|x| wanted.contains(x)));
        let image = block.find("<img").and_then(|i| {
            let tag = &block[i..];
            let s = tag.find("src=\"")? + 5;
            Some(tag[s..s + tag[s..].find('"')?].to_string())
        });
        out.push(PendingGift {
            id,
            name: text_of(&block, &["gift_name", "pending_gift_title", "gift_title", "giftname"]),
            sender,
            sender_name: text_of(&block, &["gift_sender", "persona", "sender_name"]),
            message: text_of(&block, &["gift_message", "gift_note", "giftmessage"]),
            image: image.filter(|s| s.starts_with("https://")).unwrap_or_default(),
            can_add_to_library: accept(["true", "1"]),
            can_keep_in_inventory: accept(["false", "0"]),
        });
    }
    out
}

pub fn accept_gift_form(session: &str) -> Form {
    vec![("sessionid", session.to_string())]
}

pub fn decline_gift_form(session: &str, sender: SteamId, note: &str) -> Form {
    vec![("sessionid", session.to_string()), ("steamid_sender", sender.to_string()), ("note", note.to_string())]
}

fn gift_answer(ok: bool, v: &Value, fallback: &str) -> Result<(), String> {
    if !v.is_object() {
        return Err(UNKNOWN_RESULT.into());
    }
    if ok && v.get("success").and_then(num) == Some(1) { Ok(()) } else { Err(message(v).unwrap_or_else(|| fallback.to_string())) }
}

async fn accept_gift(web: &Web, id: u64, to_library: bool) -> Result<(), String> {
    let action = if to_library { "acceptunpack" } else { "accept" };
    let (ok, v) = post(web, &format!("{COMMUNITY}/gifts/{id}/{action}"), &format!("{}/inventory/", profile(web)), &accept_gift_form(&web.session_id)).await.map_err(|_| UNKNOWN_RESULT.to_string())?;
    gift_answer(ok, &v, "Steam couldn't accept this gift. It may already have been redeemed.")
}

async fn decline_gift(web: &Web, id: u64, sender: SteamId, note: &str) -> Result<(), String> {
    let (ok, v) = post(web, &format!("{COMMUNITY}/gifts/{id}/decline"), &format!("{}/inventory/", profile(web)), &decline_gift_form(&web.session_id, sender, note)).await.map_err(|_| UNKNOWN_RESULT.to_string())?;
    gift_answer(ok, &v, "Steam couldn't decline this gift. It may already have been answered.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn get<'a>(f: &'a Form, k: &str) -> &'a str {
        f.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str()).unwrap_or_else(|| panic!("no {k}"))
    }

    fn info() -> PurchaseInfo {
        PurchaseInfo { wallet_balance: 1000, wallet_currency: 3, market_minimum: 1, country: "DE".into(), ..Default::default() }
    }

    /// The shape of a current market page, with made-up ids and no seller data.
    const PAGE: &str = r#"<script>x = JSON.parse("{\"queries\":[{\"state\":{\"data\":{\"pages\":[{\"more\":false,\"listings\":[{\"listingid\":\"1001\",\"unPrice\":263,\"unFee\":39,\"eCurrency\":3,\"bMine\":true,\"unPricePerUnit\":40,\"unFeePerUnit\":5,\"unSteamFeePerUnit\":2,\"unPublisherFeePerUnit\":3,\"description\":{\"name\":\"Mine\",\"icon_url\":\"m\"}},
        {\"listingid\":\"1003\",\"eCurrency\":3,\"bMine\":false,\"unPricePerUnit\":70,\"unFeePerUnit\":10,\"unSteamFeePerUnit\":3,\"unPublisherFeePerUnit\":7,\"description\":{\"name\":\"Sack of Gems\",\"icon_url\":\"b\"}},
        {\"listingid\":\"1002\",\"eCurrency\":3,\"bMine\":false,\"unPricePerUnit\":53,\"unFeePerUnit\":7,\"unSteamFeePerUnit\":2,\"unPublisherFeePerUnit\":5,\"description\":{\"name\":\"Sack of Gems\",\"icon_url\":\"a\"}}
    ]}]}}}]}");</script>"#;

    #[test]
    fn finds_the_cheapest_listing_that_isnt_yours() {
        let l = parse_cheapest_listing(&PAGE.replace('\n', " "), 753, "753-Sack of Gems", "x").unwrap();
        assert_eq!((l.listing_id, l.price, l.fee, l.steam_fee, l.game_fee, l.currency), (1002, 53, 7, 2, 5, 3));
        assert_eq!((l.name.as_str(), l.icon.as_str()), ("Sack of Gems", "a"));
        assert!(parse_cheapest_listing("<html></html>", 753, "x", "x").is_none());
    }

    #[test]
    fn builds_the_buy_listing_form() {
        let l = MarketListing { listing_id: 1002, price: 53, fee: 7, currency: 3, ..Default::default() };
        let mut i = info();
        i.tax_percent = 20.0;
        i.billing.city = "Town".into();
        let f = buy_listing_form("sess", &l, &i);
        for (k, v) in [("sessionid", "sess"), ("currency", "3"), ("subtotal", "53"), ("fee", "7"), ("tradefee_tax", "1"), ("total", "61"), ("quantity", "1"), ("billing_country", "DE"), ("billing_city", "Town"), ("save_my_address", "0")] {
            assert_eq!(get(&f, k), v, "{k}");
        }
    }

    #[test]
    fn builds_the_buy_order_form() {
        let f = buy_order_form("sess", 730, "Chroma Case", 25, 4, &info());
        for (k, v) in [("appid", "730"), ("market_hash_name", "Chroma Case"), ("price_total", "100"), ("quantity", "4"), ("tradefee_tax", "0"), ("currency", "3")] {
            assert_eq!(get(&f, k), v, "{k}");
        }
    }

    #[test]
    fn reads_wallet_and_tax() {
        let v = json!({"wallet_info":{"wallet_currency":1,"wallet_balance":"2500","wallet_market_minimum":"1"},"require_billing_info":false,"country_code":"US",
            "tax_rate":{"tradefee_addtax":true,"tradefee_taxrate":8.25},"billing_address":{"firstname":"A","address1":"B","postcode":"1"}});
        let i = parse_purchase_info(&v).unwrap();
        assert_eq!((i.wallet_balance, i.wallet_currency, i.tax_percent, i.requires_billing), (2500, 1, 8.25, false));
        assert_eq!((i.billing.first_name.as_str(), i.billing.address.as_str(), i.billing.postal_code.as_str()), ("A", "B", "1"));
        // Tax only counts when Steam says to add it.
        let no = json!({"wallet_info":{"wallet_currency":3},"tax_rate":{"tradefee_addtax":false,"tradefee_taxrate":19}});
        assert_eq!(parse_purchase_info(&no).unwrap().tax_percent, 0.0);
        assert!(parse_purchase_info(&json!({"success":false})).is_err());
    }

    #[test]
    fn reads_purchase_answers() {
        assert_eq!(purchase_step(true, &json!({"wallet_info":{"success":1}}), false, false), Step::Done);
        assert_eq!(purchase_step(true, &json!({"success":1,"buy_orderid":"5"}), true, false), Step::Done);
        // An order answer with a message is a refusal, even with status 200.
        assert_eq!(purchase_step(true, &json!({"success":1,"message":"Nope"}), true, false), Step::Failed("Nope".into()));
        assert_eq!(purchase_step(false, &json!({"need_confirmation":true,"confirmation":{"confirmation_id":"77"}}), false, false), Step::Confirm("77".into()));
        // No id to wait on: never loop.
        assert!(matches!(purchase_step(false, &json!({"need_confirmation":true}), true, false), Step::Failed(_)));
        assert_eq!(purchase_step(false, &json!({"message":"Sold"}), false, false), Step::Failed("Sold".into()));
        // Waiting on the phone and Steam stops asking: denied or expired.
        assert!(matches!(purchase_step(false, &json!({}), false, true), Step::Failed(m) if m.contains("declined")));
    }

    #[test]
    fn a_page_instead_of_an_answer_is_never_a_success() {
        let page = Value::Null;
        assert_eq!(purchase_step(true, &page, false, false), Step::Failed(UNKNOWN_RESULT.into()));
        assert_eq!(purchase_step(true, &page, true, false), Step::Failed(UNKNOWN_RESULT.into()));
        assert_eq!(parse_grind(true, &page, 40), Err(UNKNOWN_RESULT.into()));
        assert_eq!(parse_made_booster(true, &page), Err(UNKNOWN_RESULT.into()));
        assert_eq!(parse_opened_booster(&page), Err(UNKNOWN_RESULT.into()));
        assert_eq!(gift_answer(true, &page, "x"), Err(UNKNOWN_RESULT.into()));
    }

    #[test]
    fn builds_gem_forms() {
        let f = grind_form("s", 730, 6, 123, 40);
        for (k, v) in [("appid", "730"), ("contextid", "6"), ("assetid", "123"), ("goo_value_expected", "40")] {
            assert_eq!(get(&f, k), v);
        }
        let pack = exchange_form("s", 753, 9, 1, 1000, 3000).unwrap();
        for (k, v) in [("goo_denomination_in", "1"), ("goo_amount_in", "3000"), ("goo_denomination_out", "1000"), ("goo_amount_out_expected", "3")] {
            assert_eq!(get(&pack, k), v);
        }
        let unpack = exchange_form("s", 753, 9, 1000, 1, 2).unwrap();
        assert_eq!(get(&unpack, "goo_amount_out_expected"), "2000");
        // Part of a sack can't be packed.
        assert!(exchange_form("s", 753, 9, 1, 1000, 1500).is_none());
        assert!(exchange_form("s", 753, 9, 1, 1000, 0).is_none());
    }

    #[test]
    fn reads_grind_answers() {
        assert_eq!(parse_grind(true, &json!({"success":1,"goo_value_received ":"46"}), 40), Ok(46));
        assert_eq!(parse_grind(true, &json!({"strHTML":"ok"}), 40), Ok(40));
        assert!(parse_grind(false, &json!({"message":"No"}), 40).is_err());
        assert!(parse_grind(true, &json!({"success":2}), 40).is_err());
    }

    #[test]
    fn reads_the_booster_page() {
        let page = r#"<script>$J(function(){ CBoosterCreatorPage.Init(
            [{"appid":440,"name":"Team &amp; Fortress","series":1,"price":"400","unavailable":true,"available_at_time":"25 Nov @ 4:11am"},
             {"appid":570,"name":"Alpha Game","series":2,"price":"600"}],
            "1,250", 1000, 250, "<div>template, with (commas)</div>", [440] ); });</script>"#;
        let c = parse_boosters(page).unwrap();
        assert_eq!((c.gems, c.tradable_gems, c.untradable_gems), (1250, 1000, 250));
        assert_eq!(c.games[0], BoosterGame { app_id: 570, name: "Alpha Game".into(), series: 2, gems: 600, available: true, available_at: None });
        assert_eq!((c.games[1].name.as_str(), c.games[1].available, c.games[1].available_at.as_deref()), ("Team & Fortress", false, Some("25 Nov @ 4:11am")));
        assert!(parse_boosters("<html></html>").is_none());
    }

    #[test]
    fn reads_booster_answers() {
        let f = make_booster_form("s", 440, 1, 2);
        assert_eq!((get(&f, "appid"), get(&f, "series"), get(&f, "tradability_preference")), ("440", "1", "2"));
        let made = json!({"purchase_result":{"success":1,"communityitemid":"9"},"goo_amount":"850","tradable_goo_amount":"600","untradable_goo_amount":250});
        assert_eq!(parse_made_booster(true, &made), Ok((850, 600, 250)));
        assert!(parse_made_booster(false, &json!({"goo_amount":1})).is_err());
        let f = open_booster_form("s", 440, 9);
        assert_eq!((get(&f, "appid"), get(&f, "communityitemid")), ("440", "9"));
        let cards = parse_opened_booster(&json!({"success":1,"rgItems":[{"image":"https://x/1","name":"Scout","series":1,"foil":false},{"image":"https://x/2","name":"Spy","series":1,"foil":true}]})).unwrap();
        assert_eq!((cards.len(), cards[1].name.as_str(), cards[1].foil), (2, "Spy", true));
        assert!(parse_opened_booster(&json!({"success":2})).is_err());
    }

    #[test]
    fn reads_pending_gifts() {
        let html = r#"<div id="tabcontent_pendinggifts">
        <div class="pending_gift" id="pending_gift_111">
          <img src="https://cdn.example/capsule.jpg">
          <div class="gift_name">Portal &amp; Friends</div>
          <div class="gift_sender">A Friend</div>
          <div class="gift_message">Happy birthday!</div>
          <a onclick="DoAcceptGift( '111', true );">Add</a><a onclick="DoAcceptGift( &#39;111&#39;, false );">Keep</a>
          <a onclick="ShowDeclineGiftOptions( '111', '76561197960265729' );">Decline</a>
        </div>
        <div class="pending_gift" id="pending_gift_222">
          <a onclick="DoAcceptGift( '222', 1 );">Add</a>
          <a onclick="ShowDeclineGiftOptions( '222', '76561197960265730' );">Decline</a>
        </div></div>"#;
        let g = parse_gifts(html);
        assert_eq!(g.len(), 2);
        assert_eq!((g[0].id, g[0].name.as_str(), g[0].sender_name.as_str(), g[0].message.as_str()), (111, "Portal & Friends", "A Friend", "Happy birthday!"));
        assert_eq!((g[0].sender, g[0].can_add_to_library, g[0].can_keep_in_inventory), (76561197960265729, true, true));
        assert_eq!(g[0].image, "https://cdn.example/capsule.jpg");
        assert_eq!((g[1].id, g[1].name.as_str(), g[1].can_add_to_library, g[1].can_keep_in_inventory), (222, "", true, false));
        assert!(parse_gifts("<div>no gifts</div>").is_empty());
    }

    #[test]
    fn builds_gift_forms() {
        assert_eq!(get(&accept_gift_form("s"), "sessionid"), "s");
        let f = decline_gift_form("s", 7, "No thanks");
        assert_eq!((get(&f, "steamid_sender"), get(&f, "note")), ("7", "No thanks"));
    }

    #[test]
    fn logs_never_hold_billing_details() {
        let mut i = info();
        i.billing.address = "Secret Street".into();
        let r = Request::BuyListing { listing: MarketListing::default(), info: i.clone() };
        assert!(!format!("{r:?}").contains("Secret"));
        assert!(!format!("{i:?}").contains("Secret"));
    }
}

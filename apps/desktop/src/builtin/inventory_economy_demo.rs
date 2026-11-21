//! `--demo`: sample gems, booster packs and gifts, and made-up answers to
//! every purchase. Nothing here talks to Steam.

use client::economy::{Reply, Request, exchange_out};
use client::{Event, State};
use model::economy::*;
use model::{BuyOrder, InventoryItem, fees};
use std::sync::{Mutex, OnceLock, mpsc};

const ME: u64 = 76561197960265729;
const SACK_ICON: &str = "a0b44yVZG6xneuQqGEgxNyzAxlmouy8Ipo0bUlJjRDfF9dQS1eD3AHU-t8xnioHyGdqSZpgsmMIM6FMBcAeKAqxvt1BvixsCnnuPb5_rQvKWpz_x_9gGR_XZ4OH82JW2_nEGj8JxPpJde4KIDssSdA";
const CARD_ICON: &str = "IzMF03bk9WpSBq-S-ekoE33L-iLqGFHVaU25ZzQNQcXdA3g5gMEPvUZZEfSMJ6dESN8p_2SVTY7V2N4MxGVIwXpaL3_L6lVZI-xhOczDnQOqpefZRHCgPWeSLHWPHVxpG7QIZmCL-zL24LiTEWnBRu0tRl1QKaYasjoeK5vWfUZoluhSpmS_khl4Gk4tJ5QScw25nCYSabwknCARdpkHkSCjIJTQ0V8waxQ5DL-yVb_CccH7xzliUBj6u3WN1A";
const CARD_ICON_2: &str = "IzMF03bk9WpSBq-S-ekoE33L-iLqGFHVaU25ZzQNQcXdA3g5gMEPvUZZEfSMJ6dESN8p_2SVTY7V2N4MxGVIwXpaL3_e_10rbOhtP86bwQH-9LqLSyH1P2GSeCfcRA0wT-ALZGnYqzug7L6TFjubRLkqQwEAfLxErDNfb5bLa0d_u95WqmK9ykF5RhN9I5FFcg3tmHVLMLxxySJBd85anXf2Jp2NhQ1hOhU8Xe3kVbzcL9_yhmkpX5t2C6_O";
const BOOSTER_ICON: &str = "IzMF03bk9WpSBq-S-ekoE33L-iLqGFHVaU25ZzQNQcXdBnY7ltYLvVIHHqLGffMZGJpx5mnYWInWiYBMnGVPwXRTdzH13Hhhtz-7TDA";
const EMOTICON_ICON: &str = "a0b44yVZG6xneuQqGEgxNyzAxlmouy8Ipo0bUlJjRDfF9dQS1eD3AHU-t8xnioHyGdqSZpgsmMIM6FMBdgSKValtt1U02E8DkiuIO5jpE_KSpz-s_NsGF6fftrOmgZPn8SMGjMd1PpJde4JaLIKdWg";

const GEM_STACK: u64 = 7001;
const SACK_STACK: u64 = 7002;

struct Sample {
    tx: mpsc::Sender<Event>,
    items: Vec<InventoryItem>,
    catalog: BoosterCatalog,
    gifts: Vec<PendingGift>,
    next: u64,
}

static SAMPLE: OnceLock<Mutex<Sample>> = OnceLock::new();

fn item(asset: u64, name: &str, kind: &str, icon: &str, amount: u64, action: String) -> InventoryItem {
    InventoryItem {
        asset_id: asset,
        class_id: asset,
        amount,
        name: name.into(),
        kind: kind.into(),
        name_color: Some("D2D2D2".into()),
        icon: icon.into(),
        tradable: true,
        marketable: true,
        market_hash_name: format!("753-{name}"),
        context_id: 6,
        tags: vec![("Item Type".into(), kind.into(), None)],
        owner_actions: vec![("Action".into(), action)],
        market_fee: Some(0.1),
        ..Default::default()
    }
}

fn card(asset: u64, name: &str, icon: &str) -> InventoryItem {
    item(asset, name, "Counter-Strike 2 Trading Card", icon, 1, format!("javascript:GetGooValue( '6', '{asset}', 730, 6, 0 )"))
}

fn gem_stack(amount: u64) -> InventoryItem {
    let mut gems = item(GEM_STACK, "Gems", "Steam Gems", SACK_ICON, amount, format!("javascript:PackGameGooIntoBarrel( 753, '{GEM_STACK}', 1, 1000, this )"));
    gems.marketable = false;
    gems
}

fn sack_stack(amount: u64) -> InventoryItem {
    item(SACK_STACK, "Sack of Gems", "Steam Gems", SACK_ICON, amount, format!("javascript:UnpackGameGooFromBarrel( 753, '{SACK_STACK}', 1000, 1, this )"))
}

fn booster(asset: u64, game: u32, name: &str) -> InventoryItem {
    item(asset, &format!("{name} Booster Pack"), "Booster Pack", BOOSTER_ICON, 1, format!("javascript:OpenBooster( {game}, '{asset}' )"))
}

fn sample_items() -> Vec<InventoryItem> {
    vec![
        gem_stack(2450),
        sack_stack(3),
        booster(7003, 431960, "Wallpaper Engine"),
        card(7004, "SWAT", CARD_ICON),
        card(7005, "FBI", CARD_ICON_2),
        item(7006, ":csgoglobe:", "Counter-Strike 2 Emoticon", EMOTICON_ICON, 1, "javascript:GetGooValue( '6', '7006', 730, 13, 0 )".into()),
    ]
}

fn game(app_id: u32, name: &str, price: u64, available: bool, when: Option<&str>) -> BoosterGame {
    BoosterGame { app_id, name: name.into(), series: 1, gems: price, available, available_at: when.map(String::from) }
}

fn sample_catalog() -> BoosterCatalog {
    BoosterCatalog {
        games: vec![
            game(730, "Counter-Strike 2", 400, true, None),
            game(570, "Dota 2", 400, false, Some("Nov 22 @ 4:11am")),
            game(1145360, "Hades", 545, true, None),
            game(620, "Portal 2", 480, true, None),
            game(413150, "Stardew Valley", 6000, true, None),
            game(440, "Team Fortress 2", 400, true, None),
        ],
        gems: 2450,
        tradable_gems: 2450,
        untradable_gems: 0,
    }
}

fn sample_gifts() -> Vec<PendingGift> {
    vec![
        PendingGift {
            id: 9101,
            name: "Stardew Valley".into(),
            sender: ME + 1,
            sender_name: "Ferris".into(),
            message: "You need this. See you on the farm!".into(),
            image: model::art::header(413150),
            can_add_to_library: true,
            can_keep_in_inventory: true,
        },
        PendingGift { id: 9102, name: "Portal 2".into(), sender: ME + 2, sender_name: "Corro".into(), message: String::new(), image: model::art::header(620), can_add_to_library: true, can_keep_in_inventory: false },
    ]
}

/// The first sample events, and where later answers go.
pub fn install(tx: mpsc::Sender<Event>) -> Vec<Event> {
    let s = Sample { tx, items: sample_items(), catalog: sample_catalog(), gifts: sample_gifts(), next: 7100 };
    let mut events = vec![inventory_event(&s.items), Event::Economy(Reply::Gifts(Ok(s.gifts.clone())))];
    // Sample prices, so these items' Market panels fill in.
    for (i, x) in s.items.iter().filter(|x| x.marketable).enumerate() {
        let cents = 18 + i as u64 * 7;
        events.push(Event::MarketPrice {
            app_id: 753,
            hash_name: x.market_hash_name.clone(),
            result: Ok(model::MarketPrice {
                lowest: Some(model::format_money(cents as i64, 1)),
                lowest_sell: Some(cents),
                highest_buy: Some(cents - 2),
                sell_count: Some(240 + i as u64 * 31),
                buy_count: Some(900 + i as u64 * 57),
                volume: Some(30 + i as u64),
                currency: 1,
                ..Default::default()
            }),
        });
    }
    let _ = SAMPLE.set(Mutex::new(s));
    events
}

/// How many Steam items the demo starts with.
pub fn item_count() -> u32 {
    sample_items().len() as u32
}

fn inventory_event(items: &[InventoryItem]) -> Event {
    Event::Inventory(model::Inventory { steam_id: ME, app_id: 753, context_id: 6, total: items.len() as u32, items: items.to_vec(), error: None })
}

fn set_amount(items: &mut Vec<InventoryItem>, asset: u64, amount: u64, make: fn(u64) -> InventoryItem) {
    match items.iter_mut().find(|i| i.asset_id == asset) {
        Some(i) if amount > 0 => i.amount = amount,
        Some(_) => items.retain(|i| i.asset_id != asset),
        None if amount > 0 => items.insert(0, make(amount)),
        None => {}
    }
}

fn gem_count(items: &[InventoryItem]) -> u64 {
    items.iter().find(|i| i.asset_id == GEM_STACK).map_or(0, |i| i.amount)
}

/// The demo's made-up listing for an item, from its sample price.
fn listing(state: &State, app: u32, hash: &str, name: &str) -> Result<MarketListing, String> {
    let total = match state.market_prices.get(&(app, hash.to_string())) {
        Some(Ok(p)) => p.lowest_sell.unwrap_or(120),
        _ => 120,
    };
    let rules = fees::Rules::default();
    let price = fees::you_receive(total, fees::DEFAULT_GAME, &rules).max(1);
    let (steam_fee, game_fee) = fees::split(price, fees::DEFAULT_GAME, &rules);
    Ok(MarketListing { listing_id: 1, app_id: app, hash_name: hash.into(), name: name.into(), icon: String::new(), currency: 1, price, fee: steam_fee + game_fee, steam_fee, game_fee })
}

/// Answer a request with sample data.
pub fn reply(request: Request, state: &State) {
    let Some(lock) = SAMPLE.get() else { return };
    let Ok(mut guard) = lock.lock() else { return };
    let s = &mut *guard;
    let mut out: Vec<Event> = Vec::new();
    let mut said = |r: Reply| out.push(Event::Economy(r));
    match request {
        Request::LoadPurchaseInfo => {
            let (balance, currency) = state.wallet.unwrap_or((1234, 1));
            said(Reply::PurchaseInfo(Ok(PurchaseInfo { wallet_balance: balance, wallet_currency: currency, market_minimum: 1, country: "US".into(), ..Default::default() })));
        }
        Request::LoadListing { app_id, hash_name, name } => {
            let result = listing(state, app_id, &hash_name, &name);
            said(Reply::Listing { app_id, hash_name, result });
        }
        Request::BuyListing { listing, info } => {
            let left = info.wallet_balance - listing.cost(info.tax_percent).total as i64;
            said(Reply::Bought { name: listing.name, result: Ok(Some((left, info.wallet_currency))) });
        }
        Request::PlaceBuyOrder { app_id, hash_name, price, quantity, info } => {
            let mut mine = match &state.my_listings {
                Some(Ok(l)) => l.clone(),
                _ => Default::default(),
            };
            s.next += 1;
            let name = hash_name.strip_prefix("753-").unwrap_or(&hash_name).to_string();
            mine.buy_orders.push(BuyOrder { order_id: s.next, app_id, name, icon: String::new(), price, currency: info.wallet_currency, quantity, remaining: quantity, market_hash_name: hash_name });
            out.push(Event::MyListings(Ok(mine)));
            out.push(Event::Economy(Reply::OrderPlaced(Ok(false))));
        }
        Request::LoadGemValue { asset_id, .. } => {
            let value = if asset_id == 7006 { 20 } else { 46 };
            said(Reply::GemValue { asset_id, result: Ok(value) });
        }
        Request::Grind { asset_id, expected, .. } => {
            s.items.retain(|i| i.asset_id != asset_id);
            let now = gem_count(&s.items) + expected;
            set_amount(&mut s.items, GEM_STACK, now, gem_stack);
            s.catalog.gems += expected;
            s.catalog.tradable_gems += expected;
            out.push(inventory_event(&s.items));
            out.push(Event::Economy(Reply::Boosters(Ok(s.catalog.clone()))));
            out.push(Event::Economy(Reply::Ground { asset_id, result: Ok(expected) }));
        }
        Request::ExchangeGems { from_size, to_size, amount_in, .. } => {
            let amount_out = exchange_out(amount_in, from_size, to_size).unwrap_or(0);
            let (g, sacks) = (gem_count(&s.items), s.items.iter().find(|i| i.asset_id == SACK_STACK).map_or(0, |i| i.amount));
            if from_size == 1 {
                set_amount(&mut s.items, GEM_STACK, g.saturating_sub(amount_in), gem_stack);
                set_amount(&mut s.items, SACK_STACK, sacks + amount_out, sack_stack);
                s.catalog.gems = s.catalog.gems.saturating_sub(amount_in);
                s.catalog.tradable_gems = s.catalog.tradable_gems.saturating_sub(amount_in);
            } else {
                set_amount(&mut s.items, SACK_STACK, sacks.saturating_sub(amount_in), sack_stack);
                set_amount(&mut s.items, GEM_STACK, g + amount_out, gem_stack);
                s.catalog.gems += amount_out;
                s.catalog.tradable_gems += amount_out;
            }
            out.push(inventory_event(&s.items));
            out.push(Event::Economy(Reply::Boosters(Ok(s.catalog.clone()))));
            out.push(Event::Economy(Reply::Exchanged { from_size, amount_in, amount_out, result: Ok(()) }));
        }
        Request::LoadBoosters => said(Reply::Boosters(Ok(s.catalog.clone()))),
        Request::MakeBooster { app_id, .. } => {
            let cost = s.catalog.games.iter().find(|g| g.app_id == app_id).map_or(0, |g| g.gems);
            let name = s.catalog.games.iter().find(|g| g.app_id == app_id).map_or(String::new(), |g| g.name.clone());
            s.catalog.gems = s.catalog.gems.saturating_sub(cost);
            s.catalog.tradable_gems = s.catalog.tradable_gems.saturating_sub(cost);
            if let Some(g) = s.catalog.games.iter_mut().find(|g| g.app_id == app_id) {
                g.available = false;
                g.available_at = Some("tomorrow".into());
            }
            let left = gem_count(&s.items).saturating_sub(cost);
            set_amount(&mut s.items, GEM_STACK, left, gem_stack);
            s.next += 1;
            s.items.insert(0, booster(s.next, app_id, &name));
            out.push(inventory_event(&s.items));
            out.push(Event::Economy(Reply::BoosterMade { app_id, result: Ok((s.catalog.gems, s.catalog.tradable_gems, s.catalog.untradable_gems)) }));
        }
        Request::OpenBooster { asset_id, .. } => {
            s.items.retain(|i| i.asset_id != asset_id);
            let names = [("SWAT", CARD_ICON, false), ("FBI", CARD_ICON_2, false), ("SWAT", CARD_ICON, true)];
            let mut cards = Vec::new();
            for (name, icon, foil) in names {
                s.next += 1;
                let shown = if foil { format!("{name} (Foil)") } else { name.to_string() };
                s.items.push(card(s.next, &shown, icon));
                cards.push(BoosterCard { name: shown, image: model::art::econ(icon, 192), series: 1, foil });
            }
            out.push(inventory_event(&s.items));
            out.push(Event::Economy(Reply::BoosterOpened { asset_id, result: Ok(cards) }));
        }
        Request::LoadGifts => said(Reply::Gifts(Ok(s.gifts.clone()))),
        Request::AcceptGift { id, to_library } => {
            s.gifts.retain(|g| g.id != id);
            said(Reply::GiftAnswered { id, accepted: true, to_library, result: Ok(()) });
        }
        Request::DeclineGift { id, .. } => {
            s.gifts.retain(|g| g.id != id);
            said(Reply::GiftAnswered { id, accepted: false, to_library: false, result: Ok(()) });
        }
    }
    for e in out {
        let _ = s.tx.send(e);
    }
}

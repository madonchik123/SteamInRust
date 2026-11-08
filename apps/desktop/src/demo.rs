//! `--demo`: fills the UI with sample data (plus your real installed games)
//! without signing in. Handy for UI work and for testing extensions.

use client::Event;
use model::{ChatMessage, OwnedApp, Persona, PersonaState, Relationship};

pub fn events(local: &local::LocalState) -> Vec<Event> {
    const BASE: u64 = 76561197960265728;
    let me = BASE + 1;
    let app = |id, name: &str| OwnedApp { app_id: id, name: name.into(), kind: "game".into(), launch: vec![], icon: None, parent: None, ..Default::default() };
    let mut apps: Vec<OwnedApp> = local.games.iter().map(|g| app(g.app_id, &g.name)).collect();
    for (id, name) in [(440, "Team Fortress 2"), (570, "Dota 2"), (620, "Portal 2"), (730, "Counter-Strike 2"), (1145360, "Hades"), (413150, "Stardew Valley"), (292030, "The Witcher 3: Wild Hunt"), (1203220, "永劫无间 NARAKA"), (1938090, "コール オブ デューティ")] {
        if !apps.iter().any(|a| a.app_id == id) {
            apps.push(app(id, name));
        }
    }

    let friends = [
        ("Ferris", PersonaState::Online, Some(620)),
        ("Nova", PersonaState::Online, Some(570)),
        ("Pixel", PersonaState::Away, None),
        ("Quartz", PersonaState::Online, None),
        ("Juniper", PersonaState::Offline, None),
        ("Marlow", PersonaState::Offline, None),
    ];
    let personas: Vec<Persona> = std::iter::once(Persona { steam_id: me, name: "You".into(), state: PersonaState::Online, ..Default::default() })
        .chain(friends.iter().enumerate().map(|(i, (name, state, game))| Persona {
            steam_id: BASE + 100 + i as u64,
            name: (*name).into(),
            state: *state,
            game_app_id: *game,
            last_logoff: Some(1_700_000_000),
            // Nova is on a game server: shows "Join game" and "View server info".
            server: (i == 1).then(|| "127.0.0.1:27015".to_string()),
            rich_status: (i == 1).then(|| "Ranked · Dire 21 – 17 Radiant".to_string()),
            nickname: (i == 2).then(|| "Pix".to_string()),
            ..Default::default()
        }))
        .collect();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let ferris = BASE + 100;
    let chat = [
        (ferris, "hey, portal 2 co-op tonight?", 600),
        (me, "sure, after dinner", 540),
        (ferris, "nice. I'll send an invite around 8", 500),
    ];
    let mut events = vec![
        Event::LoggedIn { steam_id: me, account: "demo".into() },
        Event::AccountInfo { persona_name: "You".into(), country: "US".into() },
        Event::Level(27),
        Event::Licenses { count: apps.len() },
        Event::Apps(apps),
        Event::Friends((0..friends.len()).map(|i| (BASE + 100 + i as u64, Relationship::Friend)).collect()),
        Event::Personas(personas),
    ];
    events.push(Event::History {
        friend: ferris,
        messages: chat.iter().map(|(from, text, ago)| ChatMessage { friend: ferris, from: *from, text: (*text).into(), time: now - ago }).collect(),
    });
    events.push(Event::Profile(model::ProfileInfo {
        steam_id: me,
        background: Some(model::art::hero(1145360)),
        real_name: "Demo Player".into(),
        summary: "Collector of indie gems and long co-op nights.".into(),
        location: "Earth".into(),
        member_since: Some(now - 9 * 31_536_000),
        ..Default::default()
    }));
    events.push(Event::Wallet { balance: 1234, currency: 1, pending: 250 });
    events.push(Event::Message(ChatMessage { friend: BASE + 101, from: BASE + 101, text: "gg! one more?".into(), time: now - 30 }));
    events.extend(inventory(me, now));
    events.extend(crate::builtin::hub::demo::events());
    events
}

/// Sample inventory, market and support data (public Dota 2 items).
fn inventory(me: u64, now: u64) -> Vec<Event> {
    use model::{InventoryApp, InventoryItem, MarketPrice, MyListing, MyListings, SupportMessage, SupportTicket};
    const ITEMS: &[(&str, &str, &str, &str, u64, &str, &str)] = &[
        ("Arms of Desolation", "Immortal arms", "Immortal", "Arms", 1138, "Shadow Fiend", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGIyi3kajH-Xcws6uIkSR7FN78Mj1-FT0fhn0jZmv_DBW_M2qbbdibvHHDTLJlLZ3s7NvFyjqzE1z5znVm9uucHiWO1coCMF1RbRZsRW6l9H5d7S1tUgGsHE"),
        ("Hydrakan Latch", "Immortal Latch", "Immortal", "Weapon", 289, "Dragon Knight", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGIyi3kajH-XYwtiqenOO4UR0_4b61VXmVR750Zvl7ioU5-uqeq9sLs-fD3OTydF7v-VsQ2fll0wi6mrdnNipJC6ebg4nDZYkEeQKtBPsl9W1YuK2sQLc2NhAmCX2kGoXufdXeDr7"),
        ("The Strings of Suradan", "Immortal Wearable", "Immortal", "Weapon", 563, "Dark Willow", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGIyi3kajH_7bzM62PHWcql56-4PLuAm1EyL4k5vv7zZa4825baV9L_7cBmifxdEk4LM7cCC-mBI1pzyJpZj8IDvIMBhzDsciFuZY5ES-xoXhMenqtVTfiItEyy6viS4b6Cpv4roEBaF086GBkUifZmzryOWo"),
        ("Frosty the Sew-Man", "Immortal Bundle", "Immortal", "Bundle", 2627, "Pudge", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGIm52k-5XvHr0MmzMH6Z9hlz5ojn_lDxVA6jzsSzwjZU9uK7bKNobvPGCmSWxe0h5bI_Fny1lkUm42XVnIqgdX3CaAEnXpV4F7UOuhS7k9z5d7S1xv_hlhM"),
        ("Lumpo and Rupertus", "Legendary Bundle", "Legendary", "Bundle", 2729, "Ogre Magi", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGJGzyF-5RPPZ_NmtOm-o4Ftl4J66s1jiF0iizMLi_nsPt_esafA4c_PLCDbHw-sutbY9TH7gw08k622GysHpLyzhnw2XIg"),
        ("Jingle Fist Bundle", "Immortal Bundle", "Immortal", "Bundle", 2664, "Monkey King", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGIm52k-5XvHr0MmzMH6Z9hlz5ojn_lDxVA6jzsSzwjZO_PngP_w9dPWSDT7HxO8itrg-GS_lkUR_6jmDmdn7I3-UPQR1XJJzQu9YsQ74zIMcHg3psw"),
        ("Crownfall Treasure I", "Mythical Treasure chest", "Mythical", "Treasure Chest", 175, "", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGJG51EejH_XGzN2vM3qb6Wlm95Xx71jySiLljJPh7jdJ6rz7O6FsePbCCzKRwrpz6edvFi23wk0i6zvTzd76dXrEZgUpCZR0QOILrFDmxRCfiKj7"),
        ("Dead Reckoning Chest", "Rare Treasure chest", "Rare", "Treasure Chest", 30, "", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGJG51EejH-LGxsuyIGmS2llzy4Px613YUxjylZnu9CxcoaX8PadodKSQXT6UmLsi4uM5TXjlkR9ytm3Xmdz6di_CPAIjW5t5TO4U8k7vczduM-k"),
        ("Frostivus 2023 Treasure Chest", "Rare Treasure chest", "Rare", "Treasure Chest", 19, "", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGJG51EejH_DGzNm1PG2C9ml2_ILn_hfjEU2oncXjq3JZv6P4afZudKXAVjDIl-gm5uJrSnDhzU5z4mmBztmsbzvJOaPTTigK"),
        ("Hallowed Chest of the Diretide", "Rare Treasure chest", "Rare", "Treasure Chest", 9, "", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGJG51EejH-LGxsuyIGmS2lV98ZTg1V3uUxjll5Llr3Jk7OCvfKEjJvTDXzKSkL915bA8HC2ylk535WTcn9irJ3qWP1B0CZR5FuII50W7ksqnab2Wq9Ge1A"),
    ];
    let rarity_color = |r: &str| match r {
        "Immortal" => "E4AE39",
        "Legendary" => "D32CE6",
        "Mythical" => "8847FF",
        _ => "4B69FF",
    };
    let mut items = Vec::new();
    let mut prices = Vec::new();
    for (i, (name, kind, rarity, slot, cents, hero, icon)) in ITEMS.iter().enumerate() {
        let mut tags = vec![("Rarity".to_string(), rarity.to_string(), Some(rarity_color(rarity).to_string())), ("Type".to_string(), slot.to_string(), None)];
        if !hero.is_empty() {
            tags.push(("Hero".to_string(), hero.to_string(), None));
        }
        let copies = if slot.contains("Chest") { 3 } else { 1 };
        for c in 0..copies {
            items.push(InventoryItem {
                asset_id: 9000 + (i * 10 + c) as u64,
                class_id: 100 + i as u64,
                amount: 1,
                name: name.to_string(),
                kind: kind.to_string(),
                name_color: Some("D2D2D2".into()),
                icon: icon.to_string(),
                tradable: i != 2,
                marketable: i != 2,
                market_hash_name: name.to_string(),
                description: vec![("Used By: ".to_string() + if hero.is_empty() { "All heroes" } else { hero }, None)],
                tags: tags.clone(),
                context_id: 2,
                owner_notes: if i == 2 { vec!["Tradable/Marketable After Nov 14, 2026 (8:00:00) GMT".into()] } else { vec![] },
                trade_hold_days: 7,
                market_hold_days: 7,
                market_fee: Some(0.1),
                ..Default::default()
            });
        }
        // A wavy 60-day price history around the current price.
        let history = (0..60u64)
            .map(|d| {
                let f = 1.0 + 0.18 * ((d as f64) / 6.0).sin() + 0.002 * d as f64;
                (now - (60 - d) * 86_400, *cents as f64 / 100.0 * f, 5 + (d % 9) as u32)
            })
            .collect();
        prices.push(Event::MarketPrice {
            app_id: 570,
            hash_name: name.to_string(),
            result: Ok(MarketPrice {
                lowest: Some(model::format_money(*cents as i64, 1)),
                median: Some(model::format_money((*cents as f64 * 0.97) as i64, 1)),
                volume: Some(40 + i as u64 * 7),
                lowest_sell: Some(*cents),
                highest_buy: Some((*cents as f64 * 0.86) as u64),
                sell_count: Some(380 + i as u64 * 51),
                buy_count: Some(1200 + i as u64 * 97),
                fee_rules: None,
                currency: 1,
                sell_ladder: (0..5).map(|k| (cents + k * 2, 3 + k * 4)).collect(),
                buy_ladder: (0..5).map(|k| (((*cents as f64 * 0.86) as u64).saturating_sub(k * 2).max(1), 10 + k * 7)).collect(),
                history,
            }),
        });
    }
    let mut out = vec![
        Event::InventoryApps {
            steam_id: me,
            result: Ok(vec![
                InventoryApp { app_id: 570, name: "Dota 2".into(), icon: None, contexts: vec![(2, "Backpack".into(), items.len() as u32)] },
                InventoryApp { app_id: 753, name: "Steam".into(), icon: None, contexts: vec![(6, "Community".into(), 0)] },
            ]),
        },
        Event::Inventory(model::Inventory { steam_id: me, app_id: 570, context_id: 2, total: items.len() as u32, items, error: None }),
        Event::Inventory(model::Inventory { steam_id: me, app_id: 753, context_id: 6, ..Default::default() }),
        Event::MyListings(Ok(MyListings {
            listings: vec![
                MyListing { listing_id: 1, app_id: 570, name: "Hydrakan Latch".into(), game: "Dota 2".into(), icon: ITEMS[1].6.into(), name_color: Some("D2D2D2".into()), buyer_pays: 299, you_receive: 260, currency: 1, amount: 1, created: now - 7200, status: model::ListingStatus::Active, market_hash_name: "Hydrakan Latch".into() },
                MyListing { listing_id: 2, app_id: 570, name: "Dead Reckoning Chest".into(), game: "Dota 2".into(), icon: ITEMS[7].6.into(), name_color: Some("D2D2D2".into()), buyer_pays: 32, you_receive: 28, currency: 1, amount: 1, created: now - 600, status: model::ListingStatus::NeedsConfirmation, market_hash_name: "Dead Reckoning Chest".into() },
            ],
            buy_orders: vec![],
        })),
        Event::SupportTickets(Ok(vec![
            SupportTicket { id: "HT-DEMO-0001".into(), title: "Refund Request - Accidentally purchased twice".into(), area: "Dota 2".into(), icon: None, status: "Closed".into(), open: false, updated: "Oct 7".into(), url: "https://help.steampowered.com/en/wizard/HelpRequests".into() },
            SupportTicket { id: "HT-DEMO-0002".into(), title: "My game won't start after the update".into(), area: "Portal 2".into(), icon: None, status: "Waiting for you".into(), open: true, updated: "Nov 2".into(), url: "https://help.steampowered.com/en/wizard/HelpRequests".into() },
        ])),
        Event::SupportTicket {
            id: "HT-DEMO-0002".into(),
            result: Ok(vec![
                SupportMessage { from_steam: false, header: "Message from you on Nov 1 @ 9:12pm".into(), html: "Since the last update the game closes right after the logo.".into() },
                SupportMessage { from_steam: true, header: "Message from Steam Support on Nov 2 @ 10:03am".into(), html: "<b>Thanks for reaching out.</b><br>Please verify the game's files, then tell us if it still happens.<ul><li>Library → Portal 2 → Manage → Verify game files</li></ul>".into() },
            ]),
        },
        Event::PlayerGames {
            steam_id: me,
            result: Ok(vec![model::PlayerGame { app_id: 1145360, name: "Hades".into(), playtime_minutes: 2_710, playtime_2weeks: 185, last_played: Some(now - 3600), icon: None }]),
        },
        Event::Achievements {
            app_id: 1145360,
            result: Ok([
                ("Escaped Tartarus", "Clear Tartarus for the first time.", true, now - 3600, 92.1),
                ("Is There No Escape?", "Escape the Underworld for the first time.", true, now - 86_400 * 4, 64.3),
                ("Friends Forever", "Gain the trust of every one of Zagreus's friends.", false, 0, 18.4),
                ("Secret", "", false, 0, 7.9),
            ]
            .into_iter()
            .map(|(name, desc, unlocked, time, pct)| model::Achievement {
                name: name.into(),
                description: desc.into(),
                hidden: desc.is_empty(),
                unlocked,
                unlock_time: time,
                global_percent: pct,
                ..Default::default()
            })
            .collect()),
        },
        Event::Notifications(model::Notifications {
            items: vec![
                model::Notification { id: 1, kind: 4, time: now - 900, read: false, app_id: Some(753), source_app: Some(570), ..Default::default() },
                model::Notification { id: 2, kind: 8, time: now - 3600 * 5, read: false, app_id: Some(1145360), ..Default::default() },
                model::Notification { id: 3, kind: 11, time: now - 86_400, read: true, ticket: Some("HTDEMO0002".into()), ..Default::default() },
                model::Notification { id: 4, kind: 5, time: now - 86_400 * 3, read: true, from: Some(76561197960265728 + 103), ..Default::default() },
                model::Notification { id: 6, kind: 2, time: now - 600, read: false, app_id: Some(413150), from: Some(76561197960265728 + 100), ..Default::default() },
                model::Notification { id: 5, kind: 3, time: now - 86_400 * 6, read: true, from: Some(me), ..Default::default() },
                model::Notification { id: 7, kind: 28, time: now - 86_400 * 8, read: true, app_id: Some(1145360), ..Default::default() },
            ],
            unread: 3,
            pending_gifts: 1,
            pending_friends: 0,
            confirmations: 1,
        }),
    ];
    out.extend(prices);
    out
}

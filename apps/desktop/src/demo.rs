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
    // Steam's feature categories (1 multi-player, 2 single-player, 9 co-op,
    // 22 achievements, 28 full controller support), for filters and the
    // "Multiplayer" dynamic collection below.
    for a in apps.iter_mut() {
        a.categories = match a.app_id {
            440 | 570 | 730 | 1203220 => vec![1, 22],
            620 => vec![2, 9, 22, 28],
            1938090 => vec![1, 2, 22],
            _ => vec![2, 22],
        };
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
    // Long enough to scroll (chat screens), with one long message.
    let chat = [
        (ferris, "did you finish the last chapter?", 86_400 + 4_000),
        (me, "almost! stuck on the bridge part", 86_400 + 3_900),
        (ferris, "the trick is to place the portal on the far wall first, then jump while the light bridge is still moving. it took me like twenty tries before I noticed the panel behind the glass", 86_400 + 3_800),
        (me, "ohh that makes sense", 86_400 + 3_700),
        (me, "got it, thanks!", 86_400 + 3_000),
        (ferris, "nice", 86_400 + 2_900),
        (ferris, "want to try the co-op campaign this week?", 7_200),
        (me, "yes, any evening works", 7_000),
        (ferris, "cool", 6_900),
        (ferris, "hey, portal 2 co-op tonight? [emoticon]steamhappy[/emoticon]", 900),
        (me, "sure, after dinner", 840),
        (ferris, "nice. I'll send an invite around 8", 800),
        (ferris, "also, have you seen this one? https://store.steampowered.com/app/620/Portal_2/", 700),
        (me, "[quote=Ferris]have you seen this one?[/quote]played it twice [emoticon]steamthumbsup[/emoticon]", 650),
        (ferris, "the ending though... [spoiler]the cake is a lie[/spoiler]", 600),
        (ferris, "[code]sv_cheats 1[/code] works in the co-op maps too", 560),
        (me, r#"[random min="1" max="100" result="42"][/random]"#, 520),
        (ferris, r#"[flip result="heads"][/flip]"#, 500),
        (ferris, r#"[img src="https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/620/header.jpg" width="460" height="215"][/img]"#, 400),
        (me, "[emoticon]steamhappy[/emoticon][emoticon]steamthumbsup[/emoticon]", 300),
    ];
    // Reactions under a few of them.
    let reacted = |i: usize| -> Vec<model::Reaction> {
        match i {
            12 => vec![model::Reaction { name: "steamhappy".into(), sticker: false, count: 2, mine: true }],
            14 => vec![model::Reaction { name: "steammocking".into(), sticker: false, count: 1, mine: false }, model::Reaction { name: "steamfacepalm".into(), sticker: false, count: 1, mine: true }],
            _ => Vec::new(),
        }
    };
    let mut events = vec![
        Event::LoggedIn { steam_id: me, account: "demo".into() },
        Event::AccountInfo { persona_name: "You".into(), country: "US".into() },
        Event::Level(27),
        Event::Licenses { count: apps.len() },
        Event::Apps(apps),
        Event::Friends((0..friends.len()).map(|i| (BASE + 100 + i as u64, Relationship::Friend)).chain(friend_requests().iter().map(|(id, _, rel)| (*id, *rel))).collect()),
        Event::Personas(personas),
    ];
    events.push(Event::History {
        friend: ferris,
        messages: chat
            .iter()
            .enumerate()
            .map(|(i, (from, raw, ago))| {
                let (text, invite) = model::chat::parse(raw);
                ChatMessage { friend: ferris, from: *from, text, time: now - ago, invite, raw: (*raw).into(), ordinal: i as u32, reactions: reacted(i) }
            })
            .collect(),
    });
    events.push(Event::Profile(model::ProfileInfo {
        steam_id: me,
        background: Some(model::art::hero(1145360)),
        real_name: "Demo Player".into(),
        summary: "Collector of indie gems and long co-op nights.".into(),
        location: "Earth".into(),
        theme: "Midnight".into(),
        background_full: true,
        member_since: Some(now - 9 * 31_536_000),
        ..Default::default()
    }));
    events.push(Event::Wallet { balance: 1234, currency: 1, pending: 250 });
    events.push(Event::Message(ChatMessage { friend: BASE + 101, from: BASE + 101, text: "gg! one more?".into(), time: now - 30, ..Default::default() }));
    // The emoticons, stickers and chat effects "you" own.
    let owned = |n: &str| model::OwnedEmote { name: n.into(), app_id: 753, ..Default::default() };
    events.push(Event::Emoticons(model::Emoticons {
        emoticons: ["steamhappy", "steamthumbsup", "steammocking", "steamfacepalm", "steamsad", "steamthis", "steamsalty", "steambored"].iter().map(|n| owned(n)).collect(),
        stickers: Vec::new(),
        effects: ["confetti", "fireworks"].iter().map(|n| owned(n)).collect(),
    }));
    events.extend(group_chat(me, ferris, now));
    events.extend(owned_group_chat(me, ferris, now));
    events.push(Event::Personas(friend_requests().iter().map(|(id, name, _)| Persona { steam_id: *id, name: (*name).into(), state: PersonaState::Online, ..Default::default() }).collect()));
    events.extend(inventory(me, now));
    events.extend(crate::builtin::hub::demo::events());
    events.extend(crate::builtin::store::demo::events(now));
    events.extend(crate::builtin::community_home::demo_events());
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
        // A download in progress, to show install states in the library.
        Event::Download(model::DownloadStatus {
            app_id: 1145360,
            phase: model::DownloadPhase::Downloading,
            downloaded: 6_100_000_000,
            download_total: 13_400_000_000,
            disk_done: 6_100_000_000,
            disk_total: 13_400_000_000,
            speed: 3_200_000,
            install_dir: Some(std::path::PathBuf::from(r"C:\Games\SteamLibrary\steamapps\common\Hades")),
            ..Default::default()
        }),
        // Two more waiting, one set aside part-way.
        Event::Download(model::DownloadStatus {
            app_id: 413150,
            phase: model::DownloadPhase::Downloading,
            downloaded: 150_000_000,
            download_total: 500_000_000,
            disk_done: 150_000_000,
            disk_total: 500_000_000,
            ..Default::default()
        }),
        Event::DownloadStopped { app_id: 413150 },
        Event::DownloadQueue(vec![413150, 440]),
        // A dynamic collection, like one made in Steam from its filters
        // (Features: multiplayer).
        Event::SteamCollections(Ok(vec![model::SteamCollection {
            id: "uc-demo-multiplayer".into(),
            name: "Multiplayer".into(),
            games: vec![],
            dynamic: true,
            filter: Some(r#"{"nFormatVersion":2,"strSearchText":"","filterGroups":[{"rgOptions":[],"bAcceptUnion":true},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[8],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false},{"rgOptions":[],"bAcceptUnion":false}],"setSuggestions":{}}"#.into()),
        }])),
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
    // Human Fall Flat's Workshop tag groups, for the Workshop filters.
    let cat = |name: &str, single: bool, tags: &[&str]| model::WorkshopTagCategory { name: name.into(), single, tags: tags.iter().map(|t| (t.to_string(), t.to_string())).collect() };
    out.push(Event::WorkshopTags {
        app_id: 477160,
        result: Ok(vec![
            cat("", false, &["Human's Finest", "Levels", "Lobbies", "Model", "Aug's Selection", "Monthly Throwback", "New Selection"]),
            cat("Theme", false, &["Mansion", "Train", "Carry", "Mountain", "Demolition", "Castle", "Water", "Power Plant", "Aztec", "Dark", "Steam", "Ice", "Christmas", "Model - Animal", "Model - Anime / Manga", "Model - Cartoon / TV", "Model - Christmas", "Model - Comic Book", "Model - Film", "Model - Horror", "Model - Human", "Model - Sci-fi", "Model - Sport", "Model - Video Game", "Model - Other"]),
            cat("Type", true, &["Puzzle", "Platforming", "Story", "Misc", "Sandbox", "Hide and Seek"]),
            cat("Level Type", true, &["Multiplayer", "Singleplayer", "Speedrun"]),
            cat("Recommended Players", true, &["1 Player", "2 Players", "3 Players", "4 Players", "5 Players", "6 Players", "7 Players", "8 Players"]),
            cat("Difficulty", true, &["Beginner", "Intermediate", "Expert"]),
            cat("Language", true, &["English", "Spanish", "French", "German", "Russian", "Italian", "Chinese", "Japanese", "Korean", "Polish", "Portuguese", "Vietnamese"]),
        ]),
    });
    // Your profile settings and badges, for the profile pages (made up).
    out.push(Event::ProfileEdit(Ok(model::ProfileEdit {
        persona_name: "You".into(),
        custom_url: "demo-player".into(),
        // Several paragraphs and one long unbroken line, like real summaries.
        summary: "Collector of indie gems and long co-op nights.

Currently: replaying Hades on the hardest heat, slowly finishing Stardew Valley's community center, and trying every Portal 2 workshop map my friends send me.

Add me if you want to play co-op, trade cards or just chat about games. I'm usually online in the evenings (Europe).

https://steamcommunity.com/sharedfiles/filedetails/?id=1234567890&searchtext=co-op-night-collection-favourites

No trades for Steam keys, sorry.".into(),
        privacy: model::ProfilePrivacy { profile: 3, inventory: 3, gifts: 1, games: 3, playtime: 1, friends: 2, comments: 1 },
        // Made-up items you own, with game art standing in for their pictures.
        // As Steam sends them: ids with "#ProfileTheme_…" keys for titles.
        themes: ["Summer", "Midnight", "Steel", "Cosmic", "DarkMode"].iter().map(|n| (n.to_string(), format!("#ProfileTheme_{n}"))).collect(),
        theme: "Midnight".into(),
        backgrounds: [(1145360, "Hades Underworld", "Hades"), (413150, "Pelican Town", "Stardew Valley"), (620, "Aperture Labs", "Portal 2"), (440, "Mann Co. Workshop", "Team Fortress 2"), (570, "Dire Ancient", "Dota 2")]
            .iter()
            .enumerate()
            .map(|(i, (app, name, game))| model::ProfileItemChoice { id: 9100 + i as u64, name: (*name).into(), image: model::art::header(*app), large: model::art::hero(*app), app_id: *app, app_name: (*game).into(), ..Default::default() })
            .collect(),
        mini_backgrounds: [(1145360, "Elysium Fields", "Hades"), (413150, "Starry Farm", "Stardew Valley"), (730, "Dust II Dawn", "Counter-Strike 2")]
            .iter()
            .enumerate()
            .map(|(i, (app, name, game))| model::ProfileItemChoice { id: 9200 + i as u64, name: (*name).into(), image: model::art::header(*app), large: model::art::hero(*app), app_id: *app, app_name: (*game).into(), ..Default::default() })
            .collect(),
        animated_avatars: [(413150, "Junimo Hop", "Stardew Valley"), (1145360, "Zagreus", "Hades"), (620, "Turret Blink", "Portal 2"), (440, "Heavy Grin", "Team Fortress 2"), (570, "Roshan Roar", "Dota 2"), (730, "Bomb Planted", "Counter-Strike 2"), (292030, "White Wolf", "The Witcher 3: Wild Hunt")]
            .iter()
            .enumerate()
            .map(|(i, (app, name, game))| model::ProfileItemChoice { id: 9300 + i as u64, name: (*name).into(), image: model::art::capsule(*app), app_id: *app, app_name: (*game).into(), ..Default::default() })
            .collect(),
        equipped: [9100, 9200, 0, 0],
        background_full: true,
        favorite_badge: 2,
        ..Default::default()
    })));
    let badge = |title: &str, detail: &str| model::Badge { title: title.into(), detail: detail.into(), ..Default::default() };
    out.push(Event::Badges {
        steam_id: 76561197960265728 + 1,
        result: Ok(model::Badges {
            level: 23,
            xp: "2,410 XP".into(),
            next: "90 XP to reach Level 24".into(),
            progress: 0.6,
            items: vec![
                model::Badge { url: "https://steamcommunity.com/profiles/76561197960265729/badges/1".into(), ..badge("Years of Service", "300 XP") },
                model::Badge { url: "https://steamcommunity.com/profiles/76561197960265729/badges/2".into(), ..badge("Community Leader", "100 XP") },
                model::Badge { url: "https://steamcommunity.com/profiles/76561197960265729/badges/13".into(), ..badge("Pillar of Community", "100 XP") },
            ],
        }),
    });
    // Names for the wishlist's games (you don't own them).
    out.push(Event::AppNames(vec![(1245620, "ELDEN RING".into()), (1086940, "Baldur's Gate 3".into()), (367520, "Hollow Knight".into()), (1145350, "Hades II".into()), (504230, "Celeste".into()), (1091500, "Cyberpunk 2077".into())]));
    // Someone who isn't your friend (BASE + 300), for the profile's
    // "Add Friend" and "More" buttons.
    let stranger = 76561197960265728 + 300;
    out.push(Event::Personas(vec![model::Persona { steam_id: stranger, name: "Lumen".into(), state: model::PersonaState::Online, ..Default::default() }]));
    out.push(Event::Profile(model::ProfileInfo {
        steam_id: stranger,
        real_name: "Lumen Vale".into(),
        summary: "Speedrunner. Mostly platformers.".into(),
        theme: "Summer".into(),
        location: "Somewhere".into(),
        visibility: model::Visibility::Public,
        ..Default::default()
    }));
    out
}

/// A made-up group chat of friends, for the Friends & Chat window.
fn group_chat(me: u64, ferris: u64, now: u64) -> Vec<Event> {
    use model::{ChatGroup, ChatRoom, GroupMessage};
    let (nova, quartz) = (ferris + 1, ferris + 3);
    let group = 4_200_001;
    let said = [(ferris, "who's in for Portal 2 co-op tonight?", 3600), (quartz, "me! after 9", 3400), (nova, "I'll join after this match", 3300), (me, "count me in", 1500), (ferris, "great, I'll set it up", 1200), (quartz, "map for after: https://steamcommunity.com/sharedfiles/filedetails/?id=2177193451", 900)];
    let messages: Vec<GroupMessage> = said
        .iter()
        .enumerate()
        .map(|(i, (from, text, ago))| GroupMessage {
            group,
            room: 1,
            sender: *from,
            text: (*text).into(),
            time: (now - ago) as u32,
            ordinal: i as u32,
            reactions: if i == 0 { vec![model::Reaction { name: "steamthumbsup".into(), sticker: false, count: 3, mine: true }] } else { Vec::new() },
        })
        .collect();
    let last = messages.last().cloned().unwrap_or_default();
    vec![
        Event::ChatGroups(Ok(vec![ChatGroup {
            id: group,
            name: "Co-op Night".into(),
            default_room: 1,
            active_members: 3,
            rooms: vec![
                ChatRoom { id: 1, name: "general".into(), last_message: last.text.clone(), last_time: last.time, last_sender: last.sender, unread: true, ..Default::default() },
                ChatRoom { id: 2, name: "screenshots".into(), ..Default::default() },
            ],
            unread: true,
            ..Default::default()
        }])),
        Event::ChatGroupMembers(vec![(group, vec![me, ferris, nova, quartz])]),
        Event::GroupHistory { group, room: 1, result: Ok((messages, false)), older: false },
    ]
}

/// Made-up friend invites (sent and received) and blocked people, with long
/// names, for the Friends & Chat window's Invites and Blocked pages.
fn friend_requests() -> [(u64, &'static str, Relationship); 6] {
    const BASE: u64 = 76561197960265728;
    [
        (BASE + 200, "Maximilian_the_Extraordinarily_Long_Nickname", Relationship::RequestRecipient),
        (BASE + 201, "Orbit", Relationship::RequestRecipient),
        (BASE + 202, "Cobalt Wanderer of the Northern Servers", Relationship::RequestInitiator),
        (BASE + 203, "Spammy McSpamface the Third", Relationship::Blocked),
        (BASE + 204, "Grief", Relationship::Blocked),
        (BASE + 205, "xX_NoScope_Xx", Relationship::Ignored),
    ]
}

/// A sample Steam Cloud conflict holding a game's launch (the demo's
/// stand-in for starting a game from a link or a desktop shortcut).
pub fn cloud_conflict(app_id: model::AppId) -> Event {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let conflict = |name: &str, local_ago: u64, cloud_ago: u64, size: u64| model::CloudConflict { name: name.into(), local_time: now - local_ago, cloud_time: now - cloud_ago, local_size: size, cloud_size: size + 2_048 };
    let report = model::CloudReport {
        app_id,
        conflicts: vec![conflict("%WinMyDocuments%My Games/Demo/save1.sav", 3_600, 7_200, 148_224), conflict("%WinMyDocuments%My Games/Demo/settings.cfg", 3_600, 5_400, 2_048)],
        finished: now,
        ..Default::default()
    };
    Event::CloudReport { report, held_launch: Some(client::LaunchArgs::default()) }
}

/// A made-up group chat you own, with channels (one voice), roles, invite
/// links, an invited account and a ban, for the group settings dialogs.
/// Co-op Night (Ferris's) shows what a member without rights sees.
fn owned_group_chat(me: u64, ferris: u64, now: u64) -> Vec<Event> {
    use model::chat_admin::{AdminEvent, GroupAdmin, InviteLink, Listed, Member, Notify, Role, RolePerms};
    use model::{ChatGroup, ChatRoom, GroupMessage};
    const BASE: u64 = 76561197960265728;
    let group = 4_200_002;
    let (nova, pixel, quartz) = (ferris + 1, ferris + 2, ferris + 3);
    let rooms = vec![
        ChatRoom { id: 21, name: "Home".into(), sort: 0, ..Default::default() },
        ChatRoom { id: 22, name: "links".into(), sort: 1, ..Default::default() },
        ChatRoom { id: 23, name: "games (free or on sale)".into(), sort: 2, ..Default::default() },
        ChatRoom { id: 24, name: "Lounge".into(), voice: true, sort: 3, ..Default::default() },
    ];
    let hello = GroupMessage { group, room: 21, sender: nova, text: "welcome everyone :)".into(), time: (now - 7200) as u32, ordinal: 0, reactions: Vec::new() };
    let everyone = RolePerms { chat: true, mention_all: true, broadcasts: true, invite: true, change_details: true, channels: true, view_history: true, ..Default::default() };
    let moderators = RolePerms { kick: true, ban: true, assign_roles: true, ..everyone };
    let admin = GroupAdmin {
        group,
        owner: me,
        default_role: 1,
        roles: vec![Role { id: 1, name: "Everyone".into(), ordinal: 0, perms: everyone }, Role { id: 2, name: "Moderators".into(), ordinal: 1, perms: moderators }],
        members: [me, ferris, nova, pixel, quartz].iter().map(|m| Member { steam_id: *m, roles: if *m == nova { vec![2] } else { vec![] } }).collect(),
        notify: Notify::All,
        room_notify: vec![(22, Notify::Mentions)],
        dms_allowed: true,
        links: vec![
            InviteLink { code: "Z0uG8hG0".into(), creator: me, expires: (now + 86_400 - 600) as u32, room: 0 },
            InviteLink { code: "pQ7xR2mK".into(), creator: nova, expires: 0, room: 22 },
        ],
        invited: vec![Listed { steam_id: BASE + 201, by: me, time: (now - 3600) as u32, reason: String::new() }],
        bans: vec![Listed { steam_id: BASE + 204, by: nova, time: (now - 86_400 * 3) as u32, reason: String::new() }],
    };
    vec![
        Event::ChatAdmin(AdminEvent::GroupCreated {
            group: ChatGroup { id: group, name: "Odyssey of Friends".into(), tagline: "Cheap games and good company".into(), default_room: 21, active_members: 4, rooms, ..Default::default() },
            members: vec![me, ferris, nova, pixel, quartz],
        }),
        Event::ChatAdmin(AdminEvent::Loaded { group, result: Ok(admin) }),
        Event::GroupHistory { group, room: 21, result: Ok((vec![hello], false)), older: false },
        // Co-op Night is Ferris's: you're a member with the default role.
        Event::ChatAdmin(AdminEvent::Loaded {
            group: 4_200_001,
            result: Ok(GroupAdmin {
                group: 4_200_001,
                owner: ferris,
                default_role: 1,
                roles: vec![Role { id: 1, name: "Everyone".into(), ordinal: 0, perms: RolePerms { chat: true, view_history: true, ..Default::default() } }],
                members: [me, ferris, nova, quartz].iter().map(|m| Member { steam_id: *m, roles: vec![] }).collect(),
                notify: Notify::Mentions,
                ..Default::default()
            }),
        }),
    ]
}

/// The demo's chat answers itself: what you send shows up as if Steam had
/// taken it, and nothing leaves this computer.
pub fn echo(tx: &std::sync::mpsc::Sender<Event>, me: u64, cmd: client::Command) {
    use client::{ChatTarget, Command};
    static ORDINAL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(100);
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let ordinal = ORDINAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let say = |to: ChatTarget, text: String| match to {
        ChatTarget::Friend(friend) => {
            let (plain, invite) = model::chat::parse(&text);
            Event::Message(ChatMessage { friend, from: me, text: plain, time, invite, raw: text, ordinal, reactions: Vec::new() })
        }
        ChatTarget::Group { group, room } => Event::GroupMessage(model::GroupMessage { group, room, sender: me, text, time: time as u32, ordinal, reactions: Vec::new() }),
    };
    let event = match cmd {
        Command::SendChat { to, text } => say(to, text),
        Command::SendChatImage { to, .. } => say(to, model::chat::image("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/620/header.jpg", 460, 215)),
        Command::ReactToMessage { to, time, ordinal, reaction, sticker, add } => Event::MessageReaction { to, time, ordinal, reactor: me, reaction, sticker, add },
        Command::ReportMessage { .. } => Event::MessageReported(Ok(())),
        Command::LoadTradeLink { .. } => Event::TradeLink(Ok("https://steamcommunity.com/tradeoffer/new/?partner=1&token=demo".into())),
        _ => return,
    };
    let _ = tx.send(event);
}

//! `--demo` data for the Community home: a week of made-up friends'
//! activity, a page of the community-wide feed, developers' news and the
//! games you play. Pictures are the games' own art.
//!
//! `--page=community_home` (`--arg=explore`, `--arg=developers`,
//! `--arg=discussions`).

use client::Event;
use client::feed::{ExploreQuery, Feed, Post, PostKind, Section};
use client::hub::{News, Response};
use i18n::t;
use model::art;

const BASE: u64 = 76561197960265728;
/// A broadcast link for an id Steam has no account for: it opens the watch
/// page without ever asking a real person to broadcast.
const DEMO_BROADCAST: &str = "https://steamcommunity.com/broadcast/watch/76561199999999990";
/// The demo friends (`demo.rs`): Ferris, Nova, Pixel, Quartz, Juniper, Marlow.
const fn friend(i: u64) -> u64 {
    BASE + 100 + i
}

pub fn events() -> Vec<Event> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let lang = i18n::language().steam_name().to_string();
    let avatar = Some(art::avatar(None));
    let post = |kind, author: u64, app: u32, action: &str, text: &str, images: Vec<String>, when: &str| Post {
        kind,
        author,
        avatar: avatar.clone(),
        app_id: app,
        action: action.into(),
        text: text.into(),
        images,
        when: when.into(),
        ..Default::default()
    };
    // Steam writes these lines in the reader's language: so does the demo.
    let today: &str = t!("Today");
    let yesterday: &str = t!("Yesterday");
    let friends = vec![
        Post { title: "Found the cake. It was a lie".into(), votes_up: 12, comments: 3, file_id: 1001, url: "https://steamcommunity.com/sharedfiles/filedetails/?id=1001".into(), ..post(PostKind::Screenshot, friend(0), 620, &t!("posted a screenshot of {game}", game = "Portal 2"), "", vec![art::hero(620)], today) },
        Post {
            recommended: Some(true),
            hours: t!("{h} hrs on record", h = hours(1204.5)),
            votes_up: 37,
            funny: 6,
            comments: 4,
            ..post(PostKind::Review, friend(1), 570, &t!("reviewed {game}", game = "Dota 2"), "Ten years in and I still learn something every match.\n\nThe new hero is a blast. Matchmaking is quicker than it used to be, and the patch notes are worth reading.\n\n10/10 would lose to a courier again.", vec![], today)
        },
        Post { votes_up: 4, ..post(PostKind::Achievement, friend(2), 1145360, &t!("{name} achieved {game}", name = "Pixel", game = "Hades"), "Is There No Escape?\nEscape the Underworld for the first time.", vec![art::capsule(1145360)], today) },
        // Several at once: four listed, the rest counted ("+N more").
        Post {
            more: 12,
            ..post(
                PostKind::Achievement,
                friend(5),
                413150,
                &t!("{name} achieved {game}", name = "Marlow", game = "Stardew Valley"),
                "Greenhorn\nEarn 15,000g.\n\nCowpoke\nEarn 50,000g.\n\nHomesteader\nEarn 250,000g.\n\nMother Catch\nCatch 100 fish.\n\nGofer\nComplete 10 Help Wanted requests.\n\nA Big Help\nComplete 40 Help Wanted requests.",
                vec![art::capsule(413150), art::header(413150), art::capsule(413150), art::header(413150), art::capsule(413150), art::header(413150)],
                today,
            )
        },
        Post { title: "Autumn on the farm".into(), votes_up: 58, comments: 9, file_id: 1001, url: "https://steamcommunity.com/sharedfiles/filedetails/?id=1001".into(), ..post(PostKind::Artwork, friend(3), 413150, &t!("published artwork for {game}", game = "Stardew Valley"), "Painted over a weekend, all in pixels.", vec![art::hero(413150)], today) },
        post(PostKind::Status, friend(0), 0, &t!("{a} and {b} are now friends.", a = "Ferris", b = "Nova"), "", vec![], yesterday),
        Post { app_id: 292030, ..post(PostKind::Purchase, friend(4), 292030, &t!("now owns {game}", game = "The Witcher 3: Wild Hunt"), "You are Geralt of Rivia, mercenary monster slayer. Before you stands a war-torn, monster-infested continent you can explore at will.", vec![], yesterday) },
        Post { votes_up: 2, ..post(PostKind::Status, friend(5), 440, &t!("posted a status about {game}", game = "Team Fortress 2"), "Anyone up for a few rounds of Payload tonight? Bring snacks.", vec![], yesterday) },
        Post { title: "Cozy Cottage Interiors".into(), votes_up: 21, comments: 2, file_id: 1001, url: "https://steamcommunity.com/sharedfiles/filedetails/?id=1001".into(), ..post(PostKind::Workshop, friend(1), 413150, &t!("published a Workshop item for {game}", game = "Stardew Valley"), "Warm lights and more furniture for every house upgrade.", vec![art::capsule(413150)], yesterday) },
        post(PostKind::Status, friend(3), 1145360, &t!("{name} has added {game} to their wishlist.", name = "Quartz", game = "Hades"), "", vec![], yesterday),
        Post {
            kind: PostKind::Announcement,
            app_id: 1145360,
            game: "Hades".into(),
            author_name: "Hades".into(),
            title: "The Codex gets a search box".into(),
            text: "A small update with big quality of life: search the Codex, new controller glyphs and faster loading between rooms.".into(),
            images: vec![art::hero(1145360)],
            when: yesterday.into(),
            time: now - 86_400,
            ..Default::default()
        },
    ];
    let explore_posts = vec![
        Post { kind: PostKind::Screenshot, app_id: 292030, game: "The Witcher 3: Wild Hunt".into(), author_name: "Marlow".into(), author: friend(5), avatar: avatar.clone(), title: "Toussaint at dawn".into(), images: vec![art::hero(292030)], votes_up: 336, comments: 19, file_id: 1001, url: "https://steamcommunity.com/sharedfiles/filedetails/?id=1001".into(), time: now - 3 * 3600, ..Default::default() },
        Post {
            kind: PostKind::Review,
            app_id: 413150,
            game: "Stardew Valley".into(),
            author_name: "Juniper".into(),
            author: friend(4),
            avatar: avatar.clone(),
            recommended: Some(true),
            hours: format!("{} ({})", t!("{h} hrs on record", h = hours(2130.2)), t!("{h} hrs at review time", h = hours(2069.9))),
            text: "It's fine i guess lol\n\nedit: 10/10\nedit 2: 100/10\nedit 3: 1000/10".into(),
            votes_up: 374,
            funny: 6,
            comments: 2,
            time: now - 86_400 * 67,
            ..Default::default()
        },
        Post { kind: PostKind::Broadcast, app_id: 1145360, game: "Hades".into(), author_name: "Pixel".into(), author: friend(2), avatar: avatar.clone(), title: "Hades".into(), images: vec![art::hero(1145360)], views: 50_600, live: true, url: DEMO_BROADCAST.into(), ..Default::default() },
        Post { kind: PostKind::Guide, app_id: 1145360, game: "Hades".into(), author_name: "Nova".into(), author: friend(1), avatar: avatar.clone(), title: "Your first escape: a calm walkthrough".into(), text: "Everything a new player needs for the first clear, step by step: weapons, boons and when to spend your darkness.".into(), votes_up: 980, comments: 21, file_id: 2001, url: "https://steamcommunity.com/sharedfiles/filedetails/?id=2001".into(), time: now - 86_400 * 2, ..Default::default() },
        Post { kind: PostKind::Artwork, app_id: 620, game: "Portal 2".into(), author_name: "Quartz".into(), author: friend(3), avatar: avatar.clone(), title: "Wheatley, in ink".into(), images: vec![art::capsule(620)], votes_up: 155, comments: 7, file_id: 1001, url: "https://steamcommunity.com/sharedfiles/filedetails/?id=1001".into(), time: now - 86_400, ..Default::default() },
        Post { kind: PostKind::Video, app_id: 570, game: "Dota 2".into(), author_name: "Ferris".into(), author: friend(0), avatar: avatar.clone(), title: "Rampage in 40 seconds".into(), images: vec![art::hero(570)], votes_up: 88, comments: 12, file_id: 1001, url: "https://steamcommunity.com/sharedfiles/filedetails/?id=1001".into(), time: now - 86_400 * 3, ..Default::default() },
    ];
    let broadcasts: Vec<Post> = explore_posts.iter().filter(|p| p.kind == PostKind::Broadcast).cloned().collect();
    // Hades' hub (`--page=hub --arg=1145360`): a pinned announcement and a
    // discussion first, then the same kinds of posts as Explore, about Hades.
    let pinned = vec![
        Post {
            kind: PostKind::Announcement,
            app_id: 1145360,
            title: "Patch notes: quality of life update".into(),
            text: "This update makes the codex easier to search and fixes a crash when quitting during a boss fight.".into(),
            images: vec![art::header(1145360)],
            when: "12:00".into(),
            url: "https://steamcommunity.com/games/1145360/announcements/detail/1".into(),
            ..Default::default()
        },
        Post {
            kind: PostKind::Discussion,
            app_id: 1145360,
            author: friend(3),
            avatar: avatar.clone(),
            action: t!("General Discussions").into(),
            title: "Best keepsake for the final boss?".into(),
            text: "I keep switching between the cube and the owl. What do you all take into the last room?".into(),
            comments: 31,
            when: t!("{n} hr ago", n = 3),
            url: "https://steamcommunity.com/app/1145360/discussions/0/3001/".into(),
            ..Default::default()
        },
    ];
    let hades: Vec<Post> = pinned
        .into_iter()
        .chain(explore_posts.iter().cloned().map(|mut p| {
            p.app_id = 1145360;
            p.game = "Hades".into();
            if !p.images.is_empty() {
                p.images = vec![art::hero(1145360)];
            }
            p
        }))
        .collect();
    let hub_feed = Event::Hub(Response::Explore { key: ExploreQuery { section: Section::All, recent: false, page: 1, language: lang.clone(), app: 1145360, search: String::new() }.key(), result: Ok(Feed { posts: hades, next: 0 }) });
    let explore = |section, posts: Vec<Post>| Event::Hub(Response::Explore { key: ExploreQuery { section, recent: false, page: 1, language: lang.clone(), ..Default::default() }.key(), result: Ok(Feed { posts, next: 0 }) });
    let news = |app: u32, items: &[(&str, &str, u64)]| Event::Hub(Response::News {
        app_id: app,
        result: Ok(items
            .iter()
            .enumerate()
            .map(|(i, (title, body, days))| News { id: format!("{app}-{i}"), title: (*title).into(), contents: format!("[img]{}[/img]\n{body}", if i == 0 { art::hero(app) } else { art::header(app) }), feed: "Community Announcements".into(), time: now - 86_400 * days, ..Default::default() })
            .collect()),
    });
    let game = |app_id, name: &str, total, recent, ago| model::PlayerGame { app_id, name: name.into(), playtime_minutes: total, playtime_2weeks: recent, last_played: Some(now - ago), icon: None };
    let item = |name: &str, _kind: &str, color: &str, icon: &str, listings, price: &str| model::MarketResult {
        app_id: 570,
        app_name: "Dota 2".into(),
        name: name.into(),
        hash_name: name.into(),
        icon: art::econ(icon, 96),
        color: color.into(),
        listings,
        price: price.into(),
    };
    let market = Event::MarketSearch {
        query: String::new(),
        app: None,
        start: 0,
        result: Ok((
            vec![
                item("Arms of Desolation", "Immortal", "B28A33", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGIyi3kajH-Xcws6uIkSR7FN78Mj1-FT0fhn0jZmv_DBW_M2qbbdibvHHDTLJlLZ3s7NvFyjqzE1z5znVm9uucHiWO1coCMF1RbRZsRW6l9H5d7S1tUgGsHE", 1_138, "$11.38"),
                item("Hydrakan Latch", "Immortal", "B28A33", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGIyi3kajH-XYwtiqenOO4UR0_4b61VXmVR750Zvl7ioU5-uqeq9sLs-fD3OTydF7v-VsQ2fll0wi6mrdnNipJC6ebg4nDZYkEeQKtBPsl9W1YuK2sQLc2NhAmCX2kGoXufdXeDr7", 289, "$2.89"),
                item("Crownfall Treasure I", "Mythical", "8847FF", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGJG51EejH_XGzN2vM3qb6Wlm95Xx71jySiLljJPh7jdJ6rz7O6FsePbCCzKRwrpz6edvFi23wk0i6zvTzd76dXrEZgUpCZR0QOILrFDmxRCfiKj7", 19_400, "$1.75"),
                item("Dead Reckoning Chest", "Rare", "4B69FF", "i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttydbPaERSR0Wqmu7LAocGJG51EejH-LGxsuyIGmS2llzy4Px613YUxjylZnu9CxcoaX8PadodKSQXT6UmLsi4uM5TXjlkR9ytm3Xmdz6di_CPAIjW5t5TO4U8k7vczduM-k", 190_000, "$0.30"),
            ],
            248,
        )),
    };
    // Stardew Valley's Workshop (`--page=workshop --arg=413150`).
    let mod_item = |id: u64, title: &str, pic: String, subs: u64, up: u32, tags: &[&str]| model::WorkshopEntry {
        id,
        title: title.into(),
        preview: Some(pic),
        description: "A made-up Workshop item for the demo.".into(),
        size: 12_000_000 + id * 1_000,
        subscriptions: subs,
        favorited: subs / 12,
        updated: now - 86_400 * (id % 30),
        votes_up: up,
        votes_down: up / 9,
        tags: tags.iter().map(|t| t.to_string()).collect(),
    };
    let workshop = vec![
        Event::WorkshopTags {
            app_id: 413150,
            result: Ok(vec![
                model::WorkshopTagCategory { name: "Type".into(), single: false, tags: ["Farm Maps", "Furniture", "Portraits", "Gameplay", "Fixes", "Graphics", "Overhaul", "Seasons"].iter().map(|t| (t.to_string(), t.to_string())).collect() },
                model::WorkshopTagCategory { name: "Content".into(), single: true, tags: ["Items", "Collections"].iter().map(|t| (t.to_string(), t.to_string())).collect() },
            ]),
        },
        Event::WorkshopResults {
            request_id: 0,
            total: 140,
            items: vec![
                mod_item(1001, "Cozy Cottage Interiors", art::hero(413150), 98_400, 5_000, &["Furniture", "Graphics"]),
                mod_item(1002, "Seasonal Outfits", art::capsule(413150), 61_200, 3_100, &["Portraits", "Seasons"]),
                mod_item(1003, "Quality of Life Fixes", art::header(413150), 240_000, 12_400, &["Fixes", "Gameplay"]),
                mod_item(1004, "Riverside Farm Map", art::hero(413150), 33_900, 1_480, &["Farm Maps"]),
                mod_item(1005, "Full Overhaul: Autumn", art::capsule(413150), 18_700, 920, &["Overhaul", "Seasons", "Graphics"]),
            ],
        },
    ];
    let mut out = workshop;
    out.extend(vec![
        market,
        Event::Hub(Response::Activity { start: 0, result: Ok(Feed { posts: friends, next: 0 }) }),
        explore(Section::All, explore_posts),
        // The store page's "live now" strip for Hades.
        Event::Hub(Response::Explore {
            key: ExploreQuery { section: Section::Broadcasts, page: 1, language: i18n::language().steam_name().to_string(), app: 1145360, ..Default::default() }.key(),
            result: Ok(Feed { posts: broadcasts.clone(), next: 0 }),
        }),
        explore(Section::Broadcasts, broadcasts),
        hub_feed,
        news(620, &[("Portal 2 turns another year older", "Thank you for testing with us. A weekend of free co-op maps starts Friday.", 2)]),
        news(570, &[("Gameplay update 7.40", "Map changes, a new neutral item tier and balance changes for most heroes.", 1), ("The International: tickets", "Tickets go on sale next week.", 12)]),
        news(413150, &[("Version 1.7 is in the works", "A first look at what's coming to the valley.", 4)]),
        news(440, &[("Summer event", "New maps, new cosmetics and a returning game mode.", 6)]),
        Event::PlayerGames {
            steam_id: BASE + 1,
            result: Ok(vec![
                game(1145360, "Hades", 2_710, 185, 3600),
                game(413150, "Stardew Valley", 9_400, 120, 86_400),
                game(570, "Dota 2", 72_270, 300, 86_400 * 2),
                game(620, "Portal 2", 1_200, 0, 86_400 * 9),
                game(440, "Team Fortress 2", 4_300, 0, 86_400 * 20),
                game(292030, "The Witcher 3: Wild Hunt", 6_100, 0, 86_400 * 40),
                game(730, "Counter-Strike 2", 900, 0, 86_400 * 60),
            ]),
        },
    ]);
    out
}

/// Hours the way the reader writes numbers ("1,204.5", "1 204,5").
fn hours(h: f32) -> String {
    let (thousands, decimal) = match i18n::language().steam_name() {
        "english" | "schinese" => (",", "."),
        "russian" | "french" => ("\u{a0}", ","),
        _ => (".", ","),
    };
    let whole = h.trunc() as u64;
    let tenth = ((h - h.trunc()) * 10.0).round() as u64;
    let digits = whole.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push_str(thousands);
        }
        out.push(c);
    }
    format!("{out}{decimal}{tenth}")
}

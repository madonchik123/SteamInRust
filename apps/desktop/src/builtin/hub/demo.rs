//! `--demo` data for the community pages: Hades' hub, guides, a guide,
//! discussions with a thread, a Workshop item (Stardew Valley) and a
//! friend's achievements to compare with. Pictures are the games' own art.
//!
//! `--page=hub --arg=1145360`, `--page=guides --arg=1145360`,
//! `--page=workshop_item --arg=1001` (item) or `--arg=2001` (guide),
//! `--page=discussions --arg=1145360`, `--page=thread --arg=` + [`THREAD`],
//! `--page=achievements --arg=1145360/76561197960265828`.

use super::{ListState, file_query};
use client::Event;
use client::hub::{
    ChangeNote, FileEntry, FileKind, FilePage, Forum, Guide, Item, News, Post, Preview, Response,
    Section, Thread, Topic, TopicPage,
};
use model::art;

pub const THREAD: &str = "https://steamcommunity.com/app/1145360/discussions/0/3001/";
const BASE: u64 = 76561197960265728;
const HADES: u32 = 1145360;
const STARDEW: u32 = 413150;

fn entry(
    id: u64,
    app: u32,
    title: &str,
    pic: String,
    creator: u64,
    up: u32,
    down: u32,
    desc: &str,
    now: u64,
) -> FileEntry {
    FileEntry {
        id,
        app_id: app,
        title: title.into(),
        preview: Some(pic.clone()),
        image: Some(pic),
        description: desc.into(),
        creator,
        size: 48_000_000 + id * 1_000,
        subscriptions: 12_000 + id * 37,
        favorited: 900 + id * 3,
        views: 40_000 + id * 11,
        created: now - 86_400 * (30 + id % 200),
        updated: now - 86_400 * (id % 9 + 1),
        votes_up: up,
        votes_down: down,
        comments: (id % 40) as u32,
        ..Default::default()
    }
}

pub fn events() -> Vec<Event> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let lang = i18n::language().steam_name().to_string();
    let l = ListState::default();
    let friend = |i: u64| BASE + 100 + i;
    let pics = [HADES, STARDEW, 620, 292030, 570, 440, 730, 1203220];
    let mut out = Vec::new();
    let files = |kind: FileKind, per: u32, items: Vec<FileEntry>| {
        Event::Hub(Response::Files {
            key: file_query(HADES, kind, &l, per, 1, lang.clone()).key(),
            result: Ok(FilePage {
                total: items.len() as u64,
                items,
            }),
        })
    };

    let shots: Vec<FileEntry> = pics
        .iter()
        .enumerate()
        .map(|(i, p)| {
            entry(
                500 + i as u64,
                HADES,
                [
                    "Boss down on the first try",
                    "Fishing spot",
                    "The lounge after a long night",
                    "",
                    "Max heat run",
                    "Garden view",
                    "Final room",
                    "Mirror build",
                ][i],
                art::hero(*p),
                friend(i as u64 % 6),
                300 - i as u32 * 20,
                4,
                "",
                now,
            )
        })
        .collect();
    let art_items: Vec<FileEntry> = pics
        .iter()
        .rev()
        .take(4)
        .enumerate()
        .map(|(i, p)| {
            entry(
                600 + i as u64,
                HADES,
                [
                    "Fan art: the house",
                    "Sketchbook page",
                    "Pixel tribute",
                    "Charcoal study",
                ][i],
                art::capsule(*p),
                friend(i as u64 + 1),
                120 - i as u32 * 10,
                2,
                "",
                now,
            )
        })
        .collect();
    let guides: Vec<FileEntry> = [
        (
            "Your first escape: a calm walkthrough",
            "Everything a new player needs for the first clear, step by step.",
            980,
            21,
        ),
        (
            "Every weapon aspect, ranked",
            "Which aspects carry which builds, with sample runs.",
            760,
            40,
        ),
        (
            "100% achievement checklist",
            "A route that picks up every achievement with the least grinding.",
            540,
            9,
        ),
        (
            "Fishing locations by region",
            "Where every catch hides and which rod spot works best.",
            310,
            12,
        ),
        (
            "Heat settings explained",
            "What each pact condition does and how to stack them.",
            220,
            30,
        ),
        (
            "Keepsakes and when to swap them",
            "A short reference card for the keepsake rack.",
            150,
            6,
        ),
    ]
    .iter()
    .enumerate()
    .map(|(i, (title, desc, up, down))| {
        entry(
            2001 + i as u64,
            HADES,
            title,
            art::header(pics[i % pics.len()]),
            friend(i as u64 % 6),
            *up,
            *down,
            desc,
            now,
        )
    })
    .collect();
    let mut video = entry(
        700,
        HADES,
        "Speedrun commentary",
        art::hero(1203220),
        friend(3),
        88,
        1,
        "",
        now,
    );
    video.image = None;
    out.push(files(FileKind::Screenshots, 12, shots.clone()));
    out.push(files(FileKind::Screenshots, 24, shots));
    out.push(files(FileKind::Art, 8, art_items.clone()));
    out.push(files(FileKind::Art, 24, art_items));
    out.push(files(
        FileKind::Guides,
        4,
        guides.iter().take(4).cloned().collect(),
    ));
    out.push(files(FileKind::Guides, 20, guides.clone()));
    out.push(files(FileKind::Videos, 4, vec![video.clone()]));
    out.push(files(FileKind::Videos, 24, vec![video]));
    out.push(files(FileKind::Items, 4, vec![]));

    // A guide with sections.
    let mut g = guides[0].clone();
    g.tags = vec![
        "Gameplay Basics".into(),
        "Walkthroughs".into(),
        "English".into(),
    ];
    out.push(Event::Hub(Response::Item {
        file_id: 2001,
        result: Ok(Item {
            entry: g,
            app_name: "Hades".into(),
            description: "Everything a new player needs for the first clear.".into(),
            file_type: 9,
            my_vote: Some(true),
            ..Default::default()
        }),
    }));
    out.push(Event::Hub(Response::Guide {
        file_id: 2001,
        result: Ok(Guide {
            title: "Your first escape: a calm walkthrough".into(),
            authors: "Ferris".into(),
            overview: "Everything a new player needs for the <b>first clear</b>, step by step. No spoilers past the first ending.".into(),
            sections: vec![
                Section { title: "Before you start".into(), html: "Take your time in the house between runs. Talk to everyone: conversations unlock upgrades.<br><br><div class=\"bb_h2\">What to buy first</div><ul class=\"bb_ul\"><li>The extra death defiance from the mirror</li><li>Cheaper healing in shops</li><li>More gold from rooms</li></ul>".into() },
                Section { title: "Picking a weapon".into(), html: format!("The starting sword is a fine choice. If a run feels slow, try the bow from a distance.<br><img src=\"{}\"><br><blockquote class=\"bb_blockquote\"><div class=\"bb_quoteauthor\">Originally posted by <b>Nova</b>:</div>Dash-strike builds are the easiest to learn.</blockquote>", art::hero(HADES)) },
                Section { title: "Boss tips".into(), html: "Watch for the tell before each big attack and dash <i>through</i> it, not away. <span class=\"bb_spoiler\"><span>The last boss has a second phase.</span></span><br>More in the <a href=\"https://steamcommunity.com/app/1145360/discussions/0/3001/\" class=\"bb_link\">discussion thread</a>.".into() },
            ],
        }),
    }));

    // A Workshop item for Stardew Valley.
    let mut item = entry(
        1001,
        STARDEW,
        "Cozy Farmhouse Interiors",
        art::hero(STARDEW),
        friend(0),
        2_480,
        61,
        "",
        now,
    );
    item.tags = vec!["Furniture".into(), "Decor".into(), "Content Patcher".into()];
    item.views = 182_440;
    let required = vec![
        entry(
            1002,
            STARDEW,
            "Content Patcher",
            art::header(STARDEW),
            friend(2),
            9_000,
            40,
            "",
            now,
        ),
        entry(
            1003,
            STARDEW,
            "SMAPI Helpers",
            art::capsule(STARDEW),
            friend(3),
            4_100,
            22,
            "",
            now,
        ),
    ];
    out.push(Event::Hub(Response::Item {
        file_id: 1001,
        result: Ok(Item {
            entry: item,
            app_name: "Stardew Valley".into(),
            description: "[h1]Cozy Farmhouse Interiors[/h1]\nNew wallpapers, floors and [b]42 pieces of furniture[/b] for every farmhouse upgrade.\n\n[h2]Install[/h2]\n[list]\n[*]Subscribe, then start the game once.\n[*]Find the new items at the carpenter's shop.\n[/list]\n[quote=Ferris]Works with every farm map.[/quote]\nQuestions? See the [url=https://steamcommunity.com/app/413150/discussions/]discussions[/url].".into(),
            previews: vec![Preview { url: Some(art::hero(STARDEW)), youtube: None }, Preview { url: Some(art::header(STARDEW)), youtube: None }, Preview { url: Some(art::capsule(STARDEW)), youtube: None }],
            required,
            change_notes: vec![
                ChangeNote { time: now - 86_400 * 2, text: "Added winter wallpapers.\nFixed a rug that floated above the floor.".into() },
                ChangeNote { time: now - 86_400 * 40, text: "First release.".into() },
            ],
            change_total: 2,
            lifetime_subscriptions: 61_200,
            file_type: 0,
            ..Default::default()
        }),
    }));

    // Discussions.
    let forums = vec![
        Forum {
            name: "General Discussions".into(),
            path: "discussions/0/".into(),
            threads: 18_204,
        },
        Forum {
            name: "Bug Reports".into(),
            path: "discussions/1/".into(),
            threads: 2_301,
        },
        Forum {
            name: "Trading".into(),
            path: "tradingforum/".into(),
            threads: 41,
        },
    ];
    let topics: Vec<Topic> = [
        ("Community guidelines", "Moderator", 0, true, true),
        (
            "How do I unlock the third weapon?",
            "Pixel",
            12,
            false,
            false,
        ),
        (
            "Best keepsake for the final boss?",
            "Quartz",
            31,
            false,
            false,
        ),
        (
            "Controller vibration stops after an hour",
            "Juniper",
            4,
            false,
            false,
        ),
        ("Finally beat 32 heat!", "Nova", 57, false, false),
        (
            "Fishing is my favorite part, change my mind",
            "Marlow",
            19,
            false,
            false,
        ),
    ]
    .iter()
    .enumerate()
    .map(|(i, (title, author, replies, pinned, locked))| Topic {
        id: 3000 + i as u64,
        title: (*title).into(),
        author: (*author).into(),
        replies: *replies,
        last_post: now - 600 * (i as u64 * 7 + 1),
        pinned: *pinned,
        locked: *locked,
        preview: "Short preview of the first post…".into(),
        url: format!(
            "https://steamcommunity.com/app/1145360/discussions/0/{}/",
            3000 + i
        ),
    })
    .collect();
    out.push(Event::Hub(Response::Topics {
        key: client::hub::topics_key(HADES, "", 1),
        result: Ok(TopicPage {
            forum: "General Discussions".into(),
            total: 247,
            page_size: 15,
            topics,
            forums,
        }),
    }));
    let post = |who: u64, name: &str, ago: u64, html: &str| Post {
        id: ago,
        author: friend(who),
        name: name.into(),
        avatar: None,
        time: now - ago,
        html: html.into(),
    };
    out.push(Event::Hub(Response::Thread {
        key: client::hub::thread_key(THREAD, 1),
        result: Ok(Thread {
            title: "How do I unlock the third weapon?".into(),
            forum: "General Discussions".into(),
            forum_url: "https://steamcommunity.com/app/1145360/discussions/0/".into(),
            op: post(2, "Pixel", 7200, "I have the sword and the spear. Where does the <b>third</b> one come from?"),
            posts: vec![
                post(1, "Nova", 6400, "You buy it with the keys you find during runs, from the weapon rack in the courtyard."),
                post(3, "Quartz", 5000, "<blockquote class=\"bb_blockquote\"><div class=\"bb_quoteauthor\">Originally posted by <b>Nova</b>:</div>from the weapon rack</blockquote>This. Keys also drop from some reward rooms."),
                post(2, "Pixel", 4100, "Got it, thanks both!"),
            ],
            total: 3,
            page_size: 15,
            locked: false,
        }),
    }));

    out.push(Event::Hub(Response::News {
        app_id: HADES,
        result: Ok(vec![
            News { id: "n1".into(), title: "Patch notes: quality of life update".into(), url: String::new(), author: "Dev team".into(), contents: format!("[img]{}[/img]\nThis update makes the codex easier to search and fixes a crash when quitting during a boss fight.\n[list][*]Faster loading between rooms[*]New controller glyphs[/list]", art::hero(HADES)), feed: "Community Announcements".into(), time: now - 86_400 * 3, external: false },
            News { id: "n2".into(), title: "Soundtrack now on streaming services".into(), url: String::new(), author: String::new(), contents: "The full soundtrack is out now.".into(), feed: "Community Announcements".into(), time: now - 86_400 * 30, external: false },
        ]),
    }));

    // Ferris' achievements, to compare with yours (the names match the demo's own list).
    out.push(Event::Hub(Response::Achievements {
        steam_id: friend(0),
        app_id: HADES,
        result: Ok([
            ("Escaped Tartarus", true, now - 86_400 * 20),
            ("Is There No Escape?", true, now - 86_400 * 9),
            ("Friends Forever", true, now - 86_400 * 2),
            ("Secret", false, 0),
        ]
        .into_iter()
        .map(|(name, unlocked, time)| model::Achievement {
            name: name.into(),
            unlocked,
            unlock_time: time,
            ..Default::default()
        })
        .collect()),
    }));
    out
}

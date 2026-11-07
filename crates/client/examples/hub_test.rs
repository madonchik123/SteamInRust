//! Live check of the community pages: `cargo run -p client --example hub_test [appid]`
//! Signs in with the saved account and only reads: a game's guides,
//! screenshots, Workshop items, one item and one guide, the forums and a
//! thread, news, someone's files and achievements (yours and a friend's).

use client::hub::{FileKind, FileQuery, Request, Response};
use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};

fn main() {
    tracing_subscriber::fmt().with_env_filter("warn").init();
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;
    let app: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(413150);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });
    let hub = |r: Request| Command::Hub(r);
    let q = |kind, sort, per| FileQuery { app_id: app, kind, sort, days: 7, page: 1, per_page: per, language: "english".into(), ..Default::default() };
    let mut me = 0;
    let mut friend = None;
    let mut asked_item = false;
    let mut asked_guide = false;
    let mut asked_thread = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        match e {
            Event::LoggedIn { steam_id, .. } => {
                me = steam_id;
                println!("signed in");
                c.send(hub(Request::Files(q(FileKind::Guides, 3, 10))));
                c.send(hub(Request::Files(q(FileKind::Screenshots, 3, 10))));
                c.send(hub(Request::Files(q(FileKind::Items, 3, 10))));
                c.send(hub(Request::Topics { app_id: app, forum: String::new(), page: 1, language: "english".into() }));
                c.send(hub(Request::News { app_id: app, count: 3 }));
                c.send(hub(Request::UserFiles { steam_id: me, kind: FileKind::Screenshots, page: 1, language: "english".into() }));
                c.send(Command::LoadPlayerGames { steam_id: me, language: "english".into() });
            }
            // Achievements: your most played game, for you and a friend.
            Event::PlayerGames { result: Ok(games), .. } => {
                if let Some(top) = games.iter().max_by_key(|g| g.playtime_minutes) {
                    println!("most played: {} ({})", top.name, top.app_id);
                    c.send(hub(Request::Achievements { steam_id: me, app_id: top.app_id, language: "english".into() }));
                    if let Some(f) = friend {
                        c.send(hub(Request::Achievements { steam_id: f, app_id: top.app_id, language: "english".into() }));
                    }
                }
            }
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::Friends(list) if friend.is_none() => {
                friend = list.iter().find(|(_, r)| *r == model::Relationship::Friend).map(|(id, _)| *id);
            }
            Event::Hub(r) => match r {
                Response::Files { key, result } => match result {
                    Ok(p) => {
                        println!("files {}: {} of {}", &key[..key.len().min(70)], p.items.len(), p.total);
                        for i in p.items.iter().take(3) {
                            println!("   {} {:?} by {} up {} down {} preview {}", i.id, i.title, i.creator, i.votes_up, i.votes_down, i.preview.is_some());
                        }
                        if key.contains("Guides") && !asked_guide {
                            if let Some(g) = p.items.first() {
                                asked_guide = true;
                                c.send(hub(Request::Guide { file_id: g.id, language: "english".into() }));
                                c.send(hub(Request::Item { file_id: g.id, language: "english".into() }));
                                c.send(hub(Request::FileComments { file_id: g.id, creator: g.creator, start: 0 }));
                            }
                        }
                        if key.contains("Items") && !asked_item {
                            if let Some(i) = p.items.first() {
                                asked_item = true;
                                c.send(hub(Request::Item { file_id: i.id, language: "english".into() }));
                            }
                        }
                    }
                    Err(e) => println!("files {key}: ERROR {e}"),
                },
                Response::Item { file_id, result } => match result {
                    Ok(i) => println!("item {file_id}: {:?} type {} previews {} required {} notes {}/{} vote {:?} tags {:?} desc {} chars", i.entry.title, i.file_type, i.previews.len(), i.required.len(), i.change_notes.len(), i.change_total, i.my_vote, i.entry.tags, i.description.len()),
                    Err(e) => println!("item {file_id}: ERROR {e}"),
                },
                Response::Guide { file_id, result } => match result {
                    Ok(g) => println!("guide {file_id}: {:?} by {:?}, {} sections, overview {} chars", g.title, g.authors, g.sections.len(), g.overview.len()),
                    Err(e) => println!("guide {file_id}: ERROR {e}"),
                },
                Response::FileComments { file_id, result } => match result {
                    Ok(c) => println!("comments {file_id}: {} of {}", c.items.len(), c.total),
                    Err(e) => println!("comments {file_id}: {e}"),
                },
                Response::Topics { key, result } => match result {
                    Ok(p) => {
                        println!("forum {key}: {:?} {} topics of {}, forums {:?}", p.forum, p.topics.len(), p.total, p.forums.iter().map(|f| &f.name).collect::<Vec<_>>());
                        if let Some(t) = p.topics.iter().find(|t| !t.pinned && t.replies > 0) {
                            if !asked_thread {
                                asked_thread = true;
                                c.send(hub(Request::Thread { url: t.url.clone(), page: 1, language: "english".into() }));
                            }
                        }
                    }
                    Err(e) => println!("forum {key}: ERROR {e}"),
                },
                Response::Thread { key, result } => match result {
                    Ok(t) => println!("thread {key}: {:?} op {:?} {} posts of {}", t.title, t.op.name, t.posts.len(), t.total),
                    Err(e) => println!("thread {key}: ERROR {e}"),
                },
                Response::News { result, .. } => match result {
                    Ok(n) => println!("news: {:?}", n.iter().map(|n| &n.title).collect::<Vec<_>>()),
                    Err(e) => println!("news: ERROR {e}"),
                },
                Response::Achievements { steam_id, result, .. } => match result {
                    Ok(l) => println!("achievements of {}: {}/{}", if steam_id == me { "me".to_string() } else { "a friend".to_string() }, l.iter().filter(|a| a.unlocked).count(), l.len()),
                    Err(e) => println!("achievements of {}: {e}", if steam_id == me { "me" } else { "a friend" }),
                },
                other => println!("{other:?}"),
            },
            _ => {}
        }
    }
}

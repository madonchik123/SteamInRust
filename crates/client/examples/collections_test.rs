//! Live check of collection sync: `cargo run -p client --example collections_test`
//! Signs in with the saved account, makes a throwaway collection
//! ("SteamInRust sync test"), renames it, checks Steam has each change, then
//! deletes it. Your other collections aren't touched.
//! With `list`, it only prints what Steam has.

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt().with_env_filter("warn").init();
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });

    let next = |want: &dyn Fn(&Event) -> bool| -> Event {
        loop {
            let e = rx.recv_timeout(Duration::from_secs(45)).expect("timed out");
            if let Event::LoginFailed(e) = &e {
                panic!("login failed: {e}");
            }
            if want(&e) {
                return e;
            }
        }
    };
    let load = || -> Vec<model::SteamCollection> {
        c.send(Command::LoadSteamCollections);
        match next(&|e| matches!(e, Event::SteamCollections(_))) {
            Event::SteamCollections(r) => r.expect("download"),
            _ => unreachable!(),
        }
    };
    let save = |edit: model::CollectionEdit| {
        c.send(Command::SaveSteamCollections(vec![edit]));
        match next(&|e| matches!(e, Event::SteamCollectionsSaved(_))) {
            Event::SteamCollectionsSaved(r) => r.expect("upload"),
            _ => unreachable!(),
        }
    };

    next(&|e| matches!(e, Event::LoggedIn { .. }));
    let before = load();
    println!("signed in: {} collections on Steam", before.len());
    if std::env::args().nth(1).as_deref() == Some("list") {
        for x in &before {
            println!("  {} \"{}\" {} games{} {:?}", x.id, x.name, x.games.len(), if x.dynamic { " (dynamic)" } else { "" }, x.games);
        }
        return;
    }

    let id = client::collections::new_id();
    let mut edit = model::CollectionEdit { id: id.clone(), name: "SteamInRust sync test".into(), games: vec![570, 620], ..Default::default() };
    save(edit.clone());
    let after = load();
    let made = after.iter().find(|x| x.id == id).expect("Steam doesn't have the new collection");
    assert_eq!(made.name, "SteamInRust sync test");
    assert_eq!(made.games, vec![570, 620]);
    println!("created: ok ({} games)", made.games.len());

    edit.name = "SteamInRust sync test (renamed)".into();
    edit.games = vec![620];
    save(edit.clone());
    let after = load();
    let changed = after.iter().find(|x| x.id == id).expect("collection vanished after an edit");
    assert_eq!(changed.name, "SteamInRust sync test (renamed)");
    assert_eq!(changed.games, vec![620]);
    println!("renamed and removed a game: ok");

    edit.deleted = true;
    save(edit);
    let after = load();
    assert!(after.iter().all(|x| x.id != id), "the test collection is still on Steam");
    assert_eq!(after.len(), before.len(), "collection count changed");
    println!("deleted: ok; {} collections, same as before", after.len());
}

//! Live check of the Add a friend page's calls:
//! `cargo run -p client --example add_friend_test [name]`
//! Searches profiles by name and reads your Quick Invite link (Steam makes
//! one if you have none, as its own page does).

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt().with_env_filter(std::env::var("LOG").unwrap_or("warn".into())).init();
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;
    let text = std::env::args().nth(1).unwrap_or_else(|| "ea".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });
    let mut left = 2;
    while left > 0 {
        match rx.recv_timeout(Duration::from_secs(60)).expect("timed out") {
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::LoggedIn { .. } => {
                c.send(Command::SearchUsers { text: text.clone(), page: 1 });
                c.send(Command::LoadQuickInvite { new: false });
            }
            Event::UserSearch(r) => {
                left -= 1;
                let r = r.expect("search");
                println!("search \"{}\": {} matches, {} on page {}", r.text, r.total, r.users.len(), r.page);
                for u in r.users.iter().take(5) {
                    println!("  {} | {} | avatar:{} | {:?}", u.steam_id, u.name, !u.avatar.is_empty(), u.details);
                }
            }
            Event::QuickInvite(r) => {
                left -= 1;
                let q = r.expect("quick invite");
                let days = (q.expires as i64 - std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64) / 86400;
                println!("quick invite: {} (works for {days} more days)", q.link.split('/').take(5).collect::<Vec<_>>().join("/") + "/…");
            }
            _ => {}
        }
    }
}

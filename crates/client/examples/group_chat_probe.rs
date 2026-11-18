//! Read-only check of group chats: `cargo run -p client --example group_chat_probe`
//! Signs in with the saved account, lists your group chats and loads one
//! room's history (prints counts, not messages). Sends nothing.

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};

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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut asked_history = false;
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        match e {
            Event::LoggedIn { .. } => c.send(Command::LoadChatGroups),
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::ChatGroupMembers(m) => println!("members: {:?}", m.iter().map(|(g, w)| (g, w.len())).collect::<Vec<_>>()),
            Event::ChatGroups(r) => match r {
                Ok(groups) => {
                    println!("groups: {}", groups.len());
                    for g in &groups {
                        println!("  {} (clan {}, {} rooms, {} active, unread {}, avatar {})", g.name, g.clan_id, g.rooms.len(), g.active_members, g.unread, !g.avatar.is_empty());
                    }
                    if let Some(g) = groups.first().filter(|_| !asked_history) {
                        asked_history = true;
                        let room = if g.default_room != 0 { g.default_room } else { g.rooms.first().map(|r| r.id).unwrap_or(0) };
                        c.send(Command::LoadGroupHistory { group: g.id, room, before: None });
                    } else if groups.is_empty() {
                        break;
                    }
                }
                Err(e) => {
                    println!("groups: ERROR {e}");
                    break;
                }
            },
            Event::GroupHistory { result, .. } => {
                match result {
                    Ok((msgs, more)) => println!("history: {} messages, older available: {more}, newest at {:?}", msgs.len(), msgs.last().map(|m| m.time)),
                    Err(e) => println!("history: ERROR {e}"),
                }
                break;
            }
            _ => {}
        }
    }
}

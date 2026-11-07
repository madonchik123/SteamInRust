//! End-to-end check of game launching with Steam features:
//! `cargo run -p client --example steamclient_test -- <app id> <install dir> [game args]`
//!
//! Signs in with the saved account (its own session, like a second PC),
//! prints the launch mode, starts the game exactly like the Play button and
//! waits until it closes. Set `STEAMINRUST_STEAMCLIENT64` to the built DLL
//! and `STEAMINRUST_STEAMCLIENT_LOG` to a file to see what the game asked.

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};

fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let mut args = std::env::args().skip(1);
    let app_id: u32 = args.next().expect("app id").parse().expect("app id");
    let dir = std::path::PathBuf::from(args.next().expect("install dir"));
    let game_args = args.collect::<Vec<_>>().join(" ");
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });

    while let Ok(e) = rx.recv() {
        match e {
            Event::LoggedIn { steam_id, .. } => println!("signed in as {steam_id}"),
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::Apps(apps) => {
                match apps.iter().find(|a| a.app_id == app_id) {
                    Some(a) => {
                        println!("{} ({app_id}) is in the library: {:?} categories {:?}", a.name, client::launch_mode(a, &dir), a.categories);
                        for o in &a.launch {
                            println!("  launch option: {} {} [{}]", o.executable, o.arguments, o.os_list);
                        }
                    }
                    None => panic!("app {app_id} isn't in this account's library"),
                }
                c.send(Command::Launch { app_id, install_dir: dir.clone(), args: game_args.clone() });
            }
            Event::LaunchMode { mode, .. } => println!("launch mode: {mode:?}"),
            Event::Personas(ps) => {
                for p in ps.iter().filter(|p| p.game_app_id.is_some() || p.game_name.is_some()) {
                    println!("persona {} {:?}: game {:?} {:?}", p.steam_id, p.state, p.game_app_id, p.game_name);
                }
            }
            Event::GameStarted { app_id } => println!("game {app_id} running"),
            Event::CloudReport { report, .. } => println!("cloud: {report:?}"),
            Event::Error(e) => println!("error: {e}"),
            Event::GameStopped { app_id } => {
                println!("game {app_id} closed");
                // Give the post-game cloud upload a moment.
                std::thread::sleep(std::time::Duration::from_secs(3));
                c.send(Command::LogOut);
            }
            Event::LoggedOut { reason } => {
                println!("signed out: {reason}");
                break;
            }
            _ => {}
        }
    }
}

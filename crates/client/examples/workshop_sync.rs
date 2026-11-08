//! Download a game's subscribed Workshop items, like before a launch:
//! `cargo run -p client --example workshop_sync -- <app id>`
//! Signs in with the saved account (its own session) and reads only.

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};

fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let app_id: u32 = std::env::args().nth(1).expect("app id").parse().expect("app id");
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });
    while let Ok(e) = rx.recv_timeout(std::time::Duration::from_secs(600)) {
        match e {
            Event::LoggedIn { .. } => c.send(Command::SyncWorkshop { app_id }),
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::WorkshopSync { result, .. } => {
                match result {
                    Ok(r) => {
                        println!("downloaded: {:?}", r.downloaded);
                        println!("failed: {:?}", r.failed);
                    }
                    Err(e) => println!("error: {e}"),
                }
                break;
            }
            _ => {}
        }
    }
}

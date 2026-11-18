//! Read-only check of market history: `cargo run -p client --example market_history_probe`
//! Signs in with the saved account and loads the first page of your
//! Community Market history. Prints counts and which fields were found (not
//! names, prices or people). Changes nothing.

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
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        match e {
            Event::LoggedIn { .. } => c.send(Command::LoadMarketHistory { start: 0, language: "english".into() }),
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::MarketHistory { result, .. } => {
                match result {
                    Ok((rows, total)) => {
                        println!("history: {} rows of {total}", rows.len());
                        for r in rows.iter().take(15) {
                            println!(
                                "  {:?}: name {} game {} price {} acted {:?} listed {:?} image {} other {} note {:?}",
                                r.kind,
                                !r.name.is_empty(),
                                !r.game.is_empty(),
                                !r.price.is_empty(),
                                r.acted,
                                r.listed,
                                !r.image.is_empty(),
                                !r.other.is_empty(),
                                r.note
                            );
                        }
                    }
                    Err(e) => println!("history: ERROR {e}"),
                }
                break;
            }
            _ => {}
        }
    }
}

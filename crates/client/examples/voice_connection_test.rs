//! Does Valve's voice server accept a connection from us?
//! `cargo run -p client --example voice_connection_test -- --run`
//!
//! Signs in with the saved account, sets up one audio connection, reports
//! how far it got and signs out. It rings nobody, joins no channel and sends
//! no audio. Without `--run` it does nothing, so it can't start by accident.

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

fn main() {
    if !std::env::args().any(|a| a == "--run") {
        println!("Uses your saved Steam account. Pass --run to go ahead.");
        return;
    }
    tracing_subscriber::fmt().with_env_filter("warn").init();
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut code = 1;
    let mut started = false;
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        match e {
            Event::LoggedIn { .. } if !started => {
                started = true;
                println!("signed in, asking for a voice connection");
                c.send(Command::VoiceConnectionTest);
            }
            Event::LoginFailed(e) => {
                println!("sign-in failed: {e}");
                break;
            }
            Event::VoiceConnectionTest(result) => {
                match result {
                    Ok(t) => {
                        println!("Steam answered in {} ms", t.answer_ms);
                        println!("encrypted connection up in {} ms", t.connect_ms);
                        println!("server uses simplified checks: {}", t.server_ice_lite);
                        println!("Steam confirmed it saw us: {}", t.steam_confirmed);
                        code = if t.steam_confirmed { 0 } else { 2 };
                    }
                    Err(e) => println!("FAILED: {e}"),
                }
                c.send(Command::LogOut);
            }
            Event::LoggedOut { .. } if started => break,
            _ => {}
        }
    }
    std::process::exit(code);
}

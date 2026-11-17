//! Read-only check of the Edit Profile data:
//! `cargo run -p client --example profile_edit_probe`
//! Signs in with the saved account and prints what the edit page and the
//! profile item lists gave (counts and which fields are filled, not values).

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
    let mut left = 2;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        match e {
            Event::LoggedIn { .. } => {
                c.send(Command::LoadProfileEdit { language: "english".into() });
                c.send(Command::LoadLocations { country: None, state: None });
            }
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::ProfileEdit(r) => {
                left -= 1;
                match r {
                    Ok(p) => println!(
                        "edit: name={} real={} url={} summary={} country={} state={} city={} privacy={:?} awards_hidden={} theme={:?} themes={} backgrounds={} mini={} frames={} animated={} equipped={:?}",
                        !p.persona_name.is_empty(), !p.real_name.is_empty(), !p.custom_url.is_empty(), !p.summary.is_empty(),
                        !p.country.is_empty(), !p.state.is_empty(), !p.city.is_empty(), p.privacy, p.hide_awards, p.theme, p.themes.len(),
                        p.backgrounds.len(), p.mini_backgrounds.len(), p.frames.len(), p.animated_avatars.len(), p.equipped.map(|x| x != 0)
                    ),
                    Err(e) => println!("edit: ERROR {e}"),
                }
            }
            Event::Locations { result, .. } => {
                left -= 1;
                match result {
                    Ok(l) => println!("countries: {} (first {:?})", l.len(), l.first()),
                    Err(e) => println!("countries: ERROR {e}"),
                }
            }
            _ => {}
        }
        if left == 0 {
            break;
        }
    }
}

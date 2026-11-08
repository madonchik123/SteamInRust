//! Read-only check of the account pages:
//! `cargo run -p client --example account_probe [language]`
//! Signs in with the saved account and prints Steam Guard details, the
//! first page of purchase history, licenses and the Steam Family.

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};

fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let language = std::env::args().nth(1).unwrap_or_else(|| "english".into());
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });

    let mut left = 5;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        match e {
            Event::LoggedIn { steam_id, .. } => {
                println!("signed in as {steam_id}");
                c.send(Command::LoadAccountSecurity);
                c.send(Command::LoadPurchases { language: language.clone(), cursor: None });
                c.send(Command::LoadLicenses { language: language.clone() });
                c.send(Command::LoadFamily { language: language.clone() });
            }
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::AccountSecurity(r) => {
                left -= 1;
                match r {
                    Ok(s) => println!(
                        "security: guard={} since={:?} authenticator={} since={:?} phone={} password changed={:?} email changed={:?} devices={}",
                        s.steam_guard, s.steam_guard_since, s.authenticator, s.authenticator_since, s.phone_verified, s.password_changed, s.email_changed, s.devices.len()
                    ),
                    Err(e) => println!("security: {e}"),
                }
            }
            Event::Purchases { result, more } => {
                left -= 1;
                match result {
                    Ok(p) => {
                        println!("purchases (more={more}): {} rows, continues={}", p.rows.len(), p.more.is_some());
                        // One "load more", like the button.
                        match (&p.more, more) {
                            (Some(cursor), false) => c.send(Command::LoadPurchases { language: language.clone(), cursor: Some(cursor.clone()) }),
                            (None, false) => left -= 1,
                            _ => {}
                        }
                        for r in p.rows.iter().take(6) {
                            println!("  {} | {:?} | {} / {} | {} | {} {}", r.date, r.items, r.kind, r.payment, r.total, r.wallet_change, r.wallet_balance);
                        }
                    }
                    Err(e) => println!("purchases: {e}"),
                }
            }
            Event::LicenseList(r) => {
                left -= 1;
                match r {
                    Ok(l) => {
                        println!("licenses: {}", l.len());
                        for x in l.iter().take(5) {
                            println!("  {} | {} | {} | {:?}", x.date, x.name, x.method, x.package_id);
                        }
                    }
                    Err(e) => println!("licenses: {e}"),
                }
            }
            Event::Family(r) => {
                left -= 1;
                match r {
                    Ok(f) => match &f.family {
                        Some(g) => println!("family '{}': {} members, {} invites, {} free spots, {} shared apps", g.name, g.members.len(), g.invites.len(), g.free_spots, g.shared.len()),
                        None => println!("family: none (cooldown {} s, {} invites)", f.cooldown, f.invites.len()),
                    },
                    Err(e) => println!("family: {e}"),
                }
            }
            _ => {}
        }
        if left == 0 {
            break;
        }
    }
}

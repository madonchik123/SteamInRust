//! Live check of the Community home feeds: `cargo run -p client --example feed_test`
//! Signs in with the saved account and only reads: a page of your friends'
//! activity and a page of each section of the community-wide feed. Prints
//! counts and kinds only (no names), so the output is safe to share.

use client::feed::{ExploreQuery, Section};
use client::hub::{Request, Response};
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
    let mut left = 0;
    let mut started = false;
    let mut pages = 0;
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        match e {
            Event::LoggedIn { .. } => {
                println!("signed in");
                started = true;
                c.send(Command::Hub(Request::Activity { start: 0, language: "english".into() }));
                left += 1;
                for section in Section::ALL.into_iter().filter(|_| std::env::var("FEED_EXPLORE").is_ok()) {
                    c.send(Command::Hub(Request::Explore(ExploreQuery { section, recent: false, page: 1, language: "english".into(), ..Default::default() })));
                    left += 1;
                }
            }
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::Hub(Response::Activity { start, result }) => {
                left -= 1;
                match result {
                    Ok(f) => {
                        println!("activity {start}: {} posts, next {}", f.posts.len(), f.next);
                        pages += 1;
                        if f.next > 0 && pages < 8 {
                            c.send(Command::Hub(Request::Activity { start: f.next, language: "english".into() }));
                            left += 1;
                        }
                        for p in f.posts.iter().take(40) {
                            println!("   {:?} app {} images {} text {} url {}", p.kind, p.app_id, p.images.len(), p.text.len(), !p.url.is_empty());
                        }
                    }
                    Err(e) => println!("activity: ERROR {e}"),
                }
            }
            Event::Hub(Response::Explore { key, result }) => {
                left -= 1;
                match result {
                    Ok(f) => {
                        println!("explore {key}: {} posts", f.posts.len());
                        for p in f.posts.iter().take(4) {
                            println!("   {:?} app {} images {} text {} up {} live {}", p.kind, p.app_id, p.images.len(), p.text.len(), p.votes_up, p.live);
                        }
                    }
                    Err(e) => println!("explore {key}: ERROR {e}"),
                }
            }
            _ => {}
        }
        if left == 0 && started {
            break;
        }
    }
}

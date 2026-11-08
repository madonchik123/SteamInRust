//! Run a test probe against this PC's signed-in account without launching
//! a game: `cargo run -p client --example steamclient_attach -- <app id>
//! <install dir> <probe.exe> [probe args]`.
//!
//! Signs in with the saved account (its own session), attaches the probe
//! with `Command::AttachGame` (nothing is reported to Steam as played, so a
//! game running on another PC isn't disturbed) and waits for it to exit.

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};

fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let mut args = std::env::args().skip(1);
    let app_id: u32 = args.next().expect("app id").parse().expect("app id");
    let dir = std::path::PathBuf::from(args.next().expect("install dir"));
    let probe = args.next().expect("probe.exe");
    let probe_args: Vec<String> = args.collect();
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });
    let mut code = 1;
    while let Ok(e) = rx.recv() {
        match e {
            Event::LoggedIn { steam_id, .. } => println!("signed in as {steam_id}"),
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::Apps(_) => c.send(Command::AttachGame { app_id, install_dir: dir.clone() }),
            Event::GameAttached { env, .. } => {
                let status = std::process::Command::new(&probe).args(&probe_args).env("STEAMINRUST_IPC", env).status();
                code = status.map(|s| s.code().unwrap_or(1)).unwrap_or(1);
                println!("probe exited with {code}");
                c.send(Command::LogOut);
            }
            Event::LoggedOut { .. } => break,
            _ => {}
        }
    }
    std::process::exit(code);
}

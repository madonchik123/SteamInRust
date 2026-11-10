//! End-to-end downloader check:
//! `cargo run -p client --example download_test -- <empty folder> [app id]`
//! Signs in with the saved account, installs the given game (or the smallest
//! owned Windows game) into `<folder>/steamapps/common/...`, then verifies
//! every file.

use client::{Client, Command, Event, download::InstallRequest};
use std::sync::{Arc, Mutex, mpsc};

fn main() {
    tracing_subscriber::fmt().with_env_filter("info,depot=info").init();
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("folder"));
    let wanted: Option<u32> = std::env::args().nth(2).map(|a| a.parse().expect("app id"));
    std::fs::create_dir_all(dir.join("steamapps")).unwrap();
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });

    let mut picked = None;
    while let Ok(e) = rx.recv() {
        match e {
            Event::LoggedIn { .. } => println!("signed in"),
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::Apps(apps) => {
                // Smallest game with Windows content.
                let best = apps
                    .iter()
                    .filter(|a| a.kind == "game" && wanted.is_none_or(|w| a.app_id == w))
                    .filter_map(|a| {
                        let size: u64 = a
                            .depots
                            .iter()
                            .filter(|d| d.dlc_app.is_none() && !d.shared_install && (d.os.is_empty() || d.os.contains("windows")))
                            .filter_map(|d| d.manifest("public").map(|m| m.1))
                            .sum();
                        (size > 1_000_000).then_some((size, a))
                    })
                    .min_by_key(|(s, _)| *s);
                let Some((size, app)) = best else { panic!("no small game found") };
                println!("installing {} ({}) ~{} MB", app.name, app.app_id, size / 1_048_576);
                picked = Some(app.clone());
                c.send(Command::Install(InstallRequest {
                    app_id: app.app_id,
                    library: dir.clone(),
                    steam_root: dir.clone(),
                    branch: "public".into(),
                    language: "english".into(),
                    skip_dlc: vec![], add_dlc: vec![],
                }));
            }
            Event::Download(d) => println!(
                "{:?} {:>5.1}%  {:>6} KB/s  {}/{} MB",
                d.phase,
                d.disk_done as f64 * 100.0 / d.disk_total.max(1) as f64,
                d.speed / 1024,
                d.downloaded / 1_048_576,
                d.download_total / 1_048_576
            ),
            Event::DownloadFinished { result, .. } => {
                println!("finished: {result:?}");
                let app = picked.unwrap();
                let state = local::load(Some(dir.clone())).unwrap();
                let game = state.games.iter().find(|g| g.app_id == app.app_id).expect("app manifest written");
                let report = local::manage::verify(&dir, game, &local::manage::Progress::default());
                println!(
                    "verify: {} files, {} ok, {} missing, {} corrupt, unchecked {:?}",
                    report.files,
                    report.ok,
                    report.missing.len(),
                    report.corrupt.len(),
                    report.unchecked_depots
                );
                break;
            }
            _ => {}
        }
    }
    c.send(Command::LogOut);
    std::thread::sleep(std::time::Duration::from_millis(500));
}

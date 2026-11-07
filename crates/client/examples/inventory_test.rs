//! Live inventory check: `cargo run -p client --example inventory_test [steamid]`
//! Signs in with the saved account, lists every game with items in the
//! inventory (yours, or the given account's), then loads the largest one.

use client::{Client, Command, Event};
use std::sync::{Arc, Mutex, mpsc};

fn main() {
    tracing_subscriber::fmt().with_env_filter("info,client::market=debug").init();
    let account = client::store::load().into_iter().next().expect("a saved account").account_name;
    let target: Option<u64> = std::env::args().nth(1).and_then(|a| a.parse().ok());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let c = Client::spawn(rt.handle(), Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    }));
    c.send(Command::LoginSaved { account });

    let mut owner = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        match e {
            Event::LoggedIn { steam_id, .. } => {
                owner = target.unwrap_or(steam_id);
                println!("signed in; reading inventory of {owner}");
                c.send(Command::LoadInventoryApps { steam_id: owner });
                c.send(Command::LoadPlayerGames { steam_id: owner, language: "english".into() });
                c.send(Command::LoadSupportTickets);
                c.send(Command::LoadSteamCollections);
                c.send(Command::LoadBadges { steam_id: owner });
                c.send(Command::LoadComments { steam_id: owner, start: 0 });
            }
            Event::LoginFailed(e) => panic!("login failed: {e}"),
            Event::PlayerGames { result, .. } => match result {
                Ok(g) => println!("profile games: {}", g.len()),
                Err(e) => println!("profile games: {e}"),
            },
            Event::InventoryApps { result, .. } => match result {
                Ok(apps) => {
                    println!("{} games with inventories:", apps.len());
                    for a in &apps {
                        println!("  {:>8} {:<40} {:?}", a.app_id, a.name, a.contexts);
                    }
                    let Some(first) = apps.first() else { break };
                    let cx = first.contexts.iter().max_by_key(|c| c.2).map(|c| c.0).unwrap_or(2);
                    c.send(Command::LoadInventory { steam_id: owner, app_id: first.app_id, context_id: cx, language: "english".into() });
                }
                Err(e) => {
                    println!("game list unavailable: {e}; trying the Steam inventory");
                    c.send(Command::LoadInventory { steam_id: owner, app_id: 753, context_id: 6, language: "english".into() });
                }
            },
            Event::Inventory(inv) => {
                println!("app {} context {}: {} items (total {}) error={:?}", inv.app_id, inv.context_id, inv.items.len(), inv.total, inv.error);
                for i in inv.items.iter().take(5) {
                    println!("  {} | {} | tradable={} marketable={} fee={:?} holds={}/{} notes={:?}", i.name, i.kind, i.tradable, i.marketable, i.market_fee, i.trade_hold_days, i.market_hold_days, i.owner_notes);
                }
                // Read-only market checks: one price, your listings.
                if let Some(i) = inv.items.iter().filter(|i| i.marketable).nth(3) {
                    c.send(Command::MarketPrice { app_id: inv.app_id, hash_name: i.market_hash_name.clone(), currency: 37, country: "KZ".into(), full: true });
                }
                c.send(Command::LoadMyListings);
            }
            Event::MarketPrice { hash_name, result, .. } => println!("price of {hash_name}: {:?}", result.map(|p| (p.lowest, p.median, p.volume, p.lowest_sell, p.highest_buy, p.sell_count, p.buy_count, p.sell_ladder.len(), p.history.len()))),
            Event::SupportTickets(r) => println!("support: {:?}", r.map(|t| t.iter().map(|x| (x.id.clone(), x.title.clone(), x.status.clone())).collect::<Vec<_>>())),
            Event::Notifications(n) => {
                println!("notifications: {} (unread {}, gifts {}, friends {}, confirmations {})", n.items.len(), n.unread, n.pending_gifts, n.pending_friends, n.confirmations);
                for x in n.items.iter().take(12) {
                    println!("  type {:>2} read={} app={:?} from={} text={:?} data={}", x.kind, x.read, x.app_id, x.from.is_some(), x.text, x.data.chars().take(160).collect::<String>());
                }
            }
            Event::Badges { result, .. } => println!("badges: {:?}", result.map(|b| (b.level, b.xp, b.next, b.progress, b.items.len(), b.items.first().map(|x| (x.title.clone(), x.detail.clone(), x.icon.is_some()))))),
            Event::Comments { result, .. } => println!("comments: {:?}", result.map(|c| (c.total, c.items.len(), c.items.first().map(|x| (x.name.clone(), x.time, x.avatar.is_some(), x.html.chars().count()))))),
            Event::Apps(apps) => {
                for a in apps.iter().filter(|a| a.parent == Some(570) || a.parent == Some(4204530) || a.parent == Some(252490)) {
                    println!("dlc of {:?}: {} {} kind={} store_assets={} art={:?}", a.parent, a.app_id, a.name, a.kind, a.store_assets, a.art);
                }
                let hashed = apps.iter().filter(|a| !a.art.is_empty()).count();
                println!("apps with new-style header art: {hashed} of {}", apps.len());
                let dlc: Vec<_> = apps.iter().filter(|a| a.is_dlc()).collect();
                println!("dlc total {}, with store page {}; e.g. {:?}", dlc.len(), dlc.iter().filter(|a| a.store_assets).count(), dlc.iter().filter(|a| a.store_assets).take(3).map(|a| a.name.clone()).collect::<Vec<_>>());
                let with_cloud: Vec<_> = apps.iter().filter(|a| a.cloud.is_some()).collect();
                println!("apps with Steam Cloud: {}", with_cloud.len());
                // Read-only check: lists cloud files, writes nothing.
                // Download test into a temp folder: only for a game that saves
                // through Steam's API (so nothing outside the temp folder is touched).
                if std::env::var("SIR_CLOUD_DL").is_ok() {
                    if let Some(a) = with_cloud.iter().find(|a| a.app_id == 204360 && a.cloud.as_ref().is_some_and(|c| c.save_files.is_empty())) {
                        let tmp = std::env::temp_dir().join("sir-cloud-dl");
                        c.send(Command::CloudSettings { steam_root: Some(tmp.clone()), disabled: vec![] });
                        c.send(Command::CloudSync { app_id: a.app_id, install_dir: tmp.join("game"), mode: client::cloud::Mode::Download, keep_local: vec![], keep_cloud: vec![], then_launch: None });
                        println!("download test for {} into {}", a.name, tmp.display());
                    }
                }
                for a in with_cloud.iter().filter(|a| [570u32, 413150, 105600, 204360, 524220, 250900].contains(&a.app_id)).take(3) {
                    println!("  {} {:?}", a.name, a.cloud.as_ref().map(|c| (c.quota, c.save_files.iter().map(|r| format!("{}/{}/{}", r.root, r.path, r.pattern)).collect::<Vec<_>>())));
                    c.send(Command::CloudSync { app_id: a.app_id, install_dir: std::env::temp_dir().join("sir-cloud-check"), mode: client::cloud::Mode::Check, keep_local: vec![], keep_cloud: vec![], then_launch: None });
                }
            }
            Event::CloudReport { report, .. } => println!("cloud {}: files {} bytes {} quota {} downloaded {} up_to_date {} conflicts {} errors {:?}", report.app_id, report.cloud_files, report.cloud_bytes, report.quota_bytes, report.downloaded, report.up_to_date, report.conflicts.len(), report.errors),
            Event::SteamCollections(r) => println!("steam collections: {:?}", r.map(|l| l.iter().map(|c| (c.id.clone(), c.games.len(), c.dynamic)).collect::<Vec<_>>())),
            Event::FeeRules(r) => println!("fee rules: {r:?}"),
            Event::MyListings(r) => {
                match r {
                    Ok(l) => println!("my listings: {} listings, {} buy orders; first: {:?}", l.listings.len(), l.buy_orders.len(), l.listings.first().map(|x| (&x.name, x.buyer_pays, x.you_receive, x.status))),
                    Err(e) => println!("my listings: {e}"),
                }
            }
            _ => {}
        }
    }
}

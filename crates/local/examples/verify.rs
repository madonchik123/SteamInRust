//! `cargo run -p local --example verify` — read-only check of the smallest installed game.
fn main() {
    let state = local::load(None).expect("steam install");
    let mut games = state.games.clone();
    games.sort_by_key(|g| g.size_on_disk);
    let Some(g) = games.into_iter().find(|g| g.size_on_disk > 0) else { return };
    let depots = local::manage::installed_depots(&g);
    println!("{} ({} depots, {} bytes)", g.name, depots.len(), g.size_on_disk);
    let p = local::manage::Progress::default();
    let r = local::manage::verify(&state.steam_root, &g, &p);
    println!("files {} ok {} missing {} corrupt {} unchecked depots {:?}", r.files, r.ok, r.missing.len(), r.corrupt.len(), r.unchecked_depots);
}

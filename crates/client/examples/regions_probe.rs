//! `cargo run -p client --example regions_probe`: list download regions, fastest first.
#[tokio::main]
async fn main() {
    let list = client::regions::probe().await;
    println!("{} regions", list.len());
    for r in list.iter().take(15) {
        println!("  {:<20} cell {:>3}  ping {:>5?}  load {:>3}  tier {}", r.name, r.cell_id, r.ping_ms, r.load, r.speed_tier());
    }
}

//! `cargo run -p store --example events_probe -- 570 4204530`: print each game's update posts.
#[tokio::main]
async fn main() {
    for app in std::env::args().skip(1).filter_map(|a| a.parse::<u32>().ok()) {
        let api = store::StoreClient::new("us", "russian");
        match api.events(app, 12).await {
            Ok(list) => println!("{app}: {} posts; first: {:?}", list.len(), list.first().map(|e| e.title.clone())),
            Err(e) => println!("{app}: error {e}"),
        }
    }
}

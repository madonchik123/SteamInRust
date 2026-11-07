//! `cargo run -p auth --example qr_probe`: asks Steam for a QR login challenge
//! and prints its URL. Nobody scans it, so it simply expires.
use proto::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let servers = net::directory::websocket_servers().await?;
    let (conn, _events) = net::Connection::connect(&servers[0]).await?;
    conn.send(net::emsg::CLIENT_HELLO, &CMsgClientHello { protocol_version: Some(65580) })?;
    let pending = auth::begin_qr(&conn, &auth::Device::default()).await?;
    let url = pending.challenge_url.clone().unwrap_or_default();
    println!("QR challenge ok: {}... (poll every {:?})", &url[..url.len().min(32)], pending.interval);
    Ok(())
}

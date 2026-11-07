//! Connectivity check: `cargo run -p auth --example probe`.
//! Connects to a CM and fetches a public RSA key. Does not log in.
use proto::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let servers = net::directory::websocket_servers().await?;
    println!("{} websocket CMs, using {}", servers.len(), servers[0]);
    let (conn, _events) = net::Connection::connect(&servers[0]).await?;
    conn.send(net::emsg::CLIENT_HELLO, &CMsgClientHello { protocol_version: Some(65580) })?;
    let key: CAuthenticationGetPasswordRsaPublicKeyResponse = conn
        .call(
            "Authentication.GetPasswordRSAPublicKey#1",
            &CAuthenticationGetPasswordRsaPublicKeyRequest { account_name: Some("steam".into()) },
        )
        .await?;
    println!("RSA modulus: {}... ({} hex chars), exp {}", &key.publickey_mod()[..16], key.publickey_mod().len(), key.publickey_exp());
    Ok(())
}

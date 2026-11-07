//! Finds CM servers through Steam's public directory API.

use crate::{Error, Result};
use serde::Deserialize;

const DIRECTORY: &str =
    "https://api.steampowered.com/ISteamDirectory/GetCMListForConnect/v1/?cellid=0&format=json";

/// Fallback servers if the directory itself is unreachable.
const FALLBACK: &[&str] = &["ext1-fra2.steamserver.net:443", "ext2-ams1.steamserver.net:443"];

#[derive(Deserialize)]
struct Envelope {
    response: Body,
}
#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    serverlist: Vec<Server>,
}
#[derive(Deserialize)]
struct Server {
    endpoint: String,
    #[serde(rename = "type")]
    kind: String,
}

/// WebSocket CM endpoints (`host:port`), best first.
pub async fn websocket_servers() -> Result<Vec<String>> {
    let fetched: Result<Vec<String>> = async {
        // Without a timeout a dead network can hang here for a long time.
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(8)).build()?;
        let env: Envelope = http.get(DIRECTORY).send().await?.error_for_status()?.json().await?;
        Ok(env
            .response
            .serverlist
            .into_iter()
            .filter(|s| s.kind == "websockets")
            .map(|s| s.endpoint)
            .collect())
    }
    .await;

    match fetched {
        Ok(list) if !list.is_empty() => Ok(list),
        Ok(_) => fallback(),
        Err(e) => {
            tracing::warn!("CM directory unavailable ({e}), using fallback servers");
            fallback()
        }
    }
}

fn fallback() -> Result<Vec<String>> {
    if FALLBACK.is_empty() {
        return Err(Error::NoServers);
    }
    Ok(FALLBACK.iter().map(|s| s.to_string()).collect())
}

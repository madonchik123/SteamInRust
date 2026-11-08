//! The server browser for games (ISteamMatchmakingServers in our
//! steamclient): Steam's master server list, like Steam's client asks it.

use crate::ipc::protocol::{GameServer, Reply};
use model::AppId;
use net::Connection;
use proto::*;

/// Steam's own cap for one list.
const MAX_SERVERS: u32 = 5000;

fn ipv4(a: &Option<CMsgIpAddress>) -> u32 {
    match a.as_ref().and_then(|a| a.ip.as_ref()) {
        Some(c_msg_ip_address::Ip::V4(v)) => *v,
        _ => 0,
    }
}

/// A string field, either inline or by index into the reply's string table.
fn text(inline: &str, index: Option<u32>, strings: &[String], default: &str) -> String {
    if !inline.is_empty() {
        return inline.to_string();
    }
    match index {
        Some(i) => strings.get(i as usize).cloned().unwrap_or_default(),
        None => default.to_string(),
    }
}

pub fn servers(r: &CMsgGmsClientServerQueryResponse) -> Vec<GameServer> {
    let d = r.default_server_data.clone().unwrap_or_default();
    let strings = &r.server_strings;
    r.servers
        .iter()
        .map(|s| {
            let pick = |a: Option<u32>, b: Option<u32>| a.or(b).unwrap_or(0);
            let flags = pick(s.flags, d.flags);
            GameServer {
                ip: if ipv4(&s.server_ip) != 0 { ipv4(&s.server_ip) } else { s.deprecated_server_ip() },
                query_port: pick(s.query_port, d.query_port) as u16,
                game_port: pick(s.game_port, d.game_port) as u16,
                steam_id: s.steam_id(),
                app_id: pick(s.app_id, d.app_id),
                name: text(s.name_str(), s.name_strindex, strings, &text(d.name_str(), d.name_strindex, strings, "")),
                map: text(s.map_str(), s.map_strindex, strings, &text(d.map_str(), d.map_strindex, strings, "")),
                game_dir: text(s.gamedir_str(), s.gamedir_strindex, strings, &text(d.gamedir_str(), d.gamedir_strindex, strings, "")),
                description: text(s.game_description_str(), s.game_description_strindex, strings, &text(d.game_description_str(), d.game_description_strindex, strings, "")),
                tags: text(s.gametype_str(), s.gametype_strindex, strings, &text(d.gametype_str(), d.gametype_strindex, strings, "")),
                version: pick(s.revision, d.revision),
                players: pick(s.players, d.players),
                max_players: pick(s.max_players, d.max_players),
                bots: pick(s.bots, d.bots),
                password: flags & 1 != 0,
                secure: flags & 2 != 0,
                ping: s.ping_estimate(),
            }
        })
        .collect()
}

pub async fn list(conn: &Connection, app_id: AppId, filter: &str, max: u32) -> Reply {
    let filter = if filter.contains("\\appid\\") { filter.to_string() } else { format!("\\appid\\{app_id}{filter}") };
    let req = CGameServersMasterServerQueryRequest {
        appid: Some(app_id),
        query_data: Some(CMsgClientGmsServerQuery { app_id: Some(app_id), filter_text: Some(filter), max_servers: Some(if max == 0 { MAX_SERVERS } else { max.min(MAX_SERVERS) }), ..Default::default() }),
    };
    match conn.call::<_, CGameServersMasterServerQueryResponse>("GameServers.MasterServerQuery#1", &req).await {
        Ok(r) => {
            let r = r.query_response.unwrap_or_default();
            if !r.error().is_empty() {
                tracing::info!("server list for {app_id}: {}", r.error());
            }
            Reply::Servers { servers: servers(&r) }
        }
        Err(e) => {
            tracing::info!("server list for {app_id}: {e}");
            Reply::Servers { servers: Vec::new() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_by_index_and_defaults() {
        use c_msg_gms_client_server_query_response::Server;
        let r = CMsgGmsClientServerQueryResponse {
            servers: vec![Server { query_port: Some(27015), name_strindex: Some(1), players: Some(3), ..Default::default() }],
            default_server_data: Some(Server { map_str: Some("de_dust2".into()), max_players: Some(10), flags: Some(2), ..Default::default() }),
            server_strings: vec!["x".into(), "My server".into()],
            ..Default::default()
        };
        let s = &servers(&r)[0];
        assert_eq!((s.name.as_str(), s.map.as_str(), s.players, s.max_players, s.secure, s.query_port), ("My server", "de_dust2", 3, 10, true, 27015));
    }
}

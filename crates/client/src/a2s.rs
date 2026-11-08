//! "View server info": ask a game server about itself with Valve's server
//! query protocol (A2S_INFO over UDP), like Steam's server info window.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub name: String,
    pub map: String,
    pub game: String,
    pub players: u8,
    pub max_players: u8,
    pub bots: u8,
    pub vac: bool,
    pub password: bool,
    pub ping_ms: u32,
}

const INFO: &[u8] = b"\xFF\xFF\xFF\xFFTSource Engine Query\0";

/// Query `addr` ("ip:port"). Blocking: call it off the UI thread.
pub fn query(addr: &str) -> Result<ServerInfo, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
    socket.connect(addr).map_err(|e| e.to_string())?;
    let start = Instant::now();
    socket.send(INFO).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 1400];
    let mut n = socket.recv(&mut buf).map_err(|_| "The server didn't answer (it may block queries).".to_string())?;
    // Newer servers answer with a challenge first; repeat with it appended.
    if n >= 9 && buf[4] == b'A' {
        let mut req = INFO.to_vec();
        req.extend_from_slice(&buf[5..9]);
        socket.send(&req).map_err(|e| e.to_string())?;
        n = socket.recv(&mut buf).map_err(|_| "The server didn't answer (it may block queries).".to_string())?;
    }
    let ping_ms = start.elapsed().as_millis() as u32;
    let mut info = parse(&buf[..n])?;
    info.ping_ms = ping_ms;
    Ok(info)
}

/// Parse an A2S_INFO reply (header 0xFFFFFFFF 'I').
pub fn parse(b: &[u8]) -> Result<ServerInfo, String> {
    if b.len() < 6 || b[..4] != [0xFF; 4] || b[4] != b'I' {
        return Err("Unexpected answer from the server".into());
    }
    let mut i = 6; // header + protocol byte
    let mut string = || -> String {
        let end = b[i..].iter().position(|c| *c == 0).map(|p| i + p).unwrap_or(b.len());
        let s = String::from_utf8_lossy(&b[i..end]).into_owned();
        i = (end + 1).min(b.len());
        s
    };
    let name = string();
    let map = string();
    let _folder = string();
    let game = string();
    let byte = |at: usize| b.get(at).copied().unwrap_or(0);
    // id (2 bytes), players, max, bots, type, os, visibility, vac
    let at = i + 2;
    Ok(ServerInfo { name, map, game, players: byte(at), max_players: byte(at + 1), bots: byte(at + 2), password: byte(at + 5) == 1, vac: byte(at + 6) == 1, ping_ms: 0 })
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_a2s_info() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF, b'I', 17];
        for s in ["My Server", "rp_bangclaw", "garrysmod", "DarkRP"] {
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        }
        b.extend_from_slice(&[0x04, 0x00, 37, 64, 2, b'd', b'w', 0, 1]);
        let i = super::parse(&b).unwrap();
        assert_eq!((i.name.as_str(), i.map.as_str(), i.game.as_str()), ("My Server", "rp_bangclaw", "DarkRP"));
        assert_eq!((i.players, i.max_players, i.bots, i.password, i.vac), (37, 64, 2, false, true));
    }
}

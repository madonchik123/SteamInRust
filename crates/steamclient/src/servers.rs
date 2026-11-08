//! ISteamMatchmakingServers: the server browser. Internet lists come from
//! Steam's master server through SteamInRust; every server is then asked
//! for its current state and ping with Valve's server query protocol
//! (A2S_INFO over UDP), like Steam's client does. LAN lists broadcast
//! A2S_INFO. Ping, player and rules queries ask the server directly.
//!
//! Results reach the game through its own response objects, called on its
//! thread during SteamAPI_RunCallbacks. Friends, favorites, history and
//! spectator lists come back empty: SteamInRust keeps no such lists yet.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, text};
use crate::protocol::{GameServer, Reply, Request};
use crate::state::{Bytes, on_game_thread};
use crate::vt::Fp;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub fn imp(name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match name {
        "RequestInternetServerList" => f!(internet),
        "RequestLANServerList" => f!(lan),
        "RequestFriendsServerList" | "RequestFavoritesServerList" | "RequestHistoryServerList" | "RequestSpectatorServerList" => f!(empty_list),
        "ReleaseRequest" => f!(release),
        "GetServerDetails" => f!(details),
        "CancelQuery" => f!(cancel),
        "RefreshQuery" => f!(refresh),
        "IsRefreshing" => f!(is_refreshing),
        "GetServerCount" => f!(count),
        "RefreshServer" => f!(refresh_server),
        "PingServer" => f!(ping),
        "PlayerDetails" => f!(players),
        "ServerRules" => f!(rules),
        "CancelServerQuery" => f!(cancel_server_query),
        _ => None,
    }
}

// EMatchMakingServerResponse
const RESPONDED: i32 = 0;
const FAILED: i32 = 1;
const NONE_LISTED: i32 = 2;

/// gameserveritem_t (Windows packing).
const ITEM: usize = 376;

struct List {
    app_id: u32,
    filter: String,
    response: usize,
    /// Boxed so GetServerDetails pointers stay put.
    items: Vec<Box<[u8; ITEM]>>,
    refreshing: bool,
    cancelled: Arc<AtomicBool>,
    lan: bool,
}

#[derive(Default)]
struct Servers {
    next: usize,
    lists: HashMap<usize, List>,
    queries: HashMap<i32, Arc<AtomicBool>>,
}

static S: LazyLock<Mutex<Servers>> = LazyLock::new(|| Mutex::new(Servers { next: 0x5000, ..Default::default() }));

fn s() -> MutexGuard<'static, Servers> {
    S.lock().unwrap_or_else(|e| e.into_inner())
}

// --------------------------------------------- calls into the game's objects

fn method(obj: usize, slot: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(slot) }
}

#[cfg(target_arch = "x86")]
macro_rules! virt {
    ($obj:expr, $slot:expr, fn($($t:ty),*), $($a:expr),*) => {{
        let f: extern "thiscall" fn(usize, $($t),*) = unsafe { std::mem::transmute(method($obj, $slot)) };
        f($obj, $($a),*)
    }};
}

#[cfg(not(target_arch = "x86"))]
macro_rules! virt {
    ($obj:expr, $slot:expr, fn($($t:ty),*), $($a:expr),*) => {{
        let f: extern "C" fn(usize, $($t),*) = unsafe { std::mem::transmute(method($obj, $slot)) };
        f($obj, $($a),*)
    }};
}

// ----------------------------------------------------------- A2S queries

const A2S_INFO: &[u8] = b"\xFF\xFF\xFF\xFFTSource Engine Query\0";

fn cstr(d: &[u8], at: &mut usize) -> String {
    let start = (*at).min(d.len());
    let end = d[start..].iter().position(|b| *b == 0).map(|p| start + p).unwrap_or(d.len());
    *at = (end + 1).min(d.len() + 1);
    String::from_utf8_lossy(&d[start..end]).into_owned()
}

/// An A2S_INFO reply ('I') into a server entry.
fn parse_info(d: &[u8], ip: u32, query_port: u16) -> Option<GameServer> {
    if d.len() < 6 || d[..4] != [0xFF; 4] || d[4] != b'I' {
        return None;
    }
    let mut at = 6;
    let name = cstr(d, &mut at);
    let map = cstr(d, &mut at);
    let game_dir = cstr(d, &mut at);
    let description = cstr(d, &mut at);
    let b = d.get(at..at + 9)?;
    let app_id = u16::from_le_bytes([b[0], b[1]]) as u32;
    let (players, max_players, bots) = (b[2] as u32, b[3] as u32, b[4] as u32);
    let (password, secure) = (b[7] == 1, b[8] == 1);
    at += 9;
    let version_text = cstr(d, &mut at);
    let mut s = GameServer { ip, query_port, game_port: query_port, app_id, name, map, game_dir, description, players, max_players, bots, password, secure, ..Default::default() };
    s.version = version_text.split('.').fold(0u32, |v, p| v.saturating_mul(10).saturating_add(p.parse::<u32>().unwrap_or(0)));
    if let Some(&edf) = d.get(at) {
        at += 1;
        if edf & 0x80 != 0 {
            s.game_port = u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?);
            at += 2;
        }
        if edf & 0x10 != 0 {
            s.steam_id = u64::from_le_bytes(d.get(at..at + 8)?.try_into().ok()?);
            at += 8;
        }
        if edf & 0x40 != 0 {
            at += 2;
            cstr(d, &mut at);
        }
        if edf & 0x20 != 0 {
            s.tags = cstr(d, &mut at);
        }
        if edf & 0x01 != 0 {
            let id = u64::from_le_bytes(d.get(at..at + 8)?.try_into().ok()?);
            s.app_id = (id & 0xFF_FFFF) as u32;
        }
    }
    Some(s)
}

fn addr(ip: u32, port: u16) -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(ip), port))
}

/// Ask many servers at once; (ip, port) -> (fresh info, ping ms). Replies
/// are read while queries still go out, so pings stay real.
fn query_all(targets: &[(u32, u16)], wait: Duration, cancelled: &AtomicBool, broadcast: bool) -> HashMap<(u32, u16), (GameServer, u32)> {
    let mut out = HashMap::new();
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return out };
    let _ = sock.set_broadcast(broadcast);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(5)));
    let mut sent: HashMap<(u32, u16), Instant> = HashMap::new();
    let mut next = 0;
    let mut last_batch = Instant::now() - Duration::from_secs(1);
    let mut finished: Option<Instant> = None;
    let mut buf = [0u8; 1500];
    while !cancelled.load(Ordering::Relaxed) {
        // ~500 queries a second, so neither the link nor routers choke.
        if next < targets.len() && last_batch.elapsed() >= Duration::from_millis(50) {
            last_batch = Instant::now();
            for &(ip, port) in &targets[next..(next + 25).min(targets.len())] {
                let _ = sock.send_to(A2S_INFO, addr(ip, port));
                sent.insert((ip, port), Instant::now());
            }
            next = (next + 25).min(targets.len());
        }
        if next == targets.len() && finished.is_none() {
            finished = Some(Instant::now());
        }
        if finished.is_some_and(|f| f.elapsed() > wait) || !broadcast && !targets.is_empty() && out.len() == targets.len() {
            break;
        }
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let SocketAddr::V4(from) = from else { continue };
        let key = (u32::from(*from.ip()), from.port());
        let d = &buf[..n];
        // S2C_CHALLENGE: ask again with the challenge appended (the ping
        // counts from this second query).
        if n >= 9 && d[..4] == [0xFF; 4] && d[4] == b'A' {
            let mut q = A2S_INFO.to_vec();
            q.extend_from_slice(&d[5..9]);
            let _ = sock.send_to(&q, SocketAddr::V4(from));
            sent.insert(key, Instant::now());
            continue;
        }
        if let Some(s) = parse_info(d, key.0, key.1) {
            let ping = sent.get(&key).map(|t| t.elapsed().as_millis() as u32).unwrap_or(0);
            out.insert(key, (s, ping.max(1)));
        }
    }
    out
}

/// One request/challenge/reply exchange (A2S_PLAYER 'U' → 'D', A2S_RULES 'V' → 'E').
fn query_one(ip: u32, port: u16, kind: u8, want: u8, cancelled: &AtomicBool) -> Option<Vec<u8>> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(100))).ok()?;
    let mut q = vec![0xFF, 0xFF, 0xFF, 0xFF, kind, 0xFF, 0xFF, 0xFF, 0xFF];
    sock.send_to(&q, addr(ip, port)).ok()?;
    let start = Instant::now();
    let mut parts: HashMap<u8, Vec<u8>> = HashMap::new();
    let mut buf = [0u8; 1500];
    while start.elapsed() < Duration::from_secs(3) && !cancelled.load(Ordering::Relaxed) {
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        let d = &buf[..n];
        if n >= 9 && d[..4] == [0xFF; 4] && d[4] == b'A' {
            q.truncate(5);
            q.extend_from_slice(&d[5..9]);
            sock.send_to(&q, addr(ip, port)).ok()?;
        } else if n >= 5 && d[..4] == [0xFF; 4] && d[4] == want {
            return Some(d[5..].to_vec());
        } else if n >= 12 && d[..4] == [0xFE, 0xFF, 0xFF, 0xFF] {
            // Split reply (Source format): id, total, number, max size.
            let (total, number) = (d[8], d[9]);
            parts.insert(number, d[12..].to_vec());
            if parts.len() == total as usize {
                let whole: Vec<u8> = (0..total).flat_map(|i| parts.remove(&i).unwrap_or_default()).collect();
                if whole.len() >= 5 && whole[4] == want {
                    return Some(whole[5..].to_vec());
                }
                return None;
            }
        }
    }
    None
}

/// gameserveritem_t for a server.
fn item(s: &GameServer, ping: u32, responded: bool) -> Box<[u8; ITEM]> {
    let mut b = Bytes::new(ITEM);
    b.u32(0, s.game_port as u32 | (s.query_port as u32) << 16).u32(4, s.ip).i32(8, ping as i32).u8(12, responded as u8);
    b.str(14, 32, &s.game_dir).str(46, 32, &s.map).str(78, 64, &s.description);
    b.u32(144, s.app_id).i32(148, s.players as i32).i32(152, s.max_players as i32).i32(156, s.bots as i32);
    b.u8(160, s.password as u8).u8(161, s.secure as u8).i32(168, s.version as i32).str(172, 64, &s.name).str(236, 128, &s.tags).u64(368, s.steam_id);
    Box::new(b.0.try_into().expect("size"))
}

// --------------------------------------------------------------- lists

/// MatchMakingKeyValuePair_t (256-byte key, 256-byte value) list → "\key\value...".
fn filter_text(filters: *const *const u8, n: u32) -> String {
    if filters.is_null() || n == 0 {
        return String::new();
    }
    let list = unsafe { *filters };
    if list.is_null() {
        return String::new();
    }
    let mut out = String::new();
    for i in 0..n.min(64) as usize {
        let at = unsafe { list.add(i * 512) };
        let (k, v) = unsafe { (text(at as *const c_char), text(at.add(256) as *const c_char)) };
        out.push_str(&format!("\\{k}\\{v}"));
    }
    out
}

fn new_list(app_id: u32, filter: String, response: usize, lan: bool) -> usize {
    let mut st = s();
    st.next += 8;
    let h = st.next;
    st.lists.insert(h, List { app_id, filter, response, items: Vec::new(), refreshing: true, cancelled: Arc::new(AtomicBool::new(false)), lan });
    h
}

/// Fetch and query a list, then tell the game server by server.
fn run_list(h: usize) {
    let Some((app_id, filter, response, cancelled, lan)) = s().lists.get(&h).map(|l| (l.app_id, l.filter.clone(), l.response, l.cancelled.clone(), l.lan)) else { return };
    std::thread::spawn(move || {
        let (listed, fresh) = if lan {
            let targets: Vec<(u32, u16)> = (27015..=27020).chain(26900..=26905).map(|p| (u32::MAX, p)).collect();
            let found = query_all(&targets, Duration::from_millis(1500), &cancelled, true);
            (found.values().map(|(s, _)| s.clone()).collect::<Vec<_>>(), found)
        } else {
            let filter = if app_id != 0 && !filter.contains("\\appid\\") { format!("\\appid\\{app_id}{filter}") } else { filter };
            let listed = match crate::ipc::request(Request::ServerList { filter, max: 0 }) {
                Some(Reply::Servers { servers }) => servers,
                _ => Vec::new(),
            };
            let targets: Vec<(u32, u16)> = listed.iter().map(|s| (s.ip, s.query_port)).collect();
            let fresh = query_all(&targets, Duration::from_millis(2500), &cancelled, false);
            (listed, fresh)
        };
        crate::log!("servers: list {h}: {} listed, {} answered", listed.len(), fresh.len());
        let mut answered = Vec::new();
        let items: Vec<Box<[u8; ITEM]>> = listed
            .iter()
            .map(|s| match fresh.get(&(s.ip, s.query_port)) {
                Some((f, ping)) => {
                    answered.push(true);
                    // Fresh state from the server, the address from the master list.
                    item(&GameServer { ip: s.ip, query_port: s.query_port, steam_id: if f.steam_id != 0 { f.steam_id } else { s.steam_id }, ..f.clone() }, *ping, true)
                }
                None => {
                    answered.push(false);
                    item(s, 9999, false)
                }
            })
            .collect();
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        if let Some(l) = s().lists.get_mut(&h) {
            l.items = items;
        }
        on_game_thread(move || {
            for (i, ok) in answered.iter().enumerate() {
                if cancelled.load(Ordering::Relaxed) || !s().lists.contains_key(&h) {
                    return;
                }
                if *ok {
                    virt!(response, 0, fn(usize, i32), h, i as i32);
                } else {
                    virt!(response, 1, fn(usize, i32), h, i as i32);
                }
            }
            if let Some(l) = s().lists.get_mut(&h) {
                l.refreshing = false;
            }
            let result = if answered.is_empty() { NONE_LISTED } else if answered.iter().any(|a| *a) { RESPONDED } else { FAILED };
            virt!(response, 2, fn(usize, i32), h, result);
        });
    });
}

extern "C" fn internet(_t: This, app_id: u32, filters: *const *const u8, n: u32, response: *mut c_void) -> usize {
    if response.is_null() {
        return 0;
    }
    let h = new_list(app_id, filter_text(filters, n), response as usize, false);
    run_list(h);
    h
}

extern "C" fn lan(_t: This, app_id: u32, response: *mut c_void) -> usize {
    if response.is_null() {
        return 0;
    }
    let h = new_list(app_id, String::new(), response as usize, true);
    run_list(h);
    h
}

extern "C" fn empty_list(_t: This, app_id: u32, _filters: *const *const u8, _n: u32, response: *mut c_void) -> usize {
    if response.is_null() {
        return 0;
    }
    let h = new_list(app_id, String::new(), response as usize, false);
    let r = response as usize;
    on_game_thread(move || {
        if let Some(l) = s().lists.get_mut(&h) {
            l.refreshing = false;
        }
        virt!(r, 2, fn(usize, i32), h, NONE_LISTED);
    });
    h
}

extern "C" fn release(_t: This, h: usize) {
    if let Some(l) = s().lists.remove(&h) {
        l.cancelled.store(true, Ordering::Relaxed);
    }
}

extern "C" fn details(_t: This, h: usize, i: i32) -> *const u8 {
    let st = s();
    st.lists.get(&h).and_then(|l| l.items.get(i as usize)).map(|b| b.as_ptr()).unwrap_or(std::ptr::null())
}

extern "C" fn cancel(_t: This, h: usize) {
    if let Some(l) = s().lists.get_mut(&h) {
        l.cancelled.store(true, Ordering::Relaxed);
        l.refreshing = false;
    }
}

extern "C" fn refresh(_t: This, h: usize) {
    if let Some(l) = s().lists.get_mut(&h) {
        l.cancelled = Arc::new(AtomicBool::new(false));
        l.refreshing = true;
    } else {
        return;
    }
    run_list(h);
}

extern "C" fn is_refreshing(_t: This, h: usize) -> bool {
    s().lists.get(&h).is_some_and(|l| l.refreshing)
}

extern "C" fn count(_t: This, h: usize) -> i32 {
    s().lists.get(&h).map(|l| l.items.len() as i32).unwrap_or(0)
}

/// Ask one listed server again; ServerResponded / ServerFailedToRespond after.
extern "C" fn refresh_server(_t: This, h: usize, i: i32) {
    let Some((response, ip, port)) = s().lists.get(&h).and_then(|l| {
        let b = l.items.get(i as usize)?;
        Some((l.response, u32::from_le_bytes(b[4..8].try_into().unwrap()), u16::from_le_bytes(b[2..4].try_into().unwrap())))
    }) else {
        return;
    };
    std::thread::spawn(move || {
        let found = query_all(&[(ip, port)], Duration::from_secs(2), &AtomicBool::new(false), false);
        let ok = if let Some((f, ping)) = found.get(&(ip, port)) {
            if let Some(slot) = s().lists.get_mut(&h).and_then(|l| l.items.get_mut(i as usize)) {
                *slot = item(&GameServer { ip, query_port: port, ..f.clone() }, *ping, true);
            }
            true
        } else {
            false
        };
        on_game_thread(move || {
            if s().lists.contains_key(&h) {
                virt!(response, if ok { 0 } else { 1 }, fn(usize, i32), h, i);
            }
        });
    });
}

// ---------------------------------------------------- single-server queries

fn new_query() -> (i32, Arc<AtomicBool>) {
    let mut st = s();
    st.next += 1;
    let id = st.next as i32;
    let flag = Arc::new(AtomicBool::new(false));
    st.queries.insert(id, flag.clone());
    (id, flag)
}

fn done(id: i32) {
    s().queries.remove(&id);
}

/// ISteamMatchmakingPingResponse: ServerResponded(gameserveritem_t&), ServerFailedToRespond().
extern "C" fn ping(_t: This, ip: u32, port: u16, response: *mut c_void) -> i32 {
    let r = response as usize;
    if r == 0 {
        return -1;
    }
    let (id, cancelled) = new_query();
    std::thread::spawn(move || {
        let found = query_all(&[(ip, port)], Duration::from_secs(3), &cancelled, false).remove(&(ip, port));
        on_game_thread(move || {
            done(id);
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            match found {
                Some((s, p)) => {
                    let it = item(&GameServer { ip, query_port: port, ..s }, p, true);
                    virt!(r, 0, fn(*const u8), it.as_ptr());
                }
                None => virt!(r, 1, fn(),),
            }
        });
    });
    id
}

/// ISteamMatchmakingPlayersResponse: AddPlayerToList(name, score, seconds), failed, complete.
extern "C" fn players(_t: This, ip: u32, port: u16, response: *mut c_void) -> i32 {
    let r = response as usize;
    if r == 0 {
        return -1;
    }
    let (id, cancelled) = new_query();
    std::thread::spawn(move || {
        let list = query_one(ip, port, b'U', b'D', &cancelled).map(|d| {
            let mut at = 1;
            let mut out = Vec::new();
            for _ in 0..d.first().copied().unwrap_or(0) {
                at += 1;
                let name = cstr(&d, &mut at);
                let Some(b) = d.get(at..at + 8) else { break };
                out.push((name, i32::from_le_bytes(b[0..4].try_into().unwrap()), f32::from_le_bytes(b[4..8].try_into().unwrap())));
                at += 8;
            }
            out
        });
        on_game_thread(move || {
            done(id);
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            match list {
                Some(list) => {
                    for (name, score, time) in list {
                        let c = CString::new(name.replace('\0', "")).unwrap_or_default();
                        virt!(r, 0, fn(*const c_char, i32, f32), c.as_ptr(), score, time);
                    }
                    virt!(r, 2, fn(),);
                }
                None => virt!(r, 1, fn(),),
            }
        });
    });
    id
}

/// ISteamMatchmakingRulesResponse: RulesResponded(rule, value), failed, complete.
extern "C" fn rules(_t: This, ip: u32, port: u16, response: *mut c_void) -> i32 {
    let r = response as usize;
    if r == 0 {
        return -1;
    }
    let (id, cancelled) = new_query();
    std::thread::spawn(move || {
        let list = query_one(ip, port, b'V', b'E', &cancelled).map(|d| {
            let n = d.get(0..2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
            let mut at = 2;
            (0..n).map_while(|_| (at < d.len()).then(|| (cstr(&d, &mut at), cstr(&d, &mut at)))).collect::<Vec<_>>()
        });
        on_game_thread(move || {
            done(id);
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            match list {
                Some(list) => {
                    for (k, v) in list {
                        let (k, v) = (CString::new(k.replace('\0', "")).unwrap_or_default(), CString::new(v.replace('\0', "")).unwrap_or_default());
                        virt!(r, 0, fn(*const c_char, *const c_char), k.as_ptr(), v.as_ptr());
                    }
                    virt!(r, 2, fn(),);
                }
                None => virt!(r, 1, fn(),),
            }
        });
    });
    id
}

extern "C" fn cancel_server_query(_t: This, id: i32) {
    if let Some(f) = s().queries.remove(&id) {
        f.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a2s_info_and_item_layout() {
        let mut d = vec![0xFF, 0xFF, 0xFF, 0xFF, b'I', 17];
        for s in ["My Server", "de_dust2", "csgo", "Counter-Strike"] {
            d.extend_from_slice(s.as_bytes());
            d.push(0);
        }
        d.extend_from_slice(&730u16.to_le_bytes());
        d.extend_from_slice(&[5, 10, 1, b'd', b'w', 1, 1]);
        d.extend_from_slice(b"1.0.0.1\0");
        d.push(0x80 | 0x10 | 0x20);
        d.extend_from_slice(&27015u16.to_le_bytes());
        d.extend_from_slice(&90000000000000001u64.to_le_bytes());
        d.extend_from_slice(b"secure,casual\0");
        let s = parse_info(&d, 0x7F000001, 27016).unwrap();
        assert_eq!((s.name.as_str(), s.map.as_str(), s.players, s.max_players, s.bots, s.password, s.secure), ("My Server", "de_dust2", 5, 10, 1, true, true));
        assert_eq!((s.game_port, s.steam_id, s.tags.as_str(), s.version), (27015, 90000000000000001, "secure,casual", 1001));
        let it = item(&s, 42, true);
        assert_eq!(u16::from_le_bytes(it[0..2].try_into().unwrap()), 27015);
        assert_eq!(u16::from_le_bytes(it[2..4].try_into().unwrap()), 27016);
        assert_eq!(i32::from_le_bytes(it[8..12].try_into().unwrap()), 42);
        assert_eq!(&it[172..181], b"My Server");
        assert_eq!(u64::from_le_bytes(it[368..376].try_into().unwrap()), 90000000000000001);
    }

    #[test]
    fn filters_become_text() {
        let mut pairs = vec![0u8; 1024];
        pairs[..7].copy_from_slice(b"gamedir");
        pairs[256..260].copy_from_slice(b"csgo");
        pairs[512..516].copy_from_slice(b"full");
        pairs[768..769].copy_from_slice(b"1");
        let p = pairs.as_ptr();
        assert_eq!(filter_text(&p, 2), "\\gamedir\\csgo\\full\\1");
    }
}

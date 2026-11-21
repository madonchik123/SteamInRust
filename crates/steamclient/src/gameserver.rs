//! `ISteamGameServer` and `ISteamGameServerStats`: a game hosting a listen or
//! dedicated server.
//!
//! The server is a Steam identity of its own. SteamInRust signs it in on a
//! separate connection (anonymous, or with a game server login token) and
//! keeps its listing up to date; this side keeps the details the game set,
//! answers server-browser queries (A2S) on the query port, and turns Steam's
//! answers into the callbacks the game's server code waits for. They arrive
//! on the game server's own callback pipe, apart from the player's.
//!
//! Never advertised as VAC-secured: games with VAC don't run on this DLL.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, put, slice, text};
use crate::protocol::{GsInfo, Reply, Request};
use crate::state::{self, Bytes, StatsState, lock};
use crate::vt::Fp;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::{c_char, c_void};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

// Callback ids.
const CB_SERVERS_CONNECTED: i32 = 101;
const CB_CONNECT_FAILURE: i32 = 102;
const CB_SERVERS_DISCONNECTED: i32 = 103;
const CB_POLICY_RESPONSE: i32 = 115;
const CB_VALIDATE_TICKET: i32 = 143;
const CB_STATS_RECEIVED: i32 = 1800;
const CB_STATS_STORED: i32 = 1801;

/// The game reads the query port's traffic itself and passes it to
/// HandleIncomingPacket (STEAMGAMESERVER_QUERY_PORT_SHARED).
const QUERY_SHARED: u16 = 0xFFFF;
/// k_EBeginAuthSessionResult*.
const BEGIN_OK: i32 = 0;
const BEGIN_INVALID: i32 = 1;
const BEGIN_DUPLICATE: i32 = 2;
/// k_EAuthSessionResponseUserNotConnectedToSteam.
const RESPONSE_NOT_CONNECTED: u32 = 1;
const STAT_INT: u8 = 1;
const STAT_FLOAT: u8 = 2;
const STAT_AVG: u8 = 3;

struct Player {
    id: u64,
    name: String,
    score: u32,
    joined: Instant,
}

/// What a player's auth ticket says they own.
#[derive(Debug, Clone, PartialEq)]
pub struct Licenses {
    pub steam_id: u64,
    pub apps: Vec<u32>,
}

#[derive(Default)]
struct Server {
    info: GsInfo,
    /// LogOn was called and LogOff wasn't.
    wanted: bool,
    token: String,
    logged_on: bool,
    steam_id: u64,
    public_ip: u32,
    rules: BTreeMap<String, String>,
    players: Vec<Player>,
    /// From the tickets of players in an auth session.
    licenses: HashMap<u64, Option<Licenses>>,
    outgoing: VecDeque<(Vec<u8>, u32, u16)>,
    /// Details changed since the last update to Steam.
    dirty: bool,
    stats: HashMap<u64, StatsState>,
    next_anon: u32,
    /// The query port being served and the flag that stops its thread.
    query: Option<(u16, Arc<AtomicBool>)>,
}

static SERVER: LazyLock<Mutex<Server>> = LazyLock::new(Mutex::default);
static SECRET: LazyLock<u64> = LazyLock::new(rand::random);

fn s() -> MutexGuard<'static, Server> {
    SERVER.lock().unwrap_or_else(|e| e.into_inner())
}

/// The game server's Steam ID (0 until it has signed in).
pub fn steam_id() -> u64 {
    let g = s();
    if g.logged_on { g.steam_id } else { 0 }
}

/// Wait for the sign-in; the Steam ID, or 0 if it didn't happen in time.
pub fn wait_signed_in(wait: Duration) -> u64 {
    let end = Instant::now() + wait;
    loop {
        let id = steam_id();
        if id != 0 || Instant::now() >= end {
            return id;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn imp(kind: &str, name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match (kind, name) {
        ("ISteamGameServer", n) => match n {
            "InitGameServer" => f!(init),
            "SetProduct" => f!(set_product),
            "SetGameDescription" => f!(set_description),
            "SetModDir" => f!(set_mod_dir),
            "SetDedicatedServer" => f!(set_dedicated),
            "LogOn" => f!(log_on_token),
            "LogOnAnonymous" => f!(log_on_anonymous),
            "LogOff" => f!(log_off),
            "BLoggedOn" => f!(logged_on),
            "GetSteamID" => f!(ret::get_steam_id),
            "SetMaxPlayerCount" => f!(set_max_players),
            "SetBotPlayerCount" => f!(set_bots),
            "SetServerName" => f!(set_name),
            "SetMapName" => f!(set_map),
            "SetPasswordProtected" => f!(set_password),
            "SetSpectatorPort" => f!(set_spectator_port),
            "SetSpectatorServerName" => f!(set_spectator_name),
            "ClearAllKeyValues" => f!(clear_rules),
            "SetKeyValue" => f!(set_rule),
            "SetGameTags" => f!(set_tags),
            "SetGameData" => f!(set_game_data),
            "SetRegion" => f!(set_region),
            "SetAdvertiseServerActive" | "EnableHeartbeats" => f!(set_active),
            "GetAuthSessionTicket" => f!(no_ticket),
            "BeginAuthSession" => f!(begin_auth),
            "EndAuthSession" => f!(end_auth),
            "UserHasLicenseForApp" => f!(has_license),
            "GetPublicIP" => f!(ret::get_public_ip),
            "HandleIncomingPacket" => f!(handle_packet),
            "GetNextOutgoingPacket" => f!(next_packet),
            "CreateUnauthenticatedUserConnection" => f!(ret::new_unauthenticated),
            "SendUserDisconnect" | "SendUserDisconnect_DEPRECATED" => f!(end_auth),
            "BUpdateUserData" => f!(update_user),
            // Everything else stays a stub: false, 0, or no API call.
            _ => None,
        },
        ("ISteamGameServerStats", n) => match n {
            "RequestUserStats" => f!(stats_request),
            "GetUserStatInt32" => f!(stats_get_int),
            "GetUserStatFloat" => f!(stats_get_float),
            "GetUserAchievement" => f!(stats_get_achievement),
            "SetUserStatInt32" => f!(stats_set_int),
            "SetUserStatFloat" => f!(stats_set_float),
            "UpdateUserAvgRateStat" => f!(stats_update_avg),
            "SetUserAchievement" => f!(stats_set_achievement),
            "ClearUserAchievement" => f!(stats_clear_achievement),
            "StoreUserStats" => f!(stats_store),
            _ => None,
        },
        _ => None,
    }
}

// ------------------------------------------------------------ details

fn change(f: impl FnOnce(&mut GsInfo)) {
    let mut g = s();
    let before = g.info.clone();
    f(&mut g.info);
    if g.info != before {
        g.dirty = true;
    }
}

extern "C" fn init(_t: This, ip: u32, game_port: u16, query_port: u16, _flags: u32, app: u32, version: *const c_char) -> bool {
    let version = unsafe { text(version) };
    let app = if app != 0 { app } else { lock().app_id() };
    crate::log!("game server: init app {app}, game port {game_port}, query port {query_port}, version {version}");
    change(|i| {
        i.app_id = app;
        i.ip = ip;
        i.game_port = game_port;
        i.query_port = query_port;
        i.version = version;
    });
    serve_queries(ip, query_port);
    true
}

macro_rules! text_setter {
    ($name:ident, $field:ident) => {
        extern "C" fn $name(_t: This, v: *const c_char) {
            let v = unsafe { text(v) };
            change(|i| i.$field = v);
        }
    };
}
text_setter!(set_product, product);
text_setter!(set_description, description);
text_setter!(set_mod_dir, mod_dir);
text_setter!(set_name, name);
text_setter!(set_map, map);
text_setter!(set_spectator_name, spectator_name);
text_setter!(set_tags, tags);
text_setter!(set_game_data, game_data);
text_setter!(set_region, region);

extern "C" fn set_dedicated(_t: This, v: bool) {
    change(|i| i.dedicated = v);
}
extern "C" fn set_max_players(_t: This, v: i32) {
    change(|i| i.max_players = v);
}
extern "C" fn set_bots(_t: This, v: i32) {
    change(|i| i.bots = v);
}
extern "C" fn set_password(_t: This, v: bool) {
    change(|i| i.password = v);
}
extern "C" fn set_spectator_port(_t: This, v: u16) {
    change(|i| i.spectator_port = v);
}
extern "C" fn set_active(_t: This, v: bool) {
    change(|i| i.active = v);
}
extern "C" fn clear_rules(_t: This) {
    s().rules.clear();
}
extern "C" fn set_rule(_t: This, k: *const c_char, v: *const c_char) {
    let (k, v) = unsafe { (text(k), text(v)) };
    if !k.is_empty() {
        s().rules.insert(k, v);
    }
}

// ------------------------------------------------------------ sign-in

extern "C" fn log_on_token(_t: This, token: *const c_char) {
    log_on(unsafe { text(token) });
}
extern "C" fn log_on_anonymous(_t: This) {
    log_on(String::new());
}

fn log_on(token: String) {
    let info = {
        let mut g = s();
        if g.wanted && g.token == token {
            return;
        }
        g.wanted = true;
        g.token = token.clone();
        g.dirty = false;
        g.info.clone()
    };
    start_updates();
    crate::log!("game server: signing in ({})", if token.is_empty() { "anonymous" } else { "login token" });
    std::thread::spawn(move || {
        let reply = crate::ipc::request(Request::GsLogOn { token, info: Box::new(info) });
        if !s().wanted {
            let _ = crate::ipc::request(Request::GsLogOff);
            return;
        }
        match reply {
            Some(Reply::GsLoggedOn { eresult: state::RESULT_OK, steam_id, public_ip }) => on_connected(steam_id, public_ip),
            other => {
                let code = match other {
                    Some(Reply::GsLoggedOn { eresult, .. }) => eresult,
                    _ => state::RESULT_NO_CONNECTION,
                };
                crate::log!("game server: sign-in failed ({code})");
                s().wanted = false;
                let mut b = Bytes::new(8);
                b.i32(0, code);
                lock().post_gs(CB_CONNECT_FAILURE, b.0);
            }
        }
    });
}

fn on_connected(steam_id: u64, public_ip: u32) {
    let changed = {
        let mut g = s();
        let changed = g.steam_id != steam_id;
        g.logged_on = true;
        g.steam_id = steam_id;
        g.public_ip = public_ip;
        // What the game set while signing in goes out now.
        g.dirty = true;
        changed
    };
    if changed {
        crate::sns::server_identity_changed();
    }
    crate::log!("game server: signed in as {steam_id}");
    let mut st = lock();
    st.post_gs(CB_SERVERS_CONNECTED, vec![0]);
    // GSPolicyResponse_t: not VAC-secured.
    st.post_gs(CB_POLICY_RESPONSE, vec![0]);
}

/// Steam's connection for the server came back or went away.
pub fn on_status(connected: bool, eresult: i32, steam_id: u64, public_ip: u32) {
    if !s().wanted {
        return;
    }
    if connected {
        on_connected(steam_id, public_ip);
    } else {
        crate::log!("game server: lost Steam ({eresult})");
        s().logged_on = false;
        let mut b = Bytes::new(4);
        b.i32(0, eresult);
        lock().post_gs(CB_SERVERS_DISCONNECTED, b.0);
    }
}

extern "C" fn log_off(_t: This) {
    shutdown();
}

/// LogOff, or the game released the server's Steam user.
pub fn shutdown() {
    let was_on = {
        let mut g = s();
        let on = g.wanted;
        g.wanted = false;
        g.logged_on = false;
        g.token.clear();
        g.licenses.clear();
        g.players.clear();
        g.info.players.clear();
        on
    };
    if was_on {
        crate::log!("game server: signing off");
        let _ = crate::ipc::request(Request::GsLogOff);
        let mut b = Bytes::new(4);
        b.i32(0, state::RESULT_NO_CONNECTION);
        lock().post_gs(CB_SERVERS_DISCONNECTED, b.0);
    }
}

extern "C" fn logged_on(_t: This) -> bool {
    s().logged_on
}

/// Send changed details to Steam, at most once a second.
fn start_updates() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let info = {
                    let mut g = s();
                    if !(g.dirty && g.logged_on) {
                        continue;
                    }
                    g.dirty = false;
                    g.info.clone()
                };
                let _ = crate::ipc::request(Request::GsUpdate(Box::new(info)));
            }
        });
    });
}

// ------------------------------------------------- players and tickets

/// An id from CreateUnauthenticatedUserConnection (a bot or similar).
fn is_unauthenticated(id: u64) -> bool {
    id >> 52 == 0x01A
}

fn sync_players(g: &mut Server) {
    // Steam only hears about real accounts.
    let list: Vec<u64> = g.players.iter().map(|p| p.id).filter(|id| !is_unauthenticated(*id)).collect();
    if g.info.players != list {
        g.info.players = list;
        g.dirty = true;
    }
}

fn add_player(g: &mut Server, id: u64) {
    if !g.players.iter().any(|p| p.id == id) {
        g.players.push(Player { id, name: String::new(), score: 0, joined: Instant::now() });
        sync_players(g);
    }
}

/// What a session ticket says about its owner: `[len][token][24][session
/// header][len][ownership ticket]`, the ownership ticket holding the Steam
/// ID, app and DLC. `None` if it doesn't have that shape.
pub fn ticket_licenses(ticket: &[u8]) -> Option<Licenses> {
    let u32_at = |d: &[u8], at: usize| Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?));
    let u16_at = |d: &[u8], at: usize| Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?) as usize);
    let token = u32_at(ticket, 0)? as usize;
    let session_at = 4usize.checked_add(token)?;
    if u32_at(ticket, session_at)? != 24 {
        return None;
    }
    let own_len_at = session_at + 4 + 24;
    let len = u32_at(ticket, own_len_at)? as usize;
    let t = ticket.get(own_len_at + 4..(own_len_at + 4).checked_add(len)?)?;
    // length, version, Steam ID, app, two addresses, flags, two times.
    let steam_id = u64::from_le_bytes(t.get(8..16)?.try_into().ok()?);
    let mut apps = vec![u32_at(t, 16)?];
    let mut at = 40;
    at += 2 + u16_at(t, at)? * 4; // licenses
    let dlc = u16_at(t, at)?;
    at += 2;
    for _ in 0..dlc {
        apps.push(u32_at(t, at)?);
        at += 4 + 2 + u16_at(t, at + 4)? * 4;
    }
    t.get(..at)?;
    Some(Licenses { steam_id, apps })
}

/// An anonymous server gets no game connect tokens from Steam, so it can't
/// make a ticket of its own: k_HAuthTicketInvalid, like Steam without tokens.
extern "C" fn no_ticket(_t: This, _buf: *mut u8, _max: i32, len: *mut u32) -> u32 {
    put(len, 0);
    0
}

extern "C" fn begin_auth(_t: This, ticket: *const u8, len: i32, user: u64) -> i32 {
    if ticket.is_null() || len <= 0 || len > 8192 {
        return BEGIN_INVALID;
    }
    let ticket = slice(ticket, len as usize).to_vec();
    let licenses = ticket_licenses(&ticket);
    // A ticket made for someone else never starts a session.
    if licenses.as_ref().is_some_and(|l| l.steam_id != user) {
        crate::log!("game server: ticket isn't {user}'s");
        return BEGIN_INVALID;
    }
    {
        let mut g = s();
        if g.licenses.contains_key(&user) {
            return BEGIN_DUPLICATE;
        }
        g.licenses.insert(user, licenses);
        add_player(&mut g, user);
    }
    std::thread::spawn(move || {
        let (response, owner) = match crate::ipc::request(Request::GsBeginAuth { steam_id: user, ticket }) {
            Some(Reply::AuthResult { response, owner }) => (response, owner),
            _ => (RESPONSE_NOT_CONNECTED, 0),
        };
        crate::log!("game server: ticket of {user}: response {response}");
        if response != 0 {
            let mut g = s();
            g.licenses.remove(&user);
            g.players.retain(|p| p.id != user);
            sync_players(&mut g);
        }
        let mut b = Bytes::new(24);
        b.u64(0, user).i32(8, response as i32).u64(16, if owner == 0 { user } else { owner });
        lock().post_gs(CB_VALIDATE_TICKET, b.0);
    });
    BEGIN_OK
}

extern "C" fn end_auth(_t: This, user: u64) {
    let had = {
        let mut g = s();
        let had = g.licenses.remove(&user).is_some();
        g.players.retain(|p| p.id != user);
        sync_players(&mut g);
        g.stats.remove(&user);
        had
    };
    if had {
        std::thread::spawn(move || crate::ipc::request(Request::GsEndAuth { steam_id: user }));
    }
}

/// EUserHasLicenseForAppResult: 0 has it, 1 doesn't, 2 no auth session.
extern "C" fn has_license(_t: This, user: u64, app: u32) -> i32 {
    let g = s();
    match g.licenses.get(&user) {
        None => 2,
        Some(Some(l)) => !l.apps.contains(&app) as i32,
        // A ticket we couldn't read: only the served game is known.
        Some(None) if app == g.info.app_id => 0,
        Some(None) => 2,
    }
}

extern "C" fn update_user(_t: This, user: u64, name: *const c_char, score: u32) -> bool {
    let name = unsafe { text(name) };
    let mut g = s();
    add_player(&mut g, user);
    if let Some(p) = g.players.iter_mut().find(|p| p.id == user) {
        p.name = name;
        p.score = score;
    }
    true
}

fn new_unauthenticated_id() -> u64 {
    let mut g = s();
    g.next_anon += 1;
    // An anonymous user in the public universe, unique on this server.
    let id = (1 << 56) | (10 << 52) | g.next_anon as u64;
    add_player(&mut g, id);
    id
}

fn public_ip_struct() -> [u8; 20] {
    // SteamIPAddress_t: IPv4 in the union, type 0 (k_ESteamIPTypeIPv4).
    let mut b = [0u8; 20];
    b[..4].copy_from_slice(&s().public_ip.to_le_bytes());
    b
}

#[cfg(windows)]
mod ret {
    use super::This;
    pub extern "C" fn get_steam_id(_t: This, out: *mut u64) -> *mut u64 {
        crate::ifaces::put(out, super::s().steam_id);
        out
    }
    pub extern "C" fn get_public_ip(_t: This, out: *mut [u8; 20]) -> *mut [u8; 20] {
        crate::ifaces::put(out, super::public_ip_struct());
        out
    }
    pub extern "C" fn new_unauthenticated(_t: This, out: *mut u64) -> *mut u64 {
        crate::ifaces::put(out, super::new_unauthenticated_id());
        out
    }
}

#[cfg(not(windows))]
mod ret {
    use super::This;
    #[repr(C)]
    pub struct Ip([u8; 20]);
    pub extern "C" fn get_steam_id(_t: This) -> u64 {
        super::s().steam_id
    }
    pub extern "C" fn get_public_ip(_t: This) -> Ip {
        Ip(super::public_ip_struct())
    }
    pub extern "C" fn new_unauthenticated(_t: This) -> u64 {
        super::new_unauthenticated_id()
    }
}

// ------------------------------------------------- server-browser queries

/// What A2S queries are answered from.
#[derive(Default)]
pub struct QueryView {
    pub info: GsInfo,
    pub steam_id: u64,
    /// (name, score, seconds on the server).
    pub players: Vec<(String, u32, f32)>,
    pub rules: Vec<(String, String)>,
    pub secret: u64,
    /// How many of `players` are bots the game registered with
    /// CreateUnauthenticatedUserConnection (already in its bot count).
    pub listed_bots: usize,
}

fn view(g: &Server) -> QueryView {
    QueryView {
        info: g.info.clone(),
        steam_id: g.steam_id,
        players: g.players.iter().map(|p| (p.name.clone(), p.score, p.joined.elapsed().as_secs_f32())).collect(),
        rules: g.rules.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        secret: *SECRET,
        listed_bots: g.players.iter().filter(|p| is_unauthenticated(p.id)).count(),
    }
}

/// The challenge a client at `ip` must send back, so replies can't be
/// aimed at a spoofed address.
fn challenge(secret: u64, ip: u32) -> u32 {
    let mixed = (secret ^ ip as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    ((mixed >> 32) as u32).min(0xFFFF_FFFE)
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend(s.bytes().filter(|b| *b != 0));
    out.push(0);
}

fn info_reply(v: &QueryView) -> Vec<u8> {
    let i = &v.info;
    let mut o = vec![0xFF, 0xFF, 0xFF, 0xFF, b'I', 17];
    push_str(&mut o, &i.name);
    push_str(&mut o, &i.map);
    push_str(&mut o, &i.mod_dir);
    push_str(&mut o, &i.description);
    o.extend_from_slice(&(i.app_id.min(0xFFFF) as u16).to_le_bytes());
    let byte = |n: usize| n.min(255) as u8;
    // Bots count as players; those the game registered are in the list already.
    o.push(byte(v.players.len() + (i.bots.max(0) as usize).saturating_sub(v.listed_bots)));
    o.push(byte(i.max_players.max(0) as usize));
    o.push(byte(i.bots.max(0) as usize));
    o.push(if i.dedicated { b'd' } else { b'l' });
    o.push(if cfg!(windows) { b'w' } else if cfg!(target_os = "macos") { b'm' } else { b'l' });
    o.push(i.password as u8);
    o.push(0); // never VAC-secured
    push_str(&mut o, &i.version);
    // Extra data: game port, Steam ID, tags, full app id.
    let mut edf = 0x80 | 0x01;
    if v.steam_id != 0 {
        edf |= 0x10;
    }
    if !i.tags.is_empty() {
        edf |= 0x20;
    }
    o.push(edf);
    o.extend_from_slice(&i.game_port.to_le_bytes());
    if v.steam_id != 0 {
        o.extend_from_slice(&v.steam_id.to_le_bytes());
    }
    if !i.tags.is_empty() {
        push_str(&mut o, &i.tags);
    }
    o.extend_from_slice(&(i.app_id as u64).to_le_bytes());
    o
}

fn players_reply(v: &QueryView) -> Vec<u8> {
    let mut o = vec![0xFF, 0xFF, 0xFF, 0xFF, b'D', v.players.len().min(255) as u8];
    for (index, (name, score, seconds)) in v.players.iter().take(255).enumerate() {
        o.push(index as u8);
        push_str(&mut o, name);
        o.extend_from_slice(&score.to_le_bytes());
        o.extend_from_slice(&seconds.to_le_bytes());
    }
    o
}

fn rules_reply(v: &QueryView) -> Vec<u8> {
    let mut o = vec![0xFF, 0xFF, 0xFF, 0xFF, b'E'];
    o.extend_from_slice(&(v.rules.len().min(0xFFFF) as u16).to_le_bytes());
    for (k, val) in v.rules.iter().take(0xFFFF) {
        push_str(&mut o, k);
        push_str(&mut o, val);
    }
    o
}

/// One reply as UDP packets: long ones use Valve's split format.
fn packets(reply: Vec<u8>, id: u32) -> Vec<Vec<u8>> {
    const MAX: usize = 1400;
    const PART: usize = 1248;
    if reply.len() <= MAX {
        return vec![reply];
    }
    let parts: Vec<&[u8]> = reply.chunks(PART).take(255).collect();
    let total = parts.len() as u8;
    parts
        .iter()
        .enumerate()
        .map(|(n, part)| {
            let mut p = vec![0xFE, 0xFF, 0xFF, 0xFF];
            p.extend_from_slice(&id.to_le_bytes());
            p.push(total);
            p.push(n as u8);
            p.extend_from_slice(&(PART as u16).to_le_bytes());
            p.extend_from_slice(part);
            p
        })
        .collect()
}

/// The answer to an A2S query from `ip`; nothing if it isn't one. Every
/// query needs the client's challenge first.
pub fn answer(q: &[u8], ip: u32, v: &QueryView) -> Vec<Vec<u8>> {
    const INFO: &[u8] = b"Source Engine Query\0";
    if q.len() < 5 || q[..4] != [0xFF; 4] {
        return Vec::new();
    }
    let given = match q[4] {
        b'T' if q[5..].starts_with(INFO) => q.get(5 + INFO.len()..9 + INFO.len()),
        b'U' | b'V' => q.get(5..9),
        _ => return Vec::new(),
    };
    let expected = challenge(v.secret, ip);
    if given.map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) != Some(expected) {
        let mut c = vec![0xFF, 0xFF, 0xFF, 0xFF, b'A'];
        c.extend_from_slice(&expected.to_le_bytes());
        return vec![c];
    }
    let reply = match q[4] {
        b'T' => info_reply(v),
        b'U' => players_reply(v),
        _ => rules_reply(v),
    };
    packets(reply, expected)
}

/// Answer queries on our own UDP port, unless the game shares its socket.
fn serve_queries(ip: u32, port: u16) {
    let mut g = s();
    if g.query.as_ref().is_some_and(|(p, _)| *p == port) {
        return;
    }
    if let Some((_, stop)) = g.query.take() {
        stop.store(true, Ordering::Relaxed);
    }
    if port == 0 || port == QUERY_SHARED {
        return;
    }
    let bind = std::net::Ipv4Addr::from(ip);
    let sock = match UdpSocket::bind((bind, port)) {
        Ok(sock) => sock,
        Err(e) => {
            // Usually the game's own socket on the same port.
            crate::log!("game server: can't open query port {port}: {e}");
            return;
        }
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let stop = Arc::new(AtomicBool::new(false));
    g.query = Some((port, stop.clone()));
    drop(g);
    crate::log!("game server: answering queries on port {port}");
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        while !stop.load(Ordering::Relaxed) {
            let Ok((n, SocketAddr::V4(from))) = sock.recv_from(&mut buf) else { continue };
            let snapshot = view(&s());
            for p in answer(&buf[..n], u32::from(*from.ip()), &snapshot) {
                let _ = sock.send_to(&p, from);
            }
        }
    });
}

/// A packet from the game's shared socket: true if it was a query for us.
extern "C" fn handle_packet(_t: This, data: *const c_void, len: i32, ip: u32, port: u16) -> bool {
    let d = slice(data.cast(), len.max(0) as usize);
    let snapshot = view(&s());
    let out = answer(d, ip, &snapshot);
    if out.is_empty() {
        return false;
    }
    let mut g = s();
    for p in out {
        if g.outgoing.len() < 256 {
            g.outgoing.push_back((p, ip, port));
        }
    }
    true
}

/// The next reply for the game to send from its shared socket (size, 0 if none).
extern "C" fn next_packet(_t: This, out: *mut u8, max: i32, ip: *mut u32, port: *mut u16) -> i32 {
    let mut g = s();
    let fits = g.outgoing.front().is_some_and(|(p, _, _)| p.len() <= max.max(0) as usize);
    if !fits || out.is_null() {
        return 0;
    }
    let Some((p, to, to_port)) = g.outgoing.pop_front() else { return 0 };
    unsafe { std::ptr::copy_nonoverlapping(p.as_ptr(), out, p.len()) };
    put(ip, to);
    put(port, to_port);
    p.len() as i32
}

// ------------------------------------------------- ISteamGameServerStats

fn stats_result(user: u64, eresult: i32) -> Vec<u8> {
    let mut b = Bytes::new(16);
    b.i32(0, eresult).u64(8, user);
    b.0
}

extern "C" fn stats_request(_t: This, user: u64) -> u64 {
    let call = lock().new_call();
    std::thread::spawn(move || {
        let eresult = match crate::ipc::request(Request::GsUserStats { steam_id: user }) {
            Some(Reply::Stats { result: Ok(stats) }) => {
                let mut st = StatsState::default();
                st.load(&stats);
                s().stats.insert(user, st);
                state::RESULT_OK
            }
            _ => state::RESULT_FAIL,
        };
        lock().complete_gs(call, CB_STATS_RECEIVED, stats_result(user, eresult), false);
    });
    call
}

extern "C" fn stats_get_int(_t: This, user: u64, name: *const c_char, out: *mut i32) -> bool {
    let n = unsafe { text(name) };
    let mut g = s();
    match g.stats.get_mut(&user).and_then(|st| st.stat(&n)) {
        Some(e) if e.def.kind == STAT_INT => {
            put(out, e.value as i32);
            true
        }
        _ => false,
    }
}

extern "C" fn stats_get_float(_t: This, user: u64, name: *const c_char, out: *mut f32) -> bool {
    let n = unsafe { text(name) };
    let mut g = s();
    match g.stats.get_mut(&user).and_then(|st| st.stat(&n)) {
        Some(e) if matches!(e.def.kind, STAT_FLOAT | STAT_AVG) => {
            put(out, f32::from_bits(e.value));
            true
        }
        _ => false,
    }
}

extern "C" fn stats_get_achievement(_t: This, user: u64, name: *const c_char, out: *mut bool) -> bool {
    let n = unsafe { text(name) };
    let g = s();
    let Some(st) = g.stats.get(&user) else { return false };
    let Some(i) = st.ach(&n) else { return false };
    put(out, st.achs[i].unlocked);
    true
}

/// Whether a stat may take this value (its limits, increment-only).
fn allowed(e: &state::StatEntry, value: f64, current: f64) -> bool {
    e.def.min.is_none_or(|m| value >= m) && e.def.max.is_none_or(|m| value <= m) && (!e.def.increment_only || value >= current)
}

extern "C" fn stats_set_int(_t: This, user: u64, name: *const c_char, v: i32) -> bool {
    let n = unsafe { text(name) };
    let mut g = s();
    let Some(e) = g.stats.get_mut(&user).and_then(|st| st.stat(&n)) else { return false };
    if e.def.kind != STAT_INT || !allowed(e, v as f64, e.value as i32 as f64) {
        return false;
    }
    e.value = v as u32;
    true
}

extern "C" fn stats_set_float(_t: This, user: u64, name: *const c_char, v: f32) -> bool {
    let n = unsafe { text(name) };
    let mut g = s();
    let Some(e) = g.stats.get_mut(&user).and_then(|st| st.stat(&n)) else { return false };
    if e.def.kind != STAT_FLOAT || !v.is_finite() || !allowed(e, v as f64, f32::from_bits(e.value) as f64) {
        return false;
    }
    e.value = v.to_bits();
    true
}

extern "C" fn stats_update_avg(_t: This, user: u64, name: *const c_char, count: f32, seconds: f64) -> bool {
    let n = unsafe { text(name) };
    let mut g = s();
    let Some(e) = g.stats.get_mut(&user).and_then(|st| st.stat(&n)) else { return false };
    if e.def.kind != STAT_AVG || seconds <= 0.0 {
        return false;
    }
    e.value = ((count as f64 / seconds) as f32).to_bits();
    true
}

fn set_achievement(user: u64, name: *const c_char, on: bool) -> bool {
    let n = unsafe { text(name) };
    let mut g = s();
    let Some(st) = g.stats.get_mut(&user) else { return false };
    let Some(i) = st.ach(&n) else { return false };
    st.set_ach(i, on);
    true
}

extern "C" fn stats_set_achievement(_t: This, user: u64, name: *const c_char) -> bool {
    set_achievement(user, name, true)
}

extern "C" fn stats_clear_achievement(_t: This, user: u64, name: *const c_char) -> bool {
    set_achievement(user, name, false)
}

extern "C" fn stats_store(_t: This, user: u64) -> u64 {
    let call = lock().new_call();
    let pending = {
        let g = s();
        g.stats.get(&user).map(|st| (st.stats.iter().filter(|e| e.value != e.stored).map(|e| (e.def.id, e.value)).collect::<Vec<_>>(), st.crc))
    };
    let Some((changes, crc)) = pending else {
        lock().complete_gs(call, CB_STATS_STORED, stats_result(user, state::RESULT_FAIL), false);
        return call;
    };
    if changes.is_empty() {
        lock().complete_gs(call, CB_STATS_STORED, stats_result(user, state::RESULT_OK), false);
        return call;
    }
    std::thread::spawn(move || {
        let eresult = match crate::ipc::request(Request::GsStoreStats { steam_id: user, stats: changes.clone(), crc }) {
            Some(Reply::Stored { eresult, crc }) => {
                if eresult == state::RESULT_OK
                    && let Some(st) = s().stats.get_mut(&user)
                {
                    st.crc = crc;
                    for (id, v) in &changes {
                        if let Some(e) = st.stats.iter_mut().find(|e| e.def.id == *id) {
                            e.stored = *v;
                        }
                    }
                }
                eresult
            }
            _ => state::RESULT_NO_CONNECTION,
        };
        lock().complete_gs(call, CB_STATS_STORED, stats_result(user, eresult), false);
    });
    call
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QueryView {
        QueryView {
            info: GsInfo {
                app_id: 480,
                game_port: 27015,
                query_port: 27016,
                name: "Test server".into(),
                map: "arena".into(),
                mod_dir: "spacewar".into(),
                description: "Spacewar".into(),
                version: "1.2.3.4".into(),
                tags: "a,b".into(),
                dedicated: true,
                max_players: 8,
                bots: 1,
                ..Default::default()
            },
            steam_id: 90_000_000_000_000_001,
            players: vec![("Ann".into(), 5, 12.5)],
            rules: vec![("mode".into(), "ffa".into())],
            secret: 42,
            listed_bots: 0,
        }
    }

    /// Ask with the challenge the server hands out, like a real browser.
    fn ask(query: &[u8], v: &QueryView) -> Vec<u8> {
        let first = answer(query, 0x7F00_0001, v);
        assert_eq!((first.len(), first[0][4]), (1, b'A'));
        let mut q = query.to_vec();
        if query[4] != b'T' {
            q.truncate(5);
        }
        q.extend_from_slice(&first[0][5..9]);
        let mut parts = answer(&q, 0x7F00_0001, v);
        assert_eq!(parts.len(), 1);
        parts.remove(0)
    }

    #[test]
    fn info_reply_reads_back_in_our_own_server_browser() {
        let d = ask(b"\xFF\xFF\xFF\xFFTSource Engine Query\0", &sample());
        assert_eq!(&d[..6], &[0xFF, 0xFF, 0xFF, 0xFF, b'I', 17]);
        let text = String::from_utf8_lossy(&d);
        for s in ["Test server", "arena", "spacewar", "Spacewar", "1.2.3.4", "a,b"] {
            assert!(text.contains(s), "{s}");
        }
        // After the four strings: app, players (1 + 1 bot), max, bots, type, OS, password, VAC.
        let at = 6 + "Test server\0arena\0spacewar\0Spacewar\0".len();
        assert_eq!(&d[at..at + 2], &480u16.to_le_bytes());
        assert_eq!((d[at + 2], d[at + 3], d[at + 4], d[at + 5]), (2, 8, 1, b'd'));
        assert_eq!(d[at + 8], 0, "never VAC-secured");
    }

    #[test]
    fn registered_bots_are_not_counted_twice() {
        let mut v = sample();
        v.players.push(("Bot".into(), 0, 1.0));
        v.listed_bots = 1;
        let d = ask(b"\xFF\xFF\xFF\xFFTSource Engine Query\0", &v);
        let at = 6 + "Test server\0arena\0spacewar\0Spacewar\0".len();
        assert_eq!((d[at + 2], d[at + 4]), (2, 1), "one person and one bot");
        assert!(is_unauthenticated((1 << 56) | (10 << 52) | 7));
        assert!(!is_unauthenticated(76561197960287930));
    }

    #[test]
    fn players_and_rules_need_a_challenge() {
        let v = sample();
        let none = [0xFF, 0xFF, 0xFF, 0xFF, b'U', 0xFF, 0xFF, 0xFF, 0xFF];
        let d = ask(&none, &v);
        assert_eq!((d[4], d[5], d[6]), (b'D', 1, 0));
        assert_eq!(&d[7..11], b"Ann\0");
        assert_eq!(&d[11..15], &5u32.to_le_bytes());
        let d = ask(&[0xFF, 0xFF, 0xFF, 0xFF, b'V', 0xFF, 0xFF, 0xFF, 0xFF], &v);
        assert_eq!((d[4], u16::from_le_bytes([d[5], d[6]])), (b'E', 1));
        assert_eq!(&d[7..], b"mode\0ffa\0");
        // A challenge for one address isn't valid from another.
        assert_ne!(challenge(42, 1), challenge(42, 2));
        assert!(answer(b"not a query", 1, &v).is_empty());
    }

    #[test]
    fn long_replies_split_like_valves() {
        let reply = vec![7u8; 3000];
        let p = packets(reply.clone(), 9);
        assert_eq!(p.len(), 3);
        assert_eq!(&p[1][..4], &[0xFE, 0xFF, 0xFF, 0xFF]);
        assert_eq!(u32::from_le_bytes(p[1][4..8].try_into().unwrap()), 9);
        assert_eq!((p[1][8], p[1][9]), (3, 1));
        assert!(p.iter().all(|x| x.len() <= 1400));
        let joined: Vec<u8> = p.iter().flat_map(|x| x[12..].to_vec()).collect();
        assert_eq!(joined, reply);
    }

    fn ticket(owner: u64, app: u32, dlc: &[u32]) -> Vec<u8> {
        let mut own = Vec::new();
        own.extend_from_slice(&0u32.to_le_bytes());
        own.extend_from_slice(&4u32.to_le_bytes());
        own.extend_from_slice(&owner.to_le_bytes());
        own.extend_from_slice(&app.to_le_bytes());
        own.extend_from_slice(&[0; 20]); // addresses, flags, times
        own.extend_from_slice(&1u16.to_le_bytes());
        own.extend_from_slice(&7u32.to_le_bytes()); // one license
        own.extend_from_slice(&(dlc.len() as u16).to_le_bytes());
        for d in dlc {
            own.extend_from_slice(&d.to_le_bytes());
            own.extend_from_slice(&0u16.to_le_bytes());
        }
        let mut t = Vec::new();
        t.extend_from_slice(&20u32.to_le_bytes());
        t.extend_from_slice(&[1; 20]);
        t.extend_from_slice(&24u32.to_le_bytes());
        t.extend_from_slice(&[2; 24]);
        t.extend_from_slice(&(own.len() as u32).to_le_bytes());
        t.extend_from_slice(&own);
        t
    }

    #[test]
    fn reads_owner_and_dlc_from_a_ticket() {
        let l = ticket_licenses(&ticket(76561197960287930, 480, &[481, 482])).unwrap();
        assert_eq!(l, Licenses { steam_id: 76561197960287930, apps: vec![480, 481, 482] });
        assert!(ticket_licenses(&[1, 2, 3]).is_none());
        let mut cut = ticket(1, 480, &[481]);
        cut.truncate(cut.len() - 3);
        assert!(ticket_licenses(&cut).is_none());
    }
}

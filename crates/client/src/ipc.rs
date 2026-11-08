//! The app's side of the pipe that games with our steamclient DLL talk to
//! (see `crates/steamclient`).
//!
//! [`Server`] accepts connections on a per-run pipe name. Every request
//! carries the token SteamInRust gave that launch; known tokens become a
//! [`Call`] for the online loop in `session.rs`, which answers from its own
//! state and the CM connection (helpers below: user stats, storing stats,
//! rich presence, player counts).

#[path = "../../steamclient/src/protocol.rs"]
pub mod protocol;

use model::AppId;
use net::Connection;
use protocol::{Achievement, GameEvent, Reply, Request, Stat, Stats};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

/// A game's request, for the online loop to answer through `reply`.
pub struct Call {
    pub app_id: AppId,
    pub token: String,
    pub pid: u32,
    pub req: Request,
    pub reply: oneshot::Sender<Reply>,
}

pub struct Server {
    name: String,
    tokens: Tokens,
    task: Option<tokio::task::AbortHandle>,
}

/// One running launch: its game and the events waiting for its next Poll.
struct Slot {
    app_id: AppId,
    events: std::collections::VecDeque<GameEvent>,
    notify: Arc<tokio::sync::Notify>,
}

type Tokens = Arc<Mutex<HashMap<String, Slot>>>;

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(t) = &self.task {
            t.abort();
        }
    }
}

/// Hard-to-guess hex from the OS-seeded hasher keys, time and a counter.
fn random_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut out = String::new();
    for _ in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

impl Server {
    /// Start listening. Must run inside the tokio runtime.
    pub fn start() -> (Server, mpsc::UnboundedReceiver<Call>) {
        let name = format!(r"\\.\pipe\steaminrust-{}-{}", std::process::id(), &random_hex()[..12]);
        let tokens: Tokens = Arc::default();
        let (tx, rx) = mpsc::unbounded_channel();
        #[cfg(windows)]
        let task = Some(tokio::spawn(accept(name.clone(), tokens.clone(), tx)).abort_handle());
        #[cfg(not(windows))]
        let task = {
            drop(tx);
            None
        };
        (Server { name, tokens, task }, rx)
    }

    /// A new token for a game about to start.
    pub fn register(&self, app_id: AppId) -> String {
        let token = random_hex();
        self.tokens.lock().unwrap().insert(token.clone(), Slot { app_id, events: Default::default(), notify: Arc::default() });
        token
    }

    /// Queue an event for every running copy of `app_id`.
    pub fn push(&self, app_id: AppId, event: GameEvent) {
        for slot in self.tokens.lock().unwrap().values_mut().filter(|s| s.app_id == app_id) {
            if slot.events.len() < 4096 {
                slot.events.push_back(event.clone());
            }
            slot.notify.notify_one();
        }
    }

    /// Games running right now (for routing Steam's pushes).
    pub fn running_apps(&self) -> Vec<AppId> {
        let mut apps: Vec<AppId> = self.tokens.lock().unwrap().values().map(|s| s.app_id).collect();
        apps.sort_unstable();
        apps.dedup();
        apps
    }

    pub fn forget(&self, token: &str) {
        self.tokens.lock().unwrap().remove(token);
    }

    /// Value for the game's `STEAMINRUST_IPC` variable.
    pub fn env_value(&self, token: &str) -> String {
        format!("{}|{}", self.name, token)
    }
}

#[cfg(windows)]
async fn accept(name: String, tokens: Tokens, tx: mpsc::UnboundedSender<Call>) {
    use tokio::net::windows::named_pipe::ServerOptions;
    let mut first = true;
    loop {
        let server = match ServerOptions::new().first_pipe_instance(first).reject_remote_clients(true).create(&name) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("steamclient pipe {name}: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };
        first = false;
        if server.connect().await.is_err() {
            continue;
        }
        tokio::spawn(serve_one(server, tokens.clone(), tx.clone()));
    }
}

#[cfg(windows)]
async fn serve_one(pipe: tokio::net::windows::named_pipe::NamedPipeServer, tokens: Tokens, tx: mpsc::UnboundedSender<Call>) {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let (r, mut w) = tokio::io::split(pipe);
    let mut line = String::new();
    let mut reader = BufReader::new(r).take(1 << 20);
    let read = tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line)).await;
    let reply = match (read, serde_json::from_str::<protocol::Envelope>(line.trim())) {
        (Ok(Ok(_)), Ok(env)) => {
            let app_id = tokens.lock().unwrap().get(&env.token).map(|s| s.app_id);
            match app_id {
                None => Reply::Err { message: "unknown launch".into() },
                Some(_) if matches!(env.req, Request::Poll) => Reply::Events { events: poll(&tokens, &env.token).await },
                Some(app_id) => {
                    let (rtx, rrx) = oneshot::channel();
                    let _ = tx.send(Call { app_id, token: env.token, pid: env.pid, req: env.req, reply: rtx });
                    match tokio::time::timeout(Duration::from_secs(40), rrx).await {
                        Ok(Ok(r)) => r,
                        _ => Reply::Err { message: "SteamInRust didn't answer".into() },
                    }
                }
            }
        }
        _ => Reply::Err { message: "bad request".into() },
    };
    if let Ok(mut out) = serde_json::to_string(&reply) {
        out.push('\n');
        let _ = w.write_all(out.as_bytes()).await;
        let _ = w.flush().await;
    }
}

/// Hand over queued events, waiting up to 20 s for the first one.
#[cfg(windows)]
async fn poll(tokens: &Tokens, token: &str) -> Vec<GameEvent> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let notify = {
            let mut t = tokens.lock().unwrap();
            let Some(slot) = t.get_mut(token) else { return Vec::new() };
            if !slot.events.is_empty() {
                return slot.events.drain(..).collect();
            }
            slot.notify.clone()
        };
        if tokio::time::timeout_at(deadline, notify.notified()).await.is_err() {
            return Vec::new();
        }
    }
}

// ---------------------------------------------------------- user stats

/// Steam's user-stats messages (steammessages_clientserver_userstats.proto),
/// written out here because that .proto isn't vendored.
mod msg {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct GetUserStats {
        #[prost(fixed64, optional, tag = "1")]
        pub game_id: Option<u64>,
        #[prost(uint32, optional, tag = "2")]
        pub crc_stats: Option<u32>,
        #[prost(int32, optional, tag = "3")]
        pub schema_local_version: Option<i32>,
        #[prost(fixed64, optional, tag = "4")]
        pub steam_id_for_user: Option<u64>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct StatValue {
        #[prost(uint32, optional, tag = "1")]
        pub stat_id: Option<u32>,
        #[prost(uint32, optional, tag = "2")]
        pub stat_value: Option<u32>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct AchievementBlock {
        #[prost(uint32, optional, tag = "1")]
        pub achievement_id: Option<u32>,
        #[prost(fixed32, repeated, packed = "false", tag = "2")]
        pub unlock_time: Vec<u32>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct GetUserStatsResponse {
        #[prost(fixed64, optional, tag = "1")]
        pub game_id: Option<u64>,
        #[prost(int32, optional, tag = "2")]
        pub eresult: Option<i32>,
        #[prost(uint32, optional, tag = "3")]
        pub crc_stats: Option<u32>,
        #[prost(bytes = "vec", optional, tag = "4")]
        pub schema: Option<Vec<u8>>,
        #[prost(message, repeated, tag = "5")]
        pub stats: Vec<StatValue>,
        #[prost(message, repeated, tag = "6")]
        pub achievement_blocks: Vec<AchievementBlock>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct StoreUserStats2 {
        #[prost(fixed64, optional, tag = "1")]
        pub game_id: Option<u64>,
        #[prost(fixed64, optional, tag = "2")]
        pub settor_steam_id: Option<u64>,
        #[prost(fixed64, optional, tag = "3")]
        pub settee_steam_id: Option<u64>,
        #[prost(uint32, optional, tag = "4")]
        pub crc_stats: Option<u32>,
        #[prost(bool, optional, tag = "5")]
        pub explicit_reset: Option<bool>,
        #[prost(message, repeated, tag = "6")]
        pub stats: Vec<StatValue>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct StoreUserStatsResponse {
        #[prost(fixed64, optional, tag = "1")]
        pub game_id: Option<u64>,
        #[prost(int32, optional, tag = "2")]
        pub eresult: Option<i32>,
        #[prost(uint32, optional, tag = "3")]
        pub crc_stats: Option<u32>,
        #[prost(message, repeated, tag = "4")]
        pub stats_failed_validation: Vec<StatValue>,
        #[prost(bool, optional, tag = "5")]
        pub stats_out_of_date: Option<bool>,
    }
}

const EMSG_GET_USER_STATS: u32 = 818;
const EMSG_STORE_USER_STATS2: u32 = 5466;
const EMSG_RICH_PRESENCE_UPLOAD: u32 = 7501;
const EMSG_CURRENT_PLAYERS: u32 = 5592;

/// A user's stats and achievements for a game, with names from the game's
/// schema. `language` is Steam's name ("english") for display strings.
pub async fn user_stats(conn: &Connection, app_id: AppId, steam_id: u64, language: &str) -> Result<Stats, String> {
    let req = msg::GetUserStats { game_id: Some(app_id as u64), crc_stats: Some(0), schema_local_version: Some(-1), steam_id_for_user: Some(steam_id) };
    let reply = conn.request(EMSG_GET_USER_STATS, &req).await.map_err(|e| e.to_string())?;
    let r: msg::GetUserStatsResponse = reply.body().map_err(|e| e.to_string())?;
    let code = r.eresult.unwrap_or(2);
    if code != net::eresult::OK {
        return Err(net::eresult::describe(code, "user stats"));
    }
    let values: HashMap<u32, u32> = r.stats.iter().map(|s| (s.stat_id.unwrap_or(0), s.stat_value.unwrap_or(0))).collect();
    let times: HashMap<u32, Vec<u32>> = r.achievement_blocks.iter().map(|b| (b.achievement_id.unwrap_or(0), b.unlock_time.clone())).collect();
    let schema = r.schema.as_deref().and_then(|s| vdf::binary::parse(s).ok()).unwrap_or_default();
    let mut stats = parse_schema(&schema, app_id, language);
    stats.crc = r.crc_stats.unwrap_or(0);
    for s in &mut stats.stats {
        s.value = values.get(&s.id).copied().unwrap_or(s.default);
    }
    for a in &mut stats.achievements {
        let bits = values.get(&a.block).copied().unwrap_or(0);
        let time = times.get(&a.block).and_then(|t| t.get(a.bit as usize)).copied().unwrap_or(0);
        a.unlocked = (bits >> a.bit) & 1 == 1 || time != 0;
        a.unlock_time = if a.unlocked { time } else { 0 };
    }
    Ok(stats)
}

/// Stat and achievement definitions from the schema Steam sends with user
/// stats (binary KeyValues: `<appid> { stats { <id> { type name … bits { <bit> { name display {…} } } } } }`).
pub fn parse_schema(schema: &vdf::Obj, app_id: AppId, language: &str) -> Stats {
    let mut out = Stats::default();
    let root = schema.obj(&app_id.to_string()).or_else(|| schema.iter().find_map(|(_, v)| v.as_obj()));
    let Some(stats) = root.and_then(|r| r.obj("stats")) else { return out };
    for (key, v) in stats.iter() {
        let Some(o) = v.as_obj() else { continue };
        let Ok(id) = key.parse::<u32>() else { continue };
        let kind = match o.str("type_int").or(o.str("type")).unwrap_or("") {
            "1" | "INT" => 1,
            "2" | "FLOAT" => 2,
            "3" | "AVGRATE" => 3,
            "4" | "ACHIEVEMENTS" => 4,
            "5" | "GROUPACHIEVEMENTS" => 5,
            other => other.parse().unwrap_or(0),
        };
        match kind {
            1..=3 => {
                let num = |k: &str| o.str(k).and_then(|s| s.parse::<f64>().ok());
                let default = match kind {
                    1 => num("default").map(|d| d as i32 as u32).unwrap_or(0),
                    _ => num("default").map(|d| (d as f32).to_bits()).unwrap_or(0),
                };
                out.stats.push(Stat {
                    id,
                    name: o.str("name").unwrap_or_default().to_string(),
                    kind,
                    value: default,
                    default,
                    min: num("min"),
                    max: num("max"),
                    increment_only: o.str("incrementonly") == Some("1"),
                });
            }
            4 | 5 => {
                out.stats.push(Stat { id, name: String::new(), kind: 4, ..Default::default() });
                for (bit_key, bit) in o.obj("bits").map(|b| b.iter().collect::<Vec<_>>()).unwrap_or_default() {
                    let Some(b) = bit.as_obj() else { continue };
                    let bit_no = b.str("bit").or(Some(bit_key)).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let display = b.obj("display");
                    let text = |k: &str| display.and_then(|d| d.get(k)).map(|v| localized(v, language)).unwrap_or_default();
                    out.achievements.push(Achievement {
                        name: b.str("name").unwrap_or_default().to_string(),
                        block: id,
                        bit: bit_no,
                        display_name: text("name"),
                        description: text("desc"),
                        hidden: display.and_then(|d| d.str("hidden")) == Some("1"),
                        icon: display.and_then(|d| d.str("icon")).unwrap_or_default().to_string(),
                        icon_gray: display.and_then(|d| d.str("icon_gray")).unwrap_or_default().to_string(),
                        unlocked: false,
                        unlock_time: 0,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn localized(v: &vdf::Value, language: &str) -> String {
    match v {
        vdf::Value::Str(s) => s.clone(),
        vdf::Value::Obj(o) => o.str(language).or(o.str("english")).or_else(|| o.iter().find_map(|(k, v)| (k != "token").then(|| v.as_str()).flatten())).unwrap_or_default().to_string(),
    }
}

/// Save changed stats (achievement blocks included). Returns (EResult, new CRC).
pub async fn store_stats(conn: &Connection, app_id: AppId, me: u64, stats: &[(u32, u32)], crc: u32, reset: bool) -> (i32, u32) {
    let req = msg::StoreUserStats2 {
        game_id: Some(app_id as u64),
        settor_steam_id: Some(me),
        settee_steam_id: Some(me),
        crc_stats: Some(crc),
        explicit_reset: Some(reset),
        stats: stats.iter().map(|&(id, v)| msg::StatValue { stat_id: Some(id), stat_value: Some(v) }).collect(),
    };
    match conn.request(EMSG_STORE_USER_STATS2, &req).await {
        Ok(p) => match p.body::<msg::StoreUserStatsResponse>() {
            Ok(r) => {
                if !r.stats_failed_validation.is_empty() {
                    tracing::warn!("app {app_id}: {} stat(s) failed validation", r.stats_failed_validation.len());
                }
                (r.eresult.unwrap_or(2), r.crc_stats.unwrap_or(crc))
            }
            Err(_) => (2, crc),
        },
        Err(e) => {
            tracing::warn!("storing stats for {app_id}: {e}");
            (3, crc)
        }
    }
}

/// Send the game's rich presence (shown to friends: "In a match", …).
pub fn rich_presence(conn: &Connection, pairs: &[(String, String)]) {
    let msg = proto::CMsgClientRichPresenceUpload { rich_presence_kv: Some(rich_presence_kv(pairs)), steamid_broadcast: vec![] };
    let _ = conn.send(EMSG_RICH_PRESENCE_UPLOAD, &msg);
}

/// Binary KeyValues `"RP" { key value … }`.
fn rich_presence_kv(pairs: &[(String, String)]) -> Vec<u8> {
    let mut out = vec![0x00];
    out.extend_from_slice(b"RP\0");
    for (k, v) in pairs {
        out.push(0x01);
        out.extend_from_slice(k.replace('\0', "").as_bytes());
        out.push(0);
        out.extend_from_slice(v.replace('\0', "").as_bytes());
        out.push(0);
    }
    out.extend_from_slice(&[0x08, 0x08]);
    out
}

pub async fn current_players(conn: &Connection, app_id: AppId) -> Option<i32> {
    let reply = conn.request(EMSG_CURRENT_PLAYERS, &proto::CMsgDpGetNumberOfCurrentPlayers { appid: Some(app_id) }).await.ok()?;
    let r: proto::CMsgDpGetNumberOfCurrentPlayersResponse = reply.body().ok()?;
    (r.eresult() == net::eresult::OK).then(|| r.player_count())
}

// ---------------------------------------------------- auth tickets

const EMSG_GET_OWNERSHIP_TICKET: u32 = 857;
pub const EMSG_AUTH_LIST: u32 = 5432;
pub const EMSG_GAME_CONNECT_TOKENS: u32 = 779;
pub const EMSG_AUTH_LIST_ACK: u32 = 5575;
pub const EMSG_TICKET_AUTH_COMPLETE: u32 = 5429;
const EMSG_REQUEST_ENCRYPTED_TICKET: u32 = 5526;

/// Auth session tickets, the way Steam's client makes them: a game connect
/// token (Steam sends a stock at sign-in), a session header and the game's
/// ownership ticket. Steam learns about each ticket from `ClientAuthList`,
/// so a game server can then check it with Steam.
pub struct Auth {
    tokens: std::collections::VecDeque<Vec<u8>>,
    keep: u32,
    /// Tickets Steam should know about: ours (estate 0) and other
    /// players' we're checking as a host (estate 1).
    tickets: Vec<proto::CMsgAuthTicket>,
    seq_me: u32,
    seq_them: u32,
    acked: u32,
    ack_waiters: Vec<(u32, oneshot::Sender<()>)>,
    validations: Vec<(u32, oneshot::Sender<(u32, u64)>)>,
    ownership: HashMap<AppId, Vec<u8>>,
    public_ip: u32,
    since: std::time::Instant,
    connections: u32,
}

pub type SharedAuth = Arc<Mutex<Auth>>;

impl Auth {
    pub fn new(public_ip: u32) -> SharedAuth {
        Arc::new(Mutex::new(Auth {
            tokens: Default::default(),
            keep: 10,
            tickets: Vec::new(),
            seq_me: 0,
            seq_them: 0,
            acked: 0,
            ack_waiters: Vec::new(),
            validations: Vec::new(),
            ownership: HashMap::new(),
            public_ip,
            since: std::time::Instant::now(),
            connections: 0,
        }))
    }

    /// `ClientGameConnectTokens`: Steam's stock of tokens for new tickets.
    pub fn add_tokens(&mut self, m: &proto::CMsgClientGameConnectTokens) {
        self.keep = m.max_tokens_to_keep().max(1);
        self.tokens.extend(m.tokens.iter().cloned());
        while self.tokens.len() > self.keep as usize {
            self.tokens.pop_front();
        }
    }

    pub fn on_ack(&mut self, m: &proto::CMsgClientAuthListAck) {
        self.acked = self.acked.max(m.message_sequence());
        let acked = self.acked;
        let (done, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut self.ack_waiters).into_iter().partition(|(seq, _)| *seq <= acked);
        self.ack_waiters = wait;
        for (_, tx) in done {
            let _ = tx.send(());
        }
    }

    pub fn on_auth_complete(&mut self, m: &proto::CMsgClientTicketAuthComplete) {
        self.seq_them = self.seq_them.max(m.ticket_sequence());
        let crc = m.ticket_crc();
        tracing::info!("Steam checked ticket {crc:08x}: state {} response {}", m.estate(), m.eauth_session_response());
        let (done, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut self.validations).into_iter().partition(|(c, _)| *c == crc);
        self.validations = wait;
        for (_, tx) in done {
            let _ = tx.send((m.eauth_session_response(), m.owner_steam_id()));
        }
    }

    /// The list to send Steam now (bumps the sequence number).
    fn list(&mut self) -> (u32, proto::CMsgClientAuthList) {
        self.seq_me += 1;
        let mut apps: Vec<u32> = self.tickets.iter().map(|t| t.gameid() as u32).collect();
        apps.sort_unstable();
        apps.dedup();
        let msg = proto::CMsgClientAuthList {
            tokens_left: Some(self.tokens.len() as u32),
            last_request_seq: Some(self.seq_me - 1),
            last_request_seq_from_server: Some(self.seq_them),
            tickets: self.tickets.clone(),
            app_ids: apps,
            message_sequence: Some(self.seq_me),
            filtered: None,
        };
        (self.seq_me, msg)
    }

    /// A full session ticket for `app_id` and its CRC (Steam's id for it).
    fn new_ticket(&mut self, app_id: AppId, pipe: u32, ownership: &[u8]) -> Result<(Vec<u8>, u32), String> {
        let token = self.tokens.pop_front().ok_or("Steam hasn't sent game connect tokens yet")?;
        self.connections += 1;
        let ticket = session_ticket(&token, self.public_ip, self.since.elapsed().as_millis() as u32, self.connections, ownership);
        let auth = &ticket[..8 + token.len() + 24];
        let crc = crc32(auth);
        self.tickets.push(proto::CMsgAuthTicket {
            estate: Some(0),
            eresult: None,
            steamid: Some(0),
            gameid: Some(app_id as u64),
            h_steam_pipe: Some(pipe),
            ticket_crc: Some(crc),
            ticket: Some(auth.to_vec()),
            server_secret: None,
            ticket_type: None,
        });
        Ok((ticket, crc))
    }

    /// Forget tickets (ours and others') of a game that closed. True if any.
    pub fn drop_app(&mut self, app_id: AppId) -> bool {
        let before = self.tickets.len();
        self.tickets.retain(|t| t.gameid() != app_id as u64);
        before != self.tickets.len()
    }
}

/// `[len][connect token][24][session header][len][ownership ticket]`.
fn session_ticket(token: &[u8], public_ip: u32, ms_connected: u32, connections: u32, ownership: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(64 + ownership.len());
    b.extend_from_slice(&(token.len() as u32).to_le_bytes());
    b.extend_from_slice(token);
    b.extend_from_slice(&24u32.to_le_bytes());
    for v in [1u32, 2, public_ip, 0, ms_connected, connections] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(ownership.len() as u32).to_le_bytes());
    b.extend_from_slice(ownership);
    b
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(data);
    c.sum()
}

/// The part of a player's ticket Steam checks (without the ownership ticket).
fn auth_part(ticket: &[u8]) -> &[u8] {
    let len = ticket.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).unwrap_or(0);
    let end = 4 + len + 4 + 24;
    if (1..=64).contains(&len) && ticket.len() >= end { &ticket[..end] } else { ticket }
}

async fn ownership_ticket(conn: &Connection, auth: &SharedAuth, app_id: AppId) -> Result<Vec<u8>, String> {
    if let Some(t) = auth.lock().unwrap().ownership.get(&app_id) {
        return Ok(t.clone());
    }
    let reply = conn.request(EMSG_GET_OWNERSHIP_TICKET, &proto::CMsgClientGetAppOwnershipTicket { app_id: Some(app_id) }).await.map_err(|e| e.to_string())?;
    let r: proto::CMsgClientGetAppOwnershipTicketResponse = reply.body().map_err(|e| e.to_string())?;
    if r.eresult() as i32 != net::eresult::OK || r.ticket().is_empty() {
        return Err(net::eresult::describe(r.eresult() as i32, "ownership ticket"));
    }
    auth.lock().unwrap().ownership.insert(app_id, r.ticket().to_vec());
    Ok(r.ticket().to_vec())
}

fn send_list(conn: &Connection, auth: &SharedAuth) -> u32 {
    let (seq, list) = auth.lock().unwrap().list();
    let _ = conn.send(EMSG_AUTH_LIST, &list);
    seq
}

/// A new session ticket for the game, registered with Steam: (ticket, CRC, list number).
pub async fn auth_ticket(conn: &Connection, auth: &SharedAuth, app_id: AppId, pid: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let own = ownership_ticket(conn, auth, app_id).await?;
    let (ticket, crc) = auth.lock().unwrap().new_ticket(app_id, pid, &own)?;
    let seq = send_list(conn, auth);
    tracing::info!("app {app_id}: auth ticket {crc:08x} ({} bytes)", ticket.len());
    Ok((ticket, crc, seq))
}

/// Wait until Steam acknowledged ticket list `seq`.
pub async fn wait_ack(auth: &SharedAuth, seq: u32) -> bool {
    let rx = {
        let mut a = auth.lock().unwrap();
        if a.acked >= seq {
            return true;
        }
        let (tx, rx) = oneshot::channel();
        a.ack_waiters.push((seq, tx));
        rx
    };
    matches!(tokio::time::timeout(std::time::Duration::from_secs(15), rx).await, Ok(Ok(())))
}

pub fn cancel_ticket(conn: &Connection, auth: &SharedAuth, crc: u32) {
    let removed = {
        let mut a = auth.lock().unwrap();
        let before = a.tickets.len();
        a.tickets.retain(|t| !(t.estate() == 0 && t.ticket_crc() == crc));
        before != a.tickets.len()
    };
    if removed {
        send_list(conn, auth);
    }
}

/// Check another player's ticket with Steam: (EAuthSessionResponse, owner).
pub async fn begin_auth(conn: &Connection, auth: &SharedAuth, app_id: AppId, pid: u32, steam_id: u64, ticket: &[u8]) -> (u32, u64) {
    let part = auth_part(ticket).to_vec();
    let crc = crc32(&part);
    let rx = {
        let mut a = auth.lock().unwrap();
        a.tickets.retain(|t| !(t.estate() == 1 && t.steamid() == steam_id && t.gameid() == app_id as u64));
        a.tickets.push(proto::CMsgAuthTicket {
            estate: Some(1),
            eresult: None,
            steamid: Some(steam_id),
            gameid: Some(app_id as u64),
            h_steam_pipe: Some(pid),
            ticket_crc: Some(crc),
            ticket: Some(part),
            server_secret: None,
            ticket_type: None,
        });
        let (tx, rx) = oneshot::channel();
        a.validations.push((crc, tx));
        rx
    };
    send_list(conn, auth);
    match tokio::time::timeout(std::time::Duration::from_secs(20), rx).await {
        Ok(Ok(r)) => r,
        // k_EAuthSessionResponseAuthTicketInvalid
        _ => (8, 0),
    }
}

pub fn end_auth(conn: &Connection, auth: &SharedAuth, app_id: AppId, steam_id: u64) {
    let removed = {
        let mut a = auth.lock().unwrap();
        let before = a.tickets.len();
        a.tickets.retain(|t| !(t.estate() == 1 && t.steamid() == steam_id && t.gameid() == app_id as u64));
        before != a.tickets.len()
    };
    if removed {
        send_list(conn, auth);
    }
}

/// A game closed: its tickets go away, like with Steam.
pub fn game_closed(conn: &Connection, auth: &SharedAuth, app_id: AppId) {
    if auth.lock().unwrap().drop_app(app_id) {
        send_list(conn, auth);
    }
}

/// An encrypted app ticket (serialized `EncryptedAppTicket`) with `data` inside.
pub async fn encrypted_ticket(conn: &Connection, app_id: AppId, data: &[u8]) -> Result<Vec<u8>, String> {
    let req = proto::CMsgClientRequestEncryptedAppTicket { app_id: Some(app_id), userdata: Some(data.to_vec()) };
    let reply = conn.request(EMSG_REQUEST_ENCRYPTED_TICKET, &req).await.map_err(|e| e.to_string())?;
    let r: proto::CMsgClientRequestEncryptedAppTicketResponse = reply.body().map_err(|e| e.to_string())?;
    if r.eresult() != net::eresult::OK {
        return Err(net::eresult::describe(r.eresult(), "encrypted app ticket"));
    }
    let t = r.encrypted_app_ticket.ok_or("no ticket in Steam's answer")?;
    Ok(prost::Message::encode_to_vec(&t))
}

// ------------------------------------------------- lobbies (matchmaking)

pub const EMSG_MMS_CREATE_LOBBY: u32 = 6601;
pub const EMSG_MMS_JOIN_LOBBY: u32 = 6603;
pub const EMSG_MMS_LEAVE_LOBBY: u32 = 6605;
pub const EMSG_MMS_GET_LOBBY_LIST: u32 = 6607;
pub const EMSG_MMS_SET_LOBBY_DATA: u32 = 6609;
pub const EMSG_MMS_GET_LOBBY_DATA: u32 = 6611;
pub const EMSG_MMS_LOBBY_DATA: u32 = 6612;
pub const EMSG_MMS_SEND_LOBBY_CHAT: u32 = 6613;
pub const EMSG_MMS_LOBBY_CHAT: u32 = 6614;
pub const EMSG_MMS_SET_LOBBY_OWNER: u32 = 6615;
pub const EMSG_MMS_LOBBY_GAME_SERVER_SET: u32 = 6618;
pub const EMSG_MMS_USER_JOINED_LOBBY: u32 = 6619;
pub const EMSG_MMS_USER_LEFT_LOBBY: u32 = 6620;
pub const EMSG_MMS_INVITE_TO_LOBBY: u32 = 6621;
pub const EMSG_P2P_CONNECTION_INFO: u32 = 5434;

use proto::webui::{
    CMsgClientMmsCreateLobby, CMsgClientMmsCreateLobbyResponse, CMsgClientMmsGetLobbyData, CMsgClientMmsGetLobbyList, CMsgClientMmsGetLobbyListFilter,
    CMsgClientMmsGetLobbyListResponse, CMsgClientMmsInviteToLobby, CMsgClientMmsJoinLobby, CMsgClientMmsJoinLobbyResponse, CMsgClientMmsLeaveLobby,
    CMsgClientMmsLobbyChatMsg, CMsgClientMmsLobbyData, CMsgClientMmsLobbyGameServerSet, CMsgClientMmsSendLobbyChatMsg, CMsgClientMmsSetLobbyData,
    CMsgClientMmsSetLobbyDataResponse, CMsgClientMmsSetLobbyOwner, CMsgClientMmsUserJoinedLobby, CMsgClientMmsUserLeftLobby, MmsKeyValuePair,
};

/// Matchmaking messages go to the game's own matchmaking service: Steam
/// routes them by the app id in the header.
async fn mms_request(conn: &Connection, app_id: AppId, emsg: u32, body: &impl prost::Message) -> net::Result<net::Packet> {
    conn.request_with(emsg, proto::CMsgProtoBufHeader { routing_appid: Some(app_id), ..Default::default() }, body).await
}

fn mms_send(conn: &Connection, app_id: AppId, emsg: u32, body: &impl prost::Message) -> net::Result<()> {
    conn.send_with(emsg, proto::CMsgProtoBufHeader { routing_appid: Some(app_id), ..Default::default() }, body)
}

/// Lobby metadata as Steam's matchmaking server stores it: binary
/// KeyValues `"" { key value … }`.
pub fn lobby_kv(pairs: &[(String, String)]) -> Vec<u8> {
    let mut out = vec![0x00, 0x00];
    for (k, v) in pairs {
        out.push(0x01);
        out.extend_from_slice(k.replace('\0', "").as_bytes());
        out.push(0);
        out.extend_from_slice(v.replace('\0', "").as_bytes());
        out.push(0);
    }
    out.extend_from_slice(&[0x08, 0x08]);
    out
}

/// Metadata from either form Steam may send (key/value list, or binary KV).
fn lobby_pairs(pairs: &[MmsKeyValuePair], kv: Option<&[u8]>) -> Vec<(String, String)> {
    if !pairs.is_empty() {
        return pairs.iter().map(|p| (p.name().to_string(), p.value().to_string())).collect();
    }
    let Some(obj) = kv.filter(|b| !b.is_empty()).and_then(|b| vdf::binary::parse(b).ok()) else { return Vec::new() };
    // Children of the (unnamed) root object.
    let root = obj.iter().find_map(|(_, v)| v.as_obj()).unwrap_or(&obj);
    root.iter().filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string()))).collect()
}

fn lobby_from_data(m: &CMsgClientMmsLobbyData) -> protocol::Lobby {
    protocol::Lobby {
        id: m.steam_id_lobby(),
        owner: m.steam_id_owner(),
        kind: m.lobby_type(),
        flags: m.lobby_flags(),
        max_members: m.max_members(),
        num_members: m.num_members().max(m.members.len() as i32),
        data: lobby_pairs(&m.metadata, m.metadata_kv.as_deref()),
        members: m
            .members
            .iter()
            .map(|x| protocol::LobbyMember { steam_id: x.steam_id(), name: x.persona_name().to_string(), data: lobby_pairs(&x.metadata, x.metadata_kv.as_deref()) })
            .collect(),
    }
}

pub async fn lobby_create(conn: &Connection, app_id: AppId, kind: i32, max_members: i32, name: &str) -> (i32, u64) {
    let req = CMsgClientMmsCreateLobby {
        app_id: Some(app_id),
        max_members: Some(max_members),
        lobby_type: Some(kind),
        lobby_flags: Some(0),
        persona_name_owner: Some(name.to_string()),
        metadata_kv: Some(lobby_kv(&[])),
        ..Default::default()
    };
    match mms_request(conn, app_id, EMSG_MMS_CREATE_LOBBY, &req).await.and_then(|p| p.body::<CMsgClientMmsCreateLobbyResponse>()) {
        Ok(r) => (r.eresult(), r.steam_id_lobby()),
        Err(e) => {
            tracing::warn!("create lobby: {e}");
            (3, 0)
        }
    }
}

pub async fn lobby_join(conn: &Connection, app_id: AppId, lobby: u64, name: &str) -> (i32, Option<protocol::Lobby>) {
    let req = CMsgClientMmsJoinLobby { app_id: Some(app_id), steam_id_lobby: Some(lobby), persona_name: Some(name.to_string()), ..Default::default() };
    match mms_request(conn, app_id, EMSG_MMS_JOIN_LOBBY, &req).await.and_then(|p| p.body::<CMsgClientMmsJoinLobbyResponse>()) {
        Ok(r) => {
            let response = r.chat_room_enter_response();
            let info = (response == 1).then(|| protocol::Lobby {
                id: lobby,
                owner: r.steam_id_owner(),
                kind: r.lobby_type(),
                flags: r.lobby_flags(),
                max_members: r.max_members(),
                num_members: r.members.len() as i32,
                data: lobby_pairs(&r.metadata, r.metadata_kv.as_deref()),
                members: r
                    .members
                    .iter()
                    .map(|x| protocol::LobbyMember { steam_id: x.steam_id(), name: x.persona_name().to_string(), data: lobby_pairs(&x.metadata, x.metadata_kv.as_deref()) })
                    .collect(),
            });
            (response, info)
        }
        Err(e) => {
            tracing::warn!("join lobby: {e}");
            (5, None) // k_EChatRoomEnterResponseError
        }
    }
}

pub fn lobby_leave(conn: &Connection, app_id: AppId, lobby: u64) {
    let _ = mms_send(conn, app_id, EMSG_MMS_LEAVE_LOBBY, &CMsgClientMmsLeaveLobby { app_id: Some(app_id), steam_id_lobby: Some(lobby) });
}

pub async fn lobby_list(conn: &Connection, app_id: AppId, filters: &[protocol::LobbyFilter], count: i32) -> Vec<protocol::Lobby> {
    let req = CMsgClientMmsGetLobbyList {
        app_id: Some(app_id),
        num_lobbies_requested: Some(if count > 0 { count } else { 50 }),
        filters: filters
            .iter()
            .map(|f| CMsgClientMmsGetLobbyListFilter { key: Some(f.key.clone()), value: Some(f.value.clone()), comparision: Some(f.cmp), filter_type: Some(f.kind) })
            .collect(),
        ..Default::default()
    };
    match mms_request(conn, app_id, EMSG_MMS_GET_LOBBY_LIST, &req).await.and_then(|p| p.body::<CMsgClientMmsGetLobbyListResponse>()) {
        Ok(r) => r
            .lobbies
            .iter()
            .map(|l| protocol::Lobby {
                id: l.steam_id(),
                owner: 0,
                kind: l.lobby_type(),
                flags: l.lobby_flags(),
                max_members: l.max_members(),
                num_members: l.num_members(),
                data: lobby_pairs(&l.metadata, l.metadata_kv.as_deref()),
                members: Vec::new(),
            })
            .collect(),
        Err(e) => {
            tracing::warn!("lobby list: {e}");
            Vec::new()
        }
    }
}

/// Replace the lobby's data, or (`member`) our own member data. EResult.
pub async fn lobby_set_data(conn: &Connection, app_id: AppId, me: u64, lobby: u64, member: bool, data: &[(String, String)], max_members: i32, kind: i32, flags: i32) -> i32 {
    let req = CMsgClientMmsSetLobbyData {
        app_id: Some(app_id),
        steam_id_lobby: Some(lobby),
        steam_id_member: Some(if member { me } else { 0 }),
        max_members: (!member).then_some(max_members),
        lobby_type: (!member).then_some(kind),
        lobby_flags: (!member).then_some(flags),
        metadata_kv: Some(lobby_kv(data)),
        ..Default::default()
    };
    match mms_request(conn, app_id, EMSG_MMS_SET_LOBBY_DATA, &req).await.and_then(|p| p.body::<CMsgClientMmsSetLobbyDataResponse>()) {
        Ok(r) => r.eresult(),
        Err(e) => {
            tracing::warn!("set lobby data: {e}");
            3
        }
    }
}

/// Ask for a lobby's data; it arrives as a `LobbyData` push.
pub fn lobby_refresh(conn: &Connection, app_id: AppId, lobby: u64) {
    let _ = mms_send(conn, app_id, EMSG_MMS_GET_LOBBY_DATA, &CMsgClientMmsGetLobbyData { app_id: Some(app_id), steam_id_lobby: Some(lobby) });
}

/// A lobby chat message, to everyone or (`target`) one member.
pub fn lobby_chat(conn: &Connection, app_id: AppId, lobby: u64, target: u64, data: &[u8]) {
    let _ = mms_send(conn, app_id, EMSG_MMS_SEND_LOBBY_CHAT,
        &CMsgClientMmsSendLobbyChatMsg { app_id: Some(app_id), steam_id_lobby: Some(lobby), steam_id_target: Some(target), lobby_message: Some(data.to_vec()) },
    );
}

pub fn lobby_set_owner(conn: &Connection, app_id: AppId, lobby: u64, owner: u64) {
    let _ = mms_send(conn, app_id, EMSG_MMS_SET_LOBBY_OWNER, &CMsgClientMmsSetLobbyOwner { app_id: Some(app_id), steam_id_lobby: Some(lobby), steam_id_new_owner: Some(owner) });
}

pub fn lobby_invite(conn: &Connection, app_id: AppId, lobby: u64, user: u64) {
    let _ = mms_send(conn, app_id, EMSG_MMS_INVITE_TO_LOBBY, &CMsgClientMmsInviteToLobby { app_id: Some(app_id), steam_id_lobby: Some(lobby), steam_id_user_invited: Some(user) });
}

/// A P2P signalling message for another player, carried by Steam
/// (`ClientP2PConnectionInfo`, the message Steam's own P2P set-up uses).
pub fn signal(conn: &Connection, app_id: AppId, me: u64, to: u64, data: &[u8], lobby: u64) {
    let msg = proto::CMsgClientP2pConnectionInfo { steam_id_dest: Some(to), steam_id_src: Some(me), app_id: Some(app_id), candidate: Some(data.to_vec()), ..Default::default() };
    let _ = conn.send(EMSG_P2P_CONNECTION_INFO, &msg);
    if lobby != 0 {
        let mut chat = protocol::SIGNAL_PREFIX.to_vec();
        chat.extend_from_slice(data);
        lobby_chat(conn, app_id, lobby, to, &chat);
    }
}

/// A matchmaking / signalling push from Steam, as (game, event).
pub fn game_event(p: &net::Packet) -> Option<(AppId, GameEvent)> {
    Some(match p.emsg {
        EMSG_MMS_LOBBY_DATA => {
            let m: CMsgClientMmsLobbyData = p.body().ok()?;
            (m.app_id(), GameEvent::LobbyData(lobby_from_data(&m)))
        }
        EMSG_MMS_LOBBY_CHAT => {
            let m: CMsgClientMmsLobbyChatMsg = p.body().ok()?;
            (m.app_id(), GameEvent::LobbyChat { lobby: m.steam_id_lobby(), sender: m.steam_id_sender(), data: m.lobby_message().to_vec() })
        }
        EMSG_MMS_USER_JOINED_LOBBY => {
            let m: CMsgClientMmsUserJoinedLobby = p.body().ok()?;
            (m.app_id(), GameEvent::LobbyMember { lobby: m.steam_id_lobby(), user: m.steam_id_user(), name: m.persona_name().to_string(), joined: true })
        }
        EMSG_MMS_USER_LEFT_LOBBY => {
            let m: CMsgClientMmsUserLeftLobby = p.body().ok()?;
            (m.app_id(), GameEvent::LobbyMember { lobby: m.steam_id_lobby(), user: m.steam_id_user(), name: m.persona_name().to_string(), joined: false })
        }
        EMSG_MMS_LOBBY_GAME_SERVER_SET => {
            let m: CMsgClientMmsLobbyGameServerSet = p.body().ok()?;
            let ip = m.game_server_ip.as_ref().and_then(|i| i.v4).unwrap_or(m.deprecated_game_server_ip());
            (m.app_id(), GameEvent::LobbyGameServer { lobby: m.steam_id_lobby(), ip, port: m.game_server_port() as u16, server: m.game_server_steam_id() })
        }
        EMSG_P2P_CONNECTION_INFO => {
            let m: proto::CMsgClientP2pConnectionInfo = p.body().ok()?;
            (m.app_id(), GameEvent::Signal { from: m.steam_id_src(), data: m.candidate().to_vec() })
        }
        _ => return None,
    })
}

// ------------------------------------------------------- leaderboards

/// Steam's leaderboard messages (steammessages_clientserver_lbs.proto),
/// written out here because that .proto isn't vendored.
mod lbs {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct FindOrCreate {
        #[prost(uint32, optional, tag = "1")]
        pub app_id: Option<u32>,
        #[prost(int32, optional, tag = "2")]
        pub leaderboard_sort_method: Option<i32>,
        #[prost(int32, optional, tag = "3")]
        pub leaderboard_display_type: Option<i32>,
        #[prost(bool, optional, tag = "4")]
        pub create_if_not_found: Option<bool>,
        #[prost(string, optional, tag = "5")]
        pub leaderboard_name: Option<String>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct FindOrCreateResponse {
        #[prost(int32, optional, tag = "1")]
        pub eresult: Option<i32>,
        #[prost(int32, optional, tag = "2")]
        pub leaderboard_id: Option<i32>,
        #[prost(int32, optional, tag = "3")]
        pub leaderboard_entry_count: Option<i32>,
        #[prost(int32, optional, tag = "4")]
        pub leaderboard_sort_method: Option<i32>,
        #[prost(int32, optional, tag = "5")]
        pub leaderboard_display_type: Option<i32>,
        #[prost(string, optional, tag = "6")]
        pub leaderboard_name: Option<String>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct GetEntries {
        #[prost(int32, optional, tag = "1")]
        pub app_id: Option<i32>,
        #[prost(int32, optional, tag = "2")]
        pub leaderboard_id: Option<i32>,
        #[prost(int32, optional, tag = "3")]
        pub range_start: Option<i32>,
        #[prost(int32, optional, tag = "4")]
        pub range_end: Option<i32>,
        #[prost(int32, optional, tag = "5")]
        pub leaderboard_data_request: Option<i32>,
        #[prost(fixed64, repeated, packed = "false", tag = "6")]
        pub steamids: Vec<u64>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Entry {
        #[prost(fixed64, optional, tag = "1")]
        pub steam_id_user: Option<u64>,
        #[prost(int32, optional, tag = "2")]
        pub global_rank: Option<i32>,
        #[prost(int32, optional, tag = "3")]
        pub score: Option<i32>,
        #[prost(bytes = "vec", optional, tag = "4")]
        pub details: Option<Vec<u8>>,
        #[prost(fixed64, optional, tag = "5")]
        pub ugc_id: Option<u64>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct GetEntriesResponse {
        #[prost(int32, optional, tag = "1")]
        pub eresult: Option<i32>,
        #[prost(int32, optional, tag = "2")]
        pub leaderboard_entry_count: Option<i32>,
        #[prost(message, repeated, tag = "3")]
        pub entries: Vec<Entry>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SetScore {
        #[prost(uint32, optional, tag = "1")]
        pub app_id: Option<u32>,
        #[prost(int32, optional, tag = "2")]
        pub leaderboard_id: Option<i32>,
        #[prost(int32, optional, tag = "3")]
        pub score: Option<i32>,
        #[prost(bytes = "vec", optional, tag = "4")]
        pub details: Option<Vec<u8>>,
        #[prost(int32, optional, tag = "5")]
        pub upload_score_method: Option<i32>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SetScoreResponse {
        #[prost(int32, optional, tag = "1")]
        pub eresult: Option<i32>,
        #[prost(int32, optional, tag = "2")]
        pub leaderboard_entry_count: Option<i32>,
        #[prost(bool, optional, tag = "3")]
        pub score_changed: Option<bool>,
        #[prost(int32, optional, tag = "4")]
        pub global_rank_previous: Option<i32>,
        #[prost(int32, optional, tag = "5")]
        pub global_rank_new: Option<i32>,
    }
}

const EMSG_LBS_SET_SCORE: u32 = 5414;
const EMSG_LBS_FIND_OR_CREATE: u32 = 5416;
const EMSG_LBS_GET_ENTRIES: u32 = 5418;

pub async fn lb_find(conn: &Connection, app_id: AppId, name: &str, create: bool, sort: i32, display: i32) -> Reply {
    let req = lbs::FindOrCreate {
        app_id: Some(app_id),
        leaderboard_sort_method: Some(sort),
        leaderboard_display_type: Some(display),
        create_if_not_found: Some(create),
        leaderboard_name: Some(name.to_string()),
    };
    match mms_request(conn, app_id, EMSG_LBS_FIND_OR_CREATE, &req).await.and_then(|p| p.body::<lbs::FindOrCreateResponse>()) {
        Ok(r) if r.eresult.unwrap_or(2) == 1 && r.leaderboard_id.unwrap_or(0) != 0 => Reply::LbFound {
            found: true,
            id: r.leaderboard_id(),
            count: r.leaderboard_entry_count(),
            sort: r.leaderboard_sort_method(),
            display: r.leaderboard_display_type(),
            name: r.leaderboard_name.unwrap_or_else(|| name.to_string()),
        },
        other => {
            tracing::info!("leaderboard {name}: {other:?}");
            Reply::LbFound { found: false, id: 0, count: 0, sort: 0, display: 0, name: name.to_string() }
        }
    }
}

/// Leaderboard details travel as little-endian int32s.
fn details_bytes(d: &[i32]) -> Vec<u8> {
    d.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn details_ints(b: &[u8]) -> Vec<i32> {
    b.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub async fn lb_entries(conn: &Connection, app_id: AppId, lb: i32, request: i32, start: i32, end: i32, users: &[u64]) -> Reply {
    let req = lbs::GetEntries {
        app_id: Some(app_id as i32),
        leaderboard_id: Some(lb),
        range_start: Some(start),
        range_end: Some(end),
        leaderboard_data_request: Some(request),
        steamids: users.to_vec(),
    };
    match mms_request(conn, app_id, EMSG_LBS_GET_ENTRIES, &req).await.and_then(|p| p.body::<lbs::GetEntriesResponse>()) {
        Ok(r) if r.eresult.unwrap_or(2) == 1 => Reply::LbEntries {
            total: r.leaderboard_entry_count(),
            entries: r
                .entries
                .iter()
                .map(|e| protocol::LbEntry {
                    steam_id: e.steam_id_user(),
                    rank: e.global_rank(),
                    score: e.score(),
                    details: details_ints(e.details()),
                    ugc: e.ugc_id.unwrap_or(u64::MAX),
                })
                .collect(),
        },
        other => {
            tracing::info!("leaderboard {lb} entries: {other:?}");
            Reply::LbEntries { total: 0, entries: Vec::new() }
        }
    }
}

pub async fn lb_upload(conn: &Connection, app_id: AppId, lb: i32, method: i32, score: i32, details: &[i32]) -> Reply {
    let req = lbs::SetScore { app_id: Some(app_id), leaderboard_id: Some(lb), score: Some(score), details: Some(details_bytes(details)), upload_score_method: Some(method) };
    match mms_request(conn, app_id, EMSG_LBS_SET_SCORE, &req).await.and_then(|p| p.body::<lbs::SetScoreResponse>()) {
        Ok(r) => Reply::LbUploaded { ok: r.eresult.unwrap_or(2) == 1, changed: r.score_changed(), rank_new: r.global_rank_new(), rank_prev: r.global_rank_previous() },
        Err(e) => {
            tracing::info!("leaderboard {lb} upload: {e}");
            Reply::LbUploaded { ok: false, changed: false, rank_new: 0, rank_prev: 0 }
        }
    }
}

/// Percent of players with each achievement (API name).
pub async fn global_percentages(conn: &Connection, app_id: AppId) -> Vec<(String, f32)> {
    let req = proto::CPlayerGetGameAchievementsRequest { appid: Some(app_id), language: Some("english".into()), hash_only: Some(false) };
    match conn.call::<_, proto::CPlayerGetGameAchievementsResponse>("Player.GetGameAchievements#1", &req).await {
        Ok(r) => r.achievements.iter().map(|a| (a.internal_name().to_string(), a.player_percent_unlocked().parse().unwrap_or(0.0))).collect(),
        Err(e) => {
            tracing::info!("achievement percentages for {app_id}: {e}");
            Vec::new()
        }
    }
}

fn http() -> &'static reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| reqwest::Client::builder().user_agent("Valve/Steam HTTP Client 1.0").build().unwrap_or_default())
}

/// A player's avatar, decoded to RGBA. `hash`: the avatar hash Steam sends
/// with persona data. `size` 0/1/2 = 32/64/184 pixels.
pub async fn avatar(hash: &str, size: u8) -> Option<(u32, u32, Vec<u8>)> {
    let suffix = match size {
        0 => "",
        1 => "_medium",
        _ => "_full",
    };
    let url = format!("https://avatars.steamstatic.com/{hash}{suffix}.jpg");
    let bytes = http().get(url).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    Some((img.width(), img.height(), img.into_raw()))
}

/// ISteamHTTP, done by the app.
pub async fn http_request(method: i32, url: &str, headers: &[(String, String)], params: &[(String, String)], body: Option<(String, Vec<u8>)>, timeout_ms: u32) -> Reply {
    let m = match method {
        2 => reqwest::Method::HEAD,
        3 => reqwest::Method::POST,
        4 => reqwest::Method::PUT,
        5 => reqwest::Method::DELETE,
        6 => reqwest::Method::OPTIONS,
        7 => reqwest::Method::PATCH,
        _ => reqwest::Method::GET,
    };
    let get_like = matches!(m, reqwest::Method::GET | reqwest::Method::HEAD | reqwest::Method::DELETE);
    let mut req = http().request(m, url).timeout(std::time::Duration::from_millis(if timeout_ms == 0 { 60_000 } else { timeout_ms as u64 }));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if !params.is_empty() {
        req = if get_like || body.is_some() { req.query(params) } else { req.form(params) };
    }
    if let Some((content_type, data)) = body {
        req = req.header("Content-Type", content_type).body(data);
    }
    match req.send().await {
        Ok(r) => {
            let status = r.status().as_u16() as i32;
            let headers = r.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string())).collect();
            let body = r.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
            Reply::HttpDone { status, headers, body, timed_out: false }
        }
        Err(e) => Reply::HttpDone { status: 0, headers: Vec::new(), body: Vec::new(), timed_out: e.is_timeout() },
    }
}

/// Game language and branch from Steam's `appmanifest_<id>.acf` next to
/// the install (`<library>/steamapps/common/<dir>`): (language, beta, build id).
pub fn manifest_info(app_id: AppId, install_dir: &std::path::Path) -> (Option<String>, String, u32) {
    let manifest = install_dir.parent().and_then(|common| common.parent()).map(|steamapps| steamapps.join(format!("appmanifest_{app_id}.acf")));
    let Some(text) = manifest.and_then(|m| std::fs::read_to_string(m).ok()) else { return (None, String::new(), 0) };
    let Ok(kv) = vdf::parse(&text) else { return (None, String::new(), 0) };
    let Some(state) = kv.obj("AppState") else { return (None, String::new(), 0) };
    let user = state.obj("UserConfig");
    let language = user.and_then(|u| u.str("language")).or(state.obj("MountedConfig").and_then(|m| m.str("language"))).filter(|l| !l.is_empty()).map(String::from);
    let beta = user.and_then(|u| u.str("BetaKey")).unwrap_or_default().to_string();
    let beta = if beta == "public" { String::new() } else { beta };
    (language, beta, state.u64("buildid").unwrap_or(0) as u32)
}

/// Steam's own UI language setting, if the official client is installed.
pub fn steam_ui_language() -> Option<String> {
    crate::launch::reg_string("Software\\Valve\\Steam", "Language").filter(|l| !l.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Binary KeyValues writer for test schemas.
    fn kv(obj: &vdf::Obj, out: &mut Vec<u8>) {
        for (k, v) in obj.iter() {
            match v {
                vdf::Value::Obj(o) => {
                    out.push(0);
                    out.extend_from_slice(k.as_bytes());
                    out.push(0);
                    kv(o, out);
                }
                vdf::Value::Str(s) => {
                    out.push(1);
                    out.extend_from_slice(k.as_bytes());
                    out.push(0);
                    out.extend_from_slice(s.as_bytes());
                    out.push(0);
                }
            }
        }
        out.push(8);
    }

    #[test]
    fn parses_spacewar_like_schema() {
        let text = r#""480" { "gamename" "Spacewar" "version" "11" "stats" {
            "1" { "type" "1" "name" "NumGames" "default" "0" "incrementonly" "1" }
            "3" { "type" "2" "name" "FeetTraveled" "default" "0" }
            "4" { "type" "4" "id" "4" "bits" {
                "0" { "name" "ACH_WIN_ONE_GAME" "bit" "0" "display" { "name" { "english" "Winner" "russian" "Победитель" "token" "NEW_ACHIEVEMENT_0_0_NAME" } "desc" { "english" "Win one game." } "hidden" "0" "icon" "a.jpg" "icon_gray" "b.jpg" } }
                "1" { "name" "ACH_WIN_100_GAMES" "bit" "1" "display" { "name" { "english" "Champion" } "desc" { "english" "Win 100 games." } "hidden" "1" } } } } } }"#;
        let obj = vdf::parse(text).unwrap();
        let mut bin = Vec::new();
        kv(&obj, &mut bin);
        let parsed = vdf::binary::parse(&bin).unwrap();
        let s = parse_schema(&parsed, 480, "russian");
        assert_eq!(s.stats.iter().filter(|s| s.kind != 4).count(), 2);
        let games = s.stats.iter().find(|s| s.name == "NumGames").unwrap();
        assert!(games.increment_only);
        assert_eq!(s.achievements.len(), 2);
        assert_eq!(s.achievements[0].display_name, "Победитель");
        assert_eq!(s.achievements[1].display_name, "Champion");
        assert!(s.achievements[1].hidden);
        assert_eq!((s.achievements[1].block, s.achievements[1].bit), (4, 1));
    }

    #[test]
    fn rich_presence_is_binary_kv() {
        let b = rich_presence_kv(&[("status".into(), "In menu".into())]);
        let parsed = vdf::binary::parse(&b).unwrap();
        assert_eq!(parsed.obj("RP").and_then(|o| o.str("status")), Some("In menu"));
    }

    #[test]
    fn session_ticket_layout() {
        let token = [7u8; 20];
        let t = session_ticket(&token, 0x0100007f, 5, 1, &[9u8; 10]);
        assert_eq!(t.len(), 4 + 20 + 4 + 24 + 4 + 10);
        assert_eq!(auth_part(&t).len(), 52, "Steam checks token + session header");
        assert_eq!(&t[24..28], &24u32.to_le_bytes());
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn lobby_metadata_round_trip() {
        let pairs = vec![("name".to_string(), "Room".to_string()), ("version".to_string(), "1.4".to_string())];
        let bytes = lobby_kv(&pairs);
        assert_eq!(lobby_pairs(&[], Some(&bytes)), pairs);
        assert!(lobby_pairs(&[], Some(&lobby_kv(&[]))).is_empty());
        let listed = [MmsKeyValuePair { name: Some("a".into()), value: Some("b".into()) }];
        assert_eq!(lobby_pairs(&listed, Some(&bytes)), vec![("a".to_string(), "b".to_string())]);
    }

    #[test]
    fn protocol_round_trip() {
        let env = protocol::Envelope { token: "t".into(), pid: 1, req: Request::StoreStats { stats: vec![(4, 1)], crc: 9, reset: false } };
        let back: protocol::Envelope = serde_json::from_str(&serde_json::to_string(&env).unwrap()).unwrap();
        assert!(matches!(back.req, Request::StoreStats { crc: 9, .. }));
    }
}

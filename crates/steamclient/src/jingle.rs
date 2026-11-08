//! ISteamNetworking with players on Valve's official Steam client.
//!
//! Steam's client still runs the old P2P API (SendP2PPacket…) on
//! libjingle 0.4.0, the Google Talk P2P library of 2007 (its sources sit
//! in `src/external/libjingle-0.4.0` of the client build), not on
//! SteamNetworkingSockets. This module speaks that protocol:
//!
//! - **Signalling**: one `ClientP2PConnectionInfo` (EMsg 5434) per
//!   candidate, the candidate serialized Valve's way (see [`Candidate`]).
//!   The first candidates from someone make Steam post P2PSessionRequest_t.
//! - **Connectivity checks**: libjingle's pre-ICE STUN (RFC 3489, 16-byte
//!   transaction id, no integrity). A binding request carries USERNAME =
//!   the receiver's username fragment + the sender's; the response echoes
//!   it with MAPPED-ADDRESS. A connection can send once its pings are
//!   answered and receive once the other side pinged it.
//! - **Relays**: an official player behind NAT also offers a "relay"
//!   candidate on Valve's libjingle relay servers. Those forward anything
//!   whose first STUN USERNAME bytes name the binding, so pinging that
//!   candidate reaches them through the relay without credentials of our own.
//! - **Socket** (Valve's `socketclient.cc`, "mixed" mode): each datagram is
//!   `[0][unreliable packet]` or `[1][PseudoTcp segment]`; a bare 4-byte
//!   `0xBEEFFACE` means the other side closed. The reliable stream carries
//!   `[u32 BE length][packet]`. PseudoTcp uses conversation 0.
//! - **Packets** (p2pnetworkingsession.cpp): `[0][data]` on channel 0,
//!   `[1][i32 LE channel][data]` on others.
//!
//! The state machine here is pure (no socket): [`Jingle`] takes datagrams
//! and signals in, and leaves what to send in [`Out`]. Each official peer
//! gets its own UDP socket (libjingle uses a port per session too): their
//! relay candidates are a relay's shared address, so one socket couldn't
//! tell two players on the same relay apart.

use crate::pseudotcp::{PseudoTcp, State};
use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// libjingle's channel name in Valve's socket (the candidates' `name`).
const CHANNEL_NAME: &str = "valvep2p";
const CLOSE_MAGIC: u32 = 0xBEEF_FACE;
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const STUN_BINDING_ERROR: u16 = 0x0111;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_USERNAME: u16 = 0x0006;
const ATTR_ERROR_CODE: u16 = 0x0009;
const FRAG_LEN: usize = 16;
/// libjingle times a connection's readability out after 30 s without pings.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
/// Ping every candidate this often until something answers.
const UNWRITABLE_PING: Duration = Duration::from_millis(250);
/// And the connection in use this often (libjingle: at most 900 ms).
const WRITABLE_PING: Duration = Duration::from_millis(800);
/// Give up on a peer we exchanged candidates with after this long.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(25);
/// Payload per PseudoTcp segment (plus 24-byte header and our type byte).
const MSS: u32 = 1100;
const MAX_MESSAGE: usize = 1024 * 1025;

static START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// libjingle's clock: milliseconds (never 0, which means "no timestamp").
pub fn now_ms() -> u32 {
    (START.elapsed().as_millis() as u32).wrapping_add(1000)
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut out = Vec::with_capacity(n + 8);
    while out.len() < n {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u64(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        h.write_u32(std::process::id());
        out.extend_from_slice(&h.finish().to_le_bytes());
    }
    out.truncate(n);
    out
}

/// libjingle's CreateRandomString: base64 characters.
fn random_string(n: usize) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    random_bytes(n).into_iter().map(|b| ABC[(b & 63) as usize] as char).collect()
}

// ------------------------------------------------------------ candidates

/// A libjingle candidate as Valve's client serializes it into
/// `CMsgClientP2PConnectionInfo.candidate` (a CUtlBuffer: strings end in
/// NUL, numbers are big-endian):
/// name, protocol, address (text), port u16, preference (f32 bits u32),
/// username, password, type, network name, generation u32, then one
/// optional byte (0..9).
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub protocol: String,
    pub address: String,
    pub port: u16,
    pub preference: f32,
    pub username: String,
    pub password: String,
    pub kind: String,
    pub network: String,
    pub generation: u32,
    pub extra: u8,
}

fn take_str(d: &[u8], at: &mut usize) -> Option<String> {
    let rest = d.get(*at..)?;
    let end = rest.iter().position(|&c| c == 0)?;
    let v = std::str::from_utf8(&rest[..end]).ok()?.to_string();
    *at += end + 1;
    Some(v)
}

impl Candidate {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(128);
        let s = |b: &mut Vec<u8>, v: &str| {
            b.extend_from_slice(v.as_bytes());
            b.push(0);
        };
        s(&mut b, &self.name);
        s(&mut b, &self.protocol);
        s(&mut b, &self.address);
        b.extend_from_slice(&self.port.to_be_bytes());
        b.extend_from_slice(&self.preference.to_bits().to_be_bytes());
        s(&mut b, &self.username);
        s(&mut b, &self.password);
        s(&mut b, &self.kind);
        s(&mut b, &self.network);
        b.extend_from_slice(&self.generation.to_be_bytes());
        b.push(self.extra);
        b
    }

    pub fn decode(d: &[u8]) -> Option<Candidate> {
        let mut at = 0;
        let name = take_str(d, &mut at)?;
        let protocol = take_str(d, &mut at)?;
        let address = take_str(d, &mut at)?;
        let port = u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?);
        let preference = f32::from_bits(u32::from_be_bytes(d.get(at + 2..at + 6)?.try_into().ok()?));
        at += 6;
        let username = take_str(d, &mut at)?;
        let password = take_str(d, &mut at)?;
        let kind = take_str(d, &mut at)?;
        let network = take_str(d, &mut at)?;
        let generation = u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?);
        let extra = d.get(at + 4).copied().unwrap_or(0).min(9);
        // What libjingle sends; anything else is something else.
        if name.is_empty() || protocol.is_empty() || username.len() < 4 || !preference.is_finite() {
            return None;
        }
        Some(Candidate { name, protocol, address, port, preference, username, password, kind, network, generation, extra })
    }

    fn addr(&self) -> Option<SocketAddr> {
        let ip: Ipv4Addr = self.address.parse().ok()?;
        (!ip.is_unspecified() && self.port != 0).then(|| SocketAddr::V4(SocketAddrV4::new(ip, self.port)))
    }
}

// ------------------------------------------------------------ STUN (RFC 3489)

pub struct Stun {
    pub kind: u16,
    pub txid: [u8; 16],
    pub username: Option<Vec<u8>>,
    pub mapped: Option<SocketAddr>,
    pub error: Option<u16>,
}

fn stun_encode(kind: u16, txid: &[u8; 16], attrs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let len: usize = attrs.iter().map(|(_, v)| 4 + v.len()).sum();
    let mut b = Vec::with_capacity(20 + len);
    b.extend_from_slice(&kind.to_be_bytes());
    b.extend_from_slice(&(len as u16).to_be_bytes());
    b.extend_from_slice(txid);
    for (t, v) in attrs {
        b.extend_from_slice(&t.to_be_bytes());
        b.extend_from_slice(&(v.len() as u16).to_be_bytes());
        b.extend_from_slice(v);
    }
    b
}

fn addr_attr(a: SocketAddr) -> Vec<u8> {
    let mut v = vec![0, 1];
    v.extend_from_slice(&a.port().to_be_bytes());
    match a {
        SocketAddr::V4(a) => v.extend_from_slice(&a.ip().octets()),
        SocketAddr::V6(_) => v.extend_from_slice(&[0; 4]),
    }
    v
}

/// A libjingle STUN message (old RFC 3489 form). Messages with the RFC 5389
/// magic cookie are someone else's (our own STUN server answers).
pub fn stun_decode(d: &[u8]) -> Option<Stun> {
    if d.len() < 20 {
        return None;
    }
    let kind = u16::from_be_bytes([d[0], d[1]]);
    if !matches!(kind, STUN_BINDING_REQUEST | STUN_BINDING_RESPONSE | STUN_BINDING_ERROR) {
        return None;
    }
    let len = u16::from_be_bytes([d[2], d[3]]) as usize;
    if len != d.len() - 20 || d[4..8] == [0x21, 0x12, 0xA4, 0x42] {
        return None;
    }
    let mut m = Stun { kind, txid: d[4..20].try_into().unwrap(), username: None, mapped: None, error: None };
    let mut at = 20;
    while at < d.len() {
        let t = u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?);
        let l = u16::from_be_bytes(d.get(at + 2..at + 4)?.try_into().ok()?) as usize;
        let v = d.get(at + 4..at + 4 + l)?;
        match t {
            ATTR_USERNAME => m.username = Some(v.to_vec()),
            ATTR_MAPPED_ADDRESS if l == 8 => {
                m.mapped = Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(v[4], v[5], v[6], v[7]), u16::from_be_bytes([v[2], v[3]]))))
            }
            ATTR_ERROR_CODE if l >= 4 => m.error = Some(v[2] as u16 * 100 + v[3] as u16),
            _ => {}
        }
        at += 4 + l;
    }
    Some(m)
}

fn error_attr(code: u16, reason: &str) -> Vec<u8> {
    let mut v = vec![0, 0, (code / 100) as u8, (code % 100) as u8];
    v.extend_from_slice(reason.as_bytes());
    v
}

// ------------------------------------------------------------ packets

/// Valve's legacy packet: `[0][data]` on channel 0, else `[1][i32 LE][data]`.
pub fn frame_packet(channel: i32, data: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(data.len() + 5);
    if channel == 0 {
        b.push(0);
    } else {
        b.push(1);
        b.extend_from_slice(&channel.to_le_bytes());
    }
    b.extend_from_slice(data);
    b
}

pub fn parse_packet(d: &[u8]) -> Option<(i32, &[u8])> {
    match *d.first()? {
        0 => Some((0, &d[1..])),
        1 if d.len() >= 5 => Some((i32::from_le_bytes(d[1..5].try_into().ok()?), &d[5..])),
        _ => None,
    }
}

// ------------------------------------------------------------ state

struct Remote {
    addr: SocketAddr,
    username: String,
    kind: String,
}

struct Conn {
    addr: SocketAddr,
    remote_user: String,
    kind: String,
    last_ping_sent: Option<Instant>,
    last_ping_recv: Option<Instant>,
    last_resp: Option<Instant>,
    /// Sent pings still waiting for an answer.
    pending: VecDeque<([u8; 16], Instant)>,
    rtt: Option<Duration>,
}

impl Conn {
    fn readable(&self, now: Instant) -> bool {
        self.last_ping_recv.is_some_and(|t| now - t < READ_TIMEOUT)
    }
    fn writable(&self, now: Instant) -> bool {
        self.last_resp.is_some_and(|t| now - t < WRITE_TIMEOUT)
    }
}

struct Peer {
    /// Our addresses for this peer's socket.
    local: Vec<SocketAddr>,
    public: Option<SocketAddr>,
    frag: String,
    pwd: String,
    /// Our candidates went out.
    started: bool,
    /// Their candidates arrived.
    heard: bool,
    remote: Vec<Remote>,
    conns: Vec<Conn>,
    tcp: PseudoTcp,
    stream_in: Vec<u8>,
    /// Reliable bytes waiting for room in the stream.
    stream_out: VecDeque<u8>,
    created: Instant,
    open: bool,
    announced: bool,
    /// Game packets (in, out) and bytes (in, out), for the log.
    stats: [u64; 4],
    logged: [u64; 4],
    stats_at: Instant,
}

#[derive(Default)]
pub struct Out {
    /// (peer whose socket sends it, to, data).
    pub datagrams: Vec<(u64, SocketAddr, Vec<u8>)>,
    /// Candidates for `ClientP2PConnectionInfo`.
    pub signals: Vec<(u64, Vec<u8>)>,
    /// (from, channel, data).
    pub delivered: Vec<(u64, i32, Vec<u8>)>,
    /// First data from this player (P2PSessionRequest_t).
    pub requests: Vec<u64>,
    pub opened: Vec<u64>,
    /// Closed by them, or never connected.
    pub closed: Vec<u64>,
}

#[derive(Default)]
pub struct Jingle {
    peers: HashMap<u64, Peer>,
    pub out: Out,
}

impl Jingle {
    pub fn new() -> Self {
        Jingle::default()
    }

    pub fn take_out(&mut self) -> Out {
        std::mem::take(&mut self.out)
    }

    fn our_candidates(p: &Peer) -> Vec<Candidate> {
        let mk = |a: &SocketAddr, kind: &str, pref: f32| Candidate {
            name: CHANNEL_NAME.into(),
            protocol: "udp".into(),
            address: a.ip().to_string(),
            port: a.port(),
            preference: pref,
            username: p.frag.clone(),
            password: p.pwd.clone(),
            kind: kind.into(),
            network: "SteamInRust".into(),
            generation: 0,
            extra: 0,
        };
        let mut out: Vec<Candidate> = p.local.iter().filter(|a| a.is_ipv4() && !a.ip().is_loopback()).map(|a| mk(a, "local", 1.0)).collect();
        if let Some(public) = p.public.filter(|x| !p.local.contains(x)) {
            out.push(mk(&public, "stun", 0.9));
        }
        out
    }

    /// The public address of `id`'s socket (from STUN) is known: tell them.
    pub fn set_public(&mut self, id: u64, addr: SocketAddr) {
        let Some(p) = self.peers.get_mut(&id).filter(|p| p.public != Some(addr)) else { return };
        p.public = Some(addr);
        if p.started {
            if let Some(c) = Self::our_candidates(p).into_iter().find(|c| c.kind == "stun") {
                self.out.signals.push((id, c.encode()));
            }
        }
    }

    /// Make the state for `id`, whose socket has these local addresses.
    pub fn ensure(&mut self, id: u64, local: Vec<SocketAddr>) {
        self.peers.entry(id).or_insert_with(|| Peer {
            local,
            public: None,
            frag: random_string(FRAG_LEN),
            pwd: random_string(FRAG_LEN),
            started: false,
            heard: false,
            remote: Vec::new(),
            conns: Vec::new(),
            tcp: PseudoTcp::new(0, MSS, now_ms()),
            stream_in: Vec::new(),
            stream_out: VecDeque::new(),
            created: Instant::now(),
            open: false,
            announced: false,
            stats: [0; 4],
            logged: [0; 4],
            stats_at: Instant::now(),
        });
    }

    /// Start (or keep) a session with `to` (after [`ensure`](Self::ensure)): send our candidates.
    pub fn start(&mut self, to: u64) {
        let Some(p) = self.peers.get_mut(&to).filter(|p| !p.started) else { return };
        p.started = true;
        let cands = Self::our_candidates(p);
        crate::log!("p2p/official: offering {} candidate(s) to {to}", cands.len());
        for c in cands {
            self.out.signals.push((to, c.encode()));
        }
    }

    /// Forget a player (they turned out to run SteamInRust).
    pub fn forget(&mut self, id: u64) {
        self.peers.remove(&id);
    }

    /// Close the session like Steam does (tells the other side).
    pub fn close(&mut self, id: u64) {
        if let Some(p) = self.peers.remove(&id) {
            if let Some(c) = best(&p.conns, Instant::now()) {
                self.out.datagrams.push((id, p.conns[c].addr, CLOSE_MAGIC.to_be_bytes().to_vec()));
            }
        }
    }

    pub fn knows(&self, id: u64) -> bool {
        self.peers.contains_key(&id)
    }

    pub fn heard(&self, id: u64) -> bool {
        self.peers.get(&id).is_some_and(|p| p.heard)
    }

    pub fn is_open(&self, id: u64) -> bool {
        self.peers.get(&id).is_some_and(|p| p.open)
    }

    /// The address in use, its round trip and whether it's a relay.
    pub fn link(&self, id: u64) -> Option<(SocketAddr, Option<Duration>, bool)> {
        let p = self.peers.get(&id)?;
        let c = &p.conns[best(&p.conns, Instant::now())?];
        Some((c.addr, c.rtt, c.kind == "relay"))
    }

    /// A candidate from `from` (through Steam), after [`ensure`](Self::ensure).
    pub fn on_candidate(&mut self, from: u64, c: Candidate) {
        let Some(addr) = c.addr().filter(|_| c.protocol == "udp") else {
            crate::log!("p2p/official: {from} sent a {} {} candidate, skipped", c.protocol, c.kind);
            return;
        };
        crate::log!("p2p/official: candidate from {from}: {} {addr} pref {:.2} net {:?} gen {} extra {}", c.kind, c.preference, c.network, c.generation, c.extra);
        let Some(p) = self.peers.get_mut(&from) else { return };
        p.heard = true;
        if !p.remote.iter().any(|r| r.addr == addr && r.username == c.username) {
            p.remote.push(Remote { addr, username: c.username.clone(), kind: c.kind.clone() });
        }
        if !p.conns.iter().any(|x| x.addr == addr) {
            p.conns.push(new_conn(addr, c.username, c.kind));
        }
        // Answer with ours (the game accepts sessions with everyone in its lobby).
        self.start(from);
    }

    /// A datagram on `id`'s socket.
    pub fn on_datagram(&mut self, id: u64, from: SocketAddr, d: &[u8]) {
        let now = Instant::now();
        if let Some(m) = stun_decode(d) {
            self.on_stun(id, from, m, now);
            return;
        }
        let Some(p) = self.peers.get_mut(&id) else { return };
        // libjingle: data only from an address that pinged us.
        if !p.conns.iter().any(|c| c.addr == from && c.readable(now)) {
            return;
        }
        if d.len() == 4 && u32::from_be_bytes(d.try_into().unwrap()) == CLOSE_MAGIC {
            crate::log!("p2p/official: {id} closed the session");
            self.peers.remove(&id);
            self.out.closed.push(id);
            return;
        }
        let ms = now_ms();
        match d.first() {
            Some(0) => {
                if let Some((ch, data)) = parse_packet(&d[1..]) {
                    if !p.announced {
                        p.announced = true;
                        self.out.requests.push(id);
                    }
                    p.stats[0] += 1;
                    p.stats[2] += data.len() as u64;
                    self.out.delivered.push((id, ch, data.to_vec()));
                }
            }
            Some(1) => {
                p.tcp.notify_packet(&d[1..], ms);
                let mut fresh = Vec::new();
                p.tcp.recv(&mut fresh, ms);
                p.stream_in.extend_from_slice(&fresh);
                while p.stream_in.len() >= 4 {
                    let n = u32::from_be_bytes(p.stream_in[..4].try_into().unwrap()) as usize;
                    if n > MAX_MESSAGE {
                        crate::log!("p2p/official: bad stream from {id} ({n}-byte message)");
                        p.tcp.close();
                        break;
                    }
                    if p.stream_in.len() < 4 + n {
                        break;
                    }
                    let msg: Vec<u8> = p.stream_in.drain(..4 + n).skip(4).collect();
                    if let Some((ch, data)) = parse_packet(&msg) {
                        if !p.announced {
                            p.announced = true;
                            self.out.requests.push(id);
                        }
                        p.stats[0] += 1;
                        p.stats[2] += data.len() as u64;
                        self.out.delivered.push((id, ch, data.to_vec()));
                    }
                }
                Self::flush_tcp(id, p, &mut self.out, now);
            }
            _ => {}
        }
    }

    fn on_stun(&mut self, id: u64, from: SocketAddr, m: Stun, now: Instant) {
        let Some(user) = m.username.clone() else { return };
        let Some(p) = self.peers.get_mut(&id) else { return };
        match m.kind {
            STUN_BINDING_REQUEST => {
                // USERNAME = our fragment + theirs.
                if !user.starts_with(p.frag.as_bytes()) {
                    return;
                }
                let remote_user = String::from_utf8_lossy(&user[FRAG_LEN..]).into_owned();
                let ok_resp = stun_encode(STUN_BINDING_RESPONSE, &m.txid, &[(ATTR_USERNAME, user.clone()), (ATTR_MAPPED_ADDRESS, addr_attr(from))]);
                if let Some(c) = p.conns.iter_mut().find(|c| c.addr == from) {
                    if c.remote_user != remote_user {
                        let e = stun_encode(STUN_BINDING_ERROR, &m.txid, &[(ATTR_USERNAME, user), (ATTR_ERROR_CODE, error_attr(400, "BAD REQUEST"))]);
                        self.out.datagrams.push((id, from, e));
                        return;
                    }
                    if c.last_ping_recv.is_none() {
                        crate::log!("p2p/official: {id} reaches us via {from} ({})", c.kind);
                    }
                    c.last_ping_recv = Some(now);
                    self.out.datagrams.push((id, from, ok_resp));
                    return;
                }
                // A new address (their NAT, or their relay): fine if we know the username.
                let Some(kind) = p.remote.iter().find(|r| r.username == remote_user).map(|r| r.kind.clone()) else {
                    let e = stun_encode(STUN_BINDING_ERROR, &m.txid, &[(ATTR_USERNAME, user), (ATTR_ERROR_CODE, error_attr(430, "STALE CREDENTIALS"))]);
                    self.out.datagrams.push((id, from, e));
                    return;
                };
                crate::log!("p2p/official: {id} reaches us via new address {from} ({kind})");
                let mut c = new_conn(from, remote_user, kind);
                c.last_ping_recv = Some(now);
                p.conns.push(c);
                self.out.datagrams.push((id, from, ok_resp));
            }
            STUN_BINDING_RESPONSE | STUN_BINDING_ERROR => {
                if user.len() <= FRAG_LEN || &user[user.len() - FRAG_LEN..] != p.frag.as_bytes() {
                    return;
                }
                let Some(c) = p.conns.iter_mut().find(|c| c.pending.iter().any(|(t, _)| *t == m.txid)) else { return };
                let sent = c.pending.iter().find(|(t, _)| *t == m.txid).map(|(_, at)| *at).unwrap();
                if m.kind == STUN_BINDING_ERROR {
                    crate::log!("p2p/official: ping to {} refused ({:?})", c.addr, m.error);
                    c.pending.retain(|(t, _)| *t != m.txid);
                    return;
                }
                if c.last_resp.is_none() {
                    crate::log!("p2p/official: can send to {id} via {} ({}), seen as {:?}", c.addr, c.kind, m.mapped);
                }
                c.pending.clear();
                c.last_resp = Some(now);
                let rtt = now - sent;
                c.rtt = Some(match c.rtt {
                    Some(old) => (old * 3 + rtt) / 4,
                    None => rtt,
                });
            }
            _ => {}
        }
    }

    /// Queue a packet for `to`. False if the session isn't open.
    pub fn send(&mut self, to: u64, channel: i32, reliable: bool, data: &[u8]) -> bool {
        let now = Instant::now();
        let Some(p) = self.peers.get_mut(&to).filter(|p| p.open) else { return false };
        let pkt = frame_packet(channel, data);
        p.stats[1] += 1;
        p.stats[3] += data.len() as u64;
        if reliable {
            p.stream_out.extend((pkt.len() as u32).to_be_bytes());
            p.stream_out.extend(pkt);
            Self::flush_tcp(to, p, &mut self.out, now);
        } else if let Some(c) = best(&p.conns, now) {
            let mut b = Vec::with_capacity(pkt.len() + 1);
            b.push(0);
            b.extend_from_slice(&pkt);
            self.out.datagrams.push((to, p.conns[c].addr, b));
        }
        true
    }

    /// Reliable bytes waiting to go out to `to`.
    pub fn queued(&self, to: u64) -> usize {
        self.peers.get(&to).map(|p| p.stream_out.len()).unwrap_or(0)
    }

    fn flush_tcp(id: u64, p: &mut Peer, out: &mut Out, now: Instant) {
        let ms = now_ms();
        if p.tcp.state == State::Established {
            while !p.stream_out.is_empty() && p.tcp.send_space() > 0 {
                let n = p.stream_out.len().min(p.tcp.send_space()).min(16 * 1024);
                let chunk: Vec<u8> = p.stream_out.iter().take(n).copied().collect();
                let taken = p.tcp.send(&chunk, ms);
                p.stream_out.drain(..taken);
                if taken == 0 {
                    break;
                }
            }
        }
        let segs = p.tcp.take_output();
        if segs.is_empty() {
            return;
        }
        let Some(c) = best(&p.conns, now) else { return };
        let addr = p.conns[c].addr;
        for s in segs {
            let mut b = Vec::with_capacity(s.len() + 1);
            b.push(1);
            b.extend_from_slice(&s);
            out.datagrams.push((id, addr, b));
        }
    }

    /// Pings, the stream's timers, timeouts. Call ~20 times a second.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let ms = now_ms();
        let mut dead = Vec::new();
        for (&id, p) in self.peers.iter_mut() {
            if !p.started {
                continue;
            }
            let best_idx = best(&p.conns, now);
            for (i, c) in p.conns.iter_mut().enumerate() {
                let every = if Some(i) == best_idx {
                    WRITABLE_PING
                } else if c.writable(now) {
                    Duration::from_secs(5)
                } else if now - p.created < OPEN_TIMEOUT || c.readable(now) {
                    UNWRITABLE_PING
                } else {
                    Duration::from_secs(3)
                };
                if c.last_ping_sent.is_some_and(|t| now - t < every) {
                    continue;
                }
                c.last_ping_sent = Some(now);
                let txid: [u8; 16] = random_bytes(16).try_into().unwrap();
                c.pending.push_back((txid, now));
                while c.pending.len() > 8 {
                    c.pending.pop_front();
                }
                let mut user = c.remote_user.clone().into_bytes();
                user.extend_from_slice(p.frag.as_bytes());
                self.out.datagrams.push((id, c.addr, stun_encode(STUN_BINDING_REQUEST, &txid, &[(ATTR_USERNAME, user)])));
            }
            // Writable: open the reliable stream (both sides may; libjingle's
            // PseudoTcp handles a simultaneous open).
            if best_idx.is_some() && p.tcp.state == State::Listen {
                p.tcp.connect(ms);
            }
            p.tcp.notify_clock(ms);
            if p.tcp.take_opened() {
                crate::log!("p2p/official: session with {id} open");
                p.open = true;
                self.out.opened.push(id);
            }
            if p.tcp.state == State::Closed && p.open {
                crate::log!("p2p/official: stream to {id} broke (error {})", p.tcp.error);
                dead.push(id);
                continue;
            }
            Self::flush_tcp(id, p, &mut self.out, now);
            if p.stats != p.logged && (p.logged == [0; 4] || now - p.stats_at > Duration::from_secs(30)) {
                let link = best(&p.conns, now).map(|i| (p.conns[i].addr, p.conns[i].kind.clone(), p.conns[i].rtt));
                crate::log!("p2p/official: {id}: {} packet(s) / {} B in, {} / {} B out, via {:?}", p.stats[0], p.stats[2], p.stats[1], p.stats[3], link);
                p.logged = p.stats;
                p.stats_at = now;
            }
            if !p.open && now - p.created > OPEN_TIMEOUT {
                crate::log!("p2p/official: couldn't open a session with {id} ({} address(es) tried)", p.conns.len());
                dead.push(id);
            }
            // Everything quiet for long: the other side is gone.
            if p.open && !p.conns.iter().any(|c| c.readable(now) || c.writable(now)) {
                crate::log!("p2p/official: lost {id}");
                dead.push(id);
            }
        }
        for id in dead {
            self.peers.remove(&id);
            self.out.closed.push(id);
        }
    }
}

fn new_conn(addr: SocketAddr, remote_user: String, kind: String) -> Conn {
    Conn { addr, remote_user, kind, last_ping_sent: None, last_ping_recv: None, last_resp: None, pending: VecDeque::new(), rtt: None }
}

/// The connection to send on: answering our pings, direct before relayed,
/// then the fastest.
fn best(conns: &[Conn], now: Instant) -> Option<usize> {
    conns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.writable(now))
        .min_by_key(|(_, c)| (c.kind == "relay", c.rtt.unwrap_or(Duration::from_secs(1))))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_matches_valves_layout() {
        let c = Candidate {
            name: "valvep2p".into(),
            protocol: "udp".into(),
            address: "203.0.113.7".into(),
            port: 27015,
            preference: 0.9,
            username: "abcdABCD0123+/xy".into(),
            password: "PPPPPPPPPPPPPPPP".into(),
            kind: "stun".into(),
            network: "eth0".into(),
            generation: 2,
            extra: 1,
        };
        let b = c.encode();
        let mut want = b"valvep2p\0udp\0203.0.113.7\0".to_vec();
        want.extend_from_slice(&27015u16.to_be_bytes());
        want.extend_from_slice(&0.9f32.to_bits().to_be_bytes());
        want.extend_from_slice(b"abcdABCD0123+/xy\0PPPPPPPPPPPPPPPP\0stun\0eth0\0");
        want.extend_from_slice(&[0, 0, 0, 2, 1]);
        assert_eq!(b, want);
        assert_eq!(Candidate::decode(&b), Some(c.clone()));
        // The trailing byte is optional.
        assert_eq!(Candidate::decode(&b[..b.len() - 1]).map(|x| x.extra), Some(0));
        // Our own signals are not candidates.
        assert!(Candidate::decode(b"SIR1\0\x01\x02").is_none());
    }

    #[test]
    fn stun_round_trip() {
        let tx = [7u8; 16];
        let b = stun_encode(STUN_BINDING_RESPONSE, &tx, &[(ATTR_USERNAME, b"x".repeat(32)), (ATTR_MAPPED_ADDRESS, addr_attr("198.51.100.2:5000".parse().unwrap()))]);
        assert_eq!(&b[0..4], &[0x01, 0x01, 0, 4 + 32 + 4 + 8]);
        let m = stun_decode(&b).unwrap();
        assert_eq!(m.txid, tx);
        assert_eq!(m.username.as_deref(), Some(&b"x".repeat(32)[..]));
        assert_eq!(m.mapped, Some("198.51.100.2:5000".parse().unwrap()));
        // Wrong length: not STUN (it's game data).
        assert!(stun_decode(&b[..b.len() - 1]).is_none());
    }

    #[test]
    fn packet_framing() {
        assert_eq!(frame_packet(0, b"hi"), b"\0hi");
        assert_eq!(frame_packet(3, b"hi"), [1, 3, 0, 0, 0, b'h', b'i']);
        assert_eq!(parse_packet(&[1, 3, 0, 0, 0, b'h']), Some((3, &b"h"[..])));
        assert_eq!(parse_packet(b"\0x"), Some((0, &b"x"[..])));
        assert_eq!(parse_packet(b"\x05x"), None);
    }

    /// Two state machines wired through memory.
    #[test]
    fn two_sides_open_and_exchange() {
        let a_addr: SocketAddr = "10.0.0.1:1000".parse().unwrap();
        let b_addr: SocketAddr = "10.0.0.2:2000".parse().unwrap();
        let (a_id, b_id) = (1u64, 2u64);
        let mut a = Jingle::new();
        let mut b = Jingle::new();
        a.ensure(b_id, vec![a_addr]);
        a.start(b_id);
        let mut got_a = Vec::new();
        let mut got_b = Vec::new();
        let mut opened = (false, false);
        let deadline = Instant::now() + Duration::from_secs(10);
        let big: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
        let mut sent = false;
        let mut welcomed = false;
        while Instant::now() < deadline && (got_b.len() < 2 || got_a.is_empty()) {
            a.tick();
            b.tick();
            for _ in 0..4 {
                let oa = a.take_out();
                let ob = b.take_out();
                for (to, s) in oa.signals {
                    assert_eq!(to, b_id);
                    b.ensure(a_id, vec![b_addr]);
                    b.on_candidate(a_id, Candidate::decode(&s).unwrap());
                }
                for (to, s) in ob.signals {
                    assert_eq!(to, a_id);
                    a.on_candidate(b_id, Candidate::decode(&s).unwrap());
                }
                for (peer, to, d) in oa.datagrams {
                    assert_eq!(peer, b_id);
                    if to == b_addr {
                        b.on_datagram(a_id, a_addr, &d);
                    }
                }
                for (peer, to, d) in ob.datagrams {
                    assert_eq!(peer, a_id);
                    if to == a_addr {
                        a.on_datagram(b_id, b_addr, &d);
                    }
                }
                opened.0 |= !oa.opened.is_empty();
                opened.1 |= !ob.opened.is_empty();
                got_a.extend(oa.delivered);
                got_b.extend(ob.delivered);
            }
            if a.is_open(b_id) && !sent {
                sent = true;
                assert!(a.send(b_id, 0, false, b"fast"));
                assert!(a.send(b_id, 5, true, &big));
            }
            if b.is_open(a_id) && !welcomed && !got_b.is_empty() {
                welcomed = true;
                assert!(b.send(a_id, 0, true, b"welcome"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(opened.0 && opened.1, "both sides opened");
        assert!(got_b.contains(&(a_id, 0, b"fast".to_vec())));
        assert!(got_b.contains(&(a_id, 5, big.clone())), "big reliable message intact");
        assert_eq!(got_a, vec![(b_id, 0, b"welcome".to_vec())]);
    }
}

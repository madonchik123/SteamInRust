//! ISteamNetworking (SendP2PPacket / ReadP2PPacket) between players who
//! both run SteamInRust.
//!
//! Valve's client carries these packets over the Steam Datagram Relay, a
//! closed protocol, so players on the official client can't be reached.
//! Between two SteamInRust players it works like this:
//!
//! 1. Each side opens one UDP socket and learns its addresses: LAN
//!    addresses, and the public one from a STUN server.
//! 2. They exchange addresses and a random session token through Steam
//!    (a lobby message addressed to that player when both are in the same
//!    lobby, and Steam's `ClientP2PConnectionInfo`), so only someone Steam
//!    vouches for knows the token.
//! 3. Both sides send PINGs to every address of the other (UDP hole
//!    punching) until one answers; then packets flow directly.
//! 4. "Reliable" send types get sequence numbers, acks, retransmission,
//!    in-order delivery and fragmentation; "unreliable" ones go as is.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, put, slice};
use crate::protocol::Request;
use crate::state::{Bytes, lock};
use crate::vt::Fp;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

pub fn imp(name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match name {
        "SendP2PPacket" => f!(send_p2p),
        "IsP2PPacketAvailable" => f!(available),
        "ReadP2PPacket" => f!(read_p2p),
        "AcceptP2PSessionWithUser" => f!(accept),
        "CloseP2PSessionWithUser" => f!(close),
        "CloseP2PChannelWithUser" => f!(close_channel),
        "GetP2PSessionState" => f!(session_state),
        "AllowP2PPacketRelay" => f!(allow_relay),
        _ => None,
    }
}

const P2P_SESSION_REQUEST: i32 = 1202;
const P2P_SESSION_CONNECT_FAIL: i32 = 1203;

const MAGIC: &[u8; 4] = b"SIRU";
const SIGNAL_MAGIC: &[u8; 4] = b"SIR1";
const PING: u8 = 1;
const PONG: u8 = 2;
const UNRELIABLE: u8 = 3;
const RELIABLE: u8 = 4;
const ACK: u8 = 5;
const HEADER: usize = 4 + 1 + 8 + 8 + 8;
/// Payload per UDP datagram (keeps packets under common MTUs).
const CHUNK: usize = 1100;
/// Steam's limits: 1200 bytes unreliable, 1 MB reliable.
const MAX_UNRELIABLE: usize = 1200;
const MAX_RELIABLE: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A SteamInRust player answers our signal through Steam within about a
/// second. No answer means the other side isn't SteamInRust (an official
/// client, which only talks over Valve's relays to strangers): fail the
/// session like Steam does, so the game shows its own "couldn't connect".
const NO_ANSWER_TIMEOUT: Duration = Duration::from_secs(6);
/// PINGs go every 2 s; this long without an answer means the link is gone.
const LINK_TIMEOUT: Duration = Duration::from_secs(20);

/// What a game asked for in SendP2PPacket's EP2PSend.
fn reliable(send_type: i32) -> bool {
    send_type == 2 || send_type == 3
}

#[derive(Default)]
struct Channel {
    next_seq: u32,
    /// Sent, not acked: seq -> (datagram, last sent, tries).
    unacked: BTreeMap<u32, (Vec<u8>, Instant, u32)>,
    expect: u32,
    early: BTreeMap<u32, (u16, u16, Vec<u8>)>,
    assembling: Vec<u8>,
}

struct Peer {
    token: u64,
    candidates: Vec<SocketAddr>,
    addr: Option<SocketAddr>,
    since: Instant,
    last_ping: Instant,
    last_heard: Option<Instant>,
    /// Packets waiting for the connection: (channel, reliable, data).
    waiting: VecDeque<(i32, bool, Vec<u8>)>,
    channels: HashMap<i32, Channel>,
    failed: bool,
    announced: bool,
    /// Their signal (or a packet) reached us: they run SteamInRust.
    answered: bool,
    /// Round trip of the last answered PING.
    rtt: Option<Duration>,
}

impl Peer {
    fn new() -> Self {
        let now = Instant::now();
        Peer { token: 0, candidates: Vec::new(), addr: None, since: now, last_ping: now - Duration::from_secs(1), last_heard: None, waiting: VecDeque::new(), channels: HashMap::new(), failed: false, announced: false, answered: false, rtt: None }
    }
}

/// Things the endpoint needs from outside (Steam signalling, callbacks), so
/// tests can run two endpoints without Steam.
pub trait Outside: Send + Sync {
    fn signal(&self, to: u64, payload: Vec<u8>);
    fn session_request(&self, from: u64);
    fn connect_failed(&self, to: u64);
    /// Channels another module handles (networking sockets / messages):
    /// true if consumed, so they don't reach ISteamNetworking.
    fn route(&self, _from: u64, _channel: i32, _data: &[u8]) -> bool {
        false
    }
}

pub struct Endpoint {
    me: u64,
    token: u64,
    socket: UdpSocket,
    public: Mutex<Option<SocketAddr>>,
    peers: Mutex<HashMap<u64, Peer>>,
    inbox: Mutex<HashMap<i32, VecDeque<(u64, Vec<u8>)>>>,
    outside: Box<dyn Outside>,
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.write_u32(std::process::id());
    h.finish() | 1
}

impl Endpoint {
    pub fn start(me: u64, outside: Box<dyn Outside>, stun: bool) -> std::io::Result<Arc<Endpoint>> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        let ep = Arc::new(Endpoint {
            me,
            token: random_u64(),
            socket,
            public: Mutex::new(None),
            peers: Mutex::new(HashMap::new()),
            inbox: Mutex::new(HashMap::new()),
            outside,
        });
        let e = ep.clone();
        std::thread::spawn(move || e.run(stun));
        Ok(ep)
    }

    fn port(&self) -> u16 {
        self.socket.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Our addresses: LAN ones and, once STUN answered, the public one.
    fn candidates(&self) -> Vec<SocketAddr> {
        let port = self.port();
        let mut out = Vec::new();
        if let Some(p) = *self.public.lock().unwrap() {
            out.push(p);
        }
        for ip in local_ips() {
            out.push(SocketAddr::V4(SocketAddrV4::new(ip, port)));
        }
        out.push(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)));
        out.dedup();
        out
    }

    fn signal_payload(&self, answer: bool) -> Vec<u8> {
        let mut b = SIGNAL_MAGIC.to_vec();
        b.push(answer as u8);
        b.extend_from_slice(&self.token.to_le_bytes());
        let c = self.candidates();
        b.push(c.len().min(16) as u8);
        for a in c.iter().take(16) {
            if let SocketAddr::V4(v4) = a {
                b.extend_from_slice(&v4.ip().octets());
                b.extend_from_slice(&v4.port().to_le_bytes());
            }
        }
        b
    }

    /// A signalling message from `from` (through Steam).
    pub fn on_signal(&self, from: u64, payload: &[u8]) {
        let Some((answer, token, candidates)) = parse_signal(payload) else { return };
        crate::log!("p2p: signal from {from}: {} address(es)", candidates.len());
        {
            let mut peers = self.peers.lock().unwrap();
            let p = peers.entry(from).or_insert_with(Peer::new);
            if p.token != token {
                // A new session on their side: start over.
                p.addr = None;
                p.channels.clear();
                p.failed = false;
                p.since = Instant::now();
            }
            p.token = token;
            p.answered = true;
            p.candidates = candidates;
            p.last_ping = Instant::now() - Duration::from_secs(1);
        }
        if !answer {
            self.outside.signal(from, self.signal_payload(true));
        }
    }

    /// Start connecting to `to` (sends our addresses through Steam).
    fn ensure_peer(&self, to: u64) {
        let new = {
            let mut peers = self.peers.lock().unwrap();
            let fresh = !peers.contains_key(&to);
            let p = peers.entry(to).or_insert_with(Peer::new);
            if p.failed {
                p.failed = false;
                p.since = Instant::now();
                true
            } else {
                fresh
            }
        };
        if new {
            crate::log!("p2p: connecting to {to}");
            self.outside.signal(to, self.signal_payload(false));
        }
    }

    pub fn send(&self, to: u64, data: &[u8], send_type: i32, channel: i32) -> bool {
        let rel = reliable(send_type);
        if data.is_empty() || data.len() > if rel { MAX_RELIABLE } else { MAX_UNRELIABLE } || to == 0 {
            return false;
        }
        self.ensure_peer(to);
        let mut peers = self.peers.lock().unwrap();
        let Some(p) = peers.get_mut(&to) else { return false };
        match p.addr {
            Some(addr) => {
                let token = p.token;
                self.transmit(p, to, token, addr, channel, rel, data);
            }
            None => {
                if p.waiting.len() < 1024 {
                    p.waiting.push_back((channel, rel, data.to_vec()));
                }
            }
        }
        true
    }

    fn header(&self, kind: u8, to: u64, token: u64) -> Vec<u8> {
        let mut b = Vec::with_capacity(HEADER + CHUNK + 16);
        b.extend_from_slice(MAGIC);
        b.push(kind);
        b.extend_from_slice(&self.me.to_le_bytes());
        b.extend_from_slice(&to.to_le_bytes());
        // Proves to the receiver that we got its token through Steam.
        b.extend_from_slice(&token.to_le_bytes());
        b
    }

    #[allow(clippy::too_many_arguments)]
    fn transmit(&self, p: &mut Peer, to: u64, token: u64, addr: SocketAddr, channel: i32, rel: bool, data: &[u8]) {
        if !rel {
            let mut b = self.header(UNRELIABLE, to, token);
            b.extend_from_slice(&channel.to_le_bytes());
            b.extend_from_slice(data);
            let _ = self.socket.send_to(&b, addr);
            return;
        }
        let ch = p.channels.entry(channel).or_default();
        let parts: Vec<&[u8]> = data.chunks(CHUNK).collect();
        let count = parts.len() as u16;
        for (i, part) in parts.into_iter().enumerate() {
            let seq = ch.next_seq;
            ch.next_seq = ch.next_seq.wrapping_add(1);
            let mut b = self.header(RELIABLE, to, token);
            b.extend_from_slice(&channel.to_le_bytes());
            b.extend_from_slice(&seq.to_le_bytes());
            b.extend_from_slice(&(i as u16).to_le_bytes());
            b.extend_from_slice(&count.to_le_bytes());
            b.extend_from_slice(part);
            let _ = self.socket.send_to(&b, addr);
            ch.unacked.insert(seq, (b, Instant::now(), 1));
        }
    }

    pub fn available(&self, channel: i32) -> Option<usize> {
        self.inbox.lock().unwrap().get(&channel).and_then(|q| q.front()).map(|(_, d)| d.len())
    }

    pub fn read(&self, channel: i32) -> Option<(u64, Vec<u8>)> {
        self.inbox.lock().unwrap().get_mut(&channel).and_then(|q| q.pop_front())
    }

    pub fn close(&self, peer: u64) -> bool {
        self.peers.lock().unwrap().remove(&peer).is_some()
    }

    /// (connected, connecting, failed, address).
    pub fn state(&self, peer: u64) -> Option<(bool, bool, bool, Option<SocketAddr>, usize)> {
        let peers = self.peers.lock().unwrap();
        let p = peers.get(&peer)?;
        let queued = p.waiting.len() + p.channels.values().map(|c| c.unacked.len()).sum::<usize>();
        Some((p.addr.is_some(), p.addr.is_none() && !p.failed, p.failed, p.addr, queued))
    }

    fn run(self: Arc<Self>, stun: bool) {
        let mut stun_at = Instant::now() - Duration::from_secs(60);
        let mut stun_server: Option<SocketAddr> = None;
        let mut buf = vec![0u8; 65536];
        loop {
            if stun && self.public.lock().unwrap().is_none() && stun_at.elapsed() > Duration::from_secs(5) {
                stun_at = Instant::now();
                if stun_server.is_none() {
                    stun_server = resolve_stun();
                }
                if let Some(s) = stun_server {
                    let _ = self.socket.send_to(&stun_request(), s);
                }
            }
            match self.socket.recv_from(&mut buf) {
                Ok((n, from)) => self.on_datagram(&buf[..n], from),
                Err(_) => {}
            }
            self.tick();
        }
    }

    fn on_datagram(&self, d: &[u8], from: SocketAddr) {
        if let Some(addr) = parse_stun_response(d) {
            let first = self.public.lock().unwrap().replace(addr).is_none();
            if first {
                crate::log!("p2p: public address {addr}");
                // Tell peers we're still trying to reach about it.
                let pending: Vec<u64> = self.peers.lock().unwrap().iter().filter(|(_, p)| p.addr.is_none()).map(|(id, _)| *id).collect();
                for id in pending {
                    self.outside.signal(id, self.signal_payload(false));
                }
            }
            return;
        }
        if d.len() < HEADER || &d[..4] != MAGIC {
            return;
        }
        let kind = d[4];
        let src = u64::from_le_bytes(d[5..13].try_into().unwrap());
        let dst = u64::from_le_bytes(d[13..21].try_into().unwrap());
        let token = u64::from_le_bytes(d[21..29].try_into().unwrap());
        if dst != self.me || token != self.token {
            return;
        }
        let body = &d[HEADER..];
        let mut deliver: Vec<(i32, Vec<u8>)> = Vec::new();
        let mut request = false;
        {
            let mut peers = self.peers.lock().unwrap();
            let Some(p) = peers.get_mut(&src) else { return };
            if p.token == 0 {
                return;
            }
            p.last_heard = Some(Instant::now());
            let their_token = p.token;
            if p.addr != Some(from) && matches!(kind, PING | PONG) || p.addr.is_none() {
                if p.addr.is_none() {
                    crate::log!("p2p: connected to {src} at {from}");
                }
                p.addr = Some(from);
                p.failed = false;
                // Flush what waited for the connection.
                let waiting: Vec<_> = p.waiting.drain(..).collect();
                for (ch, rel, data) in waiting {
                    self.transmit(p, src, their_token, from, ch, rel, &data);
                }
            }
            match kind {
                PING => {
                    let b = self.header(PONG, src, their_token);
                    let _ = self.socket.send_to(&b, from);
                }
                PONG => p.rtt = Some(p.last_ping.elapsed()),
                UNRELIABLE if body.len() >= 4 => {
                    deliver.push((i32::from_le_bytes(body[..4].try_into().unwrap()), body[4..].to_vec()));
                }
                RELIABLE if body.len() >= 12 => {
                    let ch = i32::from_le_bytes(body[..4].try_into().unwrap());
                    let seq = u32::from_le_bytes(body[4..8].try_into().unwrap());
                    let idx = u16::from_le_bytes(body[8..10].try_into().unwrap());
                    let count = u16::from_le_bytes(body[10..12].try_into().unwrap());
                    let mut ack = self.header(ACK, src, their_token);
                    ack.extend_from_slice(&ch.to_le_bytes());
                    ack.extend_from_slice(&seq.to_le_bytes());
                    let _ = self.socket.send_to(&ack, from);
                    let c = p.channels.entry(ch).or_default();
                    if seq.wrapping_sub(c.expect) < 1 << 30 {
                        c.early.insert(seq, (idx, count, body[12..].to_vec()));
                    }
                    while let Some((idx, count, part)) = c.early.remove(&c.expect) {
                        c.expect = c.expect.wrapping_add(1);
                        if idx == 0 {
                            c.assembling.clear();
                        }
                        c.assembling.extend_from_slice(&part);
                        if idx + 1 >= count {
                            deliver.push((ch, std::mem::take(&mut c.assembling)));
                        }
                    }
                }
                ACK if body.len() >= 8 => {
                    let ch = i32::from_le_bytes(body[..4].try_into().unwrap());
                    let seq = u32::from_le_bytes(body[4..8].try_into().unwrap());
                    if let Some(c) = p.channels.get_mut(&ch) {
                        c.unacked.remove(&seq);
                    }
                }
                _ => {}
            }
        }
        // Networking sockets / messages take their own channels.
        deliver.retain(|(ch, data)| !self.outside.route(src, *ch, data));
        if !deliver.is_empty() {
            let mut peers = self.peers.lock().unwrap();
            if let Some(p) = peers.get_mut(&src).filter(|p| !p.announced) {
                p.announced = true;
                request = true;
            }
        }
        if request {
            self.outside.session_request(src);
        }
        if !deliver.is_empty() {
            let mut inbox = self.inbox.lock().unwrap();
            for (ch, data) in deliver {
                let q = inbox.entry(ch).or_default();
                if q.len() < 8192 {
                    q.push_back((src, data));
                }
            }
        }
    }

    /// Pings, retransmissions and timeouts (called ~20 times a second).
    fn tick(&self) {
        let now = Instant::now();
        let mut failed = Vec::new();
        let mut peers = self.peers.lock().unwrap();
        for (id, p) in peers.iter_mut() {
            if p.token != 0 && (p.addr.is_none() || p.last_ping.elapsed() > Duration::from_secs(2)) && p.last_ping.elapsed() > Duration::from_millis(200) {
                p.last_ping = now;
                let b = self.header(PING, *id, p.token);
                let targets: Vec<SocketAddr> = match p.addr {
                    Some(a) => vec![a],
                    None => p.candidates.clone(),
                };
                for a in targets {
                    let _ = self.socket.send_to(&b, a);
                }
            }
            let waited = p.since.elapsed();
            if p.addr.is_none() && !p.failed && (waited > CONNECT_TIMEOUT || !p.answered && waited > NO_ANSWER_TIMEOUT) {
                if !p.answered {
                    crate::log!("p2p: {id} never answered (not on SteamInRust?)");
                }
                p.failed = true;
                p.waiting.clear();
                failed.push(*id);
            }
            // Connected, but nothing heard for a while (they quit or the
            // path broke): drop the link so it reconnects on the next send.
            if p.addr.is_some() && p.last_heard.is_some_and(|t| t.elapsed() > LINK_TIMEOUT) {
                p.addr = None;
                p.failed = true;
                p.channels.clear();
                failed.push(*id);
            }
            let Some(addr) = p.addr else { continue };
            for c in p.channels.values_mut() {
                for (_, (b, sent, tries)) in c.unacked.iter_mut() {
                    if sent.elapsed() > Duration::from_millis(150 * (*tries as u64).min(8)) {
                        let _ = self.socket.send_to(b, addr);
                        *sent = now;
                        *tries += 1;
                    }
                }
            }
        }
        drop(peers);
        for id in failed {
            crate::log!("p2p: couldn't reach {id}");
            self.outside.connect_failed(id);
        }
    }
}

fn parse_signal(d: &[u8]) -> Option<(bool, u64, Vec<SocketAddr>)> {
    if d.len() < 14 || &d[..4] != SIGNAL_MAGIC {
        return None;
    }
    let answer = d[4] == 1;
    let token = u64::from_le_bytes(d[5..13].try_into().ok()?);
    let n = d[13] as usize;
    let mut out = Vec::new();
    for i in 0..n {
        let at = 14 + i * 6;
        let c = d.get(at..at + 6)?;
        out.push(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(c[0], c[1], c[2], c[3]), u16::from_le_bytes([c[4], c[5]]))));
    }
    (token != 0).then_some((answer, token, out))
}

/// This PC's IPv4 addresses (the one used for the internet first).
fn local_ips() -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:53").is_ok() {
            if let Ok(SocketAddr::V4(a)) = s.local_addr() {
                out.push(*a.ip());
            }
        }
    }
    out
}

fn resolve_stun() -> Option<SocketAddr> {
    use std::net::ToSocketAddrs;
    ["stun.l.google.com:19302", "stun.cloudflare.com:3478"].iter().find_map(|h| h.to_socket_addrs().ok()?.find(|a| a.is_ipv4()))
}

/// STUN binding request (RFC 5389).
fn stun_request() -> Vec<u8> {
    let mut b = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
    b.extend_from_slice(&random_u64().to_le_bytes());
    b.extend_from_slice(&(random_u64() as u32).to_le_bytes());
    b
}

fn parse_stun_response(d: &[u8]) -> Option<SocketAddr> {
    if d.len() < 20 || d[0] != 0x01 || d[1] != 0x01 || d[4..8] != [0x21, 0x12, 0xA4, 0x42] {
        return None;
    }
    let mut at = 20;
    while at + 4 <= d.len() {
        let kind = u16::from_be_bytes([d[at], d[at + 1]]);
        let len = u16::from_be_bytes([d[at + 2], d[at + 3]]) as usize;
        let v = d.get(at + 4..at + 4 + len)?;
        // XOR-MAPPED-ADDRESS (or MAPPED-ADDRESS), IPv4.
        if (kind == 0x0020 || kind == 0x0001) && len >= 8 && v[1] == 0x01 {
            let mut port = u16::from_be_bytes([v[2], v[3]]);
            let mut ip = [v[4], v[5], v[6], v[7]];
            if kind == 0x0020 {
                port ^= 0x2112;
                for (i, b) in ip.iter_mut().enumerate() {
                    *b ^= [0x21, 0x12, 0xA4, 0x42][i];
                }
            }
            return Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(ip), port)));
        }
        at += 4 + len.div_ceil(4) * 4;
    }
    None
}

// ------------------------------------------------- the game's endpoint

/// Signalling through Steam and callbacks into the game.
struct Steam;

impl Outside for Steam {
    fn signal(&self, to: u64, payload: Vec<u8>) {
        std::thread::spawn(move || {
            // Steam's P2P signalling message, and a lobby message addressed
            // to that player when we share a lobby.
            let lobby = crate::lobby::members_of_joined().into_iter().find(|(_, m)| m.contains(&to)).map(|(id, _)| id).unwrap_or(0);
            let _ = crate::ipc::request(Request::Signal { to, data: payload, lobby });
        });
    }
    fn session_request(&self, from: u64) {
        let mut b = Bytes::new(8);
        b.u64(0, from);
        lock().post(P2P_SESSION_REQUEST, b.0);
    }
    fn route(&self, from: u64, channel: i32, data: &[u8]) -> bool {
        if channel == crate::sns::CH_SOCKETS || channel == crate::sns::CH_MESSAGES {
            crate::sns::on_wire(from, channel, data);
            return true;
        }
        false
    }
    fn connect_failed(&self, to: u64) {
        crate::sns::peer_failed(to);
        // P2PSessionConnectFail_t: remote, EP2PSessionError (4 = timeout).
        let mut b = Bytes::new(16);
        b.u64(0, to).u8(8, 4);
        lock().post(P2P_SESSION_CONNECT_FAIL, b.0);
    }
}

static EP: LazyLock<Option<Arc<Endpoint>>> = LazyLock::new(|| {
    let me = lock().me();
    match Endpoint::start(me, Box::new(Steam), true) {
        Ok(e) => {
            crate::log!("p2p: listening on UDP {}", e.port());
            Some(e)
        }
        Err(err) => {
            crate::log!("p2p: no UDP socket: {err}");
            None
        }
    }
});

fn ep() -> Option<&'static Arc<Endpoint>> {
    if lock().session.is_none() {
        return None;
    }
    EP.as_ref()
}

/// A signalling message arrived from another player.
pub fn signal(from: u64, payload: &[u8]) {
    if let Some(e) = ep() {
        e.on_signal(from, payload);
    }
}

/// Send on a channel for another module (networking sockets / messages).
pub fn send_raw(to: u64, data: &[u8], reliable: bool, channel: i32) -> bool {
    ep().is_some_and(|e| e.send(to, data, if reliable { 2 } else { 0 }, channel))
}

/// Round trip to a player in ms, once measured.
pub fn rtt_ms(peer: u64) -> Option<i32> {
    let e = ep()?;
    let peers = e.peers.lock().unwrap();
    peers.get(&peer)?.rtt.map(|d| d.as_millis().min(i32::MAX as u128) as i32)
}

/// (connected, still connecting) for a player we've talked to.
pub fn link(peer: u64) -> Option<(bool, bool)> {
    ep()?.state(peer).map(|(connected, connecting, ..)| (connected, connecting))
}

pub fn lobby_changed(_lobby: u64) {}
pub fn left_lobby(_lobby: u64) {}

extern "C" fn send_p2p(_t: This, to: u64, data: *const u8, len: u32, send_type: i32, channel: i32) -> bool {
    ep().is_some_and(|e| e.send(to, slice(data, len as usize), send_type, channel))
}

extern "C" fn available(_t: This, size: *mut u32, channel: i32) -> bool {
    match ep().and_then(|e| e.available(channel)) {
        Some(n) => {
            put(size, n as u32);
            true
        }
        None => {
            put(size, 0);
            false
        }
    }
}

extern "C" fn read_p2p(_t: This, dest: *mut u8, cap: u32, size: *mut u32, remote: *mut u64, channel: i32) -> bool {
    let Some((from, data)) = ep().and_then(|e| e.read(channel)) else { return false };
    let n = data.len().min(cap as usize);
    if !dest.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), dest, n) };
    }
    put(size, n as u32);
    put(remote, from);
    true
}

extern "C" fn accept(_t: This, _user: u64) -> bool {
    ep().is_some()
}

extern "C" fn close(_t: This, user: u64) -> bool {
    ep().is_some_and(|e| e.close(user))
}

extern "C" fn close_channel(_t: This, _user: u64, _channel: i32) -> bool {
    true
}

/// P2PSessionState_t: active, connecting, error, relay, bytes and packets
/// queued, remote IP and port.
extern "C" fn session_state(_t: This, user: u64, out: *mut u8) -> bool {
    let Some((active, connecting, failed, addr, queued)) = ep().and_then(|e| e.state(user)) else { return false };
    if !out.is_null() {
        let mut b = Bytes::new(20);
        b.u8(0, active as u8).u8(1, connecting as u8).u8(2, if failed { 4 } else { 0 }).i32(8, queued as i32);
        if let Some(SocketAddr::V4(a)) = addr {
            b.u32(12, u32::from(*a.ip())).u32(16, a.port() as u32);
        }
        unsafe { std::ptr::copy_nonoverlapping(b.0.as_ptr(), out, 20) };
    }
    true
}

extern "C" fn allow_relay(_t: This, _allow: bool) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Weak;

    /// Signalling between two endpoints in one process (stands in for Steam).
    struct Wire {
        other: Mutex<Weak<Endpoint>>,
        me: u64,
        requests: Mutex<Vec<u64>>,
    }

    impl Outside for Arc<Wire> {
        fn signal(&self, _to: u64, payload: Vec<u8>) {
            let other = self.other.lock().unwrap().upgrade();
            let me = self.me;
            std::thread::spawn(move || {
                if let Some(o) = other {
                    o.on_signal(me, &payload);
                }
            });
        }
        fn session_request(&self, from: u64) {
            self.requests.lock().unwrap().push(from);
        }
        fn connect_failed(&self, _to: u64) {}
    }

    fn wait_for(ep: &Endpoint, ch: i32) -> Option<(u64, Vec<u8>)> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if let Some(m) = ep.read(ch) {
                return Some(m);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn two_players_exchange_packets() {
        let (a_id, b_id) = (76561198000000001u64, 76561198000000002u64);
        let wa = Arc::new(Wire { other: Mutex::new(Weak::new()), me: a_id, requests: Mutex::default() });
        let wb = Arc::new(Wire { other: Mutex::new(Weak::new()), me: b_id, requests: Mutex::default() });
        let a = Endpoint::start(a_id, Box::new(wa.clone()), false).unwrap();
        let b = Endpoint::start(b_id, Box::new(wb.clone()), false).unwrap();
        *wa.other.lock().unwrap() = Arc::downgrade(&b);
        *wb.other.lock().unwrap() = Arc::downgrade(&a);

        // Sent before any connection exists: waits, then arrives.
        assert!(a.send(b_id, b"hello", 2, 0));
        assert_eq!(wait_for(&b, 0), Some((a_id, b"hello".to_vec())));
        assert_eq!(wb.requests.lock().unwrap().as_slice(), &[a_id], "P2PSessionRequest once");

        // A reliable message bigger than one datagram, on another channel.
        let big: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        assert!(b.send(a_id, &big, 2, 3));
        assert_eq!(wait_for(&a, 3), Some((b_id, big)));

        // Unreliable, and order kept for reliable ones.
        assert!(a.send(b_id, b"u", 0, 1));
        assert_eq!(wait_for(&b, 1).map(|m| m.1), Some(b"u".to_vec()));
        for i in 0..50u8 {
            assert!(a.send(b_id, &[i], 3, 2));
        }
        for i in 0..50u8 {
            assert_eq!(wait_for(&b, 2).map(|m| m.1), Some(vec![i]));
        }
        assert!(matches!(a.state(b_id), Some((true, false, false, Some(_), _))));
        // Unreliable packets over Steam's limit are refused, like Steam does.
        assert!(!a.send(b_id, &[0u8; 1300], 0, 0));
    }

    #[test]
    fn stun_and_signal_parsing() {
        // XOR-MAPPED-ADDRESS for 203.0.113.5:40000.
        let ip = [203u8, 0, 113, 5];
        let mut d = vec![0x01, 0x01, 0x00, 0x0c, 0x21, 0x12, 0xA4, 0x42];
        d.extend_from_slice(&[0; 12]);
        d.extend_from_slice(&[0x00, 0x20, 0x00, 0x08, 0x00, 0x01]);
        d.extend_from_slice(&(40000u16 ^ 0x2112).to_be_bytes());
        for (i, b) in ip.iter().enumerate() {
            d.push(b ^ [0x21, 0x12, 0xA4, 0x42][i]);
        }
        assert_eq!(parse_stun_response(&d), Some("203.0.113.5:40000".parse().unwrap()));
        let mut s = SIGNAL_MAGIC.to_vec();
        s.push(0);
        s.extend_from_slice(&7u64.to_le_bytes());
        s.push(1);
        s.extend_from_slice(&[192, 168, 1, 2]);
        s.extend_from_slice(&5000u16.to_le_bytes());
        assert_eq!(parse_signal(&s), Some((false, 7, vec!["192.168.1.2:5000".parse().unwrap()])));
    }
}

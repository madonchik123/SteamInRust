//! ISteamNetworkingSockets, ISteamNetworkingMessages and the parts of
//! ISteamNetworkingUtils games wait on.
//!
//! Connections to other players speak Valve's own protocol ([`crate::gns`]):
//! a Valve-signed certificate, ConnectRequest / ConnectOK in rendezvous
//! signals through Steam, then encrypted SNP packets over ICE. So players
//! on Valve's official client and SteamInRust players can connect to each
//! other either way.
//!
//! Connecting to yourself (a host joining its own game) and socket pairs
//! stay in-process. Connections by IP address (dedicated servers) would need
//! Valve's UDP connect protocol and fail cleanly instead.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::gns::conn::{Conn as Proto, Io, State as PState, VPORT_MESSAGES};
use crate::gns::crypto::LocalCert;
use crate::gns::engine::{self, Engine};
use crate::gns::msg::Rendezvous;
use crate::ifaces::{This, copy_out, put, slice};
use crate::state::Bytes;
use crate::vt::Fp;
use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_void};
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

// Callback ids and sizes (Windows packing).
const CB_CONN_STATUS: i32 = 1221;
const CB_AUTH_STATUS: i32 = 1222;
const CB_RELAY_STATUS: i32 = 1281;
const CB_SESSION_REQUEST: i32 = 1251;
const CB_SESSION_FAILED: i32 = 1252;
const IDENTITY: usize = 136;
const INFO: usize = 696;
const STATUS_CHANGED: usize = 712;
const REALTIME_STATUS: usize = 120;

// ESteamNetworkingConnectionState
const NONE: i32 = 0;
const CONNECTING: i32 = 1;
const FINDING_ROUTE: i32 = 2;
const CONNECTED: i32 = 3;
const CLOSED_BY_PEER: i32 = 4;
const PROBLEM: i32 = 5;

// ESteamNetConnectionEnd
const END_APP: i32 = 1000;
const END_MISC_GENERIC: i32 = 5001;

// EResult
const OK: i32 = 1;
const FAIL: i32 = 2;
const NO_CONNECTION: i32 = 3;
const INVALID_PARAM: i32 = 8;
const INVALID_STATE: i32 = 11;
const LIMIT_EXCEEDED: i32 = 25;

/// ESteamNetworkingAvailability::Current.
const AVAILABLE: i32 = 100;
const SEND_RELIABLE: i32 = 8;
const SEND_AUTO_RESTART: i32 = 32;
const MAX_MESSAGE: usize = 512 * 1024;

// ESteamNetworkingConfigValue callbacks and scopes.
const CFG_CONN_STATUS: i32 = 201;
const CFG_AUTH_STATUS: i32 = 202;
const CFG_RELAY_STATUS: i32 = 203;
const CFG_SESSION_REQUEST: i32 = 204;
const CFG_SESSION_FAILED: i32 = 205;
const SCOPE_LISTEN: i32 = 3;
const SCOPE_CONNECTION: i32 = 4;
const TYPE_PTR: i32 = 5;

/// A received message, before it becomes a SteamNetworkingMessage_t.
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub conn: u32,
    pub peer: u64,
    pub data: Vec<u8>,
    pub number: i64,
    pub channel: i32,
    pub user_data: i64,
    pub arrival: u64,
}

#[derive(Default)]
struct Conn {
    peer: u64,
    vport: i32,
    listen: u32,
    state: i32,
    end_reason: i32,
    end_debug: String,
    user_data: i64,
    name: String,
    poll_group: u32,
    callback: usize,
    /// The other end of a socket pair or of a connection to ourselves.
    local: u32,
    inbox: VecDeque<Received>,
    sent: i64,
    received: i64,
    /// Valve's protocol, for a connection to another player.
    proto: Option<Box<Proto>>,
}

struct Listen {
    vport: i32,
    callback: usize,
}

#[derive(Default)]
struct Session {
    accepted: bool,
    asked: bool,
    failed: bool,
    received: i64,
    /// Messages per local channel; held back until the session is accepted.
    inbox: HashMap<i32, VecDeque<Received>>,
    proto: Option<Box<Proto>>,
}

/// Global config callbacks, by ESteamNetworkingConfigValue.
#[derive(Default)]
struct Callbacks {
    conn_status: usize,
    auth_status: usize,
    relay_status: usize,
    session_request: usize,
    session_failed: usize,
}

pub struct Net {
    me: u64,
    next: u32,
    arrival: u64,
    conns: HashMap<u32, Conn>,
    listens: HashMap<u32, Listen>,
    groups: HashMap<u32, ()>,
    sessions: HashMap<u64, Session>,
    callbacks: Callbacks,
    /// Callbacks for the game: (callback id, bytes, config function or 0).
    pub events: Vec<(i32, Vec<u8>, usize)>,
    engine: Option<Engine>,
    /// Rendezvous signals to send once the lock is released.
    signals: Vec<(u64, Vec<u8>)>,
}

fn reliable(flags: i32) -> bool {
    flags & SEND_RELIABLE != 0
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(d: &[u8], at: usize) -> Option<i32> {
    u32_at(d, at).map(|v| v as i32)
}

/// SteamNetworkingIdentity for a Steam ID.
pub fn identity(steam_id: u64) -> Vec<u8> {
    let mut b = Bytes::new(IDENTITY);
    if steam_id != 0 {
        b.i32(0, 16).i32(4, 8).u64(8, steam_id);
    }
    b.0
}

/// The Steam ID in a SteamNetworkingIdentity (0 for other kinds).
fn steam_id_of(p: *const u8) -> u64 {
    if p.is_null() {
        return 0;
    }
    let d = slice(p, 16);
    if i32_at(d, 0) == Some(16) { u64::from_le_bytes(d[8..16].try_into().unwrap()) } else { 0 }
}

fn app_state(s: PState) -> i32 {
    match s {
        PState::Connecting => CONNECTING,
        PState::FindingRoute => FINDING_ROUTE,
        PState::Connected => CONNECTED,
        PState::ClosedByPeer => CLOSED_BY_PEER,
        PState::Problem => PROBLEM,
        PState::Closed => NONE,
    }
}

/// The 5-byte header ISteamNetworkingMessages puts on every message.
fn messages_frame(channel: i32, data: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(data.len() + 5);
    m.push(1);
    m.extend_from_slice(&channel.to_le_bytes());
    m.extend_from_slice(data);
    m
}

/// SteamNetworkingIPAddr (IPv6-mapped address, port) at 148 in SteamNetConnectionInfo_t.
fn put_addr(b: &mut Bytes, a: Option<SocketAddr>) {
    if let Some(SocketAddr::V4(a)) = a {
        let mut ip = [0u8; 18];
        ip[10] = 0xff;
        ip[11] = 0xff;
        ip[12..16].copy_from_slice(&a.ip().octets());
        ip[16..].copy_from_slice(&a.port().to_le_bytes());
        b.put_bytes(148, &ip);
    }
}

/// SteamNetConnectionInfo_t for a session (ISteamNetworkingMessages).
fn session_info(peer: u64, state: i32, reason: i32, debug: &str, p: Option<&Proto>) -> Vec<u8> {
    let mut b = Bytes::new(INFO);
    b.put_bytes(0, &identity(peer)).i32(176, state).i32(180, reason).str(184, 128, debug);
    if let Some(p) = p {
        b.str(312, 128, &format!("#{} P2P steamid:{} (messages)", p.local_id, peer));
        put_addr(&mut b, p.remote_addr());
        b.i32(440, if p.remote_trusted { 0 } else { 2 });
    }
    b.0
}

impl Net {
    pub fn new(me: u64, with_network: bool) -> Self {
        Net {
            me,
            next: 0x51_0000 + (std::process::id() & 0xFFFF),
            arrival: 0,
            conns: HashMap::new(),
            listens: HashMap::new(),
            groups: HashMap::new(),
            sessions: HashMap::new(),
            callbacks: Callbacks::default(),
            events: Vec::new(),
            engine: with_network.then(Engine::start),
            signals: Vec::new(),
        }
    }

    fn handle(&mut self) -> u32 {
        self.next = self.next.wrapping_add(1).max(1);
        self.next
    }

    // ---------------------------------------------------------- callbacks

    /// SteamNetConnectionInfo_t.
    fn info(&self, h: u32) -> Vec<u8> {
        let mut b = Bytes::new(INFO);
        if let Some(c) = self.conns.get(&h) {
            b.put_bytes(0, &identity(c.peer));
            b.u64(136, c.user_data as u64).u32(144, c.listen);
            b.i32(176, c.state).i32(180, c.end_reason).str(184, 128, &c.end_debug);
            let desc = if !c.name.is_empty() {
                c.name.clone()
            } else if let Some(p) = &c.proto {
                format!("#{} P2P steamid:{} vport {}", p.local_id, c.peer, c.vport)
            } else {
                format!("#{h} P2P steamid:{} vport {}", c.peer, c.vport)
            };
            b.str(312, 128, &desc);
            put_addr(&mut b, c.proto.as_ref().and_then(|p| p.remote_addr()));
            // Flags: 2 = unauthenticated (peer's cert isn't Valve's), 4 = unencrypted.
            let flags = match &c.proto {
                Some(p) if p.remote_trusted => 0,
                Some(_) => 2,
                None => 2 | 4,
            };
            b.i32(440, flags);
        }
        b.0
    }

    fn set_state(&mut self, h: u32, state: i32, reason: i32, debug: &str) {
        let Some(c) = self.conns.get_mut(&h) else { return };
        let old = c.state;
        if old == state {
            return;
        }
        c.state = state;
        if reason != 0 {
            c.end_reason = reason;
            c.end_debug = debug.to_string();
        }
        let callback = c.callback;
        let mut b = Bytes::new(STATUS_CHANGED);
        b.u32(0, h).put_bytes(8, &self.info(h)).i32(704, old);
        let f = if callback != 0 { callback } else { self.callbacks.conn_status };
        self.events.push((CB_CONN_STATUS, b.0, f));
    }

    fn session_failed(&mut self, peer: u64, reason: i32, debug: &str) {
        let info = session_info(peer, PROBLEM, reason, debug, None);
        self.events.push((CB_SESSION_FAILED, info, self.callbacks.session_failed));
    }

    /// Pass a protocol connection's news (state, messages) to the app.
    fn sync(&mut self, h: u32) {
        let Some(c) = self.conns.get_mut(&h) else { return };
        let Some(p) = c.proto.as_mut() else { return };
        let msgs: Vec<_> = p.snp.inbox.drain(..).collect();
        let news = std::mem::take(&mut p.changed).then(|| (app_state(p.state), p.end_reason as i32, p.end_debug.clone()));
        for m in msgs {
            self.arrival += 1;
            c.received += 1;
            if c.inbox.len() < 65536 {
                c.inbox.push_back(Received { conn: h, peer: c.peer, data: m.data, number: m.num, channel: 0, user_data: c.user_data, arrival: self.arrival });
            }
        }
        if let Some((state, reason, debug)) = news {
            self.set_state(h, state, reason, &debug);
        }
    }

    fn sync_session(&mut self, peer: u64) {
        let Some(s) = self.sessions.get_mut(&peer) else { return };
        let Some(p) = s.proto.as_mut() else { return };
        for m in p.snp.inbox.drain(..) {
            if m.data.len() < 5 {
                continue;
            }
            let ch = i32::from_le_bytes(m.data[1..5].try_into().unwrap());
            self.arrival += 1;
            s.received += 1;
            let q = s.inbox.entry(ch).or_default();
            if q.len() < 65536 {
                q.push_back(Received { conn: 0, peer, data: m.data[5..].to_vec(), number: m.num, channel: ch, user_data: 0, arrival: self.arrival });
            }
        }
        let ended = std::mem::take(&mut p.changed) && matches!(p.state, PState::ClosedByPeer | PState::Problem);
        let (r, d) = (p.end_reason as i32, p.end_debug.clone());
        let ask = !s.accepted && !s.asked && p.state == PState::Connecting && !p.client;
        let fail = ended && !s.failed;
        if fail {
            s.failed = true;
        }
        if ask {
            s.asked = true;
        }
        if fail {
            crate::log!("sns: messages session with {peer} ended ({r} {d})");
            self.session_failed(peer, r, &d);
        }
        if ask {
            self.events.push((CB_SESSION_REQUEST, identity(peer), self.callbacks.session_request));
        }
    }

    /// Run a protocol step on connection `h` with the network at hand.
    fn on_proto(&mut self, h: u32, f: impl FnOnce(&mut Proto, &mut dyn Io)) {
        let Net { conns, engine, signals, .. } = self;
        let (Some(c), Some(e)) = (conns.get_mut(&h), engine.as_ref()) else { return };
        if let Some(p) = c.proto.as_mut() {
            f(p, &mut e.io(signals));
        }
        self.sync(h);
    }

    fn on_session(&mut self, peer: u64, f: impl FnOnce(&mut Proto, &mut dyn Io)) {
        let Net { sessions, engine, signals, .. } = self;
        let (Some(s), Some(e)) = (sessions.get_mut(&peer), engine.as_ref()) else { return };
        if let Some(p) = s.proto.as_mut() {
            f(p, &mut e.io(signals));
        }
        self.sync_session(peer);
    }

    // ------------------------------------------------------------ sockets

    pub fn listen(&mut self, vport: i32, callback: usize) -> u32 {
        let h = self.handle();
        self.listens.insert(h, Listen { vport, callback });
        h
    }

    pub fn connect(&mut self, peer: u64, vport: i32, callback: usize, cert: Option<Arc<LocalCert>>) -> u32 {
        let h = self.handle();
        if peer == self.me {
            return self.connect_self(h, vport, callback);
        }
        let now = Instant::now();
        let proto = cert.map(|c| Box::new(Proto::connect(self.me, peer, vport, vport, c, now)));
        let has = proto.is_some();
        self.conns.insert(h, Conn { peer, vport, callback, proto, ..Default::default() });
        self.set_state(h, CONNECTING, 0, "");
        if !has || self.engine.is_none() {
            self.set_state(h, PROBLEM, END_MISC_GENERIC, "networking isn't available");
            return h;
        }
        self.on_proto(h, |p, io| p.poll(now, io));
        h
    }

    /// A host connecting to its own listen socket: two linked local ends.
    fn connect_self(&mut self, h: u32, vport: i32, callback: usize) -> u32 {
        let me = self.me;
        self.conns.insert(h, Conn { peer: me, vport, callback, ..Default::default() });
        self.set_state(h, CONNECTING, 0, "");
        let Some((&lh, l)) = self.listens.iter().find(|(_, l)| l.vport == vport) else {
            self.set_state(h, PROBLEM, 5010, "no listen socket on that virtual port");
            return h;
        };
        let lcb = l.callback;
        let other = self.handle();
        self.conns.insert(other, Conn { peer: me, vport, listen: lh, callback: lcb, local: h, ..Default::default() });
        self.conns.get_mut(&h).unwrap().local = other;
        self.set_state(other, CONNECTING, 0, "");
        h
    }

    /// A connection that can't work (by IP address): reports and fails.
    pub fn connect_unsupported(&mut self, callback: usize, why: &str) -> u32 {
        let h = self.handle();
        self.conns.insert(h, Conn { callback, ..Default::default() });
        self.set_state(h, CONNECTING, 0, "");
        self.set_state(h, PROBLEM, END_MISC_GENERIC, why);
        h
    }

    pub fn accept(&mut self, h: u32) -> i32 {
        let Some(c) = self.conns.get(&h) else { return INVALID_PARAM };
        if c.listen == 0 || c.state != CONNECTING {
            return INVALID_STATE;
        }
        if c.local != 0 {
            let other = c.local;
            self.set_state(h, CONNECTED, 0, "");
            self.set_state(other, CONNECTED, 0, "");
            return OK;
        }
        let now = Instant::now();
        let mut r = OK;
        self.on_proto(h, |p, io| {
            if let Err(e) = p.accept(now) {
                crate::log!("sns: accept failed: {e}");
                r = FAIL;
            }
            p.poll(now, io);
        });
        r
    }

    pub fn close(&mut self, h: u32, reason: i32, debug: &str) -> bool {
        let Some(mut c) = self.conns.remove(&h) else { return false };
        let reason = if reason == 0 { END_APP } else { reason };
        if c.local != 0 {
            self.set_state(c.local, CLOSED_BY_PEER, reason, debug);
            if let Some(o) = self.conns.get_mut(&c.local) {
                o.local = 0;
            }
        } else if let (Some(p), Some(e)) = (c.proto.as_mut(), self.engine.as_ref()) {
            p.close(reason as u32, debug, &mut e.io(&mut self.signals));
        }
        true
    }

    pub fn close_listen(&mut self, h: u32) -> bool {
        if self.listens.remove(&h).is_none() {
            return false;
        }
        let accepted: Vec<u32> = self.conns.iter().filter(|(_, c)| c.listen == h).map(|(h, _)| *h).collect();
        for c in accepted {
            self.close(c, END_APP, "listen socket closed");
        }
        true
    }

    /// A pair of connected sockets inside this process.
    pub fn pair(&mut self) -> (u32, u32) {
        let (a, b) = (self.handle(), self.handle());
        let me = self.me;
        self.conns.insert(a, Conn { peer: me, state: CONNECTED, local: b, ..Default::default() });
        self.conns.insert(b, Conn { peer: me, state: CONNECTED, local: a, ..Default::default() });
        (a, b)
    }

    pub fn send(&mut self, h: u32, data: &[u8], flags: i32) -> (i32, i64) {
        if data.len() > MAX_MESSAGE {
            return (INVALID_PARAM, 0);
        }
        let Some(c) = self.conns.get_mut(&h) else { return (INVALID_PARAM, 0) };
        match c.state {
            CONNECTING | FINDING_ROUTE | CONNECTED => {}
            CLOSED_BY_PEER | PROBLEM => return (NO_CONNECTION, 0),
            _ => return (INVALID_STATE, 0),
        }
        if c.local != 0 {
            if c.state != CONNECTED {
                return (OK, 0);
            }
            c.sent += 1;
            let (to, n) = (c.local, c.sent);
            self.deliver(to, data.to_vec());
            return (OK, n);
        }
        let Some(p) = c.proto.as_mut() else { return (INVALID_STATE, 0) };
        match p.send(data, reliable(flags)) {
            Ok(n) => {
                let now = Instant::now();
                self.on_proto(h, |p, io| p.poll(now, io));
                (OK, n)
            }
            Err(8) => (LIMIT_EXCEEDED, 0),
            Err(e) => (e, 0),
        }
    }

    fn deliver(&mut self, h: u32, data: Vec<u8>) {
        self.arrival += 1;
        let arrival = self.arrival;
        let Some(c) = self.conns.get_mut(&h) else { return };
        c.received += 1;
        if c.inbox.len() < 65536 {
            c.inbox.push_back(Received { conn: h, peer: c.peer, data, number: c.received, channel: 0, user_data: c.user_data, arrival });
        }
    }

    pub fn receive(&mut self, h: u32, max: usize) -> Option<Vec<Received>> {
        let c = self.conns.get_mut(&h)?;
        let n = c.inbox.len().min(max);
        Some(c.inbox.drain(..n).collect())
    }

    pub fn receive_group(&mut self, g: u32, max: usize) -> Option<Vec<Received>> {
        self.groups.get(&g)?;
        let mut out = Vec::new();
        while out.len() < max {
            let next = self.conns.iter().filter(|(_, c)| c.poll_group == g).filter_map(|(h, c)| c.inbox.front().map(|m| (m.arrival, *h))).min();
            let Some((_, h)) = next else { break };
            out.extend(self.conns.get_mut(&h).and_then(|c| c.inbox.pop_front()));
        }
        Some(out)
    }

    pub fn new_group(&mut self) -> u32 {
        let h = self.handle();
        self.groups.insert(h, ());
        h
    }

    pub fn drop_group(&mut self, g: u32) -> bool {
        for c in self.conns.values_mut().filter(|c| c.poll_group == g) {
            c.poll_group = 0;
        }
        self.groups.remove(&g).is_some()
    }

    pub fn set_group(&mut self, h: u32, g: u32) -> bool {
        if g != 0 && !self.groups.contains_key(&g) {
            return false;
        }
        self.conns.get_mut(&h).map(|c| c.poll_group = g).is_some()
    }

    // ----------------------------------------------------------- messages

    pub fn send_to_user(&mut self, peer: u64, data: &[u8], flags: i32, channel: i32, cert: Option<Arc<LocalCert>>) -> i32 {
        if peer == 0 {
            return INVALID_PARAM;
        }
        if data.len() > MAX_MESSAGE {
            return LIMIT_EXCEEDED;
        }
        if peer == self.me {
            // Messages to ourselves stay in-process.
            self.arrival += 1;
            let arrival = self.arrival;
            let s = self.sessions.entry(peer).or_default();
            s.accepted = true;
            s.received += 1;
            let m = Received { conn: 0, peer, data: data.to_vec(), number: s.received, channel, user_data: 0, arrival };
            s.inbox.entry(channel).or_default().push_back(m);
            return OK;
        }
        let now = Instant::now();
        let me = self.me;
        let s = self.sessions.entry(peer).or_default();
        let alive = s.proto.as_ref().is_some_and(|p| p.state.active());
        if !alive {
            if s.failed && flags & SEND_AUTO_RESTART == 0 {
                return NO_CONNECTION;
            }
            let Some(cert) = cert else { return NO_CONNECTION };
            s.failed = false;
            s.proto = Some(Box::new(Proto::connect(me, peer, VPORT_MESSAGES, VPORT_MESSAGES, cert, now)));
        }
        s.accepted = true;
        let p = s.proto.as_mut().unwrap();
        if !p.client && p.state == PState::Connecting {
            // They asked first and we answer by sending: that accepts.
            let _ = p.accept(now);
        }
        // Before the connection is up, Steam sends everything reliably.
        let rel = reliable(flags) || p.state != PState::Connected;
        let r = p.send(&messages_frame(channel, data), rel);
        self.on_session(peer, |p, io| p.poll(now, io));
        match r {
            Ok(_) => OK,
            Err(e) => e,
        }
    }

    pub fn accept_session(&mut self, peer: u64) -> bool {
        let Some(s) = self.sessions.get_mut(&peer) else { return false };
        s.accepted = true;
        let now = Instant::now();
        let pending = s.proto.as_ref().is_some_and(|p| !p.client && p.state == PState::Connecting);
        if pending {
            self.on_session(peer, |p, io| {
                if let Err(e) = p.accept(now) {
                    crate::log!("sns: accept session failed: {e}");
                }
                p.poll(now, io);
            });
        }
        true
    }

    pub fn close_session(&mut self, peer: u64) -> bool {
        let Some(mut s) = self.sessions.remove(&peer) else { return false };
        if let (Some(p), Some(e)) = (s.proto.as_mut(), self.engine.as_ref()) {
            p.close(END_APP as u32, "session closed", &mut e.io(&mut self.signals));
        }
        true
    }

    pub fn close_channel(&mut self, peer: u64, channel: i32) -> bool {
        self.sessions.get_mut(&peer).is_some_and(|s| s.inbox.remove(&channel).is_some())
    }

    pub fn receive_channel(&mut self, channel: i32, max: usize) -> Vec<Received> {
        let mut all: Vec<Received> = Vec::new();
        for s in self.sessions.values_mut().filter(|s| s.accepted) {
            if let Some(q) = s.inbox.get_mut(&channel) {
                all.extend(q.drain(..));
            }
        }
        all.sort_by_key(|m| m.arrival);
        if all.len() > max {
            // Put the rest back, oldest first.
            for m in all.split_off(max).into_iter().rev() {
                if let Some(s) = self.sessions.get_mut(&m.peer) {
                    s.inbox.entry(channel).or_default().push_front(m);
                }
            }
        }
        all
    }

    /// ESteamNetworkingConnectionState of a messages session.
    pub fn session_state(&self, peer: u64) -> i32 {
        match self.sessions.get(&peer) {
            None => NONE,
            Some(s) if s.failed => PROBLEM,
            Some(_) if peer == self.me => CONNECTED,
            Some(s) => s.proto.as_ref().map(|p| app_state(p.state)).unwrap_or(NONE),
        }
    }

    // ------------------------------------------------------------ network

    /// A rendezvous signal from another player.
    pub fn on_signal(&mut self, from: u64, m: &Rendezvous, cert: Option<Arc<LocalCert>>) {
        let now = Instant::now();
        let id_to = m.to_connection_id();
        let id_from = m.from_connection_id();
        // A connection we know, by our id or theirs.
        let find = |p: &Proto| p.peer == from && ((id_to != 0 && p.local_id == id_to) || (id_to == 0 && id_from != 0 && p.remote_id == id_from));
        if let Some(h) = self.conns.iter().find(|(_, c)| c.proto.as_deref().is_some_and(find)).map(|(h, _)| *h) {
            self.on_proto(h, |p, io| p.on_signal(m, now, io));
            return;
        }
        if self.sessions.get(&from).and_then(|s| s.proto.as_deref()).is_some_and(find) {
            self.on_session(from, |p, io| p.on_signal(m, now, io));
            return;
        }
        if id_to != 0 || m.connect_request.is_none() {
            crate::log!("sns: signal from {from} for unknown connection #{id_to} (theirs #{id_from}), ignored");
            return;
        }
        let Some(cert) = cert else {
            crate::log!("sns: connect request from {from} before our cert is ready; they'll retry");
            return;
        };
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let vport = m.connect_request.as_ref().and_then(|r| r.to_virtual_port).map(|v| v as i32).unwrap_or(-1);
        if vport == VPORT_MESSAGES {
            let s = self.sessions.entry(from).or_default();
            if let Some(p) = s.proto.as_mut().filter(|p| p.state.active()) {
                // Both sides started a session at once: one becomes the server.
                if p.client && p.state == PState::Connecting && p.symmetric_should_accept(m) {
                    crate::log!("sns: symmetric messages session with {from}: we accept theirs");
                    if let Err((r, e)) = p.become_server(m, now, unix) {
                        crate::log!("sns: couldn't switch roles: {r} {e}");
                    }
                    self.on_session(from, |p, io| {
                        p.on_signal(m, now, io);
                        p.poll(now, io);
                    });
                }
                return;
            }
            match Proto::incoming(self.me, m, cert, now, unix) {
                Ok(p) => {
                    crate::log!("sns: messages session request from {from}");
                    s.failed = false;
                    s.asked = false;
                    let accepted = s.accepted;
                    s.proto = Some(Box::new(p));
                    self.on_session(from, |p, io| p.on_signal(m, now, io));
                    if accepted {
                        self.accept_session(from);
                    }
                }
                Err((r, e)) => crate::log!("sns: bad messages request from {from}: {r} {e}"),
            }
            return;
        }
        let Some((&lh, l)) = self.listens.iter().find(|(_, l)| l.vport == vport || l.vport == -1) else {
            crate::log!("sns: connect request from {from} to port {vport}, but we don't listen there");
            return;
        };
        let lcb = l.callback;
        match Proto::incoming(self.me, m, cert, now, unix) {
            Ok(p) => {
                let h = self.handle();
                self.conns.insert(h, Conn { peer: from, vport, listen: lh, callback: lcb, proto: Some(Box::new(p)), ..Default::default() });
                self.set_state(h, CONNECTING, 0, "");
                self.on_proto(h, |p, io| p.on_signal(m, now, io));
            }
            Err((r, e)) => crate::log!("sns: bad connect request from {from}: {r} {e}"),
        }
    }

    /// A UDP datagram on our ICE socket.
    pub fn on_udp(&mut self, d: &[u8], from: SocketAddr) {
        let now = Instant::now();
        if let Some(m) = crate::gns::ice::Stun::parse(d) {
            if self.engine.as_mut().is_some_and(|e| e.on_stun(&m)) {
                return;
            }
            if let Some(h) = self.conns.iter().find(|(_, c)| c.proto.as_ref().is_some_and(|p| p.ice.owns(&m))).map(|(h, _)| *h) {
                self.on_proto(h, |p, io| p.on_stun(d, &m, from, io));
            } else if let Some(peer) = self.sessions.iter().find(|(_, s)| s.proto.as_ref().is_some_and(|p| p.ice.owns(&m))).map(|(k, _)| *k) {
                self.on_session(peer, |p, io| p.on_stun(d, &m, from, io));
            }
            return;
        }
        if d.len() < 5 {
            return;
        }
        // Data packets name our connection; closed / no-connection ones are
        // matched by where they came from.
        let to = if d[0] & 0x80 != 0 { u32::from_le_bytes(d[1..5].try_into().unwrap()) } else { 0 };
        let target = |p: &Proto| if to != 0 { p.local_id == to } else { p.remote_addr() == Some(from) };
        if let Some(h) = self.conns.iter().find(|(_, c)| c.proto.as_deref().is_some_and(target)).map(|(h, _)| *h) {
            self.on_proto(h, |p, io| p.on_packet(d, from, now, io));
        } else if let Some(peer) = self.sessions.iter().find(|(_, s)| s.proto.as_deref().is_some_and(target)).map(|(k, _)| *k) {
            self.on_session(peer, |p, io| p.on_packet(d, from, now, io));
        }
    }

    /// Timers for every connection.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if let Some(e) = self.engine.as_mut() {
            e.poll_stun(now);
        }
        let hs: Vec<u32> = self.conns.iter().filter(|(_, c)| c.proto.is_some()).map(|(h, _)| *h).collect();
        for h in hs {
            self.on_proto(h, |p, io| p.poll(now, io));
        }
        let peers: Vec<u64> = self.sessions.iter().filter(|(_, s)| s.proto.is_some()).map(|(k, _)| *k).collect();
        for peer in peers {
            self.on_session(peer, |p, io| p.poll(now, io));
        }
    }

    /// Set a config callback by ESteamNetworkingConfigValue and scope.
    pub fn set_callback(&mut self, value: i32, scope: i32, object: usize, f: usize) -> bool {
        match (value, scope) {
            (CFG_CONN_STATUS, SCOPE_CONNECTION) => self.conns.get_mut(&(object as u32)).map(|c| c.callback = f).is_some(),
            (CFG_CONN_STATUS, SCOPE_LISTEN) => self.listens.get_mut(&(object as u32)).map(|l| l.callback = f).is_some(),
            (CFG_CONN_STATUS, _) => {
                self.callbacks.conn_status = f;
                true
            }
            (CFG_AUTH_STATUS, _) => {
                self.callbacks.auth_status = f;
                true
            }
            (CFG_RELAY_STATUS, _) => {
                self.callbacks.relay_status = f;
                true
            }
            (CFG_SESSION_REQUEST, _) => {
                self.callbacks.session_request = f;
                true
            }
            (CFG_SESSION_FAILED, _) => {
                self.callbacks.session_failed = f;
                true
            }
            _ => false,
        }
    }

    fn ping(&self, h: u32) -> i32 {
        self.conns.get(&h).and_then(|c| c.proto.as_ref()).map(|p| p.ping_ms()).unwrap_or(0)
    }

    fn pending(&self, h: u32) -> usize {
        self.conns.get(&h).and_then(|c| c.proto.as_ref()).map(|p| p.snp.pending_bytes).unwrap_or(0)
    }
}

trait PutBytes {
    fn put_bytes(&mut self, at: usize, b: &[u8]) -> &mut Self;
}

impl PutBytes for Bytes {
    fn put_bytes(&mut self, at: usize, b: &[u8]) -> &mut Self {
        if let Some(dst) = self.0.get_mut(at..at + b.len()) {
            dst.copy_from_slice(b);
        }
        self
    }
}

// ------------------------------------------------------- the game's side

static NET: LazyLock<Mutex<Net>> = LazyLock::new(|| {
    let (me, app) = {
        let s = crate::state::lock();
        (s.me(), s.app_id())
    };
    let n = Net::new(me, true);
    if me != 0 {
        engine::fetch_cert(me, app);
    }
    let sock = n.engine.as_ref().and_then(|e| e.sock.clone());
    std::thread::spawn(move || network_loop(sock));
    Mutex::new(n)
});

/// Receives on the ICE socket and runs every connection's timers.
fn network_loop(sock: Option<Arc<std::net::UdpSocket>>) {
    if let Some(s) = &sock {
        let _ = s.set_read_timeout(Some(Duration::from_millis(10)));
    }
    let mut buf = vec![0u8; 2048];
    let mut last_tick = Instant::now();
    loop {
        let got = match &sock {
            Some(s) => s.recv_from(&mut buf).ok(),
            None => {
                std::thread::sleep(Duration::from_millis(20));
                None
            }
        };
        let due = last_tick.elapsed() >= Duration::from_millis(10);
        if got.is_none() && !due {
            continue;
        }
        let mut n = net();
        if let Some((len, from)) = got {
            n.on_udp(&buf[..len], from);
        }
        if due {
            n.tick();
            last_tick = Instant::now();
        }
        finish(&mut n);
    }
}

/// Config callbacks to call on the game's thread: (function, bytes).
static PENDING: Mutex<Vec<(usize, Vec<u8>)>> = Mutex::new(Vec::new());

fn net() -> MutexGuard<'static, Net> {
    let mut n = NET.lock().unwrap_or_else(|e| e.into_inner());
    if n.me == 0 {
        n.me = crate::state::lock().me();
    }
    n
}

/// Hand events to the game (config callbacks wait for RunCallbacks, the
/// rest go into Steam's callback queue) and send collected signals.
fn finish(n: &mut Net) {
    engine::send_signals(std::mem::take(&mut n.signals));
    if n.events.is_empty() {
        return;
    }
    let events = std::mem::take(&mut n.events);
    let mut g = crate::state::lock();
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    for (id, data, f) in events {
        if f != 0 { pending.push((f, data)) } else { g.post(id, data) }
    }
}

/// Run `f` on the network state, then publish its events and signals.
fn with<R>(f: impl FnOnce(&mut Net) -> R) -> R {
    let mut n = net();
    let r = f(&mut n);
    finish(&mut n);
    r
}

/// Our certificate for a new connection (waits briefly for Steam the first time).
fn my_cert() -> Arc<LocalCert> {
    if let Some(c) = engine::cert_ready() {
        return c;
    }
    let (me, app) = {
        let s = crate::state::lock();
        (s.me(), s.app_id())
    };
    engine::cert(me, app, Duration::from_secs(5))
}

/// A SteamNetworkingSockets rendezvous from another player (through Steam).
pub fn rendezvous(from: u64, data: &[u8]) {
    let Some(m) = crate::gns::msg::decode::<Rendezvous>(data) else {
        crate::log!("sns: unreadable rendezvous from {from}");
        return;
    };
    let cert = if m.connect_request.is_some() && m.to_connection_id.is_none() { Some(my_cert()) } else { None };
    with(|n| n.on_signal(from, &m, cert));
}

thread_local! {
    static IN_CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Call the game's config callbacks (from SteamAPI_RunCallbacks).
pub fn run_callbacks() {
    if IN_CALLBACK.with(|c| c.replace(true)) {
        return;
    }
    loop {
        let batch = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
        if batch.is_empty() {
            break;
        }
        for (f, mut data) in batch {
            let f: extern "C" fn(*mut c_void) = unsafe { std::mem::transmute(f) };
            f(data.as_mut_ptr() as *mut c_void);
        }
    }
    IN_CALLBACK.with(|c| c.set(false));
}

// -------------------------------------------- SteamNetworkingMessage_t

/// SteamNetworkingMessage_t, laid out like the SDK header for both bitnesses.
#[repr(C)]
pub struct Message {
    data: *mut u8,
    size: i32,
    conn: u32,
    peer: [u8; IDENTITY],
    #[cfg(target_pointer_width = "32")]
    _pad0: u32,
    conn_user_data: i64,
    time: i64,
    number: i64,
    free_data: Option<extern "C" fn(*mut Message)>,
    release: extern "C" fn(*mut Message),
    channel: i32,
    flags: i32,
    user_data: i64,
    lane: u16,
    _pad1: u16,
}

extern "C" fn free_buffer(m: *mut Message) {
    let m = unsafe { &mut *m };
    if !m.data.is_null() {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(m.data, m.size.max(0) as usize)) });
        m.data = std::ptr::null_mut();
    }
}

extern "C" fn release(m: *mut Message) {
    if m.is_null() {
        return;
    }
    let msg = unsafe { &mut *m };
    if let Some(f) = msg.free_data {
        f(m);
    }
    drop(unsafe { Box::from_raw(m) });
}

fn now_us() -> i64 {
    crate::state::lock().started.elapsed().as_micros() as i64 + 1
}

/// A message object the game owns until it calls m_pfnRelease.
fn new_message(data: Vec<u8>) -> *mut Message {
    let size = data.len() as i32;
    let ptr = Box::into_raw(data.into_boxed_slice()) as *mut u8;
    Box::into_raw(Box::new(Message {
        data: ptr,
        size,
        conn: 0,
        peer: [0; IDENTITY],
        #[cfg(target_pointer_width = "32")]
        _pad0: 0,
        conn_user_data: 0,
        time: 0,
        number: 0,
        free_data: Some(free_buffer),
        release,
        channel: 0,
        flags: 0,
        user_data: 0,
        lane: 0,
        _pad1: 0,
    }))
}

fn hand_out(msgs: Vec<Received>, out: *mut *mut Message) -> i32 {
    let time = now_us();
    let n = msgs.len();
    for (i, r) in msgs.into_iter().enumerate() {
        let m = new_message(r.data);
        unsafe {
            (*m).conn = r.conn;
            (*m).peer.copy_from_slice(&identity(r.peer));
            (*m).conn_user_data = r.user_data;
            (*m).time = time;
            (*m).number = r.number;
            (*m).channel = r.channel;
            *out.add(i) = m;
        }
    }
    n as i32
}

// ------------------------------------------------------------ exports

pub fn imp(kind: &str, name: &str) -> Option<Fp> {
    macro_rules! f {
        ($f:expr) => {
            Some($f as Fp)
        };
    }
    match (kind, name) {
        ("ISteamNetworkingSockets", n) => match n {
            "CreateListenSocketIP" => f!(s_listen_ip),
            "ConnectByIPAddress" => f!(s_connect_ip),
            "CreateListenSocketP2P" => f!(s_listen_p2p),
            "ConnectP2P" => f!(s_connect_p2p),
            "AcceptConnection" => f!(s_accept),
            "CloseConnection" => f!(s_close),
            "CloseListenSocket" => f!(s_close_listen),
            "SetConnectionUserData" => f!(s_set_user_data),
            "GetConnectionUserData" => f!(s_user_data),
            "SetConnectionName" => f!(s_set_name),
            "GetConnectionName" => f!(s_name),
            "SendMessageToConnection" => f!(s_send),
            "SendMessages" => f!(s_send_messages),
            "FlushMessagesOnConnection" => f!(s_flush),
            "ReceiveMessagesOnConnection" => f!(s_receive),
            "GetConnectionInfo" => f!(s_info),
            "GetQuickConnectionStatus" | "GetConnectionRealTimeStatus" => f!(s_status),
            "GetDetailedConnectionStatus" => f!(s_detailed),
            "CreateSocketPair" => f!(s_pair),
            "ConfigureConnectionLanes" => f!(s_lanes),
            "GetIdentity" => f!(s_identity),
            "InitAuthentication" => f!(s_init_auth),
            "GetAuthenticationStatus" => f!(s_auth_status),
            "CreatePollGroup" => f!(s_new_group),
            "DestroyPollGroup" => f!(s_drop_group),
            "SetConnectionPollGroup" => f!(s_set_group),
            "ReceiveMessagesOnPollGroup" => f!(s_receive_group),
            "RunCallbacks" => f!(s_run_callbacks),
            _ => None,
        },
        ("ISteamNetworkingMessages", n) => match n {
            "SendMessageToUser" => f!(m_send),
            "ReceiveMessagesOnChannel" => f!(m_receive),
            "AcceptSessionWithUser" => f!(m_accept),
            "CloseSessionWithUser" => f!(m_close),
            "CloseChannelWithUser" => f!(m_close_channel),
            "GetSessionConnectionInfo" => f!(m_info),
            _ => None,
        },
        ("ISteamNetworkingUtils", n) => match n {
            "AllocateMessage" => f!(u_allocate),
            "GetRelayNetworkStatus" => f!(u_relay_status),
            "CheckPingDataUpToDate" => f!(u_check_ping),
            "GetLocalPingLocation" => f!(u_ping_location),
            "EstimatePingTimeBetweenTwoLocations" => f!(u_ping_between),
            "EstimatePingTimeFromLocalHost" => f!(u_ping_from_here),
            "ConvertPingLocationToString" => f!(u_ping_to_string),
            "ParsePingLocationString" => f!(u_parse_ping),
            "GetPingToDataCenter" | "GetDirectPingToPOP" => f!(minus_one),
            "SetConfigValue" => f!(u_set_config),
            _ => None,
        },
        _ => None,
    }
}

extern "C" fn minus_one() -> i32 {
    -1
}

/// Config callbacks among a connect's or listen's options
/// (SteamNetworkingConfigValue_t: value, data type, 8-byte union).
fn option_callback(n: i32, options: *const u8) -> usize {
    if options.is_null() || n <= 0 {
        return 0;
    }
    let d = slice(options, n as usize * 16);
    d.chunks(16).find(|o| i32_at(o, 0) == Some(CFG_CONN_STATUS) && i32_at(o, 4) == Some(TYPE_PTR)).map(|o| usize::from_le_bytes(o[8..8 + size_of::<usize>()].try_into().unwrap())).unwrap_or(0)
}

extern "C" fn s_listen_ip(_t: This, _addr: *const u8, n: i32, options: *const u8) -> u32 {
    // Nobody can reach it (that needs Valve's UDP protocol), but a host
    // keeps working over P2P with a valid handle.
    crate::log!("sns: listen by IP (only P2P connections arrive)");
    with(|net| net.listen(-1, option_callback(n, options)))
}

extern "C" fn s_connect_ip(_t: This, _addr: *const u8, n: i32, options: *const u8) -> u32 {
    crate::log!("sns: connect by IP isn't possible");
    with(|net| net.connect_unsupported(option_callback(n, options), "connecting by IP address needs Valve's UDP connect protocol; SteamInRust connects players over P2P only"))
}

extern "C" fn s_listen_p2p(_t: This, vport: i32, n: i32, options: *const u8) -> u32 {
    crate::log!("sns: listen on virtual port {vport}");
    with(|net| net.listen(vport, option_callback(n, options)))
}

extern "C" fn s_connect_p2p(_t: This, who: *const u8, vport: i32, n: i32, options: *const u8) -> u32 {
    let peer = steam_id_of(who);
    crate::log!("sns: connect to {peer} port {vport}");
    if peer == 0 {
        return with(|net| net.connect_unsupported(option_callback(n, options), "only Steam ID identities are supported"));
    }
    let cert = my_cert();
    with(|net| net.connect(peer, vport, option_callback(n, options), Some(cert)))
}

extern "C" fn s_accept(_t: This, h: u32) -> i32 {
    with(|n| n.accept(h))
}

extern "C" fn s_close(_t: This, h: u32, reason: i32, debug: *const c_char, _linger: bool) -> bool {
    let debug = unsafe { crate::ifaces::text(debug) };
    with(|n| n.close(h, reason, &debug))
}

extern "C" fn s_close_listen(_t: This, h: u32) -> bool {
    with(|n| n.close_listen(h))
}

extern "C" fn s_set_user_data(_t: This, h: u32, v: i64) -> bool {
    net().conns.get_mut(&h).map(|c| c.user_data = v).is_some()
}

extern "C" fn s_user_data(_t: This, h: u32) -> i64 {
    net().conns.get(&h).map(|c| c.user_data).unwrap_or(-1)
}

extern "C" fn s_set_name(_t: This, h: u32, name: *const c_char) {
    let name = unsafe { crate::ifaces::text(name) };
    if let Some(c) = net().conns.get_mut(&h) {
        c.name = name;
    }
}

extern "C" fn s_name(_t: This, h: u32, buf: *mut c_char, len: i32) -> bool {
    let n = net();
    let Some(c) = n.conns.get(&h) else { return false };
    copy_out(&c.name, buf, len.max(0) as usize);
    true
}

extern "C" fn s_send(_t: This, h: u32, data: *const u8, len: u32, flags: i32, number: *mut i64) -> i32 {
    let data = slice(data, len as usize).to_vec();
    let (r, num) = with(|n| n.send(h, &data, flags));
    put(number, num);
    r
}

extern "C" fn s_send_messages(_t: This, count: i32, msgs: *const *mut Message, results: *mut i64) {
    for i in 0..count.max(0) as usize {
        let m = unsafe { *msgs.add(i) };
        if m.is_null() {
            continue;
        }
        let (conn, data, flags) = unsafe { ((*m).conn, slice((*m).data, (*m).size.max(0) as usize).to_vec(), (*m).flags) };
        let (r, num) = with(|n| n.send(conn, &data, flags));
        if !results.is_null() {
            unsafe { *results.add(i) = if r == OK { num } else { -(r as i64) } };
        }
        // Sending takes ownership of the message.
        unsafe { ((*m).release)(m) };
    }
}

extern "C" fn s_flush(_t: This, h: u32) -> i32 {
    if net().conns.contains_key(&h) { OK } else { INVALID_PARAM }
}

extern "C" fn s_receive(_t: This, h: u32, out: *mut *mut Message, max: i32) -> i32 {
    if out.is_null() || max <= 0 {
        return 0;
    }
    match net().receive(h, max as usize) {
        Some(msgs) => hand_out(msgs, out),
        None => -1,
    }
}

extern "C" fn s_info(_t: This, h: u32, out: *mut u8) -> bool {
    let n = net();
    if !n.conns.contains_key(&h) {
        return false;
    }
    let info = n.info(h);
    if !out.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(info.as_ptr(), out, INFO) };
    }
    true
}

/// SteamNetConnectionRealTimeStatus_t (and the older QuickConnectionStatus,
/// same layout).
fn realtime(state: i32, ping: i32, pending: usize) -> Vec<u8> {
    let mut b = Bytes::new(REALTIME_STATUS);
    b.i32(0, state).i32(4, ping);
    let one = 1.0f32.to_le_bytes();
    b.put_bytes(8, &one).put_bytes(12, &one);
    b.i32(32, 1_000_000).i32(40, pending as i32);
    b.0
}

extern "C" fn s_status(_t: This, h: u32, out: *mut u8) -> i32 {
    let n = net();
    // 0 for a bad handle: false for GetQuickConnectionStatus, not OK for
    // GetConnectionRealTimeStatus.
    let Some(c) = n.conns.get(&h) else { return 0 };
    let status = realtime(c.state, n.ping(h), n.pending(h));
    if !out.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(status.as_ptr(), out, REALTIME_STATUS) };
    }
    // GetQuickConnectionStatus returns bool, GetConnectionRealTimeStatus
    // EResult: 1 means success for both.
    OK
}

extern "C" fn s_detailed(_t: This, h: u32, buf: *mut c_char, len: i32) -> i32 {
    let n = net();
    let Some(c) = n.conns.get(&h) else { return -1 };
    let route = c.proto.as_ref().and_then(|p| p.remote_addr()).map(|a| a.to_string()).unwrap_or_else(|| "none".into());
    let text = format!("SteamInRust P2P to {} (state {}, ping {} ms, route {route})
", c.peer, c.state, n.ping(h));
    if buf.is_null() || (len as usize) < text.len() + 1 {
        return text.len() as i32 + 1;
    }
    copy_out(&text, buf, len as usize);
    0
}

extern "C" fn s_pair(_t: This, a: *mut u32, b: *mut u32, _loopback: bool, _id1: *const u8, _id2: *const u8) -> bool {
    let (x, y) = net().pair();
    put(a, x);
    put(b, y);
    true
}

extern "C" fn s_lanes(_t: This, h: u32) -> i32 {
    if net().conns.contains_key(&h) { OK } else { INVALID_PARAM }
}

extern "C" fn s_identity(_t: This, out: *mut u8) -> bool {
    let me = net().me;
    if !out.is_null() && me != 0 {
        unsafe { std::ptr::copy_nonoverlapping(identity(me).as_ptr(), out, IDENTITY) };
    }
    me != 0
}

fn auth_status() -> Vec<u8> {
    let mut b = Bytes::new(260);
    b.i32(0, AVAILABLE).str(4, 256, "OK");
    b.0
}

extern "C" fn s_init_auth(_t: This) -> i32 {
    let f = net().callbacks.auth_status;
    if f != 0 {
        PENDING.lock().unwrap_or_else(|e| e.into_inner()).push((f, auth_status()));
    } else {
        crate::state::lock().post(CB_AUTH_STATUS, auth_status());
    }
    AVAILABLE
}

extern "C" fn s_auth_status(_t: This, out: *mut u8) -> i32 {
    if !out.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(auth_status().as_ptr(), out, 260) };
    }
    AVAILABLE
}

extern "C" fn s_new_group(_t: This) -> u32 {
    net().new_group()
}

extern "C" fn s_drop_group(_t: This, g: u32) -> bool {
    net().drop_group(g)
}

extern "C" fn s_set_group(_t: This, h: u32, g: u32) -> bool {
    net().set_group(h, g)
}

extern "C" fn s_receive_group(_t: This, g: u32, out: *mut *mut Message, max: i32) -> i32 {
    if out.is_null() || max <= 0 {
        return 0;
    }
    match net().receive_group(g, max as usize) {
        Some(msgs) => hand_out(msgs, out),
        None => -1,
    }
}

extern "C" fn s_run_callbacks(_t: This) {
    run_callbacks();
}

extern "C" fn m_send(_t: This, who: *const u8, data: *const u8, len: u32, flags: i32, channel: i32) -> i32 {
    let peer = steam_id_of(who);
    let data = slice(data, len as usize).to_vec();
    let cert = (peer != 0).then(my_cert);
    with(|n| n.send_to_user(peer, &data, flags, channel, cert))
}

extern "C" fn m_receive(_t: This, channel: i32, out: *mut *mut Message, max: i32) -> i32 {
    if out.is_null() || max <= 0 {
        return 0;
    }
    let msgs = net().receive_channel(channel, max as usize);
    hand_out(msgs, out)
}

extern "C" fn m_accept(_t: This, who: *const u8) -> bool {
    net().accept_session(steam_id_of(who))
}

extern "C" fn m_close(_t: This, who: *const u8) -> bool {
    net().close_session(steam_id_of(who))
}

extern "C" fn m_close_channel(_t: This, who: *const u8, channel: i32) -> bool {
    net().close_channel(steam_id_of(who), channel)
}

extern "C" fn m_info(_t: This, who: *const u8, info: *mut u8, status: *mut u8) -> i32 {
    let peer = steam_id_of(who);
    let n = net();
    let state = n.session_state(peer);
    let p = n.sessions.get(&peer).and_then(|s| s.proto.as_deref());
    if !info.is_null() {
        let (r, d) = p.map(|p| (p.end_reason as i32, p.end_debug.clone())).unwrap_or_default();
        let b = session_info(peer, state, r, &d, p);
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), info, INFO) };
    }
    if !status.is_null() {
        let ping = p.map(|p| p.ping_ms()).unwrap_or(-1);
        let pending = p.map(|p| p.snp.pending_bytes).unwrap_or(0);
        unsafe { std::ptr::copy_nonoverlapping(realtime(state, ping, pending).as_ptr(), status, REALTIME_STATUS) };
    }
    state
}

extern "C" fn u_allocate(_t: This, size: i32) -> *mut Message {
    let m = new_message(vec![0; size.max(0) as usize]);
    if size <= 0 {
        // The game sets its own buffer (and free function, if any).
        unsafe {
            free_buffer(m);
            (*m).free_data = None;
            (*m).size = 0;
        }
    }
    m
}

/// SteamRelayNetworkStatus_t, all "current": our transport needs no relays.
fn relay_status() -> Vec<u8> {
    let mut b = Bytes::new(272);
    b.i32(0, AVAILABLE).i32(8, AVAILABLE).i32(12, AVAILABLE);
    b.0
}

extern "C" fn u_relay_status(_t: This, out: *mut u8) -> i32 {
    if !out.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(relay_status().as_ptr(), out, 272) };
    }
    AVAILABLE
}

static RELAY_ANNOUNCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// CheckPingDataUpToDate (InitRelayNetworkAccess calls it): announce the
/// relay status once, as Steam does when it becomes ready.
extern "C" fn u_check_ping(_t: This, _max_age: f32) -> bool {
    if !RELAY_ANNOUNCED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let f = net().callbacks.relay_status;
        if f != 0 {
            PENDING.lock().unwrap_or_else(|e| e.into_inner()).push((f, relay_status()));
        } else {
            crate::state::lock().post(CB_RELAY_STATUS, relay_status());
        }
    }
    true
}

/// SteamNetworkPingLocation_t is opaque (512 bytes); we have no relay pings.
extern "C" fn u_ping_location(_t: This, out: *mut u8) -> f32 {
    if !out.is_null() {
        unsafe { std::ptr::write_bytes(out, 0, 512) };
    }
    1.0
}

/// No relay data to estimate with: a typical direct-link value, so games
/// that filter lobbies by ping don't hide every one.
extern "C" fn u_ping_between(_t: This, _a: *const u8, _b: *const u8) -> i32 {
    60
}

extern "C" fn u_ping_from_here(_t: This, _a: *const u8) -> i32 {
    60
}

extern "C" fn u_ping_to_string(_t: This, _loc: *const u8, buf: *mut c_char, len: i32) {
    copy_out("sir", buf, len.max(0) as usize);
}

extern "C" fn u_parse_ping(_t: This, _s: *const c_char, out: *mut u8) -> bool {
    if !out.is_null() {
        unsafe { std::ptr::write_bytes(out, 0, 512) };
    }
    true
}

/// Config values: callbacks are kept, everything else is accepted as is.
extern "C" fn u_set_config(_t: This, value: i32, scope: i32, object: isize, kind: i32, arg: *const usize) -> bool {
    if (CFG_CONN_STATUS..=CFG_SESSION_FAILED).contains(&value) && kind == TYPE_PTR {
        let f = if arg.is_null() { 0 } else { unsafe { arg.read_unaligned() } };
        return net().set_callback(value, scope, object as usize, f);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(n: &mut Net) -> Vec<(u32, i32, i32)> {
        n.events
            .drain(..)
            .filter(|e| e.0 == CB_CONN_STATUS)
            .map(|(_, d, _)| (u32_at(&d, 0).unwrap(), i32_at(&d, 8 + 176).unwrap(), i32_at(&d, 704).unwrap()))
            .collect()
    }

    #[test]
    fn connect_to_ourselves_and_unsupported() {
        let me = 76561198000000001;
        let mut n = Net::new(me, false);
        let ls = n.listen(7, 0);
        let c = n.connect(me, 7, 0, None);
        let incoming = states(&mut n);
        assert_eq!(incoming.len(), 2);
        let theirs = incoming[1].0;
        assert_eq!(n.conns[&theirs].listen, ls);
        assert_eq!(n.accept(theirs), OK);
        assert_eq!(states(&mut n).len(), 2);
        assert_eq!(n.send(c, b"hi", SEND_RELIABLE).0, OK);
        assert_eq!(n.receive(theirs, 4).unwrap()[0].data, b"hi");
        n.close(c, 0, "");
        assert_eq!(states(&mut n), vec![(theirs, CLOSED_BY_PEER, CONNECTED)]);
        let h = n.connect_unsupported(0, "no");
        assert_eq!(states(&mut n), vec![(h, CONNECTING, NONE), (h, PROBLEM, CONNECTING)]);
        // Without a network, remote connections fail cleanly.
        let r = n.connect(5, 1, 0, Some(Arc::new(LocalCert::unsigned(LocalCert::new_key(), me, 480))));
        assert_eq!(states(&mut n).last().unwrap().1, PROBLEM);
        assert!(n.conns.contains_key(&r));
    }

    #[test]
    fn socket_pair_and_layouts() {
        let mut n = Net::new(9, false);
        let (x, y) = n.pair();
        assert_eq!(n.send(x, b"loop", 0).0, OK);
        assert_eq!(n.receive(y, 4).unwrap()[0].data, b"loop");
        n.close(x, 0, "");
        assert_eq!(states(&mut n), vec![(y, CLOSED_BY_PEER, CONNECTED)]);

        // SteamNetworkingMessage_t offsets from the SDK header.
        let m = new_message(b"abc".to_vec());
        let base = m as usize;
        let at = |p: usize| p - base;
        unsafe {
            if cfg!(target_pointer_width = "64") {
                assert_eq!(at(&raw const (*m).peer as usize), 16);
                assert_eq!(at(&raw const (*m).conn_user_data as usize), 152);
                assert_eq!(at(&raw const (*m).release as usize), 184);
                assert_eq!(at(&raw const (*m).lane as usize), 208);
            } else {
                assert_eq!(at(&raw const (*m).peer as usize), 12);
                assert_eq!(at(&raw const (*m).conn_user_data as usize), 152);
                assert_eq!(at(&raw const (*m).release as usize), 180);
                assert_eq!(at(&raw const (*m).lane as usize), 200);
            }
            ((*m).release)(m);
        }
        assert_eq!(steam_id_of(identity(76561197960287930).as_ptr()), 76561197960287930);
        // Remote address lands in SteamNetConnectionInfo_t.m_addrRemote.
        let mut b = Bytes::new(INFO);
        put_addr(&mut b, Some("1.2.3.4:27015".parse().unwrap()));
        assert_eq!(&b.0[158..166], &[0xff, 0xff, 1, 2, 3, 4, 0x87, 0x69]);
    }
}

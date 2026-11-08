//! ISteamNetworkingSockets, ISteamNetworkingMessages and the parts of
//! ISteamNetworkingUtils games wait on, between players who both run
//! SteamInRust.
//!
//! Valve's client carries these over the Steam Datagram Relay with
//! Valve-signed certificates; that relay protocol is closed, so players on
//! the official client can't be reached. Here connections ride on the same
//! direct UDP endpoint as ISteamNetworking ([`crate::p2p`]), on two reserved
//! channels:
//!
//! - sockets: CONNECT (their handle, virtual port) → ACCEPT or CLOSE, then
//!   DATA addressed to the receiver's connection handle;
//! - messages: DATA (channel) per user, sessions accepted like Steam does.
//!
//! Connecting to yourself (a host joining its own game) and socket pairs
//! stay in-process. Connections by IP address (dedicated servers) would need
//! Valve's encrypted UDP protocol and fail cleanly instead.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::ifaces::{This, copy_out, put, slice};
use crate::state::Bytes;
use crate::vt::Fp;
use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_void};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Endpoint channels (far from the small numbers games use with ISteamNetworking).
pub const CH_SOCKETS: i32 = 0x5349_4E53;
pub const CH_MESSAGES: i32 = 0x5349_4E4D;

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
const CONNECTED: i32 = 3;
const CLOSED_BY_PEER: i32 = 4;
const PROBLEM: i32 = 5;

// ESteamNetConnectionEnd
const END_APP: i32 = 1000;
const END_MISC_GENERIC: i32 = 5001;
const END_TIMEOUT: i32 = 5003;
const END_REMOTE_TIMEOUT: i32 = 4001;
const END_NAT: i32 = 5009;
const END_NO_CONNECTION: i32 = 5010;

// EResult
const OK: i32 = 1;
const NO_CONNECTION: i32 = 3;
const INVALID_PARAM: i32 = 8;
const INVALID_STATE: i32 = 11;
const LIMIT_EXCEEDED: i32 = 25;

/// ESteamNetworkingAvailability::Current.
const AVAILABLE: i32 = 100;
const SEND_RELIABLE: i32 = 8;
const MAX_MESSAGE: usize = 512 * 1024;
/// An unreliable message that fits one datagram; bigger ones go reliably.
const MAX_UNRELIABLE: usize = 1000;
/// How long an incoming connection waits for its listen socket.
const LISTEN_WAIT: Duration = Duration::from_secs(10);

const W_CONNECT: u8 = 1;
const W_ACCEPT: u8 = 2;
const W_CLOSE: u8 = 3;
const W_DATA: u8 = 4;

// ESteamNetworkingConfigValue callbacks and scopes.
const CFG_CONN_STATUS: i32 = 201;
const CFG_AUTH_STATUS: i32 = 202;
const CFG_RELAY_STATUS: i32 = 203;
const CFG_SESSION_REQUEST: i32 = 204;
const CFG_SESSION_FAILED: i32 = 205;
const SCOPE_LISTEN: i32 = 3;
const SCOPE_CONNECTION: i32 = 4;
const TYPE_PTR: i32 = 5;

/// A datagram for the endpoint: (peer, channel, reliable, bytes).
pub type Out = (u64, i32, bool, Vec<u8>);

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
    /// The other side's handle for this connection (0 until known).
    remote: u32,
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
    outbox: VecDeque<(bool, Vec<u8>)>,
    sent: i64,
    received: i64,
    since: Option<Instant>,
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
    /// Incoming CONNECTs waiting for a listen socket: (peer, their handle, vport, since).
    waiting: Vec<(u64, u32, i32, Instant)>,
    sessions: HashMap<u64, Session>,
    callbacks: Callbacks,
    /// Callbacks for the game: (callback id, bytes, config function or 0).
    pub events: Vec<(i32, Vec<u8>, usize)>,
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

impl Net {
    pub fn new(me: u64) -> Self {
        Net {
            me,
            next: 0x51_0000 + (std::process::id() & 0xFFFF),
            arrival: 0,
            conns: HashMap::new(),
            listens: HashMap::new(),
            groups: HashMap::new(),
            waiting: Vec::new(),
            sessions: HashMap::new(),
            callbacks: Callbacks::default(),
            events: Vec::new(),
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
            let desc = if c.name.is_empty() { format!("#{h} P2P steamid:{} vport {}", c.peer, c.vport) } else { c.name.clone() };
            b.str(312, 128, &desc);
            // Unencrypted: the link is direct UDP, not Valve's encrypted transport.
            b.i32(440, 2);
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
        let mut b = Bytes::new(INFO);
        b.put_bytes(0, &identity(peer)).i32(176, PROBLEM).i32(180, reason).str(184, 128, debug);
        self.events.push((CB_SESSION_FAILED, b.0, self.callbacks.session_failed));
    }

    // ------------------------------------------------------------ sockets

    pub fn listen(&mut self, vport: i32, callback: usize) -> (u32, Vec<Out>) {
        let h = self.handle();
        self.listens.insert(h, Listen { vport, callback });
        // Connections that arrived before the socket existed.
        let mut out = Vec::new();
        let ready: Vec<(u64, u32, i32, Instant)> = self.waiting.iter().filter(|w| w.2 == vport).cloned().collect();
        self.waiting.retain(|w| w.2 != vport);
        for (peer, theirs, vport, _) in ready {
            out.extend(self.incoming(peer, theirs, vport));
        }
        (h, out)
    }

    pub fn connect(&mut self, peer: u64, vport: i32, callback: usize) -> (u32, Vec<Out>) {
        let h = self.handle();
        self.conns.insert(h, Conn { peer, vport, callback, since: Some(Instant::now()), ..Default::default() });
        self.set_state(h, CONNECTING, 0, "");
        let mut m = vec![W_CONNECT];
        m.extend_from_slice(&h.to_le_bytes());
        m.extend_from_slice(&vport.to_le_bytes());
        (h, vec![(peer, CH_SOCKETS, true, m)])
    }

    /// A connection that can't work (by IP address): reports and fails.
    pub fn connect_unsupported(&mut self, callback: usize, why: &str) -> u32 {
        let h = self.handle();
        self.conns.insert(h, Conn { callback, ..Default::default() });
        self.set_state(h, CONNECTING, 0, "");
        self.set_state(h, PROBLEM, END_MISC_GENERIC, why);
        h
    }

    fn incoming(&mut self, peer: u64, theirs: u32, vport: i32) -> Vec<Out> {
        let Some((&lh, l)) = self.listens.iter().find(|(_, l)| l.vport == vport) else {
            self.waiting.push((peer, theirs, vport, Instant::now()));
            return Vec::new();
        };
        let callback = l.callback;
        // The same CONNECT again (their retry): nothing new.
        if self.conns.values().any(|c| c.peer == peer && c.remote == theirs) {
            return Vec::new();
        }
        let h = self.handle();
        self.conns.insert(h, Conn { peer, remote: theirs, vport, listen: lh, callback, since: Some(Instant::now()), ..Default::default() });
        self.set_state(h, CONNECTING, 0, "");
        Vec::new()
    }

    pub fn accept(&mut self, h: u32) -> (i32, Vec<Out>) {
        let Some(c) = self.conns.get(&h) else { return (INVALID_PARAM, Vec::new()) };
        if c.listen == 0 || c.state != CONNECTING {
            return (INVALID_STATE, Vec::new());
        }
        let (peer, remote) = (c.peer, c.remote);
        self.set_state(h, CONNECTED, 0, "");
        let mut m = vec![W_ACCEPT];
        m.extend_from_slice(&remote.to_le_bytes());
        m.extend_from_slice(&h.to_le_bytes());
        let mut out = vec![(peer, CH_SOCKETS, true, m)];
        out.extend(self.flush(h));
        (OK, out)
    }

    pub fn close(&mut self, h: u32, reason: i32, debug: &str) -> (bool, Vec<Out>) {
        let Some(c) = self.conns.remove(&h) else { return (false, Vec::new()) };
        let mut out = Vec::new();
        if c.local != 0 {
            let reason = if reason == 0 { END_APP } else { reason };
            self.set_state(c.local, CLOSED_BY_PEER, reason, debug);
            if let Some(o) = self.conns.get_mut(&c.local) {
                o.local = 0;
            }
        } else if c.remote != 0 && matches!(c.state, CONNECTING | CONNECTED) || c.listen == 0 && c.state == CONNECTING {
            let mut m = vec![W_CLOSE];
            m.extend_from_slice(&c.remote.to_le_bytes());
            m.extend_from_slice(&h.to_le_bytes());
            m.extend_from_slice(&(if reason == 0 { END_APP } else { reason }).to_le_bytes());
            m.extend_from_slice(debug.as_bytes());
            out.push((c.peer, CH_SOCKETS, true, m));
        }
        (true, out)
    }

    pub fn close_listen(&mut self, h: u32) -> (bool, Vec<Out>) {
        if self.listens.remove(&h).is_none() {
            return (false, Vec::new());
        }
        let accepted: Vec<u32> = self.conns.iter().filter(|(_, c)| c.listen == h).map(|(h, _)| *h).collect();
        let mut out = Vec::new();
        for c in accepted {
            out.extend(self.close(c, END_APP, "listen socket closed").1);
        }
        (true, out)
    }

    /// A pair of connected sockets inside this process.
    pub fn pair(&mut self) -> (u32, u32) {
        let (a, b) = (self.handle(), self.handle());
        let me = self.me;
        self.conns.insert(a, Conn { peer: me, state: CONNECTED, local: b, ..Default::default() });
        self.conns.insert(b, Conn { peer: me, state: CONNECTED, local: a, ..Default::default() });
        (a, b)
    }

    pub fn send(&mut self, h: u32, data: &[u8], flags: i32) -> (i32, i64, Vec<Out>) {
        if data.len() > MAX_MESSAGE {
            return (INVALID_PARAM, 0, Vec::new());
        }
        let Some(c) = self.conns.get_mut(&h) else { return (INVALID_PARAM, 0, Vec::new()) };
        match c.state {
            CONNECTING | CONNECTED => {}
            CLOSED_BY_PEER | PROBLEM => return (NO_CONNECTION, 0, Vec::new()),
            _ => return (INVALID_STATE, 0, Vec::new()),
        }
        c.sent += 1;
        let number = c.sent;
        if c.local != 0 {
            let to = c.local;
            self.deliver(to, data.to_vec());
            return (OK, number, Vec::new());
        }
        let rel = reliable(flags) || data.len() > MAX_UNRELIABLE;
        if c.state == CONNECTING || c.remote == 0 {
            if c.outbox.len() >= 4096 {
                return (LIMIT_EXCEEDED, 0, Vec::new());
            }
            // Unreliable ones are dropped while connecting, like Steam does.
            if rel {
                c.outbox.push_back((rel, data.to_vec()));
            }
            return (OK, number, Vec::new());
        }
        (OK, number, vec![(c.peer, CH_SOCKETS, rel, data_message(c.remote, data))])
    }

    fn flush(&mut self, h: u32) -> Vec<Out> {
        let Some(c) = self.conns.get_mut(&h) else { return Vec::new() };
        let (peer, remote) = (c.peer, c.remote);
        c.outbox.drain(..).map(|(rel, d)| (peer, CH_SOCKETS, rel, data_message(remote, &d))).collect()
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

    pub fn send_to_user(&mut self, peer: u64, data: &[u8], flags: i32, channel: i32) -> (i32, Vec<Out>) {
        if peer == 0 {
            return (INVALID_PARAM, Vec::new());
        }
        if data.len() > MAX_MESSAGE {
            return (LIMIT_EXCEEDED, Vec::new());
        }
        let s = self.sessions.entry(peer).or_default();
        if s.failed && flags & 32 == 0 {
            // k_nSteamNetworkingSend_AutoRestartBrokenSession not set.
            return (NO_CONNECTION, Vec::new());
        }
        s.failed = false;
        s.accepted = true;
        let mut m = vec![W_DATA];
        m.extend_from_slice(&channel.to_le_bytes());
        m.extend_from_slice(data);
        let rel = reliable(flags) || data.len() > MAX_UNRELIABLE;
        (OK, vec![(peer, CH_MESSAGES, rel, m)])
    }

    pub fn accept_session(&mut self, peer: u64) -> bool {
        match self.sessions.get_mut(&peer) {
            Some(s) => {
                s.accepted = true;
                true
            }
            None => false,
        }
    }

    pub fn close_session(&mut self, peer: u64) -> bool {
        self.sessions.remove(&peer).is_some()
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

    /// (state, connected address known) for GetSessionConnectionInfo.
    pub fn session_state(&self, peer: u64) -> i32 {
        match self.sessions.get(&peer) {
            None => NONE,
            Some(s) if s.failed => PROBLEM,
            Some(_) => CONNECTED,
        }
    }

    // --------------------------------------------------------------- wire

    /// Bytes from a player on one of our channels.
    pub fn on_wire(&mut self, from: u64, channel: i32, d: &[u8]) -> Vec<Out> {
        if channel == CH_MESSAGES {
            if d.first() == Some(&W_DATA) && d.len() >= 5 {
                let ch = i32_at(d, 1).unwrap_or(0);
                self.arrival += 1;
                let arrival = self.arrival;
                let s = self.sessions.entry(from).or_default();
                s.received += 1;
                let m = Received { conn: 0, peer: from, data: d[5..].to_vec(), number: s.received, channel: ch, user_data: 0, arrival };
                let q = s.inbox.entry(ch).or_default();
                if q.len() < 65536 {
                    q.push_back(m);
                }
                if !s.accepted && !s.asked {
                    s.asked = true;
                    self.events.push((CB_SESSION_REQUEST, identity(from), self.callbacks.session_request));
                }
            }
            return Vec::new();
        }
        if channel != CH_SOCKETS || d.is_empty() {
            return Vec::new();
        }
        match d[0] {
            W_CONNECT => {
                let (Some(theirs), Some(vport)) = (u32_at(d, 1), i32_at(d, 5)) else { return Vec::new() };
                self.incoming(from, theirs, vport)
            }
            W_ACCEPT => {
                let (Some(mine), Some(theirs)) = (u32_at(d, 1), u32_at(d, 5)) else { return Vec::new() };
                let ok = self.conns.get(&mine).is_some_and(|c| c.peer == from && c.listen == 0 && c.state == CONNECTING);
                if !ok {
                    return Vec::new();
                }
                if let Some(c) = self.conns.get_mut(&mine) {
                    c.remote = theirs;
                }
                self.set_state(mine, CONNECTED, 0, "");
                self.flush(mine)
            }
            W_CLOSE => {
                let (Some(mine), Some(theirs), Some(reason)) = (u32_at(d, 1), u32_at(d, 5), i32_at(d, 9)) else { return Vec::new() };
                let debug = String::from_utf8_lossy(d.get(13..).unwrap_or_default()).into_owned();
                // Addressed by our handle, or (before ACCEPT) by theirs.
                let target = if self.conns.get(&mine).is_some_and(|c| c.peer == from) {
                    Some(mine)
                } else {
                    self.conns.iter().find(|(_, c)| c.peer == from && c.remote == theirs).map(|(h, _)| *h)
                };
                self.waiting.retain(|w| !(w.0 == from && w.1 == theirs));
                if let Some(h) = target {
                    self.set_state(h, CLOSED_BY_PEER, reason, &debug);
                }
                Vec::new()
            }
            W_DATA => {
                let Some(mine) = u32_at(d, 1) else { return Vec::new() };
                if self.conns.get(&mine).is_some_and(|c| c.peer == from && matches!(c.state, CONNECTING | CONNECTED)) {
                    self.deliver(mine, d[5..].to_vec());
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// We couldn't reach a player, or lost them.
    pub fn peer_failed(&mut self, peer: u64) {
        let why = "couldn't reach the player directly (both players need SteamInRust)";
        let hs: Vec<(u32, i32)> =
            self.conns.iter().filter(|(_, c)| c.peer == peer && c.local == 0 && matches!(c.state, CONNECTING | CONNECTED)).map(|(h, c)| (*h, c.state)).collect();
        for (h, state) in hs {
            if state == CONNECTED {
                self.set_state(h, PROBLEM, END_REMOTE_TIMEOUT, "the other player stopped answering");
            } else {
                self.set_state(h, PROBLEM, END_NAT, why);
            }
        }
        if let Some(s) = self.sessions.get_mut(&peer) {
            if !s.failed {
                s.failed = true;
                self.session_failed(peer, END_NAT, why);
            }
        }
    }

    /// Timeouts: connects nobody listens for, and connects that never got an answer.
    pub fn tick(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        let expired: Vec<(u64, u32, i32, Instant)> = self.waiting.iter().filter(|w| w.3.elapsed() > LISTEN_WAIT).cloned().collect();
        self.waiting.retain(|w| w.3.elapsed() <= LISTEN_WAIT);
        for (peer, theirs, _, _) in expired {
            let mut m = vec![W_CLOSE];
            m.extend_from_slice(&theirs.to_le_bytes());
            m.extend_from_slice(&0u32.to_le_bytes());
            m.extend_from_slice(&END_NO_CONNECTION.to_le_bytes());
            m.extend_from_slice(b"no listen socket on that virtual port");
            out.push((peer, CH_SOCKETS, true, m));
        }
        let stuck: Vec<u32> = self
            .conns
            .iter()
            .filter(|(_, c)| c.state == CONNECTING && c.listen == 0 && c.local == 0 && c.since.is_some_and(|s| s.elapsed() > LISTEN_WAIT + Duration::from_secs(10)))
            .map(|(h, _)| *h)
            .collect();
        for h in stuck {
            self.set_state(h, PROBLEM, END_TIMEOUT, "the other player didn't answer");
        }
        out
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
}

fn data_message(remote: u32, data: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(data.len() + 5);
    m.push(W_DATA);
    m.extend_from_slice(&remote.to_le_bytes());
    m.extend_from_slice(data);
    m
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
    let me = crate::state::lock().me();
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let out = net().tick();
            send_all(out);
        }
    });
    Mutex::new(Net::new(me))
});

/// Config callbacks to call on the game's thread: (function, bytes).
static PENDING: Mutex<Vec<(usize, Vec<u8>)>> = Mutex::new(Vec::new());

fn net() -> MutexGuard<'static, Net> {
    let mut n = NET.lock().unwrap_or_else(|e| e.into_inner());
    if n.me == 0 {
        n.me = crate::state::lock().me();
    }
    n
}

/// Hand events to the game: config callbacks wait for RunCallbacks, the
/// rest go into Steam's callback queue.
fn publish(n: &mut Net) {
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

/// Send to players; to ourselves, deliver in-process.
fn send_all(out: Vec<Out>) {
    let me = net().me;
    for (peer, ch, rel, data) in out {
        if peer == me {
            let more = {
                let mut n = net();
                let more = n.on_wire(me, ch, &data);
                publish(&mut n);
                more
            };
            send_all(more);
        } else if !crate::p2p::send_raw(peer, &data, rel, ch) {
            crate::log!("sns: no P2P endpoint for {peer}");
        }
    }
}

/// Run `f` on the network state, then publish its events and send its datagrams.
fn with<R>(f: impl FnOnce(&mut Net) -> (R, Vec<Out>)) -> R {
    let (r, out) = {
        let mut n = net();
        let res = f(&mut n);
        publish(&mut n);
        res
    };
    send_all(out);
    r
}

/// Bytes from a player on [`CH_SOCKETS`] or [`CH_MESSAGES`] (endpoint thread).
pub fn on_wire(from: u64, channel: i32, data: &[u8]) {
    with(|n| ((), n.on_wire(from, channel, data)));
}

pub fn peer_failed(peer: u64) {
    with(|n| {
        n.peer_failed(peer);
        ((), Vec::new())
    });
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
    with(|net| (net.connect_unsupported(option_callback(n, options), "connecting by IP address needs Valve's encrypted protocol; SteamInRust connects players over P2P only"), Vec::new()))
}

extern "C" fn s_listen_p2p(_t: This, vport: i32, n: i32, options: *const u8) -> u32 {
    crate::log!("sns: listen on virtual port {vport}");
    with(|net| net.listen(vport, option_callback(n, options)))
}

extern "C" fn s_connect_p2p(_t: This, who: *const u8, vport: i32, n: i32, options: *const u8) -> u32 {
    let peer = steam_id_of(who);
    crate::log!("sns: connect to {peer} port {vport}");
    if peer == 0 {
        return with(|net| (net.connect_unsupported(option_callback(n, options), "only Steam ID identities are supported"), Vec::new()));
    }
    with(|net| net.connect(peer, vport, option_callback(n, options)))
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
    let (r, num) = with(|n| {
        let (r, num, out) = n.send(h, &data, flags);
        ((r, num), out)
    });
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
        let (r, num) = with(|n| {
            let (r, num, out) = n.send(conn, &data, flags);
            ((r, num), out)
        });
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
fn realtime(state: i32, peer: u64, pending: usize) -> Vec<u8> {
    let ping = crate::p2p::rtt_ms(peer).unwrap_or(-1);
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
    let status = realtime(c.state, c.peer, c.outbox.len());
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
    let text = format!("SteamInRust P2P to {} (state {}, ping {} ms)\n", c.peer, c.state, crate::p2p::rtt_ms(c.peer).unwrap_or(-1));
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
    with(|n| n.send_to_user(peer, &data, flags, channel))
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
    let mut state = net().session_state(peer);
    if state == CONNECTED {
        state = match crate::p2p::link(peer) {
            Some((true, _)) => CONNECTED,
            Some((false, true)) => CONNECTING,
            Some((false, false)) => PROBLEM,
            None => CONNECTING,
        };
    }
    if !info.is_null() {
        let mut b = Bytes::new(INFO);
        b.put_bytes(0, &identity(peer)).i32(176, state).i32(440, 2);
        unsafe { std::ptr::copy_nonoverlapping(b.0.as_ptr(), info, INFO) };
    }
    if !status.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(realtime(state, peer, 0).as_ptr(), status, REALTIME_STATUS) };
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

    /// Carry datagrams between two in-process players until nothing moves.
    fn pump(a: &mut Net, b: &mut Net, mut out: Vec<Out>, from_a: bool) {
        let mut queue: VecDeque<(bool, Out)> = out.drain(..).map(|o| (from_a, o)).collect();
        while let Some((from_a, (_, ch, _, d))) = queue.pop_front() {
            let (to, from) = if from_a { (&mut *b, a.me) } else { (&mut *a, b.me) };
            for o in to.on_wire(from, ch, &d) {
                queue.push_back((!from_a, o));
            }
        }
    }

    fn states(n: &mut Net) -> Vec<(u32, i32, i32)> {
        n.events
            .drain(..)
            .filter(|e| e.0 == CB_CONN_STATUS)
            .map(|(_, d, _)| (u32_at(&d, 0).unwrap(), i32_at(&d, 8 + 176).unwrap(), i32_at(&d, 704).unwrap()))
            .collect()
    }

    #[test]
    fn connect_accept_send_close() {
        let (ai, bi) = (76561198000000001, 76561198000000002);
        let (mut a, mut b) = (Net::new(ai), Net::new(bi));
        let (_ls, out) = b.listen(7, 0);
        assert!(out.is_empty());
        let (ca, out) = a.connect(bi, 7, 0);
        assert_eq!(states(&mut a), vec![(ca, CONNECTING, NONE)]);
        // Reliable data sent while connecting waits for the connection.
        assert_eq!(a.send(ca, b"early", SEND_RELIABLE).0, OK);
        pump(&mut a, &mut b, out, true);
        let incoming = states(&mut b);
        assert_eq!(incoming.len(), 1);
        let cb = incoming[0].0;
        assert_eq!(incoming[0].1, CONNECTING);
        let (r, out) = b.accept(cb);
        assert_eq!(r, OK);
        pump(&mut a, &mut b, out, false);
        assert_eq!(states(&mut a), vec![(ca, CONNECTED, CONNECTING)]);
        assert_eq!(b.receive(cb, 10).unwrap().iter().map(|m| m.data.clone()).collect::<Vec<_>>(), vec![b"early".to_vec()]);

        // Both ways, in order, through a poll group on b.
        let g = b.new_group();
        assert!(b.set_group(cb, g));
        for i in 0..5u8 {
            let (r, n, out) = a.send(ca, &[i], if i % 2 == 0 { SEND_RELIABLE } else { 0 });
            assert_eq!((r, n), (OK, i as i64 + 2));
            pump(&mut a, &mut b, out, true);
        }
        let got = b.receive_group(g, 100).unwrap();
        assert_eq!(got.iter().map(|m| m.data[0]).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert!(got.iter().all(|m| m.conn == cb && m.peer == ai));
        let (_, _, out) = b.send(cb, b"pong", SEND_RELIABLE);
        pump(&mut a, &mut b, out, false);
        assert_eq!(a.receive(ca, 1).unwrap()[0].data, b"pong");

        // b closes: a hears ClosedByPeer with the reason.
        let (ok, out) = b.close(cb, 1234, "bye");
        assert!(ok);
        pump(&mut a, &mut b, out, false);
        assert_eq!(states(&mut a), vec![(ca, CLOSED_BY_PEER, CONNECTED)]);
        assert_eq!(a.conns[&ca].end_reason, 1234);
        assert_eq!(a.send(ca, b"x", SEND_RELIABLE).0, NO_CONNECTION);
    }

    #[test]
    fn connect_before_listen_and_failures() {
        let (ai, bi) = (1u64 << 56 | 1, 1u64 << 56 | 2);
        let (mut a, mut b) = (Net::new(ai), Net::new(bi));
        let (ca, out) = a.connect(bi, 3, 0);
        pump(&mut a, &mut b, out, true);
        assert!(states(&mut b).is_empty(), "no listen socket yet");
        let (_, out) = b.listen(3, 0);
        assert!(out.is_empty());
        assert_eq!(states(&mut b).len(), 1, "the waiting connect arrives");
        states(&mut a);
        a.peer_failed(bi);
        assert_eq!(states(&mut a), vec![(ca, PROBLEM, CONNECTING)]);
        let h = a.connect_unsupported(0, "no");
        assert_eq!(states(&mut a), vec![(h, CONNECTING, NONE), (h, PROBLEM, CONNECTING)]);
    }

    #[test]
    fn messages_sessions() {
        let (ai, bi) = (1u64 << 56 | 3, 1u64 << 56 | 4);
        let (mut a, mut b) = (Net::new(ai), Net::new(bi));
        let (r, out) = a.send_to_user(bi, b"hi", SEND_RELIABLE, 5);
        assert_eq!(r, OK);
        pump(&mut a, &mut b, out, true);
        // A request first; nothing readable until accepted.
        assert_eq!(b.events.iter().filter(|e| e.0 == CB_SESSION_REQUEST).count(), 1);
        assert!(b.receive_channel(5, 10).is_empty());
        assert!(b.accept_session(ai));
        let got = b.receive_channel(5, 10);
        assert_eq!((got[0].peer, got[0].channel, got[0].data.as_slice()), (ai, 5, &b"hi"[..]));
        // Replies need no accept on a's side (a started the session).
        let (_, out) = b.send_to_user(ai, b"yo", 0, 5);
        pump(&mut a, &mut b, out, false);
        assert_eq!(a.receive_channel(5, 10)[0].data, b"yo");
        assert!(a.events.iter().all(|e| e.0 != CB_SESSION_REQUEST));
        a.peer_failed(bi);
        assert_eq!(a.events.iter().filter(|e| e.0 == CB_SESSION_FAILED).count(), 1);
        assert_eq!(a.send_to_user(bi, b"x", 0, 5).0, NO_CONNECTION);
        assert_eq!(a.send_to_user(bi, b"x", 32, 5).0, OK);
    }

    #[test]
    fn socket_pair_and_layouts() {
        let mut n = Net::new(9);
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
    }
}

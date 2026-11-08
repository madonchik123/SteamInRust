//! One SteamNetworkingSockets P2P connection, the way Valve's client runs
//! it (steamnetworkingsockets_p2p.cpp):
//!
//! 1. The client sends ConnectRequest (cert + signed key) in rendezvous
//!    signals through Steam until the server answers ConnectOK (or closes).
//! 2. Both sides then "find a route": ICE candidates go in reliable
//!    rendezvous messages; data goes over the first path that works
//!    (ICE, or a relay).
//! 3. A data packet that decrypts on that path means "connected".
//!
//! Data packets (steamnetworkingsockets_udp.cpp, the same on ICE):
//! `[0x80|flags][to connection id u32][packet number u16][stats?][time?][AES-GCM(SNP frames)]`.

use super::crypto::{self, CIPHER_AES_GCM, Context, LocalCert, LocalCrypt, Session};
use super::ice::{self, Agent, Candidate};
use super::msg::*;
use super::snp::{self, Snp};
use prost::Message;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// ESteamNetworkingConnectionState
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connecting = 1,
    FindingRoute = 2,
    Connected = 3,
    ClosedByPeer = 4,
    Problem = 5,
    /// Closed by us (the app is done with it).
    Closed = 6,
}

impl State {
    pub fn active(self) -> bool {
        matches!(self, State::Connecting | State::FindingRoute | State::Connected)
    }
}

// ESteamNetConnectionEnd values we use.
pub const END_APP_GENERIC: u32 = 1000;
pub const END_LOCAL_OFFLINE: u32 = 3001;
pub const END_REMOTE_TIMEOUT: u32 = 4001;
pub const END_REMOTE_BAD_CRYPT: u32 = 4002;
pub const END_REMOTE_BAD_CERT: u32 = 4003;
pub const END_MISC_GENERIC: u32 = 5001;
pub const END_MISC_INTERNAL: u32 = 5002;
pub const END_MISC_TIMEOUT: u32 = 5003;
pub const END_P2P_RENDEZVOUS: u32 = 5008;
pub const END_P2P_NAT_FIREWALL: u32 = 5009;
pub const END_PEER_SENT_NO_CONNECTION: u32 = 5010;
pub const END_INTERNAL_P2P_NO_CONNECTION: u32 = 6002;

pub const VPORT_MESSAGES: i32 = 0x7fff_ffff;

const MSG_CONNECTION_CLOSED: u8 = 36;
const MSG_NO_CONNECTION: u8 = 37;

const CONNECT_RETRY: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const ROUTE_TIMEOUT: Duration = Duration::from_secs(25);
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE: Duration = Duration::from_millis(1000);
const SIGNAL_RTO: Duration = Duration::from_secs(2);

/// What a connection needs from the outside world.
pub trait Io {
    fn udp(&mut self, to: SocketAddr, data: &[u8]);
    /// Send a serialized `CMsgSteamNetworkingP2PRendezvous` to a player.
    fn signal(&mut self, peer: u64, data: Vec<u8>);
    /// Our ICE candidates (host addresses of the shared socket, STUN result).
    fn candidates(&self) -> Vec<Candidate>;
    fn unix_time(&self) -> u64;
}

pub struct Conn {
    pub local_id: u32,
    pub remote_id: u32,
    pub me: u64,
    pub peer: u64,
    /// We sent the ConnectRequest.
    pub client: bool,
    pub vport_local: i32,
    pub vport_remote: i32,
    pub state: State,
    pub end_reason: u32,
    pub end_debug: String,
    /// The state changed since the owner last looked.
    pub changed: bool,
    cert: Arc<LocalCert>,
    crypt: Option<LocalCrypt>,
    crypt_signed: Option<CryptInfoSigned>,
    remote_cert: Option<Vec<u8>>,
    remote_crypt: Option<(Vec<u8>, CryptInfo)>,
    pub remote_trusted: bool,
    session: Option<Session>,
    pub peer_protocol: u32,
    pub snp: Snp,

    // signalling
    reliable_out: Vec<(u32, ReliableMessage, Option<Instant>)>,
    next_reliable: u32,
    last_reliable_in: u32,
    signal_due: Option<Instant>,
    last_signal: Option<Instant>,
    need_connect_ok: bool,
    /// We told them about these candidates.
    sent_candidates: Vec<SocketAddr>,
    pub ice: Agent,
    /// Peer said it does ICE.
    peer_ice: bool,

    // timing
    pub created: Instant,
    state_since: Instant,
    last_ping: Option<Instant>,
    last_sent: Option<Instant>,
    route_confirmed: bool,
    pub messages_sent: i64,
}

fn render(m: &Rendezvous) -> String {
    let mut parts = Vec::new();
    if m.connect_request.is_some() {
        parts.push("connect_request".to_string());
    }
    if m.connect_ok.is_some() {
        parts.push("connect_ok".into());
    }
    if let Some(c) = &m.connection_closed {
        parts.push(format!("closed({} {:?})", c.reason_code(), c.debug()));
    }
    if !m.reliable_messages.is_empty() {
        parts.push(format!("reliable#{}x{}", m.first_reliable_msg(), m.reliable_messages.len()));
    }
    if m.sdr_routes.is_some() {
        parts.push("sdr_routes".into());
    }
    if m.ice_enabled() {
        parts.push("ice".into());
    }
    format!("{} -> {} [{}] ack {}", m.from_connection_id(), m.to_connection_id(), parts.join(", "), m.ack_reliable_msg())
}

impl Conn {
    fn base(me: u64, peer: u64, cert: Arc<LocalCert>, client: bool, now: Instant) -> Conn {
        let local_id = loop {
            let id = crypto::random_u32();
            if id != 0 {
                break id;
            }
        };
        Conn {
            local_id,
            remote_id: 0,
            me,
            peer,
            client,
            vport_local: -1,
            vport_remote: -1,
            state: State::Connecting,
            end_reason: 0,
            end_debug: String::new(),
            changed: false,
            cert,
            crypt: None,
            crypt_signed: None,
            remote_cert: None,
            remote_crypt: None,
            remote_trusted: false,
            session: None,
            peer_protocol: 0,
            snp: Snp::new(),
            reliable_out: Vec::new(),
            next_reliable: 0,
            last_reliable_in: 0,
            signal_due: Some(now),
            last_signal: None,
            need_connect_ok: false,
            sent_candidates: Vec::new(),
            // The server is the controlling agent (GNS's choice).
            ice: Agent::new(!client, local_id, 0, now),
            peer_ice: false,
            created: now,
            state_since: now,
            last_ping: None,
            last_sent: None,
            route_confirmed: false,
            messages_sent: 0,
        }
    }

    /// Start a connection to `peer` (ConnectP2P / SendMessageToUser).
    pub fn connect(me: u64, peer: u64, vport_remote: i32, vport_local: i32, cert: Arc<LocalCert>, now: Instant) -> Conn {
        let mut c = Self::base(me, peer, cert.clone(), true, now);
        c.vport_remote = vport_remote;
        c.vport_local = vport_local;
        let crypt = LocalCrypt::new(&cert, vec![CIPHER_AES_GCM]);
        c.crypt_signed = Some(crypt.signed.clone());
        c.crypt = Some(crypt);
        c.queue_ice_auth();
        crate::log!("gns: connecting #{} to {peer} port {vport_remote}", c.local_id);
        c
    }

    /// An incoming ConnectRequest. The connection stays Connecting until accepted.
    pub fn incoming(me: u64, msg: &Rendezvous, cert: Arc<LocalCert>, now: Instant, unix: u64) -> Result<Conn, (u32, String)> {
        let peer = crypto::parse_identity(msg.from_identity()).ok_or((END_MISC_GENERIC, "bad from_identity".to_string()))?;
        let req = msg.connect_request.as_ref().ok_or((END_MISC_GENERIC, "no connect request".to_string()))?;
        let mut c = Self::base(me, peer, cert, false, now);
        c.remote_id = msg.from_connection_id();
        c.ice = Agent::new(true, c.local_id, c.remote_id, now);
        c.vport_local = req.to_virtual_port.map(|v| v as i32).unwrap_or(-1);
        c.vport_remote = req.from_virtual_port.map(|v| v as i32).unwrap_or(c.vport_local);
        c.recv_handshake(req.cert.as_ref(), req.crypt.as_ref(), unix)?;
        c.signal_due = None;
        crate::log!("gns: connect request #{} from {peer} (their #{}) to port {}", c.local_id, c.remote_id, c.vport_local);
        Ok(c)
    }

    fn queue_ice_auth(&mut self) {
        let pwd = self.ice.local_pwd.clone();
        self.queue_reliable(ReliableMessage { ice: Some(IceRendezvous { auth: Some(IceAuth { pwd_frag: Some(pwd) }), add_candidate: None }) });
    }

    fn queue_reliable(&mut self, m: ReliableMessage) {
        self.next_reliable += 1;
        self.reliable_out.push((self.next_reliable, m, None));
        self.signal_due.get_or_insert(Instant::now());
    }

    fn set_state(&mut self, s: State, now: Instant) {
        if self.state != s {
            crate::log!("gns: #{} {:?} -> {:?}{}", self.local_id, self.state, s, if self.end_reason != 0 { format!(" ({} {})", self.end_reason, self.end_debug) } else { String::new() });
            self.state = s;
            self.state_since = now;
            self.changed = true;
        }
    }

    pub fn problem(&mut self, reason: u32, debug: &str, now: Instant, io: &mut dyn Io) {
        if !self.state.active() {
            return;
        }
        self.end_reason = reason;
        self.end_debug = debug.to_string();
        self.send_closed(io);
        self.set_state(State::Problem, now);
    }

    fn closed_by_peer(&mut self, reason: u32, debug: &str, now: Instant) {
        if !self.state.active() {
            return;
        }
        self.end_reason = reason;
        self.end_debug = debug.to_string();
        self.set_state(State::ClosedByPeer, now);
    }

    /// The app closes the connection (or rejects it).
    pub fn close(&mut self, reason: u32, debug: &str, io: &mut dyn Io) {
        if self.state.active() {
            self.end_reason = reason;
            self.end_debug = debug.to_string();
            self.send_closed(io);
        }
        self.state = State::Closed;
    }

    fn send_closed(&mut self, io: &mut dyn Io) {
        let reason = if self.end_reason == 0 { END_APP_GENERIC } else { self.end_reason };
        // Through the data path, if there is one…
        if let (Some(to), true) = (self.ice.selected, self.remote_id != 0) {
            let m = UdpConnectionClosed { to_connection_id: Some(self.remote_id), from_connection_id: Some(self.local_id), debug: Some(self.end_debug.clone()), reason_code: Some(reason) };
            let body = m.encode_to_vec();
            let mut pkt = vec![MSG_CONNECTION_CLOSED];
            pkt.extend_from_slice(&(body.len() as u16).to_le_bytes());
            pkt.extend_from_slice(&body);
            pkt.resize(pkt.len().max(512), 0);
            io.udp(to, &pkt);
        }
        // …and through Steam.
        let mut m = self.signal_base();
        m.connection_closed = Some(ConnectionClosed { debug: Some(self.end_debug.clone()), reason_code: Some(reason) });
        if self.remote_id == 0 {
            m.to_connection_id = None;
        }
        crate::log!("gns: #{} signal to {}: {}", self.local_id, self.peer, render(&m));
        io.signal(self.peer, m.encode_to_vec());
    }

    fn signal_base(&self) -> Rendezvous {
        Rendezvous {
            from_identity: Some(crypto::identity_string(self.me)),
            from_connection_id: Some(self.local_id),
            to_identity: Some(crypto::identity_string(self.peer)),
            to_connection_id: (self.remote_id != 0).then_some(self.remote_id),
            ..Default::default()
        }
    }

    fn send_signal(&mut self, now: Instant, io: &mut dyn Io) {
        let mut m = self.signal_base();
        if self.need_connect_ok {
            m.connect_ok = Some(ConnectOk { crypt: self.crypt_signed.clone(), cert: Some(self.cert.signed.clone()) });
            self.need_connect_ok = false;
        }
        if self.state == State::Connecting && self.client {
            m.connect_request = Some(ConnectRequest {
                crypt: self.crypt_signed.clone(),
                cert: Some(self.cert.signed.clone()),
                to_virtual_port: (self.vport_remote >= 0).then_some(self.vport_remote as u32),
                from_virtual_port: (self.vport_local >= 0).then_some(self.vport_local as u32),
            });
            m.to_connection_id = None;
        }
        m.ice_enabled = Some(true);
        let mut size = 0;
        for (id, r, rto) in self.reliable_out.iter_mut() {
            if m.first_reliable_msg.is_none() {
                if rto.is_some_and(|t| t > now) && m.connect_request.is_none() && m.connect_ok.is_none() {
                    continue;
                }
                if size > 800 {
                    break;
                }
                m.first_reliable_msg = Some(*id);
            }
            size += r.encoded_len();
            m.reliable_messages.push(r.clone());
            *rto = Some(now + SIGNAL_RTO);
        }
        m.ack_reliable_msg = Some(self.last_reliable_in);
        crate::log!("gns: #{} signal to {}: {}", self.local_id, self.peer, render(&m));
        io.signal(self.peer, m.encode_to_vec());
        self.last_signal = Some(now);
        self.signal_due = None;
    }

    fn recv_handshake(&mut self, cert: Option<&CertSigned>, crypt: Option<&CryptInfoSigned>, unix: u64) -> Result<(), (u32, String)> {
        let (cert, crypt) = match (cert, crypt) {
            (Some(a), Some(b)) => (a, b),
            _ => return Err((END_REMOTE_BAD_CRYPT, "crypto handshake missing cert or session data".into())),
        };
        let (parsed, trusted) = crypto::check_remote_cert(cert, self.peer, unix).map_err(|e| (END_REMOTE_BAD_CERT, e))?;
        let info = crypto::check_remote_crypt(crypt, &parsed).map_err(|e| (END_REMOTE_BAD_CRYPT, e))?;
        self.peer_protocol = info.protocol_version();
        self.remote_trusted = trusted;
        self.remote_cert = cert.cert.clone();
        self.remote_crypt = Some((crypt.info.clone().unwrap_or_default(), info));
        Ok(())
    }

    fn finish_handshake(&mut self) -> Result<(), (u32, String)> {
        let (raw, info) = self.remote_crypt.clone().ok_or((END_MISC_INTERNAL, "no remote crypt".to_string()))?;
        let server = !self.client;
        if server {
            let cipher = crypto::negotiate(&[CIPHER_AES_GCM], &info.ciphers).ok_or((END_REMOTE_BAD_CRYPT, "failed to negotiate mutually-agreeable cipher".to_string()))?;
            let crypt = LocalCrypt::new(&self.cert, vec![cipher]);
            self.crypt_signed = Some(crypt.signed.clone());
            self.crypt = Some(crypt);
        } else if info.ciphers.len() != 1 {
            return Err((END_REMOTE_BAD_CRYPT, "server must select exactly only one cipher".into()));
        }
        let cipher = if server { CIPHER_AES_GCM } else { info.ciphers[0] };
        if cipher != CIPHER_AES_GCM && cipher != crypto::CIPHER_NULL {
            return Err((END_REMOTE_BAD_CRYPT, format!("unknown cipher {cipher}")));
        }
        let local = self.crypt.take().ok_or((END_MISC_INTERNAL, "no local crypt".to_string()))?;
        let local_info = local.info_bytes().to_vec();
        let local_nonce = local.info.nonce();
        let remote_cert = self.remote_cert.clone().unwrap_or_default();
        let ctx = Context {
            server,
            local_id: self.local_id,
            remote_id: self.remote_id,
            cert_local: self.cert.cert_bytes(),
            cert_remote: &remote_cert,
            crypt_local: &local_info,
            crypt_remote: &raw,
            nonce_local: local_nonce,
            nonce_remote: info.nonce(),
        };
        self.session = Some(Session::derive(local, info.key_data(), cipher, &ctx).map_err(|e| (END_REMOTE_BAD_CRYPT, e))?);
        Ok(())
    }

    /// AcceptConnection.
    pub fn accept(&mut self, now: Instant) -> Result<(), String> {
        if self.client || self.state != State::Connecting {
            return Err("not an incoming connection waiting to be accepted".into());
        }
        if let Err((r, e)) = self.finish_handshake() {
            self.end_reason = r;
            self.end_debug = e.clone();
            self.set_state(State::Problem, now);
            return Err(e);
        }
        self.need_connect_ok = true;
        self.queue_ice_auth();
        self.signal_due = Some(now);
        self.set_state(State::FindingRoute, now);
        Ok(())
    }

    /// A rendezvous signal for this connection.
    pub fn on_signal(&mut self, m: &Rendezvous, now: Instant, io: &mut dyn Io) {
        crate::log!("gns: #{} signal from {}: {}", self.local_id, self.peer, render(m));
        if self.remote_id == 0 && m.from_connection_id() != 0 && m.connect_request.is_none() {
            self.remote_id = m.from_connection_id();
            self.ice.remote_ufrag = ice::base64_30bits(self.remote_id);
        }
        if m.ice_enabled() {
            self.peer_ice = true;
        }
        if let Some(c) = &m.connection_closed {
            let reason = c.reason_code();
            if reason == END_INTERNAL_P2P_NO_CONNECTION {
                self.closed_by_peer(END_PEER_SENT_NO_CONNECTION, "received unexpected P2P 'no connection' signal", now);
            } else {
                self.closed_by_peer(reason, c.debug(), now);
            }
            return;
        }
        if m.ack_reliable_msg() > 0 {
            let ack = m.ack_reliable_msg();
            self.reliable_out.retain(|(id, _, _)| *id > ack);
        }
        if let Some(first) = m.first_reliable_msg {
            self.signal_due.get_or_insert(now + Duration::from_millis(50));
            if first <= self.last_reliable_in + 1 {
                let skip = (self.last_reliable_in + 1 - first) as usize;
                for r in m.reliable_messages.iter().skip(skip) {
                    self.last_reliable_in += 1;
                    if let Some(i) = &r.ice {
                        self.on_ice(i);
                    }
                }
            }
        }
        match self.state {
            State::Connecting if self.client => {
                if let Some(ok) = &m.connect_ok {
                    if let Err((r, e)) = self.recv_handshake(ok.cert.as_ref(), ok.crypt.as_ref(), io.unix_time()).and_then(|_| self.finish_handshake()) {
                        crate::log!("gns: #{} bad ConnectOK: {e}", self.local_id);
                        self.problem(r, &e, now, io);
                        return;
                    }
                    crate::log!("gns: #{} ConnectOK from {} (protocol {}, {})", self.local_id, self.peer, self.peer_protocol, if self.remote_trusted { "Valve-signed cert" } else { "unsigned cert" });
                    self.snp.last_recv = Some(now);
                    self.set_state(State::FindingRoute, now);
                }
            }
            State::FindingRoute | State::Connected if !self.client && m.connect_request.is_some() => {
                // Our ConnectOK got lost.
                self.need_connect_ok = true;
                self.signal_due = Some(now);
            }
            _ => {}
        }
        for a in &m.application_messages {
            self.snp.inbox.push_back(snp::Msg { data: a.data().to_vec(), num: a.msg_num() as i64, reliable: false, lane: 0 });
        }
    }

    fn on_ice(&mut self, i: &IceRendezvous) {
        if let Some(a) = &i.auth {
            crate::log!("gns: #{} peer ICE password set", self.local_id);
            self.ice.remote_pwd = Some(a.pwd_frag().to_string());
        }
        if let Some(c) = &i.add_candidate {
            match Candidate::parse(c.candidate()) {
                Some(cand) => {
                    crate::log!("gns: #{} peer candidate {:?} {}", self.local_id, cand.kind, cand.addr);
                    self.ice.add_remote(&cand);
                }
                None => crate::log!("gns: #{} skipped candidate {:?}", self.local_id, c.candidate()),
            }
        }
    }

    /// A STUN packet the owner matched to this connection.
    pub fn on_stun(&mut self, raw: &[u8], m: &ice::Stun, from: SocketAddr, io: &mut dyn Io) {
        if let Some(reply) = self.ice.on_stun(raw, m, from) {
            io.udp(from, &reply);
        }
    }

    /// A SteamNetworkingSockets packet (data, closed, no connection) from `from`.
    pub fn on_packet(&mut self, d: &[u8], from: SocketAddr, now: Instant, io: &mut dyn Io) {
        let Some(&lead) = d.first() else { return };
        if lead & 0x80 == 0 {
            match lead {
                MSG_CONNECTION_CLOSED => {
                    let len = d.get(1..3).map(|l| u16::from_le_bytes([l[0], l[1]]) as usize).unwrap_or(0);
                    if let Some(m) = d.get(3..3 + len).and_then(decode::<UdpConnectionClosed>) {
                        self.closed_by_peer(m.reason_code(), m.debug(), now);
                        let nc = UdpNoConnection { from_connection_id: Some(self.local_id), to_connection_id: Some(self.remote_id) };
                        let mut pkt = vec![MSG_NO_CONNECTION];
                        pkt.extend_from_slice(&nc.encode_to_vec());
                        io.udp(from, &pkt);
                    }
                }
                MSG_NO_CONNECTION => self.closed_by_peer(END_PEER_SENT_NO_CONNECTION, "peer has no such connection", now),
                _ => {}
            }
            return;
        }
        if d.len() < 7 || u32::from_le_bytes(d[1..5].try_into().unwrap()) != self.local_id {
            return;
        }
        let Some(session) = &self.session else { return };
        if !matches!(self.state, State::FindingRoute | State::Connected) {
            return;
        }
        let wire = u16::from_le_bytes([d[5], d[6]]);
        let mut at = 7;
        let mut flags = 0;
        if lead & 0x01 != 0 {
            let mut v = 0u64;
            let mut shift = 0;
            loop {
                let Some(&b) = d.get(at) else { return };
                at += 1;
                v |= ((b & 0x7f) as u64) << shift;
                shift += 7;
                if b & 0x80 == 0 || shift > 28 {
                    break;
                }
            }
            let Some(blob) = d.get(at..at + v as usize) else { return };
            flags = decode::<UdpStats>(blob).map(|s| s.flags()).unwrap_or(0);
            at += v as usize;
        }
        if lead & 0x02 != 0 {
            at += 2;
        }
        let Some(body) = d.get(at..) else { return };
        let Some(pkt) = self.snp.expand(wire) else { return };
        let Some(plain) = session.decrypt(pkt, body) else {
            crate::log!("gns: #{} packet {pkt} from {from} failed to decrypt", self.local_id);
            return;
        };
        if let Err(e) = self.snp.receive(pkt, &plain, now) {
            self.problem(END_MISC_INTERNAL, &e, now, io);
            return;
        }
        if flags & 2 != 0 {
            self.snp.ack_now(now);
        }
        // Data came over this path: it works both ways.
        if self.ice.selected != Some(from) && self.ice.selected.is_none() {
            self.ice.selected = Some(from);
        }
        if !self.route_confirmed {
            self.route_confirmed = true;
            crate::log!("gns: #{} route to {} confirmed via {from}", self.local_id, self.peer);
        }
        if self.state == State::FindingRoute {
            self.set_state(State::Connected, now);
        }
    }

    /// SendMessageToConnection. Returns the message number.
    pub fn send(&mut self, data: &[u8], reliable: bool) -> Result<i64, i32> {
        if data.len() > snp::MAX_MESSAGE {
            return Err(8); // InvalidParam
        }
        match self.state {
            State::Connecting | State::FindingRoute | State::Connected => {}
            State::ClosedByPeer | State::Problem => return Err(3), // NoConnection
            State::Closed => return Err(11),
        }
        // Unreliable messages sent before the route is up are dropped (like Steam).
        if !reliable && self.state != State::Connected {
            self.messages_sent += 1;
            return Ok(self.messages_sent);
        }
        let n = self.snp.queue(data, reliable);
        self.messages_sent = self.messages_sent.max(n);
        Ok(n)
    }

    fn send_packet(&mut self, now: Instant, force_ping: bool, io: &mut dyn Io) -> bool {
        let Some(to) = self.ice.selected else { return false };
        let Some(session) = &self.session else { return false };
        let Some((pkt, plain)) = self.snp.build(now, force_ping) else { return false };
        let mut out = Vec::with_capacity(plain.len() + 32);
        let mut lead = 0x80u8;
        out.push(0);
        out.extend_from_slice(&self.remote_id.to_le_bytes());
        out.extend_from_slice(&(pkt as u16).to_le_bytes());
        if force_ping {
            let blob = UdpStats { stats: None, flags: Some(2 | 4) }.encode_to_vec();
            lead |= 0x01;
            out.push(blob.len() as u8);
            out.extend_from_slice(&blob);
        }
        out[0] = lead;
        out.extend_from_slice(&session.encrypt(pkt, &plain));
        io.udp(to, &out);
        self.last_sent = Some(now);
        true
    }

    /// Timers: signals, ICE checks, packets, timeouts.
    pub fn poll(&mut self, now: Instant, io: &mut dyn Io) {
        if !self.state.active() {
            return;
        }
        // Timeouts.
        let in_state = now.duration_since(self.state_since);
        match self.state {
            State::Connecting if self.client && in_state > CONNECT_TIMEOUT => {
                self.problem(END_MISC_TIMEOUT, "timed out waiting for the other player to answer", now, io);
                return;
            }
            State::Connecting if !self.client && in_state > CONNECT_TIMEOUT => {
                self.problem(END_MISC_TIMEOUT, "connection wasn't accepted in time", now, io);
                return;
            }
            State::FindingRoute if in_state > ROUTE_TIMEOUT => {
                self.problem(END_P2P_NAT_FIREWALL, "no route to the other player (NAT traversal failed)", now, io);
                return;
            }
            State::Connected if self.snp.last_recv.is_some_and(|t| now.duration_since(t) > IDLE_TIMEOUT) => {
                self.problem(END_MISC_TIMEOUT, "the other player stopped answering", now, io);
                return;
            }
            _ => {}
        }
        // New local candidates go to the peer.
        if self.state != State::Connecting || self.client {
            for c in io.candidates() {
                if !self.sent_candidates.contains(&c.addr) {
                    self.sent_candidates.push(c.addr);
                    crate::log!("gns: #{} offering candidate {:?} {}", self.local_id, c.kind, c.addr);
                    self.queue_reliable(ReliableMessage { ice: Some(IceRendezvous { auth: None, add_candidate: Some(IceCandidate { candidate: Some(c.to_sdp()) }) }) });
                }
            }
        }
        // Signals: connect retries, reliable resends, acks.
        let retry_connect = self.state == State::Connecting && self.client && self.last_signal.is_none_or(|t| now.duration_since(t) >= CONNECT_RETRY);
        let reliable_due = self.reliable_out.iter().any(|(_, _, rto)| rto.is_none_or(|t| t <= now));
        if (retry_connect || reliable_due || self.need_connect_ok || self.signal_due.is_some_and(|t| t <= now)) && (self.state != State::Connecting || self.client) {
            self.send_signal(now, io);
        }
        // ICE checks.
        if self.state != State::Connecting {
            for (to, b) in self.ice.poll(now) {
                io.udp(to, &b);
            }
        }
        // Data.
        if matches!(self.state, State::FindingRoute | State::Connected) && self.ice.selected.is_some() {
            self.snp.tick(now);
            let mut n = 0;
            while self.snp.wants_send(now) && n < 64 {
                if !self.send_packet(now, false, io) {
                    break;
                }
                n += 1;
            }
            // Pings: until the route is confirmed, and when idle.
            let ping_every = if self.route_confirmed { KEEPALIVE } else { Duration::from_millis(200) };
            let idle = self.snp.last_recv.is_none_or(|t| now.duration_since(t) >= ping_every);
            if idle && self.last_ping.is_none_or(|t| now.duration_since(t) >= ping_every) {
                self.last_ping = Some(now);
                self.send_packet(now, true, io);
            }
        }
    }

    /// When poll wants to run next.
    pub fn next_wake(&self, now: Instant) -> Instant {
        let mut t = now + Duration::from_millis(50);
        if let Some(a) = self.snp.ack_due() {
            t = t.min(a);
        }
        if let Some(s) = self.signal_due {
            t = t.min(s);
        }
        t.max(now)
    }

    pub fn ping_ms(&self) -> i32 {
        self.snp.rtt.map(|r| r.as_millis() as i32).unwrap_or(-1)
    }

    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.ice.selected
    }

    /// Is the ConnectRequest in `m` from the peer this (client) connection
    /// is also trying to reach? Then exactly one side becomes the server:
    /// returns true if we should switch to accepting theirs.
    pub fn symmetric_should_accept(&self, m: &Rendezvous) -> bool {
        let (a, b) = (self.local_id, m.from_connection_id());
        let mut r = a.cmp(&b) as i32;
        if r == 0 {
            let theirs = m.connect_request.as_ref().and_then(|r| r.cert.as_ref()).and_then(|c| c.cert.clone()).unwrap_or_default();
            r = self.cert.cert_bytes().cmp(&theirs[..]) as i32;
        }
        if (a ^ b) & 1 != 0 {
            r = -r;
        }
        r > 0
    }

    /// Our outgoing connection becomes the server for their request (symmetric connect).
    pub fn become_server(&mut self, m: &Rendezvous, now: Instant, unix: u64) -> Result<(), (u32, String)> {
        let req = m.connect_request.as_ref().ok_or((END_MISC_GENERIC, "no connect request".to_string()))?;
        self.client = false;
        self.remote_id = m.from_connection_id();
        self.ice = Agent::new(true, self.local_id, self.remote_id, now);
        self.reliable_out.clear();
        self.next_reliable = 0;
        self.sent_candidates.clear();
        self.crypt = None;
        self.recv_handshake(req.cert.as_ref(), req.crypt.as_ref(), unix)?;
        self.accept(now).map_err(|e| (END_MISC_GENERIC, e))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Two players on a perfect network: signals and datagrams as queues.
    pub struct World {
        pub signals: VecDeque<(u64, u64, Vec<u8>)>,
        pub udp: VecDeque<(SocketAddr, SocketAddr, Vec<u8>)>,
    }

    pub struct Side<'a> {
        pub me: u64,
        pub addr: SocketAddr,
        pub w: &'a mut World,
    }

    impl Io for Side<'_> {
        fn udp(&mut self, to: SocketAddr, data: &[u8]) {
            self.w.udp.push_back((self.addr, to, data.to_vec()));
        }
        fn signal(&mut self, peer: u64, data: Vec<u8>) {
            self.w.signals.push_back((self.me, peer, data));
        }
        fn candidates(&self) -> Vec<Candidate> {
            vec![Candidate::new(self.addr, ice::Kind::Host)]
        }
        fn unix_time(&self) -> u64 {
            1_800_000_000
        }
    }

    #[test]
    fn connect_accept_exchange_close() {
        let (ai, bi) = (76561198000000001u64, 76561198000000002u64);
        let (aa, ba): (SocketAddr, SocketAddr) = ("10.0.0.1:27000".parse().unwrap(), "10.0.0.2:27001".parse().unwrap());
        let ca = Arc::new(LocalCert::unsigned(LocalCert::new_key(), ai, 480));
        let cb = Arc::new(LocalCert::unsigned(LocalCert::new_key(), bi, 480));
        let mut w = World { signals: VecDeque::new(), udp: VecDeque::new() };
        let mut t = Instant::now();
        let mut a = Conn::connect(ai, bi, 7, -1, ca, t);
        let mut b: Option<Conn> = None;
        a.send(b"early reliable", true).unwrap();
        for _ in 0..200 {
            t += Duration::from_millis(20);
            a.poll(t, &mut Side { me: ai, addr: aa, w: &mut w });
            if let Some(b) = b.as_mut() {
                b.poll(t, &mut Side { me: bi, addr: ba, w: &mut w });
            }
            while let Some((from, to, data)) = w.signals.pop_front() {
                let m = Rendezvous::decode(&data[..]).unwrap();
                if to == bi {
                    match b.as_mut() {
                        Some(b) => b.on_signal(&m, t, &mut Side { me: bi, addr: ba, w: &mut w }),
                        None => {
                            let mut c = Conn::incoming(bi, &m, cb.clone(), t, 1_800_000_000).unwrap();
                            assert_eq!(c.vport_local, 7);
                            c.on_signal(&m, t, &mut Side { me: bi, addr: ba, w: &mut w });
                            c.accept(t).unwrap();
                            b = Some(c);
                        }
                    }
                } else {
                    assert_eq!(from, bi);
                    a.on_signal(&m, t, &mut Side { me: ai, addr: aa, w: &mut w });
                }
            }
            while let Some((from, to, data)) = w.udp.pop_front() {
                let (conn, me, addr) = if to == ba { (b.as_mut().unwrap(), bi, ba) } else { (&mut a, ai, aa) };
                let mut io = Side { me, addr, w: &mut w };
                if let Some(s) = ice::Stun::parse(&data) {
                    assert!(conn.ice.owns(&s) || s.kind != 1);
                    conn.on_stun(&data, &s, from, &mut io);
                } else {
                    conn.on_packet(&data, from, t, &mut io);
                }
            }
            if a.state == State::Connected && b.as_ref().is_some_and(|b| b.state == State::Connected) && !a.snp.has_unacked() {
                break;
            }
        }
        let mut b = b.unwrap();
        assert_eq!((a.state, b.state), (State::Connected, State::Connected));
        assert_eq!(b.snp.inbox.pop_front().unwrap().data, b"early reliable");
        // Close: b hears it.
        a.close(END_APP_GENERIC, "bye", &mut Side { me: ai, addr: aa, w: &mut w });
        while let Some((from, _, data)) = w.udp.pop_front() {
            b.on_packet(&data, from, t, &mut Side { me: bi, addr: ba, w: &mut w });
        }
        assert_eq!(b.state, State::ClosedByPeer);
        assert_eq!(b.end_debug, "bye");
    }
}

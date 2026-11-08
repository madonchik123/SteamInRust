//! ICE (RFC 8445) for SteamNetworkingSockets' direct path, as Steam's
//! client runs it (WebRTC's ICE in steamwebrtc64.dll, configured by
//! steamnetworkingsockets_p2p_ice.cpp):
//!
//! - ufrag = the connection ID's low 30 bits in base64 (5 chars), the
//!   password a random 5-char fragment sent in the first reliable signal;
//! - candidates travel as SDP strings ("candidate:… 0 udp … typ host") in
//!   reliable rendezvous messages, component 0;
//! - the side that accepted the connection is the controlling agent;
//! - STUN binding checks with short-term credentials (HMAC-SHA1) and
//!   FINGERPRINT; once a pair works, data packets go to it as plain UDP.

use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

const COOKIE: [u8; 4] = [0x21, 0x12, 0xA4, 0x42];
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const BINDING_ERROR: u16 = 0x0111;
const A_MAPPED: u16 = 0x0001;
const A_USERNAME: u16 = 0x0006;
const A_INTEGRITY: u16 = 0x0008;
const A_ERROR: u16 = 0x0009;
const A_XOR_MAPPED: u16 = 0x0020;
const A_PRIORITY: u16 = 0x0024;
const A_USE_CANDIDATE: u16 = 0x0025;
const A_FINGERPRINT: u16 = 0x8028;
const A_CONTROLLED: u16 = 0x8029;
const A_CONTROLLING: u16 = 0x802A;

/// Pair checks: one every this often, each pair at most every `RETRY`.
const PACE: Duration = Duration::from_millis(40);
const RETRY: Duration = Duration::from_millis(500);
/// Keep a working pair alive (and keep NAT mappings open).
const CONSENT: Duration = Duration::from_secs(2);

pub fn base64_30bits(n: u32) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    [24, 18, 12, 6, 0].iter().map(|s| T[(n >> s & 63) as usize] as char).collect()
}

// ---------------------------------------------------------------- STUN

#[derive(Debug, Clone, PartialEq)]
pub struct Stun {
    pub kind: u16,
    pub txid: [u8; 12],
    pub attrs: Vec<(u16, Vec<u8>)>,
}

pub fn is_stun(d: &[u8]) -> bool {
    d.len() >= 20 && d[0] < 2 && d[4..8] == COOKIE
}

impl Stun {
    pub fn new(kind: u16) -> Self {
        Stun { kind, txid: super::crypto::random(), attrs: Vec::new() }
    }

    pub fn attr(&self, k: u16) -> Option<&[u8]> {
        self.attrs.iter().find(|a| a.0 == k).map(|a| a.1.as_slice())
    }

    pub fn parse(d: &[u8]) -> Option<Stun> {
        if !is_stun(d) {
            return None;
        }
        let kind = u16::from_be_bytes([d[0], d[1]]);
        let len = u16::from_be_bytes([d[2], d[3]]) as usize;
        let body = d.get(20..20 + len)?;
        let mut attrs = Vec::new();
        let mut at = 0;
        while at + 4 <= body.len() {
            let k = u16::from_be_bytes([body[at], body[at + 1]]);
            let l = u16::from_be_bytes([body[at + 2], body[at + 3]]) as usize;
            attrs.push((k, body.get(at + 4..at + 4 + l)?.to_vec()));
            at += 4 + l.div_ceil(4) * 4;
        }
        Some(Stun { kind, txid: d[8..20].try_into().unwrap(), attrs })
    }

    fn header(&self, len: usize) -> Vec<u8> {
        let mut b = Vec::with_capacity(20 + len);
        b.extend_from_slice(&self.kind.to_be_bytes());
        b.extend_from_slice(&(len as u16).to_be_bytes());
        b.extend_from_slice(&COOKIE);
        b.extend_from_slice(&self.txid);
        b
    }

    /// Wire bytes; with `key`, MESSAGE-INTEGRITY; always FINGERPRINT.
    pub fn encode(&self, key: Option<&[u8]>) -> Vec<u8> {
        let mut body = Vec::new();
        for (k, v) in &self.attrs {
            body.extend_from_slice(&k.to_be_bytes());
            body.extend_from_slice(&(v.len() as u16).to_be_bytes());
            body.extend_from_slice(v);
            body.resize(body.len().div_ceil(4) * 4, 0);
        }
        if let Some(key) = key {
            let mut m = self.header(body.len() + 24);
            m.extend_from_slice(&body);
            let mut h = <Hmac<Sha1> as Mac>::new_from_slice(key).unwrap();
            h.update(&m);
            body.extend_from_slice(&A_INTEGRITY.to_be_bytes());
            body.extend_from_slice(&20u16.to_be_bytes());
            body.extend_from_slice(&h.finalize().into_bytes());
        }
        let mut m = self.header(body.len() + 8);
        m.extend_from_slice(&body);
        let crc = crc32fast::hash(&m) ^ 0x5354_554e;
        m.extend_from_slice(&A_FINGERPRINT.to_be_bytes());
        m.extend_from_slice(&4u16.to_be_bytes());
        m.extend_from_slice(&crc.to_be_bytes());
        m
    }
}

/// Checks MESSAGE-INTEGRITY in raw message `d` with `key`.
pub fn integrity_ok(d: &[u8], key: &[u8]) -> bool {
    let Some(len) = d.get(2..4).map(|l| u16::from_be_bytes([l[0], l[1]]) as usize) else { return false };
    let mut at = 20;
    while at + 4 <= 20 + len && at + 4 <= d.len() {
        let k = u16::from_be_bytes([d[at], d[at + 1]]);
        let l = u16::from_be_bytes([d[at + 2], d[at + 3]]) as usize;
        if k == A_INTEGRITY {
            let Some(mac) = d.get(at + 4..at + 24) else { return false };
            let mut m = d[..at].to_vec();
            let fake = (at + 24 - 20) as u16;
            m[2..4].copy_from_slice(&fake.to_be_bytes());
            let mut h = <Hmac<Sha1> as Mac>::new_from_slice(key).unwrap();
            h.update(&m);
            return h.verify_slice(mac).is_ok();
        }
        at += 4 + l.div_ceil(4) * 4;
    }
    false
}

pub fn xor_address(a: SocketAddr) -> Vec<u8> {
    let SocketAddr::V4(a) = a else { return Vec::new() };
    let port = a.port() ^ 0x2112;
    let ip = u32::from(*a.ip()) ^ u32::from_be_bytes(COOKIE);
    let mut v = vec![0, 1];
    v.extend_from_slice(&port.to_be_bytes());
    v.extend_from_slice(&ip.to_be_bytes());
    v
}

pub fn mapped_address(m: &Stun) -> Option<SocketAddr> {
    if let Some(v) = m.attr(A_XOR_MAPPED).filter(|v| v.len() >= 8 && v[1] == 1) {
        let port = u16::from_be_bytes([v[2], v[3]]) ^ 0x2112;
        let ip = u32::from_be_bytes([v[4], v[5], v[6], v[7]]) ^ u32::from_be_bytes(COOKIE);
        return Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(ip), port)));
    }
    let v = m.attr(A_MAPPED).filter(|v| v.len() >= 8 && v[1] == 1)?;
    Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(v[4], v[5], v[6], v[7]), u16::from_be_bytes([v[2], v[3]]))))
}

/// A plain binding request to a STUN server (our public address).
pub fn server_request() -> Stun {
    Stun::new(BINDING_REQUEST)
}

// ----------------------------------------------------------- candidates

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Host,
    Srflx,
    Prflx,
    Relay,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub addr: SocketAddr,
    pub kind: Kind,
    pub priority: u32,
    pub foundation: u32,
}

impl Kind {
    fn pref(self) -> u32 {
        match self {
            Kind::Host => 126,
            Kind::Prflx => 110,
            Kind::Srflx => 100,
            Kind::Relay => 0,
        }
    }
}

pub fn priority(kind: Kind, local_pref: u32) -> u32 {
    (kind.pref() << 24) | ((local_pref & 0xffff) << 8) | 255
}

impl Candidate {
    pub fn new(addr: SocketAddr, kind: Kind) -> Self {
        let foundation = crc32fast::hash(format!("{kind:?}{}", addr.ip()).as_bytes());
        Candidate { addr, kind, priority: priority(kind, 65535), foundation }
    }

    /// The SDP line Steam's ICE parses.
    pub fn to_sdp(&self) -> String {
        let typ = match self.kind {
            Kind::Host => "host",
            Kind::Srflx => "srflx",
            Kind::Prflx => "prflx",
            Kind::Relay => "relay",
        };
        format!("candidate:{} 0 udp {} {} {} typ {} generation 0", self.foundation, self.priority, self.addr.ip(), self.addr.port(), typ)
    }

    /// An SDP candidate line; only UDP IPv4 ones are useful to us.
    pub fn parse(s: &str) -> Option<Candidate> {
        let s = s.trim().strip_prefix("a=").unwrap_or(s.trim());
        let s = s.strip_prefix("candidate:")?;
        let f: Vec<&str> = s.split_whitespace().collect();
        if f.len() < 8 || !f[2].eq_ignore_ascii_case("udp") || f[6] != "typ" {
            return None;
        }
        let ip: IpAddr = f[4].parse().ok()?;
        if !ip.is_ipv4() {
            return None;
        }
        let kind = match f[7] {
            "host" => Kind::Host,
            "srflx" => Kind::Srflx,
            "prflx" => Kind::Prflx,
            "relay" => Kind::Relay,
            _ => return None,
        };
        Some(Candidate { addr: SocketAddr::new(ip, f[5].parse().ok()?), kind, priority: f[3].parse().ok()?, foundation: f[0].parse().unwrap_or(0) })
    }
}

// ---------------------------------------------------------------- agent

#[derive(Debug)]
struct Pair {
    remote: SocketAddr,
    priority: u32,
    txid: Option<[u8; 12]>,
    last_sent: Option<Instant>,
    tries: u32,
    ok: bool,
    /// They checked us from here (we know it reaches us).
    heard: bool,
}

pub struct Agent {
    pub controlling: bool,
    tiebreaker: u64,
    pub local_ufrag: String,
    pub local_pwd: String,
    pub remote_ufrag: String,
    pub remote_pwd: Option<String>,
    pairs: Vec<Pair>,
    last_check: Option<Instant>,
    /// The pair data goes to.
    pub selected: Option<SocketAddr>,
    pub started: Instant,
    pub remote_kinds: u32,
}

impl Agent {
    pub fn new(controlling: bool, local_conn_id: u32, remote_conn_id: u32, now: Instant) -> Self {
        Agent {
            controlling,
            tiebreaker: u64::from_le_bytes(super::crypto::random()),
            local_ufrag: base64_30bits(local_conn_id),
            local_pwd: base64_30bits(super::crypto::random_u32()),
            remote_ufrag: base64_30bits(remote_conn_id),
            remote_pwd: None,
            pairs: Vec::new(),
            last_check: None,
            selected: None,
            started: now,
            remote_kinds: 0,
        }
    }

    pub fn add_remote(&mut self, c: &Candidate) {
        self.remote_kinds |= 1 << (c.kind as u32);
        if let Some(p) = self.pairs.iter_mut().find(|p| p.remote == c.addr) {
            p.priority = p.priority.max(c.priority);
            return;
        }
        self.pairs.push(Pair { remote: c.addr, priority: c.priority, txid: None, last_sent: None, tries: 0, ok: false, heard: false });
        self.pairs.sort_by_key(|p| std::cmp::Reverse(p.priority));
    }

    fn check(&self, txid: [u8; 12]) -> Option<Vec<u8>> {
        let pwd = self.remote_pwd.as_ref()?;
        let mut m = Stun { kind: BINDING_REQUEST, txid, attrs: Vec::new() };
        m.attrs.push((A_USERNAME, format!("{}:{}", self.remote_ufrag, self.local_ufrag).into_bytes()));
        m.attrs.push((A_PRIORITY, priority(Kind::Prflx, 65535).to_be_bytes().to_vec()));
        if self.controlling {
            m.attrs.push((A_CONTROLLING, self.tiebreaker.to_be_bytes().to_vec()));
            // Aggressive nomination: every check nominates.
            m.attrs.push((A_USE_CANDIDATE, Vec::new()));
        } else {
            m.attrs.push((A_CONTROLLED, self.tiebreaker.to_be_bytes().to_vec()));
        }
        Some(m.encode(Some(pwd.as_bytes())))
    }

    /// Checks to send now: (to, bytes).
    pub fn poll(&mut self, now: Instant) -> Vec<(SocketAddr, Vec<u8>)> {
        let mut out = Vec::new();
        if self.remote_pwd.is_none() || self.last_check.is_some_and(|t| now.duration_since(t) < PACE) {
            return out;
        }
        let selected = self.selected;
        let pick = self
            .pairs
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                let gap = if p.ok { CONSENT } else if p.tries > 20 { Duration::from_secs(3) } else { RETRY };
                p.last_sent.is_none_or(|t| now.duration_since(t) >= gap) && (selected.is_none() || p.ok || p.tries < 6)
            })
            .min_by_key(|(_, p)| (p.last_sent.is_some(), p.last_sent))
            .map(|(i, _)| i);
        if let Some(i) = pick {
            let txid = super::crypto::random();
            if let Some(b) = self.check(txid) {
                let p = &mut self.pairs[i];
                p.txid = Some(txid);
                p.last_sent = Some(now);
                p.tries += 1;
                self.last_check = Some(now);
                out.push((p.remote, b));
            }
        }
        out
    }

    /// A STUN message for this session (its USERNAME names us, or its
    /// transaction is one of our checks). Returns a reply to send.
    pub fn on_stun(&mut self, raw: &[u8], m: &Stun, from: SocketAddr) -> Option<Vec<u8>> {
        match m.kind {
            BINDING_REQUEST => {
                if !integrity_ok(raw, self.local_pwd.as_bytes()) {
                    crate::log!("gns/ice: check from {from} with bad integrity");
                    return None;
                }
                let mut r = Stun { kind: BINDING_SUCCESS, txid: m.txid, attrs: Vec::new() };
                r.attrs.push((A_XOR_MAPPED, xor_address(from)));
                let reply = r.encode(Some(self.local_pwd.as_bytes()));
                // A peer-reflexive candidate, if new.
                if !self.pairs.iter().any(|p| p.remote == from) {
                    let prio = m.attr(A_PRIORITY).and_then(|v| v.try_into().ok()).map(u32::from_be_bytes).unwrap_or(0);
                    self.add_remote(&Candidate { addr: from, kind: Kind::Prflx, priority: prio, foundation: 0 });
                }
                if let Some(p) = self.pairs.iter_mut().find(|p| p.remote == from) {
                    p.heard = true;
                    // Triggered check: answer theirs with ours soon.
                    if !p.ok {
                        p.last_sent = None;
                    }
                }
                if !self.controlling && m.attr(A_USE_CANDIDATE).is_some() && self.selected != Some(from) {
                    crate::log!("gns/ice: peer nominated {from}");
                    self.selected = Some(from);
                }
                Some(reply)
            }
            BINDING_SUCCESS => {
                let p = self.pairs.iter_mut().find(|p| p.txid == Some(m.txid))?;
                if let Some(pwd) = &self.remote_pwd
                    && !integrity_ok(raw, pwd.as_bytes())
                {
                    crate::log!("gns/ice: answer from {from} with bad integrity");
                    return None;
                }
                if !p.ok {
                    crate::log!("gns/ice: pair to {} works", p.remote);
                }
                p.ok = true;
                p.tries = 0;
                let remote = p.remote;
                if self.controlling && self.selected.is_none() {
                    self.selected = Some(remote);
                }
                if !self.controlling && self.selected.is_none() && p.heard {
                    // Both directions work; use it until they nominate.
                    self.selected = Some(remote);
                }
                None
            }
            BINDING_ERROR => {
                if let Some(p) = self.pairs.iter_mut().find(|p| p.txid == Some(m.txid)) {
                    let code = m.attr(A_ERROR).filter(|e| e.len() >= 4).map(|e| e[2] as u32 * 100 + e[3] as u32).unwrap_or(0);
                    crate::log!("gns/ice: check to {} failed ({code})", p.remote);
                    if code == 487 {
                        // Role conflict: flip like RFC 8445 says.
                        self.controlling = !self.controlling;
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Does this STUN message belong to us?
    pub fn owns(&self, m: &Stun) -> bool {
        match m.kind {
            BINDING_REQUEST => m.attr(A_USERNAME).is_some_and(|u| u.starts_with(format!("{}:", self.local_ufrag).as_bytes())),
            _ => self.pairs.iter().any(|p| p.txid == Some(m.txid)),
        }
    }

    pub fn working(&self) -> bool {
        self.selected.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ufrag_and_candidates() {
        assert_eq!(base64_30bits(0), "AAAAA");
        assert_eq!(base64_30bits(0x3fff_ffff), "/////");
        let c = Candidate::parse("candidate:2442523459 0 udp 2122262784 192.168.1.5 63368 typ host generation 0 ufrag WLM82 network-id 2").unwrap();
        assert_eq!((c.addr.port(), c.kind, c.priority), (63368, Kind::Host, 2122262784));
        assert!(Candidate::parse("candidate:1 0 udp 1 2602:801::1 5 typ host").is_none());
        let ours = Candidate::new("1.2.3.4:5".parse().unwrap(), Kind::Srflx);
        assert_eq!(Candidate::parse(&ours.to_sdp()).unwrap().addr, ours.addr);
    }

    #[test]
    fn two_agents_find_each_other() {
        let now = Instant::now();
        let (a_addr, b_addr): (SocketAddr, SocketAddr) = ("10.0.0.1:1000".parse().unwrap(), "10.0.0.2:2000".parse().unwrap());
        let mut a = Agent::new(false, 111, 222, now);
        let mut b = Agent::new(true, 222, 111, now);
        a.remote_pwd = Some(b.local_pwd.clone());
        b.remote_pwd = Some(a.local_pwd.clone());
        a.add_remote(&Candidate::new(b_addr, Kind::Host));
        b.add_remote(&Candidate::new(a_addr, Kind::Host));
        let mut t = now;
        for _ in 0..20 {
            t += Duration::from_millis(50);
            for (to, m) in b.poll(t) {
                assert_eq!(to, a_addr);
                let s = Stun::parse(&m).unwrap();
                assert!(a.owns(&s));
                if let Some(r) = a.on_stun(&m, &s, b_addr) {
                    let rs = Stun::parse(&r).unwrap();
                    assert!(b.owns(&rs));
                    assert_eq!(mapped_address(&rs), Some(b_addr));
                    b.on_stun(&r, &rs, a_addr);
                }
            }
            for (_, m) in a.poll(t) {
                let s = Stun::parse(&m).unwrap();
                if let Some(r) = b.on_stun(&m, &s, a_addr) {
                    let rs = Stun::parse(&r).unwrap();
                    a.on_stun(&r, &rs, b_addr);
                }
            }
        }
        assert_eq!(b.selected, Some(a_addr));
        assert_eq!(a.selected, Some(b_addr));
    }

    #[test]
    fn stun_vectors_rfc5769() {
        // RFC 5769 2.1 sample request.
        let hex = "000100582112a442b7e7a701bc34d686fa87dfae802200105354554e207465737420636c69656e74                   002400046e0001ff80290008932ff9b151263b36000600096576746a3a68367659202020                   000800149aeaa70cbfd8cb56781ef2b5b2d3f249c1b571a280280004e57a3bcf";
        let hex: String = hex.split_whitespace().collect();
        let sample: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        assert!(integrity_ok(&sample, b"VOkJxbRl1RmTxUk/WvJxBt"));
        let n = sample.len();
        assert_eq!(&sample[n - 4..], &(crc32fast::hash(&sample[..n - 8]) ^ 0x5354_554e).to_be_bytes());
        let mut m = Stun::new(BINDING_REQUEST);
        m.attrs.push((A_USERNAME, b"evtj:h6vY".to_vec()));
        let raw = m.encode(Some(b"VOkJxbRl1RmTxUk/WvJxBt"));
        assert!(integrity_ok(&raw, b"VOkJxbRl1RmTxUk/WvJxBt"));
        assert!(!integrity_ok(&raw, b"wrong"));
        let p = Stun::parse(&raw).unwrap();
        assert_eq!(p.attr(A_USERNAME), Some(&b"evtj:h6vY"[..]));
        // Fingerprint is last and correct.
        let n = raw.len();
        let crc = crc32fast::hash(&raw[..n - 8]) ^ 0x5354_554e;
        assert_eq!(&raw[n - 4..], &crc.to_be_bytes());
    }
}

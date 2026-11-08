//! What all SteamNetworkingSockets connections in the game share: one UDP
//! socket for ICE, our public address (STUN), the Valve-signed certificate,
//! and the queue of rendezvous signals going out through SteamInRust.

use super::conn::Io;
use super::crypto::LocalCert;
use super::ice::{self, Candidate, Kind, Stun};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// STUN servers for our public address (Valve's first, like Steam).
const STUN_SERVERS: [&str; 3] = ["stun.steampowered.com:3478", "stun.l.google.com:19302", "stun.cloudflare.com:3478"];

pub struct Engine {
    pub sock: Option<Arc<UdpSocket>>,
    host: Vec<SocketAddr>,
    srflx: Option<SocketAddr>,
    /// STUN server addresses, looked up on a thread of their own (DNS can
    /// be slow, and the network state is locked while we poll).
    stun_servers: Arc<Mutex<Vec<SocketAddr>>>,
    stun_txid: Option<[u8; 12]>,
    /// Where the last request went (and, once one answers, where they keep going).
    stun_to: Option<SocketAddr>,
    stun_sent: Option<Instant>,
    stun_tries: u32,
}

impl Engine {
    pub fn start() -> Engine {
        let sock = UdpSocket::bind("0.0.0.0:0").ok().map(Arc::new);
        let port = sock.as_ref().and_then(|s| s.local_addr().ok()).map(|a| a.port()).unwrap_or(0);
        let host = local_ips().into_iter().map(|ip| SocketAddr::new(ip.into(), port)).collect::<Vec<_>>();
        crate::log!("gns: UDP port {port}, host addresses {host:?}");
        let stun_servers = Arc::new(Mutex::new(Vec::new()));
        let found = stun_servers.clone();
        std::thread::spawn(move || {
            for name in STUN_SERVERS {
                if let Some(a) = name.to_socket_addrs().ok().and_then(|mut a| a.find(|a| a.is_ipv4())) {
                    found.lock().unwrap_or_else(|e| e.into_inner()).push(a);
                }
            }
        });
        Engine { sock, host, srflx: None, stun_servers, stun_txid: None, stun_to: None, stun_sent: None, stun_tries: 0 }
    }

    pub fn port(&self) -> u16 {
        self.sock.as_ref().and_then(|s| s.local_addr().ok()).map(|a| a.port()).unwrap_or(0)
    }

    pub fn candidates(&self) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = self.host.iter().map(|a| Candidate::new(*a, Kind::Host)).collect();
        if let Some(s) = self.srflx.filter(|s| !self.host.contains(s)) {
            out.push(Candidate::new(s, Kind::Srflx));
        }
        out
    }

    /// Ask a STUN server for our public address (and refresh it now and then).
    pub fn poll_stun(&mut self, now: Instant) {
        let Some(sock) = &self.sock else { return };
        let gap = if self.srflx.is_some() { Duration::from_secs(25) } else if self.stun_tries < 6 { Duration::from_millis(700) } else { Duration::from_secs(10) };
        if self.stun_sent.is_some_and(|t| now.duration_since(t) < gap) {
            return;
        }
        // The first server that answers keeps being used; while none has,
        // move to the next one every three tries.
        let server = match (self.srflx, self.stun_to) {
            (Some(_), Some(to)) => to,
            _ => {
                let list = self.stun_servers.lock().unwrap_or_else(|e| e.into_inner());
                if list.is_empty() {
                    return;
                }
                list[(self.stun_tries as usize / 3) % list.len()]
            }
        };
        self.stun_to = Some(server);
        let m = ice::server_request();
        self.stun_txid = Some(m.txid);
        let _ = sock.send_to(&m.encode(None), server);
        self.stun_sent = Some(now);
        self.stun_tries += 1;
    }

    /// A STUN server's answer? (Ours: takes it.)
    pub fn on_stun(&mut self, m: &Stun) -> bool {
        if self.stun_txid != Some(m.txid) {
            return false;
        }
        if let Some(a) = ice::mapped_address(m) {
            if self.srflx != Some(a) {
                crate::log!("gns: public address {a}");
            }
            self.srflx = Some(a);
        }
        true
    }

    pub fn io<'a>(&'a self, out: &'a mut Vec<(u64, Vec<u8>)>) -> EngineIo<'a> {
        EngineIo { e: self, signals: out }
    }
}

/// The connections' view of the engine while the network state is locked:
/// datagrams go out right away, signals are collected and sent afterwards.
pub struct EngineIo<'a> {
    e: &'a Engine,
    signals: &'a mut Vec<(u64, Vec<u8>)>,
}

impl Io for EngineIo<'_> {
    fn udp(&mut self, to: SocketAddr, data: &[u8]) {
        if let Some(s) = &self.e.sock {
            let _ = s.send_to(data, to);
        }
    }
    fn signal(&mut self, peer: u64, data: Vec<u8>) {
        self.signals.push((peer, data));
    }
    fn candidates(&self) -> Vec<Candidate> {
        self.e.candidates()
    }
    fn unix_time(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
}

/// This PC's IPv4 addresses (the one used for the internet first).
fn local_ips() -> Vec<std::net::Ipv4Addr> {
    let mut out = Vec::new();
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0")
        && s.connect("8.8.8.8:53").is_ok()
        && let Ok(SocketAddr::V4(a)) = s.local_addr()
    {
        out.push(*a.ip());
    }
    out
}

// ------------------------------------------------------------- signals

static SIGNALS: OnceLock<Mutex<Sender<(u64, Vec<u8>)>>> = OnceLock::new();

/// Send rendezvous signals through SteamInRust, in order, off the caller's thread.
pub fn send_signals(list: Vec<(u64, Vec<u8>)>) {
    if list.is_empty() {
        return;
    }
    let tx = SIGNALS.get_or_init(|| {
        let (tx, rx) = channel::<(u64, Vec<u8>)>();
        std::thread::spawn(move || {
            while let Ok((to, data)) = rx.recv() {
                let _ = crate::ipc::request(crate::protocol::Request::Rendezvous { to, data });
            }
        });
        Mutex::new(tx)
    });
    let tx = tx.lock().unwrap_or_else(|e| e.into_inner());
    for s in list {
        let _ = tx.send(s);
    }
}

// --------------------------------------------------------- certificate

struct CertSlot {
    cert: Mutex<Option<Arc<LocalCert>>>,
    ready: Condvar,
    started: Mutex<bool>,
}

static CERT: OnceLock<CertSlot> = OnceLock::new();

fn slot() -> &'static CertSlot {
    CERT.get_or_init(|| CertSlot { cert: Mutex::new(None), ready: Condvar::new(), started: Mutex::new(false) })
}

/// Start getting our certificate from Steam (once).
pub fn fetch_cert(me: u64, app_id: u32) {
    let s = slot();
    {
        let mut started = s.started.lock().unwrap_or_else(|e| e.into_inner());
        if *started {
            return;
        }
        *started = true;
    }
    std::thread::spawn(move || {
        let key = LocalCert::new_key();
        let public_key = key.verifying_key().to_bytes().to_vec();
        let started = Instant::now();
        let cert = match crate::ipc::request(crate::protocol::Request::NetworkingCert { public_key }) {
            Some(crate::protocol::Reply::NetworkingCert { cert, ca_key_id, ca_signature }) => {
                let c = LocalCert::signed(key, cert, ca_key_id, ca_signature);
                let parsed = prost::Message::decode(c.cert_bytes()).ok().map(|x: super::msg::Cert| (super::crypto::cert_steam_id(&x), x.app_ids.clone(), x.time_expiry));
                crate::log!("gns: Steam signed our networking cert in {} ms (CA {}, {:?})", started.elapsed().as_millis(), ca_key_id, parsed);
                c
            }
            other => {
                crate::log!("gns: no signed cert ({other:?}); using an unsigned one (only SteamInRust players accept it)");
                LocalCert::unsigned(key, me, app_id)
            }
        };
        *s.cert.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(cert));
        s.ready.notify_all();
    });
}

/// Our certificate, waiting up to `wait` for Steam's answer.
pub fn cert(me: u64, app_id: u32, wait: Duration) -> Arc<LocalCert> {
    fetch_cert(me, app_id);
    let s = slot();
    let g = s.cert.lock().unwrap_or_else(|e| e.into_inner());
    let (g, _) = s.ready.wait_timeout_while(g, wait, |c| c.is_none()).unwrap_or_else(|e| e.into_inner());
    match g.as_ref() {
        Some(c) => c.clone(),
        None => Arc::new(LocalCert::unsigned(LocalCert::new_key(), me, app_id)),
    }
}

pub fn cert_ready() -> Option<Arc<LocalCert>> {
    slot().cert.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

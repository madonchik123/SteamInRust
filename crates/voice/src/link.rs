//! The encrypted audio connection to a voice server.

use crate::sdp::{AUDIO_MID, Description, LocalOffer, Setup};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};
use str0m::crypto::dtls::DtlsVersion;
use str0m::format::Codec;
use str0m::media::{MediaKind, Mid};
use str0m::net::{Protocol, Receive};
use str0m::rtp::{RtpWrite, Ssrc};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

/// How far a connection got, so a failure says where it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reached {
    /// Sending reachability checks, no reply yet.
    Checking,
    /// The server replied; the encryption handshake is running.
    Reachable,
}

/// A connection that is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connected {
    pub local: SocketAddr,
    pub took: Duration,
}

/// What happened on the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    /// Reachable and encrypted.
    Connected,
    /// Someone's audio.
    Audio { ssrc: u32, seq: u64, payload: Vec<u8> },
    /// The server stopped answering (it may come back).
    Lost,
    /// It's answering again.
    Back,
}

/// One connection to a voice server.
pub struct Link {
    rtc: Rtc,
    mid: Mid,
    ssrc: Ssrc,
    opus_pt: u8,
    socket: Option<(UdpSocket, SocketAddr)>,
    reached: Reached,
    connected: bool,
    speakers: Vec<u32>,
}

impl Link {
    pub fn new() -> Self {
        let crypto = Arc::new(str0m::crypto::from_feature_flags());
        let mut rtc = Rtc::builder()
            .set_rtp_mode(true)
            .clear_codecs()
            .enable_opus(true, false)
            // What Steam's client speaks.
            .set_dtls_version(DtlsVersion::Dtls12)
            .set_crypto_provider(crypto)
            .build(Instant::now());
        let opus_pt = *rtc.codec_config().find(|p| p.spec().codec == Codec::Opus).expect("Opus was just enabled").pt();
        let mid: Mid = AUDIO_MID.into();
        let mut api = rtc.direct_api();
        api.declare_media(mid, MediaKind::Audio);
        let ssrc = api.new_ssrc();
        api.declare_stream_tx(ssrc, None, mid, None);
        Self { rtc, mid, ssrc, opus_pt, socket: None, reached: Reached::Checking, connected: false, speakers: Vec::new() }
    }

    /// The offer to send to `WebRTCClient.InitiateWebRTCConnection`.
    pub fn offer(&mut self) -> String {
        let api = self.rtc.direct_api();
        let creds = api.local_ice_credentials();
        let fingerprint = api.local_dtls_fingerprint().to_string();
        let ssrc: u32 = *self.ssrc;
        let session_id = (ssrc as u64) << 16 | std::process::id() as u64 & 0xffff;
        LocalOffer { session_id, ufrag: creds.ufrag, pwd: creds.pass, fingerprint, opus_pt: self.opus_pt, ssrc }.to_json()
    }

    /// Take the server's answer and open the socket towards it.
    pub fn apply_answer(&mut self, answer: &Description) -> Result<(), String> {
        if answer.kind != "answer" {
            return Err(format!("expected an answer, got {}", answer.kind));
        }
        if answer.opus_pt != Some(self.opus_pt) {
            return Err(format!("the server chose audio format {:?}, we offered {}", answer.opus_pt, self.opus_pt));
        }
        let candidates: Vec<Candidate> = answer
            .candidates
            .iter()
            .filter_map(|c| Candidate::from_sdp_string(c).ok())
            .filter(|c| c.proto() == Protocol::Udp && c.addr().is_ipv4())
            .collect();
        let first = candidates.first().ok_or("the server gave no address we can reach")?.addr();

        // Ask the system which of our addresses reaches the server.
        let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
        probe.connect(first).map_err(|e| e.to_string())?;
        let ip: IpAddr = probe.local_addr().map_err(|e| e.to_string())?.ip();
        let socket = UdpSocket::bind((ip, 0)).map_err(|e| e.to_string())?;
        let local = socket.local_addr().map_err(|e| e.to_string())?;

        let host = Candidate::host(local, "udp").map_err(|e| e.to_string())?;
        self.rtc.add_local_candidate(host);
        for c in candidates {
            self.rtc.add_remote_candidate(c);
        }
        let fingerprint = answer.fingerprint.parse().map_err(|e: String| format!("bad fingerprint: {e}"))?;
        let mut api = self.rtc.direct_api();
        api.set_remote_fingerprint(fingerprint);
        api.set_remote_ice_credentials(str0m::IceCreds { ufrag: answer.ufrag.clone(), pass: answer.pwd.clone() });
        // We made the offer, so we lead the reachability checks.
        api.set_ice_controlling(true);
        // Whoever isn't "active" waits for the other to start the handshake.
        api.start_dtls(answer.setup != Setup::Active).map_err(|e| e.to_string())?;
        self.socket = Some((socket, local));
        self.expect_speakers(&answer.ssrcs);
        Ok(())
    }

    /// Run until the connection is up, or say how far it got.
    pub fn wait_connected(&mut self, timeout: Duration) -> Result<Connected, String> {
        let start = Instant::now();
        let deadline = start + timeout;
        let mut events = Vec::new();
        loop {
            self.step(Duration::from_millis(50), &mut events)?;
            events.clear();
            if self.connected {
                let local = self.socket.as_ref().ok_or("no answer applied yet")?.1;
                return Ok(Connected { local, took: start.elapsed() });
            }
            if Instant::now() >= deadline {
                return Err(self.how_far());
            }
        }
    }

    /// Why a connection that never came up failed.
    pub fn how_far(&self) -> String {
        match self.reached {
            Reached::Checking => "the voice server never replied (UDP may be blocked)".to_string(),
            Reached::Reachable => "the voice server replied, but the encryption handshake didn't finish".to_string(),
        }
    }

    /// The stream number our own audio goes out on.
    pub fn ssrc(&self) -> u32 {
        *self.ssrc
    }

    /// The people we should expect audio from, replacing the last list.
    pub fn expect_speakers(&mut self, ssrcs: &[u32]) {
        let mut api = self.rtc.direct_api();
        for gone in self.speakers.iter().filter(|s| !ssrcs.contains(s)) {
            api.remove_stream_rx((*gone).into());
        }
        for ssrc in ssrcs.iter().filter(|s| **s != *self.ssrc) {
            api.expect_stream_rx((*ssrc).into(), None, self.mid, None);
        }
        self.speakers = ssrcs.to_vec();
    }

    /// Queue one Opus packet to send.
    pub fn send_audio(&mut self, seq: u64, time: u32, payload: &[u8]) {
        let (ssrc, pt) = (self.ssrc, self.opus_pt.into());
        if let Some(stream) = self.rtc.direct_api().stream_tx(&ssrc) {
            stream.write_rtp(RtpWrite::new(pt, seq.into(), time, Instant::now(), payload.to_vec()));
        }
    }

    /// Do one round of network work, waiting at most `max_wait` for a packet.
    pub fn step(&mut self, max_wait: Duration, events: &mut Vec<LinkEvent>) -> Result<(), String> {
        let (socket, local) = self.socket.take().ok_or("no answer applied yet")?;
        let result = self.step_on(&socket, local, max_wait, events);
        self.socket = Some((socket, local));
        result
    }

    fn step_on(&mut self, socket: &UdpSocket, local: SocketAddr, max_wait: Duration, events: &mut Vec<LinkEvent>) -> Result<(), String> {
        let next = self.drain(socket, events)?;
        let now = Instant::now();
        if next <= now {
            self.rtc.handle_input(Input::Timeout(now)).map_err(|e| e.to_string())?;
            self.drain(socket, events)?;
            return Ok(());
        }
        let wait = next.saturating_duration_since(now).min(max_wait).max(Duration::from_millis(1));
        socket.set_read_timeout(Some(wait)).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 2000];
        let input = match socket.recv_from(&mut buf) {
            Ok((n, source)) => match (&buf[..n]).try_into() {
                Ok(contents) => Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: local, contents }),
                Err(_) => return Ok(()), // not a packet type this connection uses
            },
            // Windows reports an unreachable port as a reset; keep trying.
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionReset) => Input::Timeout(Instant::now()),
            Err(e) => return Err(e.to_string()),
        };
        if self.rtc.accepts(&input) {
            self.rtc.handle_input(input).map_err(|e| e.to_string())?;
        }
        self.drain(socket, events)?;
        Ok(())
    }

    /// Send what's waiting and note what happened. The library holds one
    /// received packet until polled, so this runs after every input.
    fn drain(&mut self, socket: &UdpSocket, events: &mut Vec<LinkEvent>) -> Result<Instant, String> {
        loop {
            match self.rtc.poll_output().map_err(|e| e.to_string())? {
                Output::Timeout(at) => return Ok(at),
                Output::Transmit(t) => {
                    if let Err(e) = socket.send_to(&t.contents, t.destination) {
                        tracing::debug!("voice: send failed: {e}");
                    }
                }
                Output::Event(Event::Connected) => {
                    self.connected = true;
                    events.push(LinkEvent::Connected);
                }
                Output::Event(Event::IceConnectionStateChange(state)) => match state {
                    IceConnectionState::Connected | IceConnectionState::Completed => {
                        self.reached = Reached::Reachable;
                        if self.connected {
                            events.push(LinkEvent::Back);
                        }
                    }
                    IceConnectionState::Disconnected if self.connected => events.push(LinkEvent::Lost),
                    _ => {}
                },
                Output::Event(Event::RtpPacket(p)) => events.push(LinkEvent::Audio { ssrc: *p.header.ssrc, seq: *p.seq_no, payload: p.payload.to_vec() }),
                Output::Event(_) => {}
            }
        }
    }
}

impl Default for Link {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdp::wrap;
    use std::sync::mpsc;
    use std::thread;

    /// A stand-in for Valve's server on this machine: reads our real offer,
    /// answers in Steam's layout and completes the real handshake over UDP.
    fn fake_server(offer: &str, setup: &str) -> (String, thread::JoinHandle<bool>) {
        let offer = Description::from_json(offer).unwrap();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let local = socket.local_addr().unwrap();
        let mut rtc = Rtc::builder().set_rtp_mode(true).clear_codecs().enable_opus(true, false).set_dtls_version(DtlsVersion::Dtls12).set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags())).build(Instant::now());
        rtc.add_local_candidate(Candidate::host(local, "udp").unwrap());
        let mut api = rtc.direct_api();
        let creds = api.local_ice_credentials();
        let fingerprint = api.local_dtls_fingerprint().to_string();
        api.set_remote_ice_credentials(str0m::IceCreds { ufrag: offer.ufrag, pass: offer.pwd });
        api.set_remote_fingerprint(offer.fingerprint.parse().unwrap());
        api.set_ice_controlling(false);
        api.start_dtls(setup == "active").unwrap();
        let answer = format!(
            "v=0\r\ns=-\r\na=ice-lite\r\nm=audio {port} UDP/TLS/RTP/SAVPF 111\r\na=candidate:1 1 udp 2130706431 127.0.0.1 {port} typ host\r\na=ice-ufrag:{u}\r\na=ice-pwd:{p}\r\na=fingerprint:{fingerprint}\r\na=setup:{setup}\r\na=mid:audio\r\na=rtpmap:111 opus/48000/2\r\na=ssrc:1001 cname:a\r\na=ssrc:2002 cname:b\r\n",
            port = local.port(),
            u = creds.ufrag,
            p = creds.pass,
        );
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut buf = vec![0u8; 2000];
            let mut up = false;
            while Instant::now() < deadline {
                let next = loop {
                    match rtc.poll_output().unwrap() {
                        Output::Timeout(at) => break at,
                        Output::Transmit(t) => {
                            let _ = socket.send_to(&t.contents, t.destination);
                        }
                        Output::Event(Event::Connected) => up = true,
                        Output::Event(_) => {}
                    }
                };
                if up {
                    // Keep answering briefly so the client finishes too.
                    if socket.set_read_timeout(Some(Duration::from_millis(100))).is_ok()
                        && let Ok((n, source)) = socket.recv_from(&mut buf)
                        && let Ok(contents) = (&buf[..n]).try_into()
                    {
                        let _ = rtc.handle_input(Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: local, contents }));
                        continue;
                    }
                    return true;
                }
                let wait = next.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
                socket.set_read_timeout(Some(wait)).unwrap();
                let input = match socket.recv_from(&mut buf) {
                    Ok((n, source)) => match (&buf[..n]).try_into() {
                        Ok(contents) => Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: local, contents }),
                        Err(_) => continue,
                    },
                    Err(_) => Input::Timeout(Instant::now()),
                };
                if rtc.accepts(&input) {
                    rtc.handle_input(input).unwrap();
                }
            }
            up
        });
        (wrap("answer", &answer), server)
    }

    fn connects_when_server_is(setup: &str) {
        let mut link = Link::new();
        let (answer, server) = fake_server(&link.offer(), setup);
        let answer = Description::from_json(&answer).unwrap();
        assert!(answer.ice_lite);
        link.apply_answer(&answer).unwrap();
        let up = link.wait_connected(Duration::from_secs(8)).unwrap();
        assert!(up.local.ip().is_loopback());
        assert!(server.join().unwrap(), "the server side finished too");
    }

    #[test]
    fn connects_when_the_server_waits_for_us() {
        connects_when_server_is("passive");
    }

    #[test]
    fn connects_when_the_server_starts_the_handshake() {
        connects_when_server_is("active");
    }

    #[test]
    fn a_silent_server_is_reported_as_unreachable() {
        let mut link = Link::new();
        let (tx, rx) = mpsc::channel();
        // A socket that receives but never replies.
        let silent = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = silent.local_addr().unwrap().port();
        let hold = thread::spawn(move || {
            let _ = rx.recv();
            drop(silent);
        });
        let answer = format!("a=candidate:1 1 udp 2130706431 127.0.0.1 {port} typ host\r\na=ice-ufrag:abcd\r\na=ice-pwd:abcdefghijklmnopqrstuvwx\r\na=fingerprint:sha-256 {fp}\r\na=setup:passive\r\na=rtpmap:111 opus/48000/2\r\n", fp = vec!["AB"; 32].join(":"));
        link.apply_answer(&Description::parse("answer", &answer).unwrap()).unwrap();
        let err = link.wait_connected(Duration::from_millis(600)).unwrap_err();
        assert!(err.contains("never replied"), "{err}");
        tx.send(()).unwrap();
        hold.join().unwrap();
    }

    #[test]
    fn refuses_something_that_is_not_an_answer() {
        let mut link = Link::new();
        let offer = Description::from_json(&link.offer()).unwrap();
        assert!(link.apply_answer(&offer).unwrap_err().contains("expected an answer"));
    }
}

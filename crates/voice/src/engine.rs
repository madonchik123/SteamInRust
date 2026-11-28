//! Runs a call's audio: microphone to the server, the server to the speakers.

use crate::audio::{Backend, Stream, System};
use crate::codec::{Encoder, FRAME};
use crate::link::{Link, LinkEvent};
use crate::mix::Mixer;
use crate::ptt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Gives up if the connection doesn't come up in this long.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the server may stay silent before the call counts as dropped.
const LOST_TIMEOUT: Duration = Duration::from_secs(15);
/// During silence, still send a tiny packet this often (every 400 ms), the
/// way Steam's client does, so the server knows we're here.
const SILENCE_KEEPALIVE: u32 = 20;
/// Louder than this counts as you talking.
const TALKING_LEVEL: f32 = 0.012;
const TALKING_HOLD: u32 = 15;
/// Stands for "you" in the list of who's talking.
pub const ME: u32 = 0;

/// What a call starts with.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// `None` is the system default.
    pub input: Option<String>,
    pub output: Option<String>,
    pub input_gain: f32,
    pub output_gain: f32,
    pub muted: bool,
    pub deaf: bool,
    /// The push-to-talk key, when it's on.
    pub push_to_talk: Option<String>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { input: None, output: None, input_gain: 1.0, output_gain: 1.0, muted: false, deaf: false, push_to_talk: None }
    }
}

/// What the engine tells the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    /// The audio connection is up.
    Connected,
    /// There's no usable microphone: you can listen but not talk.
    NoMic(String),
    /// There's no usable output device.
    NoSpeakers(String),
    /// Who's talking changed (stream numbers, [`ME`] for you).
    Talking(Vec<u32>),
    /// The connection is gone for good.
    Failed(String),
}

pub type OnEvent = Box<dyn Fn(EngineEvent) + Send>;

enum Control {
    Speakers(Vec<u32>),
    PushToTalk(Option<String>),
    InputGain(f32),
    Input(Option<String>),
    Output(Option<String>),
}

struct Shared {
    stop: AtomicBool,
    muted: AtomicBool,
    /// Off until the call is really joined, so nothing is sent while ringing.
    transmit: AtomicBool,
    mixer: Mutex<Mixer>,
}

/// A running call's audio. Dropping it stops everything.
pub struct Engine {
    shared: Arc<Shared>,
    control: mpsc::Sender<Control>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    /// Start with the real microphone and speakers. `link` already has the
    /// server's answer.
    pub fn start(link: Link, config: EngineConfig, on_event: OnEvent) -> Self {
        Self::start_with(link, config, Box::new(System), on_event)
    }

    pub fn start_with(link: Link, config: EngineConfig, backend: Box<dyn Backend>, on_event: OnEvent) -> Self {
        let mut mixer = Mixer::new(config.output_gain);
        mixer.set_deaf(config.deaf);
        let shared = Arc::new(Shared { stop: AtomicBool::new(false), muted: AtomicBool::new(config.muted), transmit: AtomicBool::new(false), mixer: Mutex::new(mixer) });
        let (control, controls) = mpsc::channel();
        let state = shared.clone();
        let thread = std::thread::Builder::new().name("voice".into()).spawn(move || run(link, config, backend, state, controls, on_event)).expect("starting the voice thread");
        Self { shared, control, thread: Some(thread) }
    }

    /// Start or stop sending the microphone (only once really in the call).
    pub fn set_transmit(&self, on: bool) {
        self.shared.transmit.store(on, Ordering::Relaxed);
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Relaxed);
    }

    pub fn set_deaf(&self, deaf: bool) {
        self.shared.mixer.lock().unwrap().set_deaf(deaf);
    }

    pub fn set_output_gain(&self, gain: f32) {
        self.shared.mixer.lock().unwrap().set_master(gain);
    }

    pub fn set_input_gain(&self, gain: f32) {
        let _ = self.control.send(Control::InputGain(gain));
    }

    /// One person's volume, by their stream number.
    pub fn set_person_gain(&self, ssrc: u32, gain: f32) {
        self.shared.mixer.lock().unwrap().set_gain(ssrc, gain);
    }

    /// Who's in the call now, by stream number.
    pub fn set_speakers(&self, ssrcs: Vec<u32>) {
        let _ = self.control.send(Control::Speakers(ssrcs));
    }

    pub fn set_push_to_talk(&self, key: Option<String>) {
        let _ = self.control.send(Control::PushToTalk(key));
    }

    pub fn set_input_device(&self, device: Option<String>) {
        let _ = self.control.send(Control::Input(device));
    }

    pub fn set_output_device(&self, device: Option<String>) {
        let _ = self.control.send(Control::Output(device));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn open_mic(backend: &dyn Backend, device: Option<&str>, tx: SyncSender<Vec<f32>>) -> Result<Box<dyn Stream>, String> {
    backend.open_input(
        device,
        Box::new(move |frame| {
            // If the network side is stuck, drop audio rather than pile it up.
            if let Err(TrySendError::Disconnected(_)) = tx.try_send(frame.to_vec()) {
                tracing::debug!("voice: microphone has nowhere to go");
            }
        }),
    )
}

fn open_speakers(backend: &dyn Backend, device: Option<&str>, shared: &Arc<Shared>) -> Result<Box<dyn Stream>, String> {
    let shared = shared.clone();
    backend.open_output(device, Box::new(move |out| shared.mixer.lock().unwrap().pull(out)))
}

/// Turns microphone frames into packets.
struct Sender {
    encoder: Encoder,
    seq: u64,
    time: u32,
    gain: f32,
    silent_run: u32,
    talking: u32,
}

impl Sender {
    fn new(gain: f32, ssrc: u32) -> Result<Self, String> {
        // A start that isn't predictable, as RTP asks for.
        let seq = (ssrc as u64 ^ std::process::id() as u64) & 0x3fff;
        Ok(Self { encoder: Encoder::new()?, seq, time: ssrc.rotate_left(13), gain, silent_run: 0, talking: 0 })
    }

    /// One frame; `open` is false while muted or push-to-talk isn't held.
    fn frame(&mut self, pcm: &[f32], open: bool, link: &mut Link) {
        let mut out = [0f32; FRAME];
        if open {
            let mut power = 0.0;
            for (o, s) in out.iter_mut().zip(pcm) {
                *o = (*s * self.gain).clamp(-1.0, 1.0);
                power += *o * *o;
            }
            if (power / FRAME as f32).sqrt() > TALKING_LEVEL {
                self.talking = TALKING_HOLD;
            } else {
                self.talking = self.talking.saturating_sub(1);
            }
        } else {
            self.talking = 0;
        }
        let mut packet = [0u8; 500];
        let n = match self.encoder.encode(&out, &mut packet) {
            Ok(n) => n,
            Err(e) => {
                tracing::debug!("voice: {e}");
                return;
            }
        };
        let time = self.time;
        self.time = self.time.wrapping_add(FRAME as u32);
        // A packet of 1 or 2 bytes means "still silent".
        if n <= 2 {
            self.silent_run += 1;
            if self.silent_run % SILENCE_KEEPALIVE != 1 {
                return;
            }
        } else {
            self.silent_run = 0;
        }
        link.send_audio(self.seq, time, &packet[..n]);
        self.seq += 1;
    }
}

fn run(mut link: Link, config: EngineConfig, backend: Box<dyn Backend>, shared: Arc<Shared>, controls: Receiver<Control>, on_event: OnEvent) {
    let mut sender = match Sender::new(config.input_gain, link.ssrc()) {
        Ok(s) => s,
        Err(e) => {
            on_event(EngineEvent::Failed(e));
            return;
        }
    };
    let (mic_tx, mic_rx) = mpsc::sync_channel::<Vec<f32>>(10);
    let mut mic = match open_mic(backend.as_ref(), config.input.as_deref(), mic_tx.clone()) {
        Ok(s) => Some(s),
        Err(e) => {
            on_event(EngineEvent::NoMic(e));
            None
        }
    };
    let mut speakers = match open_speakers(backend.as_ref(), config.output.as_deref(), &shared) {
        Ok(s) => Some(s),
        Err(e) => {
            on_event(EngineEvent::NoSpeakers(e));
            None
        }
    };

    let started = Instant::now();
    let mut connected = false;
    let mut lost_at: Option<Instant> = None;
    let mut key = config.push_to_talk.clone();
    let mut talking: Vec<u32> = Vec::new();
    let mut checked = Instant::now();
    let mut events = Vec::new();
    while !shared.stop.load(Ordering::Relaxed) {
        while let Ok(c) = controls.try_recv() {
            match c {
                Control::Speakers(list) => {
                    link.expect_speakers(&list);
                    shared.mixer.lock().unwrap().retain(&list);
                }
                Control::PushToTalk(k) => key = k,
                Control::InputGain(g) => sender.gain = g.clamp(0.0, 4.0),
                Control::Input(device) => {
                    mic = None; // close the old one first
                    while mic_rx.try_recv().is_ok() {}
                    match open_mic(backend.as_ref(), device.as_deref(), mic_tx.clone()) {
                        Ok(s) => mic = Some(s),
                        Err(e) => on_event(EngineEvent::NoMic(e)),
                    }
                }
                Control::Output(device) => {
                    speakers = None;
                    match open_speakers(backend.as_ref(), device.as_deref(), &shared) {
                        Ok(s) => speakers = Some(s),
                        Err(e) => on_event(EngineEvent::NoSpeakers(e)),
                    }
                }
            }
        }

        let transmit = connected && shared.transmit.load(Ordering::Relaxed);
        let open = transmit && !shared.muted.load(Ordering::Relaxed) && key.as_deref().is_none_or(ptt::is_down);
        while let Ok(frame) = mic_rx.try_recv() {
            if transmit {
                sender.frame(&frame, open, &mut link);
            }
        }
        if !transmit {
            sender.talking = 0;
        }

        if let Err(e) = link.step(Duration::from_millis(4), &mut events) {
            on_event(EngineEvent::Failed(e));
            break;
        }
        for e in events.drain(..) {
            match e {
                LinkEvent::Connected => {
                    connected = true;
                    on_event(EngineEvent::Connected);
                }
                LinkEvent::Audio { ssrc, seq, payload } => shared.mixer.lock().unwrap().push(ssrc, seq, &payload),
                LinkEvent::Lost => lost_at = lost_at.or(Some(Instant::now())),
                LinkEvent::Back => lost_at = None,
            }
        }
        if !connected && started.elapsed() > CONNECT_TIMEOUT {
            on_event(EngineEvent::Failed(link.how_far()));
            break;
        }
        if lost_at.is_some_and(|t| t.elapsed() > LOST_TIMEOUT) {
            on_event(EngineEvent::Failed("the connection to the voice server was lost".into()));
            break;
        }
        if checked.elapsed() >= Duration::from_millis(100) {
            checked = Instant::now();
            let mut now = shared.mixer.lock().unwrap().talking();
            if sender.talking > 0 {
                now.insert(0, ME);
            }
            if now != talking {
                talking = now.clone();
                on_event(EngineEvent::Talking(now));
            }
        }
    }
    drop(mic);
    drop(speakers);
}

/// Maps people to their stream numbers (Steam tells us who is who).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roster(HashMap<u32, u32>);

impl Roster {
    /// Replace the list: (stream number, account id).
    pub fn set(&mut self, pairs: impl IntoIterator<Item = (u32, u32)>) {
        self.0 = pairs.into_iter().collect();
    }

    pub fn ssrcs(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.0.keys().copied().collect();
        v.sort_unstable();
        v
    }

    pub fn account(&self, ssrc: u32) -> Option<u32> {
        self.0.get(&ssrc).copied()
    }

    pub fn ssrc_of(&self, account: u32) -> Option<u32> {
        self.0.iter().find(|(_, a)| **a == account).map(|(s, _)| *s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{OnFrame, Pull};
    use crate::codec::{pitch, tone};
    use crate::sdp::{Description, wrap};
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
    use str0m::crypto::dtls::DtlsVersion;
    use str0m::format::Codec;
    use str0m::media::{MediaKind, Mid};
    use str0m::net::{Protocol, Receive};
    use str0m::rtp::{RtpWrite, Ssrc};
    use str0m::{Candidate, Event, IceCreds, Input, Output, Rtc};

    const ECHO_SSRC: u32 = 5001;

    /// Plays a 440 Hz tone into the "microphone" and keeps what reaches the
    /// "speakers".
    struct FakeAudio {
        heard: Arc<Mutex<Vec<f32>>>,
        mic_works: bool,
    }

    struct FakeStream(Arc<AtomicBool>, Option<JoinHandle<()>>);

    impl Stream for FakeStream {}

    impl Drop for FakeStream {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
            if let Some(t) = self.1.take() {
                let _ = t.join();
            }
        }
    }

    fn every_20ms(mut f: impl FnMut(usize) + Send + 'static) -> Box<dyn Stream> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let t = std::thread::spawn(move || {
            let start = Instant::now();
            let mut n = 0;
            while !flag.load(Ordering::Relaxed) {
                f(n);
                n += 1;
                let next = start + Duration::from_millis(20) * n as u32;
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
            }
        });
        Box::new(FakeStream(stop, Some(t)))
    }

    impl Backend for FakeAudio {
        fn open_input(&self, _device: Option<&str>, mut on_frame: OnFrame) -> Result<Box<dyn Stream>, String> {
            if !self.mic_works {
                return Err("no microphone".into());
            }
            Ok(every_20ms(move |n| on_frame(&tone(n, 440.0))))
        }

        fn open_output(&self, _device: Option<&str>, mut pull: Pull) -> Result<Box<dyn Stream>, String> {
            let heard = self.heard.clone();
            Ok(every_20ms(move |_| {
                let mut f = [0f32; FRAME];
                pull(&mut f);
                heard.lock().unwrap().extend_from_slice(&f);
            }))
        }
    }

    /// A stand-in for Valve's server: sends everything it gets from the
    /// client back under another person's stream number.
    fn echo_server(offer: &str, stop: Arc<AtomicBool>) -> (String, JoinHandle<usize>) {
        let offer = Description::from_json(offer).unwrap();
        let client_ssrc = offer.ssrcs[0];
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let local: SocketAddr = socket.local_addr().unwrap();
        let mut rtc = Rtc::builder().set_rtp_mode(true).clear_codecs().enable_opus(true, false).set_dtls_version(DtlsVersion::Dtls12).set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags())).build(Instant::now());
        rtc.add_local_candidate(Candidate::host(local, "udp").unwrap());
        let pt = rtc.codec_config().find(|p| p.spec().codec == Codec::Opus).unwrap().pt();
        let (rx_mid, tx_mid): (Mid, Mid) = ("audio".into(), "echo".into());
        let mut api = rtc.direct_api();
        let creds = api.local_ice_credentials();
        let fingerprint = api.local_dtls_fingerprint().to_string();
        api.set_remote_ice_credentials(IceCreds { ufrag: offer.ufrag, pass: offer.pwd });
        api.set_remote_fingerprint(offer.fingerprint.parse().unwrap());
        api.set_ice_controlling(false);
        api.start_dtls(false).unwrap();
        api.declare_media(rx_mid, MediaKind::Audio);
        api.declare_media(tx_mid, MediaKind::Audio);
        api.expect_stream_rx(client_ssrc.into(), None, rx_mid, None);
        let echo: Ssrc = ECHO_SSRC.into();
        api.declare_stream_tx(echo, None, tx_mid, None);
        let answer = format!(
            "v=0\r\ns=-\r\na=ice-lite\r\nm=audio {port} UDP/TLS/RTP/SAVPF 111\r\na=candidate:1 1 udp 2130706431 127.0.0.1 {port} typ host\r\na=ice-ufrag:{u}\r\na=ice-pwd:{p}\r\na=fingerprint:{fingerprint}\r\na=setup:passive\r\na=mid:audio\r\na=rtpmap:111 opus/48000/2\r\na=ssrc:{ECHO_SSRC} cname:echo\r\n",
            port = local.port(),
            u = creds.ufrag,
            p = creds.pass,
        );
        let server = std::thread::spawn(move || {
            let mut buf = vec![0u8; 2000];
            let mut got = 0;
            while !stop.load(Ordering::Relaxed) {
                let next = loop {
                    match rtc.poll_output().unwrap() {
                        Output::Timeout(at) => break at,
                        Output::Transmit(t) => {
                            let _ = socket.send_to(&t.contents, t.destination);
                        }
                        Output::Event(Event::RtpPacket(p)) => {
                            got += 1;
                            rtc.direct_api().stream_tx(&echo).unwrap().write_rtp(RtpWrite::new(pt, (*p.seq_no).into(), p.header.timestamp, Instant::now(), p.payload.to_vec()));
                        }
                        Output::Event(_) => {}
                    }
                };
                let wait = next.saturating_duration_since(Instant::now()).clamp(Duration::from_millis(1), Duration::from_millis(5));
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
            got
        });
        (wrap("answer", &answer), server)
    }

    struct Call {
        engine: Engine,
        events: Receiver<EngineEvent>,
        heard: Arc<Mutex<Vec<f32>>>,
        stop: Arc<AtomicBool>,
        server: JoinHandle<usize>,
    }

    fn start(config: EngineConfig, mic_works: bool) -> Call {
        let mut link = Link::new();
        let stop = Arc::new(AtomicBool::new(false));
        let (answer, server) = echo_server(&link.offer(), stop.clone());
        link.apply_answer(&Description::from_json(&answer).unwrap()).unwrap();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let (tx, events) = mpsc::channel();
        let tx = Mutex::new(tx);
        let engine = Engine::start_with(
            link,
            config,
            Box::new(FakeAudio { heard: heard.clone(), mic_works }),
            Box::new(move |e| {
                let _ = tx.lock().unwrap().send(e);
            }),
        );
        Call { engine, events, heard, stop, server }
    }

    impl Call {
        fn wait_for(&self, what: impl Fn(&EngineEvent) -> bool) -> bool {
            let deadline = Instant::now() + Duration::from_secs(8);
            while let Ok(e) = self.events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                if what(&e) {
                    return true;
                }
            }
            false
        }

        fn finish(self) -> usize {
            drop(self.engine);
            self.stop.store(true, Ordering::Relaxed);
            self.server.join().unwrap()
        }
    }

    #[test]
    fn your_voice_goes_out_and_comes_back_as_another_person() {
        let call = start(EngineConfig::default(), true);
        assert!(call.wait_for(|e| *e == EngineEvent::Connected));
        call.engine.set_transmit(true);
        assert!(call.wait_for(|e| matches!(e, EngineEvent::Talking(t) if t.contains(&ME) && t.contains(&ECHO_SSRC))), "you and the echo both show as talking");
        std::thread::sleep(Duration::from_millis(1000));
        let heard = call.heard.lock().unwrap().clone();
        // The pitch of the last second, as the median of 100 ms pieces that
        // have sound: one glitch (a slow test machine falling behind and the
        // decoder filling the gap) can't fail it, a wrong pitch still does.
        let piece = FRAME * 5;
        let tail = &heard[heard.len().saturating_sub(piece * 10)..];
        let mut hz: Vec<f32> = tail.chunks_exact(piece).filter(|p| p.iter().map(|s| s * s).sum::<f32>() / p.len() as f32 > 0.003).map(pitch).collect();
        assert!(hz.len() >= 5, "only {} of 10 pieces had sound", hz.len());
        hz.sort_by(f32::total_cmp);
        let hz = hz[hz.len() / 2];
        assert!((hz - 440.0).abs() < 20.0, "heard {hz} Hz");
        assert!(call.finish() > 20);
    }

    #[test]
    fn nothing_is_sent_until_the_call_is_joined() {
        let call = start(EngineConfig::default(), true);
        assert!(call.wait_for(|e| *e == EngineEvent::Connected));
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(call.finish(), 0, "the microphone stays private while ringing");
    }

    #[test]
    fn muted_sends_no_voice() {
        let call = start(EngineConfig { muted: true, ..Default::default() }, true);
        assert!(call.wait_for(|e| *e == EngineEvent::Connected));
        call.engine.set_transmit(true);
        std::thread::sleep(Duration::from_millis(800));
        let heard = call.heard.lock().unwrap().clone();
        assert!(heard.iter().all(|s| s.abs() < 0.02), "only silence came back");
        call.finish();
    }

    #[test]
    fn push_to_talk_sends_nothing_while_the_key_is_up() {
        // A key no one is holding.
        let call = start(EngineConfig { push_to_talk: Some("F20".into()), ..Default::default() }, true);
        assert!(call.wait_for(|e| *e == EngineEvent::Connected));
        call.engine.set_transmit(true);
        std::thread::sleep(Duration::from_millis(600));
        let heard = call.heard.lock().unwrap().clone();
        assert!(heard.iter().all(|s| s.abs() < 0.02), "only silence came back");
        call.finish();
    }

    #[test]
    fn no_microphone_still_joins_to_listen() {
        let call = start(EngineConfig::default(), false);
        assert!(call.wait_for(|e| matches!(e, EngineEvent::NoMic(_))));
        assert!(call.wait_for(|e| *e == EngineEvent::Connected));
        call.finish();
    }

    #[test]
    fn roster_finds_people_both_ways() {
        let mut r = Roster::default();
        r.set([(10, 111), (20, 222)]);
        assert_eq!(r.ssrcs(), [10, 20]);
        assert_eq!(r.account(20), Some(222));
        assert_eq!(r.ssrc_of(111), Some(10));
        r.set([(30, 333)]);
        assert_eq!(r.account(10), None);
    }
}

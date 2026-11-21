//! Steam voice chat: the calls and notifications that go through the normal
//! Steam connection. The steps of a call are in [`machine`]; this runs them.
//! The audio itself goes over [`voice::Link`], directly to a Valve voice
//! server.

mod machine;

pub use ::voice::ptt;

use crate::{Event, Sink};
use machine::{Action, Input, Machine, Notice, Reply, Session};
use model::SteamId;
use model::voice::{AudioDevice, Call, CallEnd, CallPhase, CallTarget, Participant, VoiceEvent, VoiceOp, VoiceSettings, VoiceState};
use net::{Connection, Packet};
use proto::webui::*;
use proto::{CChatRoomJoinVoiceChatRequest, CChatRoomJoinVoiceChatResponse, CChatRoomLeaveVoiceChatRequest, CChatRoomLeaveVoiceChatResponse, CChatRoomNotifyShouldRejoinChatRoomVoiceChatNotification};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use voice::ring::Ringer;
use voice::{Description, Engine, EngineConfig, Link};

/// How long the audio connection gets in the connection test.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long Steam gets to confirm it saw the connection.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a call rings before giving up (Steam's five minutes).
const RING_TIMEOUT: Duration = Duration::from_secs(300);
const SETTINGS_FILE: &str = "voice.json";
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// What the connection test found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionTest {
    /// How long the server took to answer our offer.
    pub answer_ms: u64,
    /// How long the encrypted connection took once we had the answer.
    pub connect_ms: u64,
    /// The server uses the simplified reachability check.
    pub server_ice_lite: bool,
    /// Steam told us, over the normal connection, that it saw us connect.
    pub steam_confirmed: bool,
}

enum Msg {
    /// A new Steam connection.
    Connection(Connection, SteamId),
    Disconnected,
    Op(VoiceOp),
    Notice(Notice),
    /// About one call: ignored if that call is over.
    ForCall(u64, Input),
    /// Write the settings (dragging a slider changes them many times a second).
    SaveSettings,
    Shutdown,
}

type Listener = Arc<Mutex<Option<mpsc::UnboundedSender<Session>>>>;

/// The session's handle on voice chat.
pub struct Voice {
    tx: mpsc::UnboundedSender<Msg>,
    /// The connection test waits on this.
    test: Listener,
}

impl Voice {
    pub fn spawn(sink: Sink) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(rx, tx.clone(), sink));
        Self { tx, test: Default::default() }
    }

    pub fn set_connection(&self, conn: Connection, me: SteamId) {
        let _ = self.tx.send(Msg::Connection(conn, me));
    }

    pub fn disconnected(&self) {
        let _ = self.tx.send(Msg::Disconnected);
    }

    pub fn op(&self, op: VoiceOp) {
        let _ = self.tx.send(Msg::Op(op));
    }

    /// Take a service notification if it's a voice one. `may_ring` says
    /// whether this person's calls should reach you.
    pub fn route(&self, name: &str, p: &Packet, may_ring: impl Fn(SteamId) -> bool) -> bool {
        let method = name.split('#').next().unwrap_or(name);
        let Some(parsed) = parse(method, p) else { return false };
        match parsed {
            Ok(notices) => {
                for n in notices {
                    match &n {
                        Notice::SessionConnected(s) => {
                            if let Some(tx) = self.test.lock().unwrap().as_ref() {
                                let _ = tx.send(*s);
                            }
                        }
                        Notice::CallRequested { from, .. } if !may_ring(*from) => {
                            tracing::debug!("voice: ignoring a call (not a friend, blocked, invisible or busy)");
                            continue;
                        }
                        _ => {}
                    }
                    let _ = self.tx.send(Msg::Notice(n));
                }
            }
            Err(e) => tracing::debug!("voice: bad notification {method}: {e}"),
        }
        true
    }

    fn listen(&self) -> mpsc::UnboundedReceiver<Session> {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.test.lock().unwrap() = Some(tx);
        rx
    }

    /// Check that Valve's voice server accepts a connection from us. This
    /// only sets up the audio connection: it rings nobody, joins no channel
    /// and sends no audio.
    pub fn connection_test(&self, conn: Connection, sink: Sink) {
        let (notices, test) = (self.listen(), self.test.clone());
        tokio::spawn(async move {
            let result = run_test(&conn, notices).await;
            *test.lock().unwrap() = None;
            sink(Event::VoiceConnectionTest(result));
        });
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

/// Read a voice notification, or `None` if `method` isn't one.
fn parse(method: &str, p: &Packet) -> Option<Result<Vec<Notice>, net::Error>> {
    let status = |s: &CVoiceChatUserVoiceStatusNotification| Notice::Status {
        voice_chat: s.voice_chatid(),
        user: s.user_steamid(),
        muted: s.user_muted_mic_locally(),
        deaf: s.user_muted_output_locally(),
        no_mic: s.user_has_no_mic_for_session(),
    };
    Some(match method {
        "WebRTCClientNotifications.NotifyWebRTCSessionConnected" => p.body::<CWebRtcWebRtcSessionConnectedNotification>().map(|n| {
            vec![Notice::SessionConnected(Session { ssrc: n.ssrc(), client_ip: n.client_ip(), client_port: n.client_port(), server_ip: n.server_ip(), server_port: n.server_port() })]
        }),
        "WebRTCClientNotifications.NotifyWebRTCUpdateRemoteDescription" => p.body::<CWebRtcWebRtcUpdateRemoteDescriptionNotification>().map(|n| {
            let kind = serde_json::from_str::<serde_json::Value>(n.remote_description()).ok().and_then(|v| v["type"].as_str().map(str::to_string)).unwrap_or_default();
            vec![Notice::Remote { kind, version: n.remote_description_version(), streams: n.ssrcs_to_accountids.iter().map(|m| (m.ssrc(), m.accountid())).collect() }]
        }),
        "VoiceChatClient.NotifyOneOnOneChatRequested" => p.body::<CVoiceChatOneOnOneChatRequestedNotification>().map(|n| vec![Notice::CallRequested { voice_chat: n.voice_chatid(), from: n.steamid_partner() }]),
        "VoiceChatClient.NotifyOneOnOneChatResponse" => p.body::<CVoiceChatOneOnOneChatRequestResponseNotification>().map(|n| vec![Notice::CallResponse { voice_chat: n.voicechat_id(), from: n.steamid_partner(), accepted: n.accepted_request() }]),
        "VoiceChatClient.NotifyUserJoinedVoiceChat" => p.body::<CVoiceChatUserJoinedVoiceChatNotification>().map(|n| vec![Notice::UserJoined { voice_chat: n.voice_chatid(), user: n.user_steamid() }]),
        "VoiceChatClient.NotifyUserLeftVoiceChat" => p.body::<CVoiceChatUserLeftVoiceChatNotification>().map(|n| vec![Notice::UserLeft { voice_chat: n.voice_chatid(), user: n.user_steamid() }]),
        "VoiceChatClient.NotifyUserVoiceStatus" => p.body::<CVoiceChatUserVoiceStatusNotification>().map(|n| vec![status(&n)]),
        "VoiceChatClient.NotifyAllUsersVoiceStatus" => p.body::<CVoiceChatAllMembersStatusNotification>().map(|n| n.users.iter().map(status).collect()),
        "VoiceChatClient.NotifyVoiceChatEnded" => p.body::<CVoiceChatVoiceChatEndedNotification>().map(|n| vec![Notice::Ended { voice_chat: n.voice_chatid() }]),
        "ChatRoomClient.NotifyShouldRejoinChatRoomVoiceChat" => p.body::<CChatRoomNotifyShouldRejoinChatRoomVoiceChatNotification>().map(|n| vec![Notice::Rejoin { group: n.chat_group_id(), room: n.chat_id() }]),
        _ => return None,
    })
}

fn settings_path() -> std::path::PathBuf {
    crate::store::config_dir().join(SETTINGS_FILE)
}

fn load_settings() -> VoiceSettings {
    std::fs::read(settings_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_settings(s: &VoiceSettings) {
    match serde_json::to_vec_pretty(s) {
        Ok(b) => {
            if let Err(e) = std::fs::write(settings_path(), b) {
                tracing::warn!("voice: can't save settings: {e}");
            }
        }
        Err(e) => tracing::warn!("voice: can't save settings: {e}"),
    }
}

fn list_devices() -> VoiceEvent {
    let d = voice::audio::devices();
    let convert = |l: Vec<voice::Device>| l.into_iter().map(|d| AudioDevice { id: d.id, name: d.name }).collect();
    VoiceEvent::Devices { inputs: convert(d.inputs), outputs: convert(d.outputs) }
}

/// Carries out what the [`Machine`] says.
struct Runner {
    conn: Option<Connection>,
    tx: mpsc::UnboundedSender<Msg>,
    sink: Sink,
    /// The connection between making the offer and starting the audio.
    link: Option<Link>,
    engine: Option<Engine>,
    ring: Option<Ringer>,
    save_pending: bool,
}

impl Runner {
    /// Ask Steam something, and hand the answer to the call that asked.
    fn ask<Req, Resp>(&self, generation: u64, method: &'static str, req: Req, done: impl FnOnce(Result<Resp, String>) -> Reply + Send + 'static)
    where
        Req: prost::Message + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        let tx = self.tx.clone();
        let Some(conn) = self.conn.clone() else {
            let _ = tx.send(Msg::ForCall(generation, Input::Reply(done(Err("Not connected to Steam".to_string())))));
            return;
        };
        tokio::spawn(async move {
            let r = conn.call::<_, Resp>(method, &req).await.map_err(|e| e.to_string());
            let _ = tx.send(Msg::ForCall(generation, Input::Reply(done(r))));
        });
    }

    /// Tell Steam something without waiting on the answer.
    fn tell<Req, Resp>(&self, method: &'static str, req: Req)
    where
        Req: prost::Message + Send + 'static,
        Resp: prost::Message + Default + Send + 'static,
    {
        let Some(conn) = self.conn.clone() else { return };
        tokio::spawn(async move {
            if let Err(e) = conn.call::<_, Resp>(method, &req).await {
                tracing::debug!("voice: {method} failed: {e}");
            }
        });
    }

    fn engine(&self, f: impl FnOnce(&Engine)) {
        if let Some(e) = &self.engine {
            f(e);
        }
    }

    fn start_engine(&mut self, answer: &str, generation: u64, m: &Machine) -> Result<(), String> {
        let mut link = self.link.take().ok_or("no connection to answer")?;
        let answer = Description::from_json(answer).map_err(|e| format!("Couldn't read Steam's answer: {e}"))?;
        link.apply_answer(&answer)?;
        let config = EngineConfig {
            input: m.settings.input.clone(),
            output: m.settings.output.clone(),
            input_gain: m.settings.input_volume,
            output_gain: m.settings.output_volume,
            muted: m.muted(),
            deaf: m.deaf(),
            push_to_talk: m.push_to_talk(),
        };
        let tx = self.tx.clone();
        self.engine = Some(Engine::start(
            link,
            config,
            Box::new(move |e| {
                let _ = tx.send(Msg::ForCall(generation, Input::Engine(e)));
            }),
        ));
        Ok(())
    }

    fn stop_engine(&mut self) {
        self.link = None;
        if let Some(e) = self.engine.take() {
            // Stopping waits for the audio threads.
            tokio::task::spawn_blocking(move || drop(e));
        }
    }

    fn run(&mut self, actions: Vec<Action>, m: &Machine) {
        let g = m.generation();
        for action in actions {
            match action {
                Action::Connect => {
                    let mut link = Link::new();
                    let offer = link.offer();
                    self.link = Some(link);
                    let req = CWebRtcClientInitiateWebRtcConnectionRequest { sdp: Some(offer), browser_name: Some("SteamInRust".to_string()), browser_version: Some(env!("CARGO_PKG_VERSION").to_string()) };
                    self.ask(g, "WebRTCClient.InitiateWebRTCConnection#1", req, |r: Result<CWebRtcClientInitiateWebRtcConnectionResponse, String>| Reply::Offer(r.map(|a| a.remote_description().to_string())));
                }
                Action::StartEngine { answer } => {
                    if let Err(e) = self.start_engine(&answer, g, m) {
                        let _ = self.tx.send(Msg::ForCall(g, Input::Engine(voice::EngineEvent::Failed(e))));
                    }
                }
                Action::StopEngine => self.stop_engine(),
                Action::JoinRoom { group, room } => {
                    let req = CChatRoomJoinVoiceChatRequest { chat_group_id: Some(group), chat_id: Some(room) };
                    self.ask(g, "ChatRoom.JoinVoiceChat#1", req, |r: Result<CChatRoomJoinVoiceChatResponse, String>| Reply::Joined(r.map(|a| a.voice_chatid())));
                }
                Action::LeaveRoom { group, room } => {
                    self.tell::<_, CChatRoomLeaveVoiceChatResponse>("ChatRoom.LeaveVoiceChat#1", CChatRoomLeaveVoiceChatRequest { chat_group_id: Some(group), chat_id: Some(room) });
                }
                Action::RequestCall(friend) => {
                    let req = CVoiceChatRequestOneOnOneChatRequest { steamid_partner: Some(friend) };
                    self.ask(g, "VoiceChat.RequestOneOnOneChat#1", req, |r: Result<CVoiceChatRequestOneOnOneChatResponse, String>| Reply::Requested(r.map(|a| a.voice_chatid())));
                }
                Action::AnswerCall { friend, voice_chat, accept } => {
                    let req = CVoiceChatAnswerOneOnOneChatRequest { voice_chatid: Some(voice_chat), steamid_partner: Some(friend), accepted_request: Some(accept) };
                    if accept {
                        self.ask(g, "VoiceChat.AnswerOneOnOneChat#1", req, |r: Result<CVoiceChatAnswerOneOnOneChatResponse, String>| Reply::Answered(r.map(|_| ())));
                    } else {
                        self.tell::<_, CVoiceChatAnswerOneOnOneChatResponse>("VoiceChat.AnswerOneOnOneChat#1", req);
                    }
                }
                Action::EndCall(friend) => {
                    self.tell::<_, CVoiceChatEndOneOnOneChatResponse>("VoiceChat.EndOneOnOneChat#1", CVoiceChatEndOneOnOneChatRequest { steamid_partner: Some(friend) });
                }
                Action::Report { voice_chat, session: s } => {
                    let req = CVoiceChatUpdateVoiceChatWebRtcDataRequest {
                        voice_chatid: Some(voice_chat),
                        ip_webrtc_server: Some(s.server_ip),
                        port_webrtc_server: Some(s.server_port),
                        ip_webrtc_client: Some(s.client_ip),
                        port_webrtc_client: Some(s.client_port),
                        ssrc_my_sending_stream: Some(s.ssrc),
                        user_agent: Some(concat!("SteamInRust/", env!("CARGO_PKG_VERSION")).to_string()),
                        has_audio_worklets_support: Some(false),
                        has_webrtc_data_channel: Some(false),
                        accepts_webrtc_video: Some(false),
                        sends_webrtc_video: Some(false),
                    };
                    self.ask(g, "VoiceChat.UpdateVoiceChatWebRTCData#1", req, |r: Result<CVoiceChatUpdateVoiceChatWebRtcDataResponse, String>| Reply::Reported(r.map(|_| ())));
                }
                Action::SendStatus { voice_chat, muted, deaf, no_mic } => {
                    if let Some(conn) = &self.conn {
                        let req = CVoiceChatUserVoiceStatusNotification {
                            voice_chatid: Some(voice_chat),
                            user_steamid: Some(m.me()),
                            user_muted_mic_locally: Some(muted),
                            user_muted_output_locally: Some(deaf),
                            user_has_no_mic_for_session: Some(no_mic),
                            user_webaudio_sample_rate: Some(voice::codec::SAMPLE_RATE as i32),
                            user_sends_video: Some(false),
                        };
                        if let Err(e) = conn.notify_service("VoiceChat.NotifyUserVoiceStatus#1", &req) {
                            tracing::debug!("voice: status not sent: {e}");
                        }
                    }
                }
                Action::Acknowledge { version, session: s } => {
                    let req = CWebRtcClientAcknowledgeUpdatedRemoteDescriptionRequest {
                        ip_webrtc_server: Some(s.server_ip),
                        port_webrtc_server: Some(s.server_port),
                        ip_webrtc_session_client: Some(s.client_ip),
                        port_webrtc_session_client: Some(s.client_port),
                        remote_description_version: Some(version),
                    };
                    self.tell::<_, CWebRtcClientAcknowledgeUpdatedRemoteDescriptionResponse>("WebRTCClient.AcknowledgeUpdatedRemoteDescription#1", req);
                }
                Action::Transmit(on) => self.engine(|e| e.set_transmit(on)),
                Action::Mute(on) => self.engine(|e| e.set_muted(on)),
                Action::Deaf(on) => self.engine(|e| e.set_deaf(on)),
                Action::Speakers(list) => self.engine(|e| e.set_speakers(list)),
                Action::PersonGain(ssrc, gain) => self.engine(|e| e.set_person_gain(ssrc, gain)),
                Action::InputGain(gain) => self.engine(|e| e.set_input_gain(gain)),
                Action::OutputGain(gain) => self.engine(|e| e.set_output_gain(gain)),
                Action::PushToTalk(key) => self.engine(|e| e.set_push_to_talk(key)),
                Action::InputDevice(d) => self.engine(|e| e.set_input_device(d)),
                Action::OutputDevice(d) => self.engine(|e| e.set_output_device(d)),
                Action::Ring(on) => self.ring = on.then(|| Ringer::start(m.settings.output.clone())),
                Action::RingTimer => {
                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(RING_TIMEOUT).await;
                        let _ = tx.send(Msg::ForCall(g, Input::RingTimeout));
                    });
                }
                Action::SaveSettings => {
                    (self.sink)(Event::Voice(VoiceEvent::Settings(m.settings.clone())));
                    if !std::mem::replace(&mut self.save_pending, true) {
                        let tx = self.tx.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(SAVE_DELAY).await;
                            let _ = tx.send(Msg::SaveSettings);
                        });
                    }
                }
                Action::Ended(why) => (self.sink)(Event::Voice(VoiceEvent::Ended(why))),
            }
        }
    }
}

async fn run(mut rx: mpsc::UnboundedReceiver<Msg>, tx: mpsc::UnboundedSender<Msg>, sink: Sink) {
    let settings = load_settings();
    sink(Event::Voice(VoiceEvent::Settings(settings.clone())));
    let mut machine = Machine::new(0, settings);
    let mut runner = Runner { conn: None, tx, sink: sink.clone(), link: None, engine: None, ring: None, save_pending: false };
    let mut shown = VoiceState::default();
    while let Some(msg) = rx.recv().await {
        let input = match msg {
            Msg::Connection(conn, me) => {
                runner.conn = Some(conn);
                machine.set_me(me);
                continue;
            }
            Msg::Op(VoiceOp::ListDevices) => {
                let sink = sink.clone();
                tokio::task::spawn_blocking(move || sink(Event::Voice(list_devices())));
                continue;
            }
            Msg::Op(op) => Input::Op(op),
            Msg::Notice(n) => Input::Notice(n),
            Msg::ForCall(generation, input) => {
                if generation != machine.generation() {
                    continue;
                }
                input
            }
            Msg::Disconnected => {
                runner.conn = None;
                Input::Disconnected
            }
            Msg::SaveSettings => {
                runner.save_pending = false;
                save_settings(&machine.settings);
                continue;
            }
            Msg::Shutdown => {
                if runner.save_pending {
                    save_settings(&machine.settings);
                }
                runner.conn = None;
                let actions = machine.handle(Input::Disconnected);
                runner.run(actions, &machine);
                sink(Event::Voice(VoiceEvent::State(VoiceState::default())));
                return;
            }
        };
        let actions = machine.handle(input);
        runner.run(actions, &machine);
        let state = machine.state();
        if state != shown {
            shown = state.clone();
            sink(Event::Voice(VoiceEvent::State(state)));
        }
    }
}

async fn run_test(conn: &Connection, mut notices: mpsc::UnboundedReceiver<Session>) -> Result<ConnectionTest, String> {
    let mut link = Link::new();
    let request = CWebRtcClientInitiateWebRtcConnectionRequest {
        sdp: Some(link.offer()),
        browser_name: Some("SteamInRust".to_string()),
        browser_version: Some(env!("CARGO_PKG_VERSION").to_string()),
    };
    let asked = Instant::now();
    let reply: CWebRtcClientInitiateWebRtcConnectionResponse = conn.call("WebRTCClient.InitiateWebRTCConnection#1", &request).await.map_err(|e| format!("Steam refused the voice connection: {e}"))?;
    let answer_ms = asked.elapsed().as_millis() as u64;
    let answer = Description::from_json(reply.remote_description()).map_err(|e| format!("Couldn't read Steam's answer: {e}"))?;
    let server_ice_lite = answer.ice_lite;
    link.apply_answer(&answer)?;

    let (link, up) = tokio::task::spawn_blocking(move || {
        let up = link.wait_connected(CONNECT_TIMEOUT);
        (link, up)
    })
    .await
    .map_err(|e| e.to_string())?;
    let up = up?;

    // Keep the connection open while Steam confirms it.
    let steam_confirmed = matches!(tokio::time::timeout(CONFIRM_TIMEOUT, notices.recv()).await, Ok(Some(_)));
    drop(link);
    Ok(ConnectionTest { answer_ms, connect_ms: up.took.as_millis() as u64, server_ice_lite, steam_confirmed })
}

/// The demo's made-up call: no network and no audio.
#[derive(Default)]
struct Demo {
    me: SteamId,
    others: Vec<SteamId>,
    state: VoiceState,
    settings: Option<VoiceSettings>,
}

static DEMO: Mutex<Option<Demo>> = Mutex::new(None);

/// Who's in the demo's calls.
pub fn demo_people(me: SteamId, others: Vec<SteamId>) {
    *DEMO.lock().unwrap() = Some(Demo { me, others, ..Default::default() });
}

/// Signed out (the demo): voice chat acts out a call.
pub(crate) fn offline(op: VoiceOp, sink: &Sink) {
    let mut guard = DEMO.lock().unwrap();
    let Some(demo) = guard.as_mut() else {
        if op == VoiceOp::ListDevices {
            sink(Event::Voice(VoiceEvent::Devices { inputs: Vec::new(), outputs: Vec::new() }));
        }
        return;
    };
    let settings = demo.settings.get_or_insert_with(VoiceSettings::default);
    let person_row = |id: SteamId, i: usize| Participant { steam_id: id, talking: i == 1, muted: i == 2, deaf: false, no_mic: false };
    let call = |target: CallTarget, ids: Vec<SteamId>, state: &VoiceState| Call {
        target,
        phase: CallPhase::InCall,
        people: ids.into_iter().enumerate().map(|(i, id)| person_row(id, i)).collect(),
        muted: state.call.as_ref().is_some_and(|c| c.muted),
        deaf: state.call.as_ref().is_some_and(|c| c.deaf),
        mic_problem: None,
        speaker_problem: None,
    };
    match op {
        VoiceOp::CallFriend(f) | VoiceOp::Accept(f) => {
            demo.state.incoming.retain(|x| *x != f);
            demo.state.call = Some(call(CallTarget::Friend(f), vec![demo.me, f], &demo.state));
        }
        VoiceOp::JoinRoom { group, room } => {
            let mut ids = vec![demo.me];
            ids.extend(demo.others.iter().take(3));
            demo.state.call = Some(call(CallTarget::Room { group, room }, ids, &demo.state));
        }
        VoiceOp::Decline(f) => demo.state.incoming.retain(|x| *x != f),
        VoiceOp::Leave => {
            demo.state.call = None;
            sink(Event::Voice(VoiceEvent::Ended(CallEnd::Finished)));
        }
        VoiceOp::SetMuted(on) => {
            if let Some(c) = &mut demo.state.call {
                c.muted = on;
            }
        }
        VoiceOp::SetDeaf(on) => {
            if let Some(c) = &mut demo.state.call {
                c.deaf = on;
            }
        }
        VoiceOp::SetPersonVolume(who, v) => {
            settings.set_volume(who, v);
            sink(Event::Voice(VoiceEvent::Settings(settings.clone())));
        }
        VoiceOp::SetSettings(s) => {
            *settings = s;
            sink(Event::Voice(VoiceEvent::Settings(settings.clone())));
        }
        VoiceOp::ListDevices => {
            let d = |id: &str, name: &str| AudioDevice { id: id.into(), name: name.into() };
            sink(Event::Voice(VoiceEvent::Devices { inputs: vec![d("mic1", "Headset Microphone"), d("mic2", "Webcam Microphone")], outputs: vec![d("out1", "Headphones"), d("out2", "Speakers")] }));
            return;
        }
    }
    sink(Event::Voice(VoiceEvent::State(demo.state.clone())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::chat_admin::INDIVIDUAL;

    fn person(account: u32) -> SteamId {
        INDIVIDUAL + account as u64
    }

    #[test]
    fn the_demo_acts_out_a_call_without_a_network() {
        let events: Arc<Mutex<Vec<Event>>> = Default::default();
        let log = events.clone();
        let sink: Sink = Arc::new(move |e| log.lock().unwrap().push(e));
        let last = |events: &Arc<Mutex<Vec<Event>>>| {
            events.lock().unwrap().iter().rev().find_map(|e| match e {
                Event::Voice(VoiceEvent::State(s)) => Some(s.clone()),
                _ => None,
            })
        };
        demo_people(person(1), vec![person(2), person(3)]);
        offline(VoiceOp::JoinRoom { group: 1, room: 2 }, &sink);
        let c = last(&events).unwrap().call.unwrap();
        assert_eq!((c.phase, c.people.len()), (CallPhase::InCall, 3));
        offline(VoiceOp::SetMuted(true), &sink);
        assert!(last(&events).unwrap().call.unwrap().muted);
        offline(VoiceOp::Leave, &sink);
        assert!(last(&events).unwrap().call.is_none());
        *DEMO.lock().unwrap() = None;
    }
}

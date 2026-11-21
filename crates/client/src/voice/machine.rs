//! The steps of a call, the same ones Steam's own client takes, with no
//! network or audio in here: it takes what happened and says what to do.
//!
//! 1. Get the audio connection up (`Connect`, `StartEngine`) and wait until
//!    both our side and Steam say it works.
//! 2. Join: a channel directly; a friend by ringing them, or by having
//!    answered their call first.
//! 3. Tell Steam which audio connection belongs to the call (`Report`). Only
//!    then does the microphone start going out.

use model::chat_admin::INDIVIDUAL;
use model::voice::{Call, CallEnd, CallPhase, CallTarget, Participant, VoiceOp, VoiceSettings, VoiceState};
use model::SteamId;
use std::collections::BTreeMap;
use voice::engine::{EngineEvent, ME, Roster};

/// Where the audio connection runs (Steam wants this back).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Session {
    pub ssrc: u32,
    pub client_ip: u32,
    pub client_port: u32,
    pub server_ip: u32,
    pub server_port: u32,
}

/// A voice notification from Steam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    SessionConnected(Session),
    /// Who's on which audio stream changed.
    Remote { kind: String, version: u64, streams: Vec<(u32, u32)> },
    CallRequested { voice_chat: u64, from: SteamId },
    CallResponse { voice_chat: u64, from: SteamId, accepted: bool },
    UserJoined { voice_chat: u64, user: SteamId },
    UserLeft { voice_chat: u64, user: SteamId },
    Status { voice_chat: u64, user: SteamId, muted: bool, deaf: bool, no_mic: bool },
    Ended { voice_chat: u64 },
    Rejoin { group: u64, room: u64 },
}

/// The answer to something we asked Steam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// The server's answer to our offer.
    Offer(Result<String, String>),
    /// We told Steam we picked up.
    Answered(Result<(), String>),
    Joined(Result<u64, String>),
    Requested(Result<u64, String>),
    Reported(Result<(), String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Op(VoiceOp),
    Notice(Notice),
    Engine(EngineEvent),
    Reply(Reply),
    /// Nobody picked up in time.
    RingTimeout,
    /// The Steam connection dropped.
    Disconnected,
}

/// What to do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Make an offer and send it to Steam.
    Connect,
    StartEngine { answer: String },
    StopEngine,
    JoinRoom { group: u64, room: u64 },
    LeaveRoom { group: u64, room: u64 },
    RequestCall(SteamId),
    AnswerCall { friend: SteamId, voice_chat: u64, accept: bool },
    EndCall(SteamId),
    Report { voice_chat: u64, session: Session },
    SendStatus { voice_chat: u64, muted: bool, deaf: bool, no_mic: bool },
    Acknowledge { version: u64, session: Session },
    Transmit(bool),
    Mute(bool),
    Deaf(bool),
    Speakers(Vec<u32>),
    PersonGain(u32, f32),
    InputGain(f32),
    OutputGain(f32),
    PushToTalk(Option<String>),
    InputDevice(Option<String>),
    OutputDevice(Option<String>),
    Ring(bool),
    /// Start the timer for nobody picking up.
    RingTimer,
    SaveSettings,
    Ended(CallEnd),
}

struct Active {
    target: CallTarget,
    phase: CallPhase,
    voice_chat: Option<u64>,
    /// We picked up their call (so there's no ringing step).
    answered: bool,
    engine_up: bool,
    session: Option<Session>,
    join_started: bool,
    joined: bool,
    people: BTreeMap<SteamId, Participant>,
    roster: Roster,
    no_mic: Option<String>,
    no_speakers: Option<String>,
}

fn person(account: u32) -> SteamId {
    INDIVIDUAL + account as u64
}

fn account(id: SteamId) -> u32 {
    (id & 0xFFFF_FFFF) as u32
}

pub struct Machine {
    me: SteamId,
    /// Bumped per call, so late answers about an old call are ignored.
    generation: u64,
    active: Option<Active>,
    incoming: Vec<(SteamId, u64)>,
    muted: bool,
    deaf: bool,
    ringing: bool,
    pub settings: VoiceSettings,
}

impl Machine {
    pub fn new(me: SteamId, settings: VoiceSettings) -> Self {
        Self { me, generation: 0, active: None, incoming: Vec::new(), muted: false, deaf: false, ringing: false, settings }
    }

    pub fn set_me(&mut self, me: SteamId) {
        self.me = me;
    }

    pub fn me(&self) -> SteamId {
        self.me
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn deaf(&self) -> bool {
        self.deaf
    }

    /// What the app shows.
    pub fn state(&self) -> VoiceState {
        let call = self.active.as_ref().map(|a| Call {
            target: a.target,
            phase: a.phase,
            people: a.people.values().cloned().collect(),
            muted: self.muted,
            deaf: self.deaf,
            mic_problem: a.no_mic.clone(),
            speaker_problem: a.no_speakers.clone(),
        });
        VoiceState { call, incoming: self.incoming.iter().map(|(f, _)| *f).collect() }
    }

    pub fn handle(&mut self, input: Input) -> Vec<Action> {
        let mut out = Vec::new();
        match input {
            Input::Op(op) => self.op(op, &mut out),
            Input::Notice(n) => self.notice(n, &mut out),
            Input::Engine(e) => self.engine(e, &mut out),
            Input::Reply(r) => self.reply(r, &mut out),
            Input::RingTimeout => {
                if self.active.as_ref().is_some_and(|a| a.phase == CallPhase::Ringing) {
                    self.finish(CallEnd::NoAnswer, true, &mut out);
                }
            }
            Input::Disconnected => {
                self.incoming.clear();
                if self.active.is_some() {
                    self.finish(CallEnd::Failed("The connection to Steam was lost".into()), false, &mut out);
                }
            }
        }
        // Ring while a friend is calling (and you're not already talking to
        // someone), or while you wait for them to pick up.
        let in_call = self.active.as_ref().is_some_and(|a| a.phase == CallPhase::InCall);
        let waiting = self.active.as_ref().is_some_and(|a| a.phase == CallPhase::Ringing);
        let ring = waiting || (!self.incoming.is_empty() && !in_call);
        if ring != self.ringing {
            self.ringing = ring;
            out.push(Action::Ring(ring));
        }
        out
    }

    fn op(&mut self, op: VoiceOp, out: &mut Vec<Action>) {
        match op {
            VoiceOp::CallFriend(friend) => {
                // They're already calling you: this picks up.
                if let Some(voice_chat) = self.incoming.iter().find(|(f, _)| *f == friend).map(|(_, v)| *v) {
                    self.accept(friend, voice_chat, out);
                } else if self.active.as_ref().is_none_or(|a| a.target != CallTarget::Friend(friend)) {
                    self.start(CallTarget::Friend(friend), out);
                    out.push(Action::Connect);
                }
            }
            VoiceOp::JoinRoom { group, room } => {
                let target = CallTarget::Room { group, room };
                if self.active.as_ref().is_none_or(|a| a.target != target) {
                    self.start(target, out);
                    out.push(Action::Connect);
                }
            }
            VoiceOp::Accept(friend) => {
                if let Some(voice_chat) = self.incoming.iter().find(|(f, _)| *f == friend).map(|(_, v)| *v) {
                    self.accept(friend, voice_chat, out);
                }
            }
            VoiceOp::Decline(friend) => {
                if let Some(i) = self.incoming.iter().position(|(f, _)| *f == friend) {
                    let (friend, voice_chat) = self.incoming.remove(i);
                    out.push(Action::AnswerCall { friend, voice_chat, accept: false });
                }
            }
            VoiceOp::Leave => {
                if self.active.is_some() {
                    self.finish(CallEnd::Finished, true, out);
                }
            }
            VoiceOp::SetMuted(muted) => {
                self.muted = muted;
                out.push(Action::Mute(muted));
                self.send_status(out);
            }
            VoiceOp::SetDeaf(deaf) => {
                self.deaf = deaf;
                out.push(Action::Deaf(deaf));
                self.send_status(out);
            }
            VoiceOp::SetPersonVolume(who, volume) => {
                self.settings.set_volume(who, volume);
                let volume = self.settings.volume_of(who);
                if let Some(ssrc) = self.active.as_ref().and_then(|a| a.roster.ssrc_of(account(who))) {
                    out.push(Action::PersonGain(ssrc, volume));
                }
                out.push(Action::SaveSettings);
            }
            VoiceOp::SetSettings(new) => {
                let old = std::mem::replace(&mut self.settings, new);
                if self.active.is_some() {
                    let s = &self.settings;
                    if s.input != old.input {
                        out.push(Action::InputDevice(s.input.clone()));
                    }
                    if s.output != old.output {
                        out.push(Action::OutputDevice(s.output.clone()));
                    }
                    if s.input_volume != old.input_volume {
                        out.push(Action::InputGain(s.input_volume));
                    }
                    if s.output_volume != old.output_volume {
                        out.push(Action::OutputGain(s.output_volume));
                    }
                    if (s.push_to_talk, &s.push_to_talk_key) != (old.push_to_talk, &old.push_to_talk_key) {
                        out.push(Action::PushToTalk(self.push_to_talk()));
                    }
                }
                out.push(Action::SaveSettings);
            }
            VoiceOp::ListDevices => {}
        }
    }

    /// The push-to-talk key, when it's on and is a real key.
    pub fn push_to_talk(&self) -> Option<String> {
        (self.settings.push_to_talk && voice::ptt::known(&self.settings.push_to_talk_key)).then(|| self.settings.push_to_talk_key.clone())
    }

    /// Replace any call with a new one.
    fn start(&mut self, target: CallTarget, out: &mut Vec<Action>) {
        if self.active.is_some() {
            self.finish(CallEnd::Finished, true, out);
        }
        self.generation += 1;
        self.active = Some(Active {
            target,
            phase: CallPhase::Connecting,
            voice_chat: None,
            answered: false,
            engine_up: false,
            session: None,
            join_started: false,
            joined: false,
            people: BTreeMap::new(),
            roster: Roster::default(),
            no_mic: None,
            no_speakers: None,
        });
    }

    /// Pick up: Steam hears first, then the audio connection is set up.
    fn accept(&mut self, friend: SteamId, voice_chat: u64, out: &mut Vec<Action>) {
        self.incoming.retain(|(f, _)| *f != friend);
        self.start(CallTarget::Friend(friend), out);
        if let Some(a) = &mut self.active {
            a.voice_chat = Some(voice_chat);
            a.answered = true;
        }
        out.push(Action::AnswerCall { friend, voice_chat, accept: true });
    }

    /// End the call. `tell_steam` is false when it's already over there.
    fn finish(&mut self, why: CallEnd, tell_steam: bool, out: &mut Vec<Action>) {
        let Some(a) = self.active.take() else { return };
        self.generation += 1;
        if tell_steam {
            match a.target {
                CallTarget::Friend(friend) => {
                    if a.voice_chat.is_some() || a.join_started {
                        out.push(Action::EndCall(friend));
                    }
                }
                CallTarget::Room { group, room } => {
                    if a.join_started {
                        out.push(Action::LeaveRoom { group, room });
                    }
                }
            }
        }
        out.push(Action::Transmit(false));
        out.push(Action::StopEngine);
        out.push(Action::Ended(why));
    }

    fn fail(&mut self, why: String, out: &mut Vec<Action>) {
        self.finish(CallEnd::Failed(why), true, out);
    }

    fn engine(&mut self, e: EngineEvent, out: &mut Vec<Action>) {
        let me = self.me;
        let Some(a) = &mut self.active else { return };
        match e {
            EngineEvent::Connected => {
                a.engine_up = true;
                self.try_join(out);
            }
            EngineEvent::NoMic(why) => {
                a.no_mic = Some(why);
                self.send_status(out);
            }
            EngineEvent::NoSpeakers(why) => a.no_speakers = Some(why),
            EngineEvent::Talking(list) => {
                let talking: Vec<SteamId> = list.iter().filter_map(|s| if *s == ME { Some(me) } else { a.roster.account(*s).map(person) }).collect();
                for p in a.people.values_mut() {
                    p.talking = talking.contains(&p.steam_id);
                }
            }
            EngineEvent::Failed(why) => self.fail(why, out),
        }
    }

    /// Join once the audio connection works on both sides.
    fn try_join(&mut self, out: &mut Vec<Action>) {
        let Some(a) = &mut self.active else { return };
        let (true, Some(session), false) = (a.engine_up, a.session, a.join_started) else { return };
        a.join_started = true;
        match a.target {
            CallTarget::Room { group, room } => out.push(Action::JoinRoom { group, room }),
            CallTarget::Friend(friend) => {
                if a.answered {
                    if let Some(voice_chat) = a.voice_chat {
                        out.push(Action::Report { voice_chat, session });
                    }
                } else {
                    a.phase = CallPhase::Ringing;
                    out.push(Action::RequestCall(friend));
                    out.push(Action::RingTimer);
                }
            }
        }
    }

    fn reply(&mut self, r: Reply, out: &mut Vec<Action>) {
        if self.active.is_none() {
            return;
        }
        match r {
            Reply::Offer(Ok(answer)) => out.push(Action::StartEngine { answer }),
            Reply::Answered(Ok(())) => out.push(Action::Connect),
            Reply::Joined(Ok(voice_chat)) => {
                let Some(a) = &mut self.active else { return };
                a.voice_chat = Some(voice_chat);
                if let Some(session) = a.session {
                    out.push(Action::Report { voice_chat, session });
                }
            }
            Reply::Requested(Ok(voice_chat)) => {
                if let Some(a) = &mut self.active {
                    a.voice_chat = a.voice_chat.or(Some(voice_chat));
                }
            }
            Reply::Reported(Ok(())) => {
                let me = self.me;
                let Some(a) = &mut self.active else { return };
                a.phase = CallPhase::InCall;
                a.joined = true;
                a.people.entry(me).or_insert(Participant { steam_id: me, ..Default::default() });
                let speakers = a.roster.ssrcs();
                if !speakers.is_empty() {
                    out.push(Action::Speakers(speakers));
                }
                out.push(Action::Transmit(true));
                self.send_status(out);
            }
            Reply::Offer(Err(e)) | Reply::Answered(Err(e)) | Reply::Joined(Err(e)) | Reply::Requested(Err(e)) | Reply::Reported(Err(e)) => self.fail(e, out),
        }
    }

    fn send_status(&self, out: &mut Vec<Action>) {
        if let Some(a) = &self.active
            && a.joined
            && let Some(voice_chat) = a.voice_chat
        {
            out.push(Action::SendStatus { voice_chat, muted: self.muted, deaf: self.deaf, no_mic: a.no_mic.is_some() });
        }
    }

    /// Is this about the call we're in?
    fn is_ours(&self, voice_chat: u64) -> bool {
        self.active.as_ref().is_some_and(|a| a.voice_chat == Some(voice_chat))
    }

    fn notice(&mut self, n: Notice, out: &mut Vec<Action>) {
        match n {
            Notice::SessionConnected(session) => {
                if let Some(a) = &mut self.active {
                    a.session = Some(session);
                    self.try_join(out);
                }
            }
            Notice::Remote { kind, version, streams } => {
                let volumes: Vec<(u32, f32)> = streams.iter().map(|(s, acct)| (*s, self.settings.volume_of(person(*acct)))).collect();
                let Some(a) = &mut self.active else { return };
                a.roster.set(streams);
                out.push(Action::Speakers(a.roster.ssrcs()));
                out.extend(volumes.into_iter().map(|(s, v)| Action::PersonGain(s, v)));
                if kind == "offer"
                    && let Some(session) = a.session
                {
                    out.push(Action::Acknowledge { version, session });
                }
            }
            Notice::CallRequested { voice_chat, from } => {
                // You're both calling each other: that's a call.
                if self.active.as_ref().is_some_and(|a| a.target == CallTarget::Friend(from)) {
                    self.accept(from, voice_chat, out);
                } else if !self.incoming.iter().any(|(f, _)| *f == from) {
                    self.incoming.push((from, voice_chat));
                }
            }
            Notice::CallResponse { voice_chat, from, accepted } => {
                let Some(a) = &mut self.active else { return };
                if a.target != CallTarget::Friend(from) || a.phase != CallPhase::Ringing || a.voice_chat.is_some_and(|v| v != voice_chat) {
                    return;
                }
                if accepted {
                    a.voice_chat = Some(voice_chat);
                    a.phase = CallPhase::Connecting;
                    if let Some(session) = a.session {
                        out.push(Action::Report { voice_chat, session });
                    }
                } else {
                    self.finish(CallEnd::Declined, true, out);
                }
            }
            Notice::UserJoined { voice_chat, user } => {
                if self.is_ours(voice_chat)
                    && let Some(a) = &mut self.active
                {
                    a.people.entry(user).or_insert(Participant { steam_id: user, ..Default::default() });
                }
            }
            Notice::UserLeft { voice_chat, user } => {
                if self.is_ours(voice_chat)
                    && user != self.me
                    && let Some(a) = &mut self.active
                {
                    a.people.remove(&user);
                }
            }
            Notice::Status { voice_chat, user, muted, deaf, no_mic } => {
                if self.is_ours(voice_chat)
                    && let Some(a) = &mut self.active
                {
                    let p = a.people.entry(user).or_insert(Participant { steam_id: user, ..Default::default() });
                    (p.muted, p.deaf, p.no_mic) = (muted, deaf, no_mic);
                }
            }
            Notice::Ended { voice_chat } => {
                self.incoming.retain(|(_, v)| *v != voice_chat);
                if self.is_ours(voice_chat) {
                    self.finish(CallEnd::Finished, false, out);
                }
            }
            Notice::Rejoin { group, room } => {
                let target = CallTarget::Room { group, room };
                if self.active.as_ref().is_some_and(|a| a.target == target) {
                    // Steam moved the channel to a new voice session.
                    let Some(old) = self.active.take() else { return };
                    self.generation += 1;
                    out.push(Action::Transmit(false));
                    out.push(Action::StopEngine);
                    drop(old);
                    self.start(target, out);
                    out.push(Action::Connect);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME_ID: SteamId = INDIVIDUAL + 1;
    const FRIEND: SteamId = INDIVIDUAL + 2;
    const OTHER: SteamId = INDIVIDUAL + 3;
    const SESSION: Session = Session { ssrc: 9, client_ip: 1, client_port: 2, server_ip: 3, server_port: 4 };
    const ROOM: CallTarget = CallTarget::Room { group: 10, room: 20 };

    fn machine() -> Machine {
        Machine::new(ME_ID, VoiceSettings::default())
    }

    fn op(m: &mut Machine, o: VoiceOp) -> Vec<Action> {
        m.handle(Input::Op(o))
    }

    fn notice(m: &mut Machine, n: Notice) -> Vec<Action> {
        m.handle(Input::Notice(n))
    }

    fn reply(m: &mut Machine, r: Reply) -> Vec<Action> {
        m.handle(Input::Reply(r))
    }

    /// Answer and audio up, both sides.
    fn link_up(m: &mut Machine) -> Vec<Action> {
        let a = reply(m, Reply::Offer(Ok("answer".into())));
        assert_eq!(a, [Action::StartEngine { answer: "answer".into() }]);
        assert!(m.handle(Input::Engine(EngineEvent::Connected)).is_empty(), "waits for Steam to confirm too");
        notice(m, Notice::SessionConnected(SESSION))
    }

    fn in_room(m: &mut Machine) {
        assert_eq!(op(m, VoiceOp::JoinRoom { group: 10, room: 20 }), [Action::Connect]);
        assert_eq!(link_up(m), [Action::JoinRoom { group: 10, room: 20 }]);
        assert_eq!(reply(m, Reply::Joined(Ok(77))), [Action::Report { voice_chat: 77, session: SESSION }]);
        reply(m, Reply::Reported(Ok(())));
    }

    fn phase(m: &Machine) -> Option<CallPhase> {
        m.state().call.map(|c| c.phase)
    }

    #[test]
    fn joining_a_channel_goes_step_by_step() {
        let mut m = machine();
        assert_eq!(op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 }), [Action::Connect]);
        assert_eq!(phase(&m), Some(CallPhase::Connecting));
        assert_eq!(link_up(&mut m), [Action::JoinRoom { group: 10, room: 20 }]);
        assert_eq!(reply(&mut m, Reply::Joined(Ok(77))), [Action::Report { voice_chat: 77, session: SESSION }]);
        let a = reply(&mut m, Reply::Reported(Ok(())));
        assert_eq!(a, [Action::Transmit(true), Action::SendStatus { voice_chat: 77, muted: false, deaf: false, no_mic: false }]);
        let c = m.state().call.unwrap();
        assert_eq!((c.phase, c.target), (CallPhase::InCall, ROOM));
        assert_eq!(c.people.len(), 1, "you're in it");
    }

    #[test]
    fn the_order_steam_confirms_in_doesnt_matter() {
        let mut m = machine();
        op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 });
        reply(&mut m, Reply::Offer(Ok("answer".into())));
        assert!(notice(&mut m, Notice::SessionConnected(SESSION)).is_empty());
        assert_eq!(m.handle(Input::Engine(EngineEvent::Connected)), [Action::JoinRoom { group: 10, room: 20 }]);
    }

    #[test]
    fn calling_a_friend_rings_until_they_pick_up() {
        let mut m = machine();
        assert_eq!(op(&mut m, VoiceOp::CallFriend(FRIEND)), [Action::Connect]);
        let a = link_up(&mut m);
        assert_eq!(a, [Action::RequestCall(FRIEND), Action::RingTimer, Action::Ring(true)]);
        assert_eq!(phase(&m), Some(CallPhase::Ringing));
        assert!(reply(&mut m, Reply::Requested(Ok(55))).is_empty());
        // No microphone goes out while it rings.
        let a = notice(&mut m, Notice::CallResponse { voice_chat: 55, from: FRIEND, accepted: true });
        assert_eq!(a, [Action::Report { voice_chat: 55, session: SESSION }, Action::Ring(false)]);
        let a = reply(&mut m, Reply::Reported(Ok(())));
        assert!(a.contains(&Action::Transmit(true)));
        assert_eq!(phase(&m), Some(CallPhase::InCall));
    }

    #[test]
    fn a_declined_call_ends() {
        let mut m = machine();
        op(&mut m, VoiceOp::CallFriend(FRIEND));
        link_up(&mut m);
        reply(&mut m, Reply::Requested(Ok(55)));
        let a = notice(&mut m, Notice::CallResponse { voice_chat: 55, from: FRIEND, accepted: false });
        assert!(a.contains(&Action::EndCall(FRIEND)));
        assert!(a.contains(&Action::StopEngine));
        assert!(a.contains(&Action::Ended(CallEnd::Declined)));
        assert!(a.contains(&Action::Ring(false)));
        assert!(m.state().call.is_none());
    }

    #[test]
    fn nobody_picks_up() {
        let mut m = machine();
        op(&mut m, VoiceOp::CallFriend(FRIEND));
        link_up(&mut m);
        reply(&mut m, Reply::Requested(Ok(55)));
        let a = m.handle(Input::RingTimeout);
        assert!(a.contains(&Action::Ended(CallEnd::NoAnswer)));
        assert!(a.contains(&Action::EndCall(FRIEND)));
    }

    #[test]
    fn a_ring_timeout_doesnt_end_a_call_in_progress() {
        let mut m = machine();
        in_room(&mut m);
        assert!(m.handle(Input::RingTimeout).is_empty());
        assert_eq!(phase(&m), Some(CallPhase::InCall));
    }

    #[test]
    fn someone_elses_answer_is_ignored() {
        let mut m = machine();
        op(&mut m, VoiceOp::CallFriend(FRIEND));
        link_up(&mut m);
        reply(&mut m, Reply::Requested(Ok(55)));
        assert!(notice(&mut m, Notice::CallResponse { voice_chat: 55, from: OTHER, accepted: true }).is_empty());
        assert!(notice(&mut m, Notice::CallResponse { voice_chat: 66, from: FRIEND, accepted: true }).is_empty());
        assert_eq!(phase(&m), Some(CallPhase::Ringing));
    }

    #[test]
    fn picking_up_tells_steam_first() {
        let mut m = machine();
        let a = notice(&mut m, Notice::CallRequested { voice_chat: 55, from: FRIEND });
        assert_eq!(a, [Action::Ring(true)]);
        assert_eq!(m.state().incoming, [FRIEND]);
        let a = op(&mut m, VoiceOp::Accept(FRIEND));
        assert_eq!(a, [Action::AnswerCall { friend: FRIEND, voice_chat: 55, accept: true }, Action::Ring(false)]);
        assert!(m.state().incoming.is_empty());
        assert_eq!(reply(&mut m, Reply::Answered(Ok(()))), [Action::Connect]);
        assert_eq!(link_up(&mut m), [Action::Report { voice_chat: 55, session: SESSION }], "no ringing step");
        reply(&mut m, Reply::Reported(Ok(())));
        assert_eq!(phase(&m), Some(CallPhase::InCall));
    }

    #[test]
    fn declining_only_answers_steam() {
        let mut m = machine();
        notice(&mut m, Notice::CallRequested { voice_chat: 55, from: FRIEND });
        let a = op(&mut m, VoiceOp::Decline(FRIEND));
        assert_eq!(a, [Action::AnswerCall { friend: FRIEND, voice_chat: 55, accept: false }, Action::Ring(false)]);
        assert!(m.state().call.is_none() && m.state().incoming.is_empty());
    }

    #[test]
    fn a_caller_who_gives_up_stops_the_ringing() {
        let mut m = machine();
        notice(&mut m, Notice::CallRequested { voice_chat: 55, from: FRIEND });
        assert_eq!(notice(&mut m, Notice::Ended { voice_chat: 55 }), [Action::Ring(false)]);
        assert!(m.state().incoming.is_empty());
    }

    #[test]
    fn calling_someone_who_is_calling_you_picks_up() {
        let mut m = machine();
        notice(&mut m, Notice::CallRequested { voice_chat: 55, from: FRIEND });
        let a = op(&mut m, VoiceOp::CallFriend(FRIEND));
        assert!(a.contains(&Action::AnswerCall { friend: FRIEND, voice_chat: 55, accept: true }));
    }

    #[test]
    fn calling_each_other_at_once_connects() {
        let mut m = machine();
        op(&mut m, VoiceOp::CallFriend(FRIEND));
        link_up(&mut m);
        let a = notice(&mut m, Notice::CallRequested { voice_chat: 55, from: FRIEND });
        assert!(a.contains(&Action::AnswerCall { friend: FRIEND, voice_chat: 55, accept: true }));
    }

    #[test]
    fn a_call_while_youre_talking_doesnt_ring_or_interrupt() {
        let mut m = machine();
        in_room(&mut m);
        assert!(notice(&mut m, Notice::CallRequested { voice_chat: 55, from: FRIEND }).is_empty());
        assert_eq!(m.state().incoming, [FRIEND]);
        assert_eq!(phase(&m), Some(CallPhase::InCall));
    }

    #[test]
    fn picking_up_leaves_the_channel_youre_in() {
        let mut m = machine();
        in_room(&mut m);
        notice(&mut m, Notice::CallRequested { voice_chat: 55, from: FRIEND });
        let a = op(&mut m, VoiceOp::Accept(FRIEND));
        assert!(a.contains(&Action::LeaveRoom { group: 10, room: 20 }));
        assert!(a.contains(&Action::StopEngine));
        assert!(a.contains(&Action::AnswerCall { friend: FRIEND, voice_chat: 55, accept: true }));
        assert_eq!(m.state().call.unwrap().target, CallTarget::Friend(FRIEND));
    }

    #[test]
    fn leaving_a_channel_tells_steam_and_stops_audio() {
        let mut m = machine();
        in_room(&mut m);
        let a = op(&mut m, VoiceOp::Leave);
        assert_eq!(a, [Action::LeaveRoom { group: 10, room: 20 }, Action::Transmit(false), Action::StopEngine, Action::Ended(CallEnd::Finished)]);
        assert!(m.state().call.is_none());
    }

    #[test]
    fn hanging_up_before_joining_tells_steam_nothing() {
        let mut m = machine();
        op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 });
        let a = op(&mut m, VoiceOp::Leave);
        assert!(!a.iter().any(|x| matches!(x, Action::LeaveRoom { .. })));
        assert!(a.contains(&Action::StopEngine));
    }

    #[test]
    fn late_replies_after_leaving_do_nothing() {
        let mut m = machine();
        op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 });
        op(&mut m, VoiceOp::Leave);
        assert!(reply(&mut m, Reply::Offer(Ok("answer".into()))).is_empty());
        assert!(reply(&mut m, Reply::Joined(Ok(77))).is_empty());
    }

    #[test]
    fn each_call_gets_a_new_generation() {
        let mut m = machine();
        op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 });
        let g = m.generation();
        op(&mut m, VoiceOp::Leave);
        op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 });
        assert!(m.generation() > g);
    }

    #[test]
    fn who_is_who_comes_from_steam() {
        let mut m = machine();
        in_room(&mut m);
        notice(&mut m, Notice::UserJoined { voice_chat: 77, user: FRIEND });
        let a = notice(&mut m, Notice::Remote { kind: "offer".into(), version: 3, streams: vec![(500, 2)] });
        assert_eq!(a, [Action::Speakers(vec![500]), Action::PersonGain(500, 1.0), Action::Acknowledge { version: 3, session: SESSION }]);
        m.handle(Input::Engine(EngineEvent::Talking(vec![ME, 500])));
        let c = m.state().call.unwrap();
        assert!(c.people.iter().all(|p| p.talking), "{:?}", c.people);
        m.handle(Input::Engine(EngineEvent::Talking(vec![500])));
        let c = m.state().call.unwrap();
        assert!(!c.people.iter().find(|p| p.steam_id == ME_ID).unwrap().talking);
    }

    #[test]
    fn saved_volumes_apply_when_someone_shows_up() {
        let mut s = VoiceSettings::default();
        s.set_volume(FRIEND, 2.0);
        let mut m = Machine::new(ME_ID, s);
        in_room(&mut m);
        let a = notice(&mut m, Notice::Remote { kind: "answer".into(), version: 1, streams: vec![(500, 2)] });
        assert!(a.contains(&Action::PersonGain(500, 2.0)));
        assert!(!a.iter().any(|x| matches!(x, Action::Acknowledge { .. })), "only an offer gets confirmed");
    }

    #[test]
    fn changing_a_persons_volume_applies_right_away() {
        let mut m = machine();
        in_room(&mut m);
        notice(&mut m, Notice::Remote { kind: "answer".into(), version: 1, streams: vec![(500, 2)] });
        let a = op(&mut m, VoiceOp::SetPersonVolume(FRIEND, 0.5));
        assert_eq!(a, [Action::PersonGain(500, 0.5), Action::SaveSettings]);
        assert_eq!(m.settings.volume_of(FRIEND), 0.5);
    }

    #[test]
    fn mute_and_deafen_are_announced_and_carry_over() {
        let mut m = machine();
        in_room(&mut m);
        let a = op(&mut m, VoiceOp::SetMuted(true));
        assert_eq!(a, [Action::Mute(true), Action::SendStatus { voice_chat: 77, muted: true, deaf: false, no_mic: false }]);
        let a = op(&mut m, VoiceOp::SetDeaf(true));
        assert_eq!(a, [Action::Deaf(true), Action::SendStatus { voice_chat: 77, muted: true, deaf: true, no_mic: false }]);
        op(&mut m, VoiceOp::Leave);
        op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 });
        let c = m.state().call.unwrap();
        assert!(c.muted && c.deaf, "still muted in the next call");
    }

    #[test]
    fn muting_outside_a_call_sends_nothing_to_steam() {
        let mut m = machine();
        assert_eq!(op(&mut m, VoiceOp::SetMuted(true)), [Action::Mute(true)]);
    }

    #[test]
    fn other_peoples_status_shows() {
        let mut m = machine();
        in_room(&mut m);
        notice(&mut m, Notice::Status { voice_chat: 77, user: FRIEND, muted: true, deaf: true, no_mic: false });
        let c = m.state().call.unwrap();
        let p = c.people.iter().find(|p| p.steam_id == FRIEND).unwrap();
        assert!(p.muted && p.deaf);
        notice(&mut m, Notice::UserLeft { voice_chat: 77, user: FRIEND });
        assert_eq!(m.state().call.unwrap().people.len(), 1);
    }

    #[test]
    fn news_about_another_call_is_ignored() {
        let mut m = machine();
        in_room(&mut m);
        notice(&mut m, Notice::UserJoined { voice_chat: 999, user: FRIEND });
        notice(&mut m, Notice::Status { voice_chat: 999, user: OTHER, muted: true, deaf: false, no_mic: false });
        assert_eq!(m.state().call.unwrap().people.len(), 1);
        assert!(notice(&mut m, Notice::Ended { voice_chat: 999 }).is_empty());
        assert_eq!(phase(&m), Some(CallPhase::InCall));
    }

    #[test]
    fn steam_ending_the_call_stops_it_without_answering_back() {
        let mut m = machine();
        in_room(&mut m);
        let a = notice(&mut m, Notice::Ended { voice_chat: 77 });
        assert_eq!(a, [Action::Transmit(false), Action::StopEngine, Action::Ended(CallEnd::Finished)]);
    }

    #[test]
    fn steam_moving_a_channel_rejoins_it() {
        let mut m = machine();
        in_room(&mut m);
        let a = notice(&mut m, Notice::Rejoin { group: 10, room: 20 });
        assert_eq!(a, [Action::Transmit(false), Action::StopEngine, Action::Connect]);
        assert_eq!(phase(&m), Some(CallPhase::Connecting));
        assert!(notice(&mut m, Notice::Rejoin { group: 1, room: 2 }).is_empty());
    }

    #[test]
    fn a_failed_connection_ends_the_call_with_the_reason() {
        let mut m = machine();
        op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 });
        let a = reply(&mut m, Reply::Offer(Err("refused".into())));
        assert!(a.contains(&Action::Ended(CallEnd::Failed("refused".into()))));
        assert!(m.state().call.is_none());
    }

    #[test]
    fn losing_steam_ends_the_call_quietly() {
        let mut m = machine();
        in_room(&mut m);
        notice(&mut m, Notice::CallRequested { voice_chat: 55, from: FRIEND });
        let a = m.handle(Input::Disconnected);
        assert!(!a.iter().any(|x| matches!(x, Action::LeaveRoom { .. })), "can't reach Steam anyway");
        assert!(a.iter().any(|x| matches!(x, Action::Ended(CallEnd::Failed(_)))));
        assert!(m.state().incoming.is_empty());
    }

    #[test]
    fn no_microphone_is_reported_to_others() {
        let mut m = machine();
        op(&mut m, VoiceOp::JoinRoom { group: 10, room: 20 });
        m.handle(Input::Engine(EngineEvent::NoMic("none".into())));
        link_up(&mut m);
        reply(&mut m, Reply::Joined(Ok(77)));
        let a = reply(&mut m, Reply::Reported(Ok(())));
        assert!(a.contains(&Action::SendStatus { voice_chat: 77, muted: false, deaf: false, no_mic: true }));
        assert_eq!(m.state().call.unwrap().mic_problem.as_deref(), Some("none"));
    }

    #[test]
    fn settings_changes_reach_a_running_call() {
        let mut m = machine();
        in_room(&mut m);
        let mut s = m.settings.clone();
        s.input = Some("mic2".into());
        s.output_volume = 0.5;
        s.push_to_talk = true;
        s.push_to_talk_key = "F13".into();
        let a = op(&mut m, VoiceOp::SetSettings(s));
        assert_eq!(a, [Action::InputDevice(Some("mic2".into())), Action::OutputGain(0.5), Action::PushToTalk(Some("F13".into())), Action::SaveSettings]);
    }

    #[test]
    fn a_made_up_push_to_talk_key_is_off() {
        let mut m = machine();
        m.settings.push_to_talk = true;
        m.settings.push_to_talk_key = "Nope".into();
        assert_eq!(m.push_to_talk(), None);
    }
}

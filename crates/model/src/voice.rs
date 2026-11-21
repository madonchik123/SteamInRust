//! Voice chat: who's in a call, what you've muted, and your audio settings.

use crate::SteamId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Who or what a call is with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallTarget {
    Friend(SteamId),
    Room { group: u64, room: u64 },
}

/// How far along a call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallPhase {
    /// Setting up the audio connection.
    #[default]
    Connecting,
    /// Waiting for the friend to pick up.
    Ringing,
    InCall,
}

/// Someone in a call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Participant {
    pub steam_id: SteamId,
    pub talking: bool,
    /// They muted their microphone.
    pub muted: bool,
    /// They turned their sound off.
    pub deaf: bool,
    /// They joined without a microphone.
    pub no_mic: bool,
}

/// The call you're in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub target: CallTarget,
    pub phase: CallPhase,
    /// Everyone in it, you included.
    pub people: Vec<Participant>,
    pub muted: bool,
    pub deaf: bool,
    /// Why you can't be heard, if the microphone didn't open.
    pub mic_problem: Option<String>,
    /// Why you can't hear, if the speakers didn't open.
    pub speaker_problem: Option<String>,
}

/// Calls and who's ringing you.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoiceState {
    pub call: Option<Call>,
    /// Friends calling you right now.
    pub incoming: Vec<SteamId>,
}

/// A microphone or speaker to choose from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

/// Why a call ended, when it's worth telling you.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallEnd {
    /// You hung up, or everyone left.
    Finished,
    Declined,
    NoAnswer,
    Failed(String),
}

/// Your voice chat settings. Saved on this computer, like Steam's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    /// `None` follows the system's default.
    pub input: Option<String>,
    pub output: Option<String>,
    /// 1.0 is normal; up to 4.0.
    pub input_volume: f32,
    pub output_volume: f32,
    pub push_to_talk: bool,
    /// The key's name (see `voice::ptt::KEYS`).
    pub push_to_talk_key: String,
    /// Volume per person, when it isn't the normal 1.0.
    pub people: HashMap<SteamId, f32>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self { input: None, output: None, input_volume: 1.0, output_volume: 1.0, push_to_talk: false, push_to_talk_key: "Mouse4".into(), people: HashMap::new() }
    }
}

impl VoiceSettings {
    pub fn volume_of(&self, who: SteamId) -> f32 {
        self.people.get(&who).copied().unwrap_or(1.0)
    }

    /// Set someone's volume (0 to 4); the normal 1.0 isn't kept.
    pub fn set_volume(&mut self, who: SteamId, volume: f32) {
        let v = volume.clamp(0.0, 4.0);
        if (v - 1.0).abs() < 0.005 {
            self.people.remove(&who);
        } else {
            self.people.insert(who, v);
        }
    }
}

/// Something to do with voice chat.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceOp {
    CallFriend(SteamId),
    JoinRoom { group: u64, room: u64 },
    /// Pick up a friend's call.
    Accept(SteamId),
    Decline(SteamId),
    /// Hang up, or leave the channel.
    Leave,
    SetMuted(bool),
    SetDeaf(bool),
    SetPersonVolume(SteamId, f32),
    SetSettings(VoiceSettings),
    /// List microphones and speakers (answers with [`VoiceEvent::Devices`]).
    ListDevices,
}

/// What voice chat tells the app.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceEvent {
    State(VoiceState),
    Ended(CallEnd),
    Settings(VoiceSettings),
    Devices { inputs: Vec<AudioDevice>, outputs: Vec<AudioDevice> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_normal_volume_isnt_stored() {
        let mut s = VoiceSettings::default();
        s.set_volume(7, 2.5);
        assert_eq!(s.volume_of(7), 2.5);
        s.set_volume(7, 1.0);
        assert!(s.people.is_empty());
        assert_eq!(s.volume_of(7), 1.0);
    }

    #[test]
    fn volume_stays_in_range() {
        let mut s = VoiceSettings::default();
        s.set_volume(1, 9.0);
        s.set_volume(2, -1.0);
        assert_eq!((s.volume_of(1), s.volume_of(2)), (4.0, 0.0));
    }

    #[test]
    fn old_saved_settings_still_load() {
        let s: VoiceSettings = serde_json::from_str(r#"{"push_to_talk":true}"#).unwrap();
        assert!(s.push_to_talk);
        assert_eq!(s.output_volume, 1.0);
        assert_eq!(s.push_to_talk_key, "Mouse4");
    }
}

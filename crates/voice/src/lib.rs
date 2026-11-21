//! Steam voice chat.
//!
//! Nobody connects to anybody else directly: every person, on any client,
//! has one encrypted audio connection to a Valve voice server, and that
//! server passes the audio around.
//!
//! - [`sdp`]: the text the two sides swap to set the connection up (through
//!   the normal Steam connection).
//! - [`link`]: the connection itself.
//! - [`codec`]: Opus, the voice format Steam uses.
//! - [`mix`]: evens out each person's packets and mixes them together.
//! - [`audio`]: microphones and speakers.
//! - [`ptt`]: whether the push-to-talk key is held.
//! - [`engine`]: runs all of it during a call.

pub mod audio;
pub mod codec;
pub mod engine;
pub mod link;
pub mod mix;
pub mod ptt;
pub mod ring;
pub mod sdp;

pub use audio::{Device, Devices};
pub use engine::{Engine, EngineConfig, EngineEvent};
pub use link::{Connected, Link, LinkEvent};
pub use sdp::Description;

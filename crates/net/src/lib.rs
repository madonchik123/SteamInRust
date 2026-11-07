//! Low-level connection to Steam's Connection Manager (CM) servers.
//!
//! - [`directory`] finds CM servers.
//! - [`Connection`] owns a WebSocket, frames messages, and matches replies to
//!   requests through Steam "job ids".
//! - [`Packet`] is one decoded protobuf message.
//!
//! Nothing here knows about logins or friends; that lives in `auth` and `client`.

pub mod connection;
pub mod directory;
pub mod emsg;
pub mod eresult;
pub mod packet;

pub use connection::Connection;
pub use packet::Packet;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("websocket: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("bad message: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("message too short")]
    Truncated,
    #[error("no Steam servers available")]
    NoServers,
    #[error("request timed out")]
    Timeout,
    #[error("disconnected from Steam")]
    Disconnected,
    #[error("{}", eresult::describe(*.0, .1))]
    EResult(i32, String),
}

pub type Result<T> = std::result::Result<T, Error>;

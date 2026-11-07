//! Steam content downloader.
//!
//! - [`cdn`]: content servers, ranked by measured speed, with failover.
//! - [`manifest`]: depot manifests (file lists with chunk maps).
//! - [`chunk`]: decrypt + decompress chunks (VZip/LZMA, VZstd, zip).
//! - [`install`]: install/update a set of depots into a folder.
//!
//! Getting depot keys, manifest request codes and the server list needs a
//! logged-in CM connection; that orchestration lives in the `client` crate.

pub mod cdn;
pub mod chunk;
pub mod crypto;
pub mod install;
pub mod manifest;

pub use cdn::{Cdn, Server};
pub use install::{DepotJob, Phase, Progress, Report, install};
pub use manifest::Manifest;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("download failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("disk error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad data: {0}")]
    Format(String),
    #[error("bad manifest: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("decryption failed: {0}")]
    Crypto(String),
    #[error("no content servers available")]
    NoServers,
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

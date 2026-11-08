//! Valve's SteamNetworkingSockets protocol, the one Steam's own client
//! speaks for ISteamNetworkingSockets / ISteamNetworkingMessages: signed
//! certificates, the P2P rendezvous through Steam, the encrypted SNP
//! transport, and the ICE and relay (SDR) paths it travels on.
//! Ported from github.com/ValveSoftware/GameNetworkingSockets.

#![allow(dead_code)]

pub mod conn;
pub mod crypto;
pub mod engine;
pub mod ice;
pub mod msg;
pub mod snp;

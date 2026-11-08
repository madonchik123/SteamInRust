//! The protobuf messages of SteamNetworkingSockets' wire protocol
//! (steamnetworkingsockets_messages*.proto in Valve's GameNetworkingSockets),
//! written out by hand: only the fields we read or write.

#![allow(clippy::derive_partial_eq_without_eq)]

use prost::Message;

/// `CMsgSteamDatagramCertificate`.
#[derive(Clone, PartialEq, Message)]
pub struct Cert {
    /// 1 = Ed25519.
    #[prost(int32, optional, tag = "1")]
    pub key_type: Option<i32>,
    #[prost(bytes = "vec", optional, tag = "2")]
    pub key_data: Option<Vec<u8>>,
    #[prost(fixed64, optional, tag = "4")]
    pub legacy_steam_id: Option<u64>,
    #[prost(string, optional, tag = "12")]
    pub identity_string: Option<String>,
    #[prost(fixed32, repeated, packed = "false", tag = "5")]
    pub gameserver_datacenter_ids: Vec<u32>,
    #[prost(fixed32, optional, tag = "8")]
    pub time_created: Option<u32>,
    #[prost(fixed32, optional, tag = "9")]
    pub time_expiry: Option<u32>,
    #[prost(uint32, repeated, packed = "false", tag = "10")]
    pub app_ids: Vec<u32>,
}

/// `CMsgSteamDatagramCertificateSigned`: the serialized cert as Steam (or we)
/// made it, so the signature stays valid.
#[derive(Clone, PartialEq, Message)]
pub struct CertSigned {
    #[prost(bytes = "vec", optional, tag = "4")]
    pub cert: Option<Vec<u8>>,
    #[prost(fixed64, optional, tag = "5")]
    pub ca_key_id: Option<u64>,
    #[prost(bytes = "vec", optional, tag = "6")]
    pub ca_signature: Option<Vec<u8>>,
}

/// `CMsgSteamDatagramSessionCryptInfo`.
#[derive(Clone, PartialEq, Message)]
pub struct CryptInfo {
    /// 1 = Curve25519.
    #[prost(int32, optional, tag = "1")]
    pub key_type: Option<i32>,
    #[prost(bytes = "vec", optional, tag = "2")]
    pub key_data: Option<Vec<u8>>,
    #[prost(fixed64, optional, tag = "3")]
    pub nonce: Option<u64>,
    #[prost(uint32, optional, tag = "4")]
    pub protocol_version: Option<u32>,
    /// ESteamNetworkingSocketsCipher: 1 null, 2 AES-256-GCM.
    #[prost(int32, repeated, packed = "false", tag = "5")]
    pub ciphers: Vec<i32>,
}

/// `CMsgSteamDatagramSessionCryptInfoSigned`.
#[derive(Clone, PartialEq, Message)]
pub struct CryptInfoSigned {
    #[prost(bytes = "vec", optional, tag = "1")]
    pub info: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "2")]
    pub signature: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
pub struct IceCandidate {
    #[prost(string, optional, tag = "3")]
    pub candidate: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct IceAuth {
    #[prost(string, optional, tag = "1")]
    pub pwd_frag: Option<String>,
}

/// `CMsgICERendezvous`.
#[derive(Clone, PartialEq, Message)]
pub struct IceRendezvous {
    #[prost(message, optional, tag = "2")]
    pub auth: Option<IceAuth>,
    #[prost(message, optional, tag = "1")]
    pub add_candidate: Option<IceCandidate>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ConnectRequest {
    #[prost(message, optional, tag = "6")]
    pub crypt: Option<CryptInfoSigned>,
    #[prost(message, optional, tag = "7")]
    pub cert: Option<CertSigned>,
    #[prost(uint32, optional, tag = "9")]
    pub to_virtual_port: Option<u32>,
    #[prost(uint32, optional, tag = "10")]
    pub from_virtual_port: Option<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ConnectOk {
    #[prost(message, optional, tag = "5")]
    pub crypt: Option<CryptInfoSigned>,
    #[prost(message, optional, tag = "6")]
    pub cert: Option<CertSigned>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ConnectionClosed {
    #[prost(string, optional, tag = "5")]
    pub debug: Option<String>,
    #[prost(uint32, optional, tag = "6")]
    pub reason_code: Option<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ReliableMessage {
    #[prost(message, optional, tag = "1")]
    pub ice: Option<IceRendezvous>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ApplicationMessage {
    #[prost(bytes = "vec", optional, tag = "1")]
    pub data: Option<Vec<u8>>,
    #[prost(uint64, optional, tag = "2")]
    pub msg_num: Option<u64>,
    #[prost(uint32, optional, tag = "3")]
    pub flags: Option<u32>,
    #[prost(uint32, optional, tag = "4")]
    pub lane_idx: Option<u32>,
}

/// `CMsgSteamNetworkingP2PRendezvous`: what peers send each other through
/// Steam (`ClientP2PConnectionInfo.rendezvous`).
#[derive(Clone, PartialEq, Message)]
pub struct Rendezvous {
    #[prost(string, optional, tag = "8")]
    pub from_identity: Option<String>,
    #[prost(fixed32, optional, tag = "9")]
    pub from_connection_id: Option<u32>,
    #[prost(string, optional, tag = "10")]
    pub to_identity: Option<String>,
    #[prost(fixed32, optional, tag = "1")]
    pub to_connection_id: Option<u32>,
    /// Serialized `CMsgSteamDatagramP2PRoutes` (relay routes).
    #[prost(bytes = "vec", optional, tag = "2")]
    pub sdr_routes: Option<Vec<u8>>,
    #[prost(uint32, optional, tag = "3")]
    pub ack_peer_routes_revision: Option<u32>,
    #[prost(bool, optional, tag = "7")]
    pub ice_enabled: Option<bool>,
    #[prost(bytes = "vec", optional, tag = "14")]
    pub hosted_server_ticket: Option<Vec<u8>>,
    #[prost(message, optional, tag = "4")]
    pub connect_request: Option<ConnectRequest>,
    #[prost(message, optional, tag = "5")]
    pub connect_ok: Option<ConnectOk>,
    #[prost(message, optional, tag = "6")]
    pub connection_closed: Option<ConnectionClosed>,
    #[prost(uint32, optional, tag = "11")]
    pub ack_reliable_msg: Option<u32>,
    #[prost(uint32, optional, tag = "12")]
    pub first_reliable_msg: Option<u32>,
    #[prost(message, repeated, tag = "13")]
    pub reliable_messages: Vec<ReliableMessage>,
    #[prost(message, repeated, tag = "15")]
    pub application_messages: Vec<ApplicationMessage>,
}

/// `CMsgSteamSockets_UDP_Stats`, inline in data packets.
#[derive(Clone, PartialEq, Message)]
pub struct UdpStats {
    /// `CMsgSteamDatagramConnectionQuality`, kept opaque.
    #[prost(bytes = "vec", optional, tag = "1")]
    pub stats: Option<Vec<u8>>,
    /// 2 ack request, 4 immediately, 16 not the primary transport.
    #[prost(uint32, optional, tag = "3")]
    pub flags: Option<u32>,
}

/// `CMsgSteamSockets_UDP_ConnectionClosed`.
#[derive(Clone, PartialEq, Message)]
pub struct UdpConnectionClosed {
    #[prost(fixed32, optional, tag = "4")]
    pub to_connection_id: Option<u32>,
    #[prost(fixed32, optional, tag = "5")]
    pub from_connection_id: Option<u32>,
    #[prost(string, optional, tag = "2")]
    pub debug: Option<String>,
    #[prost(uint32, optional, tag = "3")]
    pub reason_code: Option<u32>,
}

/// `CMsgSteamSockets_UDP_NoConnection`.
#[derive(Clone, PartialEq, Message)]
pub struct UdpNoConnection {
    #[prost(fixed32, optional, tag = "2")]
    pub from_connection_id: Option<u32>,
    #[prost(fixed32, optional, tag = "3")]
    pub to_connection_id: Option<u32>,
}

pub fn decode<M: Message + Default>(b: &[u8]) -> Option<M> {
    M::decode(b).ok()
}

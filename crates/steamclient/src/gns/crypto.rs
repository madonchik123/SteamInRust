//! Keys and the session handshake, exactly as Valve's GameNetworkingSockets
//! does them (steamnetworkingsockets_connections.cpp, FinalizeLocalCrypto
//! and FinishCryptoHandshake):
//!
//! - every player has an Ed25519 key with a certificate Steam signs
//!   (`ClientNetworkingCertRequest`, EMsg 5621, for the raw public key);
//! - each connection makes an X25519 key, signs it with the Ed25519 key and
//!   sends both in ConnectRequest / ConnectOK;
//! - keys come from SHA-256(X25519) through an HMAC-SHA256 extract/expand
//!   over both certs and both crypt infos;
//! - packets are AES-256-GCM, no AAD, the IV plus the packet number.

use super::msg::{Cert, CertSigned, CryptInfo, CryptInfoSigned};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use prost::Message;
use sha2::{Digest, Sha256};

/// The protocol version we claim. 11+ understands lanes, 13 fixed the
/// ack-frame width bit; Valve's current client speaks 13.
pub const PROTOCOL_VERSION: u32 = 11;
pub const CIPHER_NULL: i32 = 1;
pub const CIPHER_AES_GCM: i32 = 2;
pub const TAG_SIZE: usize = 16;

/// Valve's certificate authority (from steamclient64.dll):
/// `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJrsoE4XUc5iaNVpACyh4fobLbwm02tOo6AIOtNygpuE`.
pub const VALVE_CA_KEY_ID: u64 = 18220590129359924542;
pub const VALVE_CA_KEY: [u8; 32] = [
    0x9a, 0xec, 0xa0, 0x4e, 0x17, 0x51, 0xce, 0x62, 0x68, 0xd5, 0x69, 0x00, 0x2c, 0xa1, 0xe1, 0xfa, 0x1b, 0x2d, 0xbc, 0x26, 0xd3, 0x6b, 0x4e, 0xa3, 0xa0, 0x08,
    0x3a, 0xd3, 0x72, 0x82, 0x9b, 0x84,
];

pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut b);
    b
}

pub fn random_u32() -> u32 {
    u32::from_le_bytes(random())
}

/// Our Ed25519 key and the certificate peers see.
#[derive(Clone)]
pub struct LocalCert {
    pub key: SigningKey,
    /// `CMsgSteamDatagramCertificateSigned` as sent to peers.
    pub signed: CertSigned,
    /// Signed by Valve (false: our own unsigned cert).
    pub trusted: bool,
}

impl LocalCert {
    pub fn new_key() -> SigningKey {
        SigningKey::from_bytes(&random())
    }

    /// An unsigned cert (used when Steam can't sign one, and between our own tests).
    pub fn unsigned(key: SigningKey, steam_id: u64, app_id: u32) -> Self {
        let cert = Cert {
            key_type: Some(1),
            key_data: Some(key.verifying_key().to_bytes().to_vec()),
            legacy_steam_id: Some(steam_id),
            identity_string: Some(identity_string(steam_id)),
            app_ids: vec![app_id],
            ..Default::default()
        };
        LocalCert { key, signed: CertSigned { cert: Some(cert.encode_to_vec()), ..Default::default() }, trusted: false }
    }

    /// A cert Steam signed for `key`.
    pub fn signed(key: SigningKey, cert: Vec<u8>, ca_key_id: u64, ca_signature: Vec<u8>) -> Self {
        LocalCert { key, signed: CertSigned { cert: Some(cert), ca_key_id: Some(ca_key_id), ca_signature: Some(ca_signature) }, trusted: true }
    }

    pub fn cert_bytes(&self) -> &[u8] {
        self.signed.cert.as_deref().unwrap_or_default()
    }

    /// Seconds (Unix time) when the cert stops being valid, if it says.
    pub fn expiry(&self) -> Option<u32> {
        Cert::decode(self.cert_bytes()).ok()?.time_expiry
    }
}

/// SteamNetworkingIdentity::ToString for a Steam ID.
pub fn identity_string(steam_id: u64) -> String {
    format!("steamid:{steam_id}")
}

pub fn parse_identity(s: &str) -> Option<u64> {
    s.strip_prefix("steamid:")?.parse().ok()
}

/// The Steam ID a cert was issued to (0: none).
pub fn cert_steam_id(c: &Cert) -> u64 {
    c.identity_string.as_deref().and_then(parse_identity).or(c.legacy_steam_id).unwrap_or(0)
}

/// Checks a peer's cert: Valve's signature (when present), the identity,
/// the expiry. Returns the parsed cert and whether Valve signed it.
pub fn check_remote_cert(signed: &CertSigned, expect: u64, now_unix: u64) -> Result<(Cert, bool), String> {
    let raw = signed.cert.as_deref().ok_or("crypto handshake missing cert")?;
    let cert = Cert::decode(raw).map_err(|_| "cert failed protobuf decode")?;
    let mut trusted = false;
    if let Some(sig) = signed.ca_signature.as_deref() {
        if signed.ca_key_id == Some(VALVE_CA_KEY_ID) {
            let ca = VerifyingKey::from_bytes(&VALVE_CA_KEY).map_err(|e| e.to_string())?;
            let sig = ed25519_dalek::Signature::from_slice(sig).map_err(|_| "bad CA signature")?;
            ca.verify(raw, &sig).map_err(|_| "cert signature isn't Valve's")?;
            trusted = true;
        } else {
            // Signed by a CA we don't know (intermediate keys); Valve's peers
            // check that, we only use the key.
            crate::log!("gns: cert from {expect} signed by CA {:?}", signed.ca_key_id);
        }
    }
    let id = cert_steam_id(&cert);
    if id != 0 && expect != 0 && id != expect {
        return Err(format!("cert was issued to steamid:{id}, not steamid:{expect}"));
    }
    if let Some(exp) = cert.time_expiry
        && trusted
        && (exp as u64) + 3600 < now_unix
    {
        return Err("cert expired".into());
    }
    if cert.key_type != Some(1) || cert.key_data.as_ref().is_none_or(|k| k.len() != 32) {
        return Err("cert has no Ed25519 key".into());
    }
    Ok((cert, trusted))
}

/// Our half of a connection's key exchange.
pub struct LocalCrypt {
    secret: x25519_dalek::StaticSecret,
    pub info: CryptInfo,
    /// `CMsgSteamDatagramSessionCryptInfoSigned` as sent.
    pub signed: CryptInfoSigned,
}

impl LocalCrypt {
    /// Makes and signs our crypt info. `ciphers`: what we offer (client), or
    /// the single one we picked (server).
    pub fn new(cert: &LocalCert, ciphers: Vec<i32>) -> Self {
        let secret = x25519_dalek::StaticSecret::from(random::<32>());
        let public = x25519_dalek::PublicKey::from(&secret);
        let info = CryptInfo {
            key_type: Some(1),
            key_data: Some(public.as_bytes().to_vec()),
            nonce: Some(u64::from_le_bytes(random())),
            protocol_version: Some(PROTOCOL_VERSION),
            ciphers,
        };
        let raw = info.encode_to_vec();
        let sig = cert.key.sign(&raw);
        LocalCrypt { secret, info, signed: CryptInfoSigned { info: Some(raw), signature: Some(sig.to_bytes().to_vec()) } }
    }

    pub fn info_bytes(&self) -> &[u8] {
        self.signed.info.as_deref().unwrap_or_default()
    }
}

/// A peer's crypt info, its signature checked against their cert key.
pub fn check_remote_crypt(signed: &CryptInfoSigned, cert: &Cert) -> Result<CryptInfo, String> {
    let raw = signed.info.as_deref().ok_or("crypto handshake missing session data")?;
    let key: [u8; 32] = cert.key_data.as_deref().and_then(|k| k.try_into().ok()).ok_or("bad cert key")?;
    let vk = VerifyingKey::from_bytes(&key).map_err(|_| "bad cert key")?;
    let sig = ed25519_dalek::Signature::from_slice(signed.signature.as_deref().unwrap_or_default()).map_err(|_| "bad crypt signature")?;
    vk.verify(raw, &sig).map_err(|_| "crypt info signature doesn't match the cert")?;
    let info = CryptInfo::decode(raw).map_err(|_| "crypt info failed protobuf decode")?;
    if info.key_type != Some(1) || info.key_data.as_ref().is_none_or(|k| k.len() != 32) {
        return Err("unsupported DH key".into());
    }
    if info.protocol_version.unwrap_or(0) < 8 {
        return Err(format!("peer runs protocol {}, too old", info.protocol_version.unwrap_or(0)));
    }
    Ok(info)
}

/// What both sides of the handshake hashed into the keys.
pub struct Context<'a> {
    pub server: bool,
    pub local_id: u32,
    pub remote_id: u32,
    pub cert_local: &'a [u8],
    pub cert_remote: &'a [u8],
    pub crypt_local: &'a [u8],
    pub crypt_remote: &'a [u8],
    pub nonce_local: u64,
    pub nonce_remote: u64,
}

type HmacSha256 = Hmac<Sha256>;

/// Valve's GenerateHMAC256(data, key).
fn hmac256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("any key size");
    m.update(data);
    m.finalize().into_bytes().into()
}

/// The session's packet keys.
pub struct Session {
    /// None: the null cipher.
    send: Option<Aes256Gcm>,
    recv: Option<Aes256Gcm>,
    send_iv: [u8; 12],
    recv_iv: [u8; 12],
}

impl Session {
    pub fn derive(local: LocalCrypt, remote_public: &[u8], cipher: i32, c: &Context) -> Result<Session, String> {
        let pk: [u8; 32] = remote_public.try_into().map_err(|_| "invalid DH key")?;
        let shared = local.secret.diffie_hellman(&x25519_dalek::PublicKey::from(pk));
        let premaster: [u8; 32] = Sha256::digest(shared.as_bytes()).into();
        let mut salt = [c.nonce_remote, c.nonce_local];
        if c.server {
            salt.swap(0, 1);
        }
        let salt_bytes: Vec<u8> = salt.iter().flat_map(|n| n.to_le_bytes()).collect();
        let prk = hmac256(&premaster, &salt_bytes);

        let mut ids = [c.local_id, c.remote_id];
        let mut ctx = [c.cert_remote, c.cert_local, c.crypt_remote, c.crypt_local];
        if c.server {
            ids.swap(0, 1);
            ctx.swap(0, 1);
            ctx.swap(2, 3);
        }
        let mut body = Vec::new();
        body.extend_from_slice(&ids[0].to_le_bytes());
        body.extend_from_slice(&ids[1].to_le_bytes());
        body.extend_from_slice(b"Steam datagram");
        for part in ctx {
            body.extend_from_slice(part);
        }
        // T(i) = HMAC(PRK, T(i-1) || body || i)
        let mut out = [[0u8; 32]; 4];
        let mut prev: Option<[u8; 32]> = None;
        for (i, o) in out.iter_mut().enumerate() {
            let mut data = Vec::with_capacity(body.len() + 33);
            if let Some(p) = prev {
                data.extend_from_slice(&p);
            }
            data.extend_from_slice(&body);
            data.push(i as u8 + 1);
            *o = hmac256(&prk, &data);
            prev = Some(*o);
        }
        // Order: send key, recv key, send IV, recv IV (the server swaps).
        let (mut ks, mut kr, mut ivs, mut ivr) = (out[0], out[1], out[2], out[3]);
        if c.server {
            std::mem::swap(&mut ks, &mut kr);
            std::mem::swap(&mut ivs, &mut ivr);
        }
        let iv = |b: [u8; 32]| -> [u8; 12] { b[..12].try_into().unwrap() };
        let aes = |k: [u8; 32]| (cipher == CIPHER_AES_GCM).then(|| Aes256Gcm::new_from_slice(&k).expect("32-byte key"));
        Ok(Session { send: aes(ks), recv: aes(kr), send_iv: iv(ivs), recv_iv: iv(ivr) })
    }

    fn nonce(iv: &[u8; 12], pkt: u64) -> [u8; 12] {
        let mut n = *iv;
        let low = u64::from_le_bytes(n[..8].try_into().unwrap()).wrapping_add(pkt);
        n[..8].copy_from_slice(&low.to_le_bytes());
        n
    }

    pub fn overhead(&self) -> usize {
        if self.send.is_some() { TAG_SIZE } else { 0 }
    }

    pub fn encrypt(&self, pkt: u64, plain: &[u8]) -> Vec<u8> {
        match &self.send {
            Some(c) => c.encrypt(Nonce::from_slice(&Self::nonce(&self.send_iv, pkt)), plain).unwrap_or_default(),
            None => plain.to_vec(),
        }
    }

    pub fn decrypt(&self, pkt: u64, data: &[u8]) -> Option<Vec<u8>> {
        match &self.recv {
            Some(c) => c.decrypt(Nonce::from_slice(&Self::nonce(&self.recv_iv, pkt)), data).ok(),
            None => Some(data.to_vec()),
        }
    }
}

/// Picks the cipher for an incoming connection: the first of ours they offer.
pub fn negotiate(ours: &[i32], theirs: &[i32]) -> Option<i32> {
    let theirs: Vec<i32> = if theirs.is_empty() { vec![CIPHER_AES_GCM] } else { theirs.to_vec() };
    ours.iter().copied().find(|c| theirs.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valve_ca_key_matches_the_ssh_string() {
        // base64 of the ssh-ed25519 blob: "ssh-ed25519" length-prefixed, then the key.
        let b64 = "AAAAC3NzaC1lZDI1NTE5AAAAIJrsoE4XUc5iaNVpACyh4fobLbwm02tOo6AIOtNygpuE";
        let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bits = 0u32;
        let mut n = 0;
        let mut out = Vec::new();
        for ch in b64.bytes() {
            let v = table.iter().position(|&t| t == ch).unwrap() as u32;
            bits = bits << 6 | v;
            n += 6;
            if n >= 8 {
                n -= 8;
                out.push((bits >> n) as u8);
            }
        }
        assert_eq!(&out[out.len() - 32..], &VALVE_CA_KEY);
    }

    #[test]
    fn both_sides_derive_matching_keys() {
        let (ca, cb) = (LocalCert::unsigned(LocalCert::new_key(), 1, 480), LocalCert::unsigned(LocalCert::new_key(), 2, 480));
        let client = LocalCrypt::new(&ca, vec![CIPHER_AES_GCM, CIPHER_NULL]);
        let server = LocalCrypt::new(&cb, vec![CIPHER_AES_GCM]);
        let (cert_a, _) = check_remote_cert(&ca.signed, 1, 0).unwrap();
        let info_a = check_remote_crypt(&client.signed, &cert_a).unwrap();
        let (cert_b, _) = check_remote_cert(&cb.signed, 2, 0).unwrap();
        let info_b = check_remote_crypt(&server.signed, &cert_b).unwrap();
        let (ci, si) = (client.info_bytes().to_vec(), server.info_bytes().to_vec());
        let (cn, sn) = (client.info.nonce.unwrap(), server.info.nonce.unwrap());
        let a = Session::derive(
            client,
            info_b.key_data.as_deref().unwrap(),
            CIPHER_AES_GCM,
            &Context { server: false, local_id: 11, remote_id: 22, cert_local: ca.cert_bytes(), cert_remote: cb.cert_bytes(), crypt_local: &ci, crypt_remote: &si, nonce_local: cn, nonce_remote: sn },
        )
        .unwrap();
        let b = Session::derive(
            server,
            info_a.key_data.as_deref().unwrap(),
            CIPHER_AES_GCM,
            &Context { server: true, local_id: 22, remote_id: 11, cert_local: cb.cert_bytes(), cert_remote: ca.cert_bytes(), crypt_local: &si, crypt_remote: &ci, nonce_local: sn, nonce_remote: cn },
        )
        .unwrap();
        let sealed = a.encrypt(7, b"hello");
        assert_eq!(sealed.len(), 5 + TAG_SIZE);
        assert_eq!(b.decrypt(7, &sealed).unwrap(), b"hello");
        assert!(b.decrypt(8, &sealed).is_none());
        assert_eq!(a.decrypt(9, &b.encrypt(9, b"back")).unwrap(), b"back");
    }
}

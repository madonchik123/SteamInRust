//! Wire format of a protobuf CM message:
//!
//! ```text
//! u32 LE  emsg | 0x8000_0000   (high bit = protobuf message)
//! u32 LE  header length
//! [..]    CMsgProtoBufHeader
//! [..]    body (a protobuf message, type depends on emsg)
//! ```

use crate::{Error, Result, emsg};
use prost::Message;
use proto::CMsgProtoBufHeader;
use std::io::Read;

const PROTO_MASK: u32 = 0x8000_0000;
/// Upper bound for a single decompressed `CMsgMulti` payload (64 MiB).
pub(crate) const MAX_MULTI_UNZIPPED: usize = 64 * 1024 * 1024;
/// Upper bound for eager `Vec::with_capacity` preallocation from untrusted headers.
const MAX_MULTI_PREALLOC: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Packet {
    pub emsg: u32,
    pub header: CMsgProtoBufHeader,
    pub body: Vec<u8>,
}

impl Packet {
    /// Decode one message. Returns `Ok(None)` for legacy non-protobuf
    /// messages, which the WebSocket transport doesn't use for anything we need.
    pub fn decode(data: &[u8]) -> Result<Option<Packet>> {
        if data.len() < 8 {
            return Err(Error::Truncated);
        }
        let raw = u32::from_le_bytes(data[0..4].try_into().unwrap());
        if raw & PROTO_MASK == 0 {
            tracing::trace!("skipping non-protobuf message {}", emsg::name(raw));
            return Ok(None);
        }
        let header_len = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        let body_start = 8usize.checked_add(header_len).ok_or(Error::Truncated)?;
        if data.len() < body_start {
            return Err(Error::Truncated);
        }
        Ok(Some(Packet {
            emsg: raw & !PROTO_MASK,
            header: CMsgProtoBufHeader::decode(&data[8..body_start])?,
            body: data[body_start..].to_vec(),
        }))
    }

    pub fn encode(emsg: u32, header: &CMsgProtoBufHeader, body: &impl Message) -> Vec<u8> {
        let header_len = header.encoded_len();
        let mut out = Vec::with_capacity(8 + header_len + body.encoded_len());
        out.extend_from_slice(&(emsg | PROTO_MASK).to_le_bytes());
        out.extend_from_slice(&(header_len as u32).to_le_bytes());
        header.encode(&mut out).expect("vec grows");
        body.encode(&mut out).expect("vec grows");
        out
    }

    /// A message in Steam's older, non-protobuf format (a few client
    /// messages still are): the 36-byte extended header, then `body`.
    pub fn encode_plain(emsg: u32, steam_id: u64, session_id: i32, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(36 + body.len());
        out.extend_from_slice(&emsg.to_le_bytes());
        out.push(36); // header size
        out.extend_from_slice(&2u16.to_le_bytes()); // header version
        out.extend_from_slice(&u64::MAX.to_le_bytes()); // target job: none
        out.extend_from_slice(&u64::MAX.to_le_bytes()); // source job: none
        out.push(239); // header canary
        out.extend_from_slice(&steam_id.to_le_bytes());
        out.extend_from_slice(&session_id.to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    pub fn body<M: Message + Default>(&self) -> Result<M> {
        Ok(M::decode(self.body.as_slice())?)
    }

    pub fn eresult(&self) -> i32 {
        self.header.eresult()
    }

    /// Fail with the header's result code unless it is OK.
    pub fn check(&self) -> Result<&Self> {
        match self.eresult() {
            crate::eresult::OK => Ok(self),
            code => Err(Error::EResult(code, self.header.error_message().to_string())),
        }
    }

    /// Expand a `Multi` message (optionally gzipped) into its inner packets.
    pub fn unpack_multi(&self) -> Result<Vec<Packet>> {
        let multi: proto::CMsgMulti = self.body()?;
        let unzipped = multi.size_unzipped();
        let mut data = multi.message_body.unwrap_or_default();
        if unzipped > 0 {
            if unzipped as usize > MAX_MULTI_UNZIPPED {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "CMsgMulti unzipped payload exceeds 64 MiB limit",
                )));
            }
            let mut out = Vec::with_capacity((unzipped as usize).min(MAX_MULTI_PREALLOC));
            flate2::read::GzDecoder::new(data.as_slice())
                .take(MAX_MULTI_UNZIPPED as u64 + 1)
                .read_to_end(&mut out)?;
            if out.len() > MAX_MULTI_UNZIPPED {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "CMsgMulti gzip stream exceeds 64 MiB limit",
                )));
            }
            data = out;
        } else if data.len() > MAX_MULTI_UNZIPPED {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "CMsgMulti payload exceeds 64 MiB limit",
            )));
        }
        let mut packets = Vec::new();
        let mut rest = data.as_slice();
        while rest.len() >= 4 {
            let len = u32::from_le_bytes(rest[0..4].try_into().unwrap()) as usize;
            let end = 4usize.checked_add(len).ok_or(Error::Truncated)?;
            let chunk = rest.get(4..end).ok_or(Error::Truncated)?;
            if let Some(p) = Packet::decode(chunk)? {
                if p.emsg == emsg::MULTI {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "nested CMsgMulti is not allowed",
                    )));
                }
                packets.push(p);
            }
            rest = &rest[end..];
        }
        Ok(packets)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn encodes_a_plain_message() {
        let out = Packet::encode_plain(745, 0x0110_0001_0000_0002, 77, &[1, 2, 3]);
        assert_eq!(out.len(), 39);
        assert_eq!(&out[0..4], &745u32.to_le_bytes());
        assert_eq!((out[4], &out[5..7]), (36, &2u16.to_le_bytes()[..]));
        assert_eq!(&out[7..23], &[0xFF; 16]);
        assert_eq!(out[23], 239);
        assert_eq!(&out[24..32], &0x0110_0001_0000_0002u64.to_le_bytes());
        assert_eq!(&out[32..36], &77i32.to_le_bytes());
        assert_eq!(&out[36..], &[1, 2, 3]);
        // Not mistaken for a protobuf message when read back.
        assert!(Packet::decode(&out).unwrap().is_none());
    }

    use super::*;

    #[test]
    fn roundtrip() {
        let header = CMsgProtoBufHeader { steamid: Some(42), ..Default::default() };
        let body = proto::CMsgClientHello { protocol_version: Some(65580) };
        let bytes = Packet::encode(emsg::CLIENT_HELLO, &header, &body);
        let p = Packet::decode(&bytes).unwrap().unwrap();
        assert_eq!(p.emsg, emsg::CLIENT_HELLO);
        assert_eq!(p.header.steamid, Some(42));
        let back: proto::CMsgClientHello = p.body().unwrap();
        assert_eq!(back.protocol_version, Some(65580));
    }

    #[test]
    fn multi_with_gzip() {
        use flate2::{Compression, write::GzEncoder};
        use std::io::Write;
        let inner = Packet::encode(
            emsg::CLIENT_HEARTBEAT,
            &CMsgProtoBufHeader::default(),
            &proto::CMsgClientHeartBeat::default(),
        );
        let mut framed = (inner.len() as u32).to_le_bytes().to_vec();
        framed.extend(&inner);
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&framed).unwrap();
        let multi = proto::CMsgMulti {
            size_unzipped: Some(framed.len() as u32),
            message_body: Some(gz.finish().unwrap()),
        };
        let bytes = Packet::encode(emsg::MULTI, &CMsgProtoBufHeader::default(), &multi);
        let packets = Packet::decode(&bytes).unwrap().unwrap().unpack_multi().unwrap();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].emsg, emsg::CLIENT_HEARTBEAT);
    }

    #[test]
    fn rejects_nested_multi_and_gzip_bomb() {
        use flate2::{Compression, write::GzEncoder};
        use std::io::Write;

        let inner_multi = Packet::encode(
            emsg::MULTI,
            &CMsgProtoBufHeader::default(),
            &proto::CMsgMulti { size_unzipped: Some(0), message_body: Some(Vec::new()) },
        );
        let mut framed = (inner_multi.len() as u32).to_le_bytes().to_vec();
        framed.extend(&inner_multi);
        let outer = Packet::encode(
            emsg::MULTI,
            &CMsgProtoBufHeader::default(),
            &proto::CMsgMulti { size_unzipped: Some(0), message_body: Some(framed) },
        );
        assert!(Packet::decode(&outer).unwrap().unwrap().unpack_multi().is_err(), "nested MULTI must be rejected");

        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        let block = vec![0u8; 1024 * 1024];
        for _ in 0..65 {
            gz.write_all(&block).unwrap();
        }
        let bomb = Packet::encode(
            emsg::MULTI,
            &CMsgProtoBufHeader::default(),
            &proto::CMsgMulti { size_unzipped: Some(65 * 1024 * 1024), message_body: Some(gz.finish().unwrap()) },
        );
        assert!(Packet::decode(&bomb).unwrap().unwrap().unpack_multi().is_err(), "oversized MULTI gzip must be rejected");
    }
}

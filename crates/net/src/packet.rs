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
        let body_start = 8 + header_len;
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
            let mut out = Vec::with_capacity(unzipped as usize);
            flate2::read::GzDecoder::new(data.as_slice()).read_to_end(&mut out)?;
            data = out;
        }
        let mut packets = Vec::new();
        let mut rest = data.as_slice();
        while rest.len() >= 4 {
            let len = u32::from_le_bytes(rest[0..4].try_into().unwrap()) as usize;
            let chunk = rest.get(4..4 + len).ok_or(Error::Truncated)?;
            if let Some(p) = Packet::decode(chunk)? {
                if p.emsg == emsg::MULTI {
                    packets.extend(p.unpack_multi()?);
                } else {
                    packets.push(p);
                }
            }
            rest = &rest[4 + len..];
        }
        Ok(packets)
    }
}

#[cfg(test)]
mod tests {
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
}

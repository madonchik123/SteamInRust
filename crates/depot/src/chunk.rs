//! Chunk decoding: decrypt, then decompress (VZip/LZMA, VZstd, or plain zip).

use crate::{Error, Result, crypto};
use sha1::{Digest, Sha1};
use std::io::Read;

pub fn decode(raw: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    let plain = crypto::symmetric_decrypt(raw, key)?;
    decompress(&plain)
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    if data.starts_with(b"VSZa") {
        vzstd(data)
    } else if data.starts_with(b"VZa") {
        vzip(data)
    } else if data.starts_with(b"PK") {
        pkzip(data)
    } else {
        Err(Error::Format("unknown chunk compression".into()))
    }
}

/// Valve's LZMA wrapper: "VZa" + u32 + 5 bytes of LZMA properties + data,
/// footer: u32 crc32, u32 uncompressed size, "zv".
fn vzip(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 7 + 5 + 10 || !data.ends_with(b"zv") {
        return Err(Error::Format("bad VZip chunk".into()));
    }
    let footer = &data[data.len() - 10..];
    let size = u32::from_le_bytes(footer[4..8].try_into().unwrap()) as u64;
    let props = &data[7..12];
    let body = &data[12..data.len() - 10];
    // Rebuild a classic .lzma header: properties + 64-bit size.
    let mut stream = Vec::with_capacity(13 + body.len());
    stream.extend_from_slice(props);
    stream.extend_from_slice(&size.to_le_bytes());
    stream.extend_from_slice(body);
    let mut out = Vec::with_capacity(size as usize);
    lzma_rs::lzma_decompress(&mut std::io::Cursor::new(stream), &mut out).map_err(|e| Error::Format(format!("lzma: {e}")))?;
    out.truncate(size as usize);
    Ok(out)
}

/// Valve's zstd wrapper: "VSZa" + u32 crc, then a zstd frame, then a footer.
fn vzstd(data: &[u8]) -> Result<Vec<u8>> {
    let mut dec = ruzstd::decoding::StreamingDecoder::new(&data[8..]).map_err(|e| Error::Format(format!("zstd: {e}")))?;
    let mut out = Vec::new();
    dec.read_to_end(&mut out).map_err(|e| Error::Format(format!("zstd: {e}")))?;
    Ok(out)
}

fn pkzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| Error::Format(format!("zip: {e}")))?;
    let mut file = archive.by_index(0).map_err(|e| Error::Format(format!("zip: {e}")))?;
    let mut out = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut out)?;
    Ok(out)
}

/// A chunk's id is the SHA-1 of its decompressed bytes.
pub fn verify(data: &[u8], sha: &[u8]) -> bool {
    sha.is_empty() || Sha1::digest(data).as_slice() == sha
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_zip_chunk() {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("z", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            z.write_all(b"chunk payload").unwrap();
            z.finish().unwrap();
        }
        let key = [9u8; 32];
        let enc = crate::crypto::symmetric_encrypt_for_tests(buf.get_ref(), &key, [1u8; 16]);
        let out = decode(&enc, &key).unwrap();
        assert_eq!(out, b"chunk payload");
        assert!(verify(&out, Sha1::digest(b"chunk payload").as_slice()));
    }
}

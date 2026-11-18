//! Chunk decoding: decrypt, then decompress (VZip/LZMA, VZstd, or plain zip).

use crate::{Error, Result, crypto};
use sha1::{Digest, Sha1};
use std::io::{Read, Write};

const MAX_CHUNK_SIZE: usize = 32 * 1024 * 1024;
const MAX_PREALLOC: usize = 1024 * 1024;

struct BoundedWriter {
    out: Vec<u8>,
    limit: usize,
}

impl Write for BoundedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.out.len().saturating_add(buf.len()) > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "decompressed chunk exceeds 32 MiB limit",
            ));
        }
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

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
    if size > MAX_CHUNK_SIZE as u64 {
        return Err(Error::Format("vzip chunk exceeds 32 MiB limit".into()));
    }
    let props = &data[7..12];
    let body = &data[12..data.len() - 10];
    // Rebuild a classic .lzma header: properties + 64-bit size.
    let mut stream = Vec::with_capacity(13 + body.len());
    stream.extend_from_slice(props);
    stream.extend_from_slice(&size.to_le_bytes());
    stream.extend_from_slice(body);
    let mut writer = BoundedWriter { out: Vec::with_capacity((size as usize).min(MAX_PREALLOC)), limit: MAX_CHUNK_SIZE };
    lzma_rs::lzma_decompress(&mut std::io::Cursor::new(stream), &mut writer).map_err(|e| Error::Format(format!("lzma: {e}")))?;
    writer.out.truncate(size as usize);
    Ok(writer.out)
}

/// Valve's zstd wrapper: "VSZa" + u32 crc, then a zstd frame, then a footer.
fn vzstd(data: &[u8]) -> Result<Vec<u8>> {
    let payload = data.get(8..).ok_or_else(|| Error::Format("bad VZstd chunk".into()))?;
    let dec = ruzstd::decoding::StreamingDecoder::new(payload).map_err(|e| Error::Format(format!("zstd: {e}")))?;
    let mut out = Vec::new();
    dec.take(MAX_CHUNK_SIZE as u64 + 1).read_to_end(&mut out).map_err(|e| Error::Format(format!("zstd: {e}")))?;
    if out.len() > MAX_CHUNK_SIZE {
        return Err(Error::Format("vzstd chunk exceeds 32 MiB limit".into()));
    }
    Ok(out)
}

fn pkzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| Error::Format(format!("zip: {e}")))?;
    let file = archive.by_index(0).map_err(|e| Error::Format(format!("zip: {e}")))?;
    if file.size() > MAX_CHUNK_SIZE as u64 {
        return Err(Error::Format("zip chunk exceeds 32 MiB limit".into()));
    }
    let mut out = Vec::with_capacity((file.size() as usize).min(MAX_PREALLOC));
    file.take(MAX_CHUNK_SIZE as u64 + 1).read_to_end(&mut out)?;
    if out.len() > MAX_CHUNK_SIZE {
        return Err(Error::Format("zip chunk exceeds 32 MiB limit".into()));
    }
    Ok(out)
}

/// A chunk's id is the 20-byte SHA-1 of its decompressed bytes.
pub fn verify(data: &[u8], sha: &[u8]) -> bool {
    sha.len() == 20 && Sha1::digest(data).as_slice() == sha
}

#[cfg(test)]
mod verify_tests {
    use super::*;

    #[test]
    fn verify_rejects_empty_or_wrong_len_sha() {
        assert!(!verify(b"chunk payload", &[]));
        assert!(!verify(b"", &[]));
        let full = Sha1::digest(b"chunk payload");
        assert!(!verify(b"chunk payload", &full[..19]));
        assert!(!verify(b"chunk payload", &[0u8; 20]));
    }
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

    #[test]
    fn rejects_oversized_and_truncated_chunks() {
        use std::io::Write;
        assert!(decompress(b"VSZa12").is_err());

        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("z", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            let block = vec![0u8; 1024 * 1024];
            for _ in 0..33 {
                z.write_all(&block).unwrap();
            }
            z.finish().unwrap();
        }
        assert!(decompress(buf.get_ref()).is_err(), "zip chunk > 32 MiB must fail");

        let mut vzip_bad = vec![b'V', b'Z', b'a', 0, 0, 0, 0, 0x5d, 0, 0, 0x10, 0, 0];
        vzip_bad.extend_from_slice(&0u32.to_le_bytes());
        vzip_bad.extend_from_slice(&(33u32 * 1024 * 1024).to_le_bytes());
        vzip_bad.extend_from_slice(b"zv");
        assert!(decompress(&vzip_bad).is_err(), "vzip chunk > 32 MiB must fail");
    }
}

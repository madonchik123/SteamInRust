//! Depot manifests: the list of files in one depot version, with the chunks
//! that make up each file. CDN manifests arrive zipped and (usually) with
//! encrypted filenames.

use crate::{Error, Result, crypto};
use prost::Message;
use std::io::Read;

const PAYLOAD: u32 = 0x71F6_17D0;
const METADATA: u32 = 0x1F48_12BE;
const SIGNATURE: u32 = 0x1B81_B817;
const END: u32 = 0x32C4_15AB;

const FLAG_EXECUTABLE: u32 = 32;
const FLAG_DIRECTORY: u32 = 64;
/// Steam's install script (installscript.vdf), run before a game's first launch.
const FLAG_INSTALL_SCRIPT: u32 = 256;
const MAX_MANIFEST_SIZE: usize = 256 * 1024 * 1024;
const MAX_MANIFEST_PREALLOC: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub sha: Vec<u8>,
    pub offset: u64,
    pub size: u32,
    pub compressed: u32,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Relative path with `/` separators.
    pub name: String,
    pub size: u64,
    pub sha: Vec<u8>,
    pub chunks: Vec<Chunk>,
    pub directory: bool,
    /// A program: needs the "may run" permission on macOS and Linux.
    pub executable: bool,
    /// One of Steam's install scripts (`installscript.vdf`).
    pub install_script: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub depot: u32,
    pub gid: u64,
    pub encrypted_names: bool,
    pub files: Vec<FileEntry>,
    /// Size on disk when installed.
    pub original_size: u64,
    /// Bytes to download.
    pub compressed_size: u64,
}

impl Manifest {
    /// Parse a manifest exactly as it is stored in `depotcache` (not zipped).
    pub fn parse(data: &[u8]) -> Result<Manifest> {
        let mut pos = 0;
        let mut m = Manifest::default();
        let mut payload = None;
        let u32_at = |p: usize| p.checked_add(4).and_then(|end| data.get(p..end)).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
        while let Some(magic) = u32_at(pos) {
            pos += 4;
            if magic == END {
                break;
            }
            let len = u32_at(pos).ok_or_else(|| Error::Format("truncated manifest".into()))? as usize;
            pos += 4;
            let end = pos.checked_add(len).ok_or_else(|| Error::Format("truncated manifest section".into()))?;
            let body = data.get(pos..end).ok_or_else(|| Error::Format("truncated manifest section".into()))?;
            pos = end;
            match magic {
                PAYLOAD => payload = Some(proto::ContentManifestPayload::decode(body)?),
                METADATA => {
                    let meta = proto::ContentManifestMetadata::decode(body)?;
                    m.depot = meta.depot_id();
                    m.gid = meta.gid_manifest();
                    m.encrypted_names = meta.filenames_encrypted();
                    m.original_size = meta.cb_disk_original();
                    m.compressed_size = meta.cb_disk_compressed();
                }
                SIGNATURE => {}
                other => return Err(Error::Format(format!("unknown manifest section {other:#x}"))),
            }
        }
        let payload = payload.ok_or_else(|| Error::Format("manifest has no file list".into()))?;
        m.files = payload
            .mappings
            .into_iter()
            .map(|f| {
                let mut chunks: Vec<Chunk> = f
                    .chunks
                    .iter()
                    .map(|c| Chunk { sha: c.sha().to_vec(), offset: c.offset(), size: c.cb_original(), compressed: c.cb_compressed() })
                    .collect();
                chunks.sort_by_key(|c| c.offset);
                FileEntry {
                    name: f.filename().replace('\\', "/"),
                    size: f.size(),
                    sha: f.sha_content().to_vec(),
                    chunks,
                    directory: f.flags() & FLAG_DIRECTORY != 0,
                    executable: f.flags() & FLAG_EXECUTABLE != 0,
                    install_script: f.flags() & FLAG_INSTALL_SCRIPT != 0,
                    link: Some(f.linktarget().to_string()).filter(|l| !l.is_empty()),
                }
            })
            .collect();
        if m.compressed_size == 0 {
            m.compressed_size = m.files.iter().flat_map(|f| &f.chunks).map(|c| c.compressed as u64).sum();
        }
        if m.original_size == 0 {
            m.original_size = m.files.iter().map(|f| f.size).sum();
        }
        Ok(m)
    }

    /// Parse a manifest downloaded from the CDN (a zip with one entry).
    pub fn from_cdn(zipped: &[u8]) -> Result<Manifest> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zipped)).map_err(|e| Error::Format(format!("manifest zip: {e}")))?;
        let file = archive.by_index(0).map_err(|e| Error::Format(format!("manifest zip: {e}")))?;
        if file.size() > MAX_MANIFEST_SIZE as u64 {
            return Err(Error::Format("manifest exceeds 256 MiB limit".into()));
        }
        let mut raw = Vec::with_capacity((file.size() as usize).min(MAX_MANIFEST_PREALLOC));
        file.take(MAX_MANIFEST_SIZE as u64 + 1).read_to_end(&mut raw)?;
        if raw.len() > MAX_MANIFEST_SIZE {
            return Err(Error::Format("manifest exceeds 256 MiB limit".into()));
        }
        Manifest::parse(&raw)
    }

    /// Encode in `depotcache` format (plain filenames), so local verification
    /// and the official client can read it.
    pub fn to_bytes(&self) -> Vec<u8> {
        use proto::content_manifest_payload::{FileMapping, file_mapping::ChunkData};
        let payload = proto::ContentManifestPayload {
            mappings: self
                .files
                .iter()
                .map(|f| FileMapping {
                    filename: Some(f.name.replace('/', "\\")),
                    size: Some(f.size),
                    flags: Some(if f.directory { FLAG_DIRECTORY } else { 0 } | if f.executable { FLAG_EXECUTABLE } else { 0 } | if f.install_script { FLAG_INSTALL_SCRIPT } else { 0 }),
                    sha_content: Some(f.sha.clone()),
                    linktarget: f.link.clone(),
                    chunks: f
                        .chunks
                        .iter()
                        .map(|c| ChunkData { sha: Some(c.sha.clone()), offset: Some(c.offset), cb_original: Some(c.size), cb_compressed: Some(c.compressed), crc: None })
                        .collect(),
                    ..Default::default()
                })
                .collect(),
        };
        let meta = proto::ContentManifestMetadata {
            depot_id: Some(self.depot),
            gid_manifest: Some(self.gid),
            filenames_encrypted: Some(false),
            cb_disk_original: Some(self.original_size),
            cb_disk_compressed: Some(self.compressed_size),
            ..Default::default()
        };
        let mut out = Vec::new();
        for (magic, body) in [(PAYLOAD, payload.encode_to_vec()), (METADATA, meta.encode_to_vec())] {
            out.extend(magic.to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
        }
        out.extend(END.to_le_bytes());
        out
    }

    /// Decrypt filenames with the depot key (no-op if they're plain).
    pub fn decrypt_names(&mut self, key: &[u8]) -> Result<()> {
        if !self.encrypted_names {
            return Ok(());
        }
        for f in &mut self.files {
            f.name = crypto::decrypt_filename(&f.name, key)?.replace('\\', "/");
            if let Some(l) = &f.link {
                f.link = Some(crypto::decrypt_filename(l, key)?);
            }
        }
        self.encrypted_names = false;
        Ok(())
    }
}

#[cfg(test)]
mod manifest_limit_tests {
    use super::*;

    #[test]
    fn rejects_oversized_cdn_manifest() {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("m", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            let block = vec![0u8; 1024 * 1024];
            for _ in 0..257 {
                z.write_all(&block).unwrap();
            }
            z.finish().unwrap();
        }
        let err = Manifest::from_cdn(buf.get_ref()).unwrap_err().to_string();
        assert!(err.contains("256 MiB") || err.contains("limit"), "expected size limit error, got {err}");
    }
}

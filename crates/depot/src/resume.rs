//! The resume record: which parts of an unfinished download are already on
//! disk, so a restart (even after the app was killed) picks up where it was
//! instead of downloading whole files again.
//!
//! It lives in the game folder as `.sirresume` while a download runs and is
//! deleted when the download completes. Each file is identified by a
//! fingerprint of its name and chunk list, so a record left by another
//! version of the game never matches.
//!
//! Saving is safe against a crash at any moment:
//! - a chunk is recorded only after its data was flushed to disk;
//! - the record is written to a temporary file that then replaces the old one,
//!   so it's always either the previous or the new record, never half of one.

use crate::manifest::FileEntry;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const HEADER: &str = "sirresume 1";
pub const FILE_NAME: &str = ".sirresume";

/// Identifies one file of one game version.
pub type Fingerprint = [u8; 20];

pub fn fingerprint(f: &FileEntry) -> Fingerprint {
    let mut h = Sha1::new();
    h.update(f.name.replace('\\', "/").to_lowercase().as_bytes());
    h.update(f.size.to_le_bytes());
    for c in &f.chunks {
        h.update(c.offset.to_le_bytes());
        h.update(c.size.to_le_bytes());
        h.update(&c.sha);
    }
    h.finalize().into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    /// The finished file is in place.
    Finished,
    /// Some chunks (by index in the file's chunk list) are in the `.sirpart`.
    Partial(Bits),
}

/// A fixed-size set of chunk indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bits {
    len: usize,
    words: Vec<u64>,
}

impl Bits {
    pub fn new(len: usize) -> Bits {
        Bits { len, words: vec![0; len.div_ceil(64)] }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn get(&self, i: usize) -> bool {
        i < self.len && self.words[i / 64] & (1 << (i % 64)) != 0
    }
    pub fn set(&mut self, i: usize) {
        if i < self.len {
            self.words[i / 64] |= 1 << (i % 64);
        }
    }
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }
    fn to_hex(&self) -> String {
        self.words.iter().map(|w| format!("{w:016x}")).collect()
    }
    fn from_hex(len: usize, s: &str) -> Option<Bits> {
        let n = len.div_ceil(64);
        if s.len() != n * 16 || !s.is_ascii() {
            return None;
        }
        let words = (0..n).map(|i| u64::from_str_radix(&s[i * 16..i * 16 + 16], 16).ok()).collect::<Option<Vec<_>>>()?;
        // No bits past the end.
        if !len.is_multiple_of(64) && words.last().is_some_and(|w| w >> (len % 64) != 0) {
            return None;
        }
        Some(Bits { len, words })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    pub files: HashMap<Fingerprint, FileState>,
}

impl Record {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    /// The saved record, or an empty one if there's none (or it's unreadable).
    pub fn load(dir: &Path) -> Record {
        fs::read_to_string(Record::path(dir)).map(|t| Record::parse(&t)).unwrap_or_default()
    }

    /// Lines that don't make sense are ignored (that part is downloaded again).
    pub fn parse(text: &str) -> Record {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Record::default();
        }
        let mut files = HashMap::new();
        for line in lines {
            let mut parts = line.split(' ');
            let Some(fp) = parts.next().and_then(parse_fp) else { continue };
            let state = match (parts.next(), parts.next(), parts.next()) {
                (Some("done"), None, None) => FileState::Finished,
                (Some(n), Some(bits), None) => match n.parse().ok().and_then(|n| Bits::from_hex(n, bits)) {
                    Some(b) => FileState::Partial(b),
                    None => continue,
                },
                _ => continue,
            };
            files.insert(fp, state);
        }
        Record { files }
    }

    pub fn to_text(&self) -> String {
        let mut entries: Vec<_> = self.files.iter().collect();
        entries.sort_by_key(|(fp, _)| **fp);
        let mut out = String::from(HEADER);
        out.push('\n');
        for (fp, state) in entries {
            out.push_str(&hex(fp));
            match state {
                FileState::Finished => out.push_str(" done"),
                FileState::Partial(b) => out.push_str(&format!(" {} {}", b.len(), b.to_hex())),
            }
            out.push('\n');
        }
        out
    }

    /// Replace the saved record in one step: the old one stays until the new
    /// one is fully on disk.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = Record::path(dir);
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(self.to_text().as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)
    }

    pub fn remove(dir: &Path) {
        let _ = fs::remove_file(Record::path(dir));
        let _ = fs::remove_file(dir.join(format!("{FILE_NAME}.tmp")));
    }

    pub fn mark_chunk(&mut self, fp: Fingerprint, chunks: usize, index: usize) {
        let state = self.files.entry(fp).or_insert_with(|| FileState::Partial(Bits::new(chunks)));
        if let FileState::Partial(b) = state {
            b.set(index);
        }
    }

    pub fn mark_finished(&mut self, fp: Fingerprint) {
        self.files.insert(fp, FileState::Finished);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn parse_fp(s: &str) -> Option<Fingerprint> {
    if s.len() != 40 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 20];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Chunk;

    fn file(name: &str, chunks: usize) -> FileEntry {
        FileEntry {
            name: name.into(),
            size: chunks as u64 * 10,
            sha: Vec::new(),
            chunks: (0..chunks).map(|i| Chunk { sha: vec![i as u8; 20], offset: i as u64 * 10, size: 10, compressed: 8 }).collect(),
            directory: false,
            executable: false,
            link: None,
        }
    }

    #[test]
    fn round_trips() {
        let a = fingerprint(&file("a.pak", 130));
        let b = fingerprint(&file("b.pak", 3));
        let mut r = Record::default();
        for i in [0, 1, 63, 64, 129] {
            r.mark_chunk(a, 130, i);
        }
        r.mark_finished(b);
        let back = Record::parse(&r.to_text());
        assert_eq!(back, r);
        let FileState::Partial(bits) = &back.files[&a] else { panic!() };
        assert_eq!(bits.count(), 5);
        assert!(bits.get(64) && bits.get(129) && !bits.get(2) && !bits.get(130));
    }

    #[test]
    fn fingerprint_changes_with_the_version() {
        let f = file("data/big.pak", 4);
        let mut g = f.clone();
        g.chunks[2].sha = vec![9; 20];
        assert_ne!(fingerprint(&f), fingerprint(&g));
        // Same file, different slashes or case: the same file on Windows.
        let mut h = f.clone();
        h.name = "DATA\\Big.pak".into();
        assert_eq!(fingerprint(&f), fingerprint(&h));
        assert_ne!(fingerprint(&f), fingerprint(&file("data/other.pak", 4)));
    }

    #[test]
    fn ignores_damaged_lines() {
        let good = fingerprint(&file("a", 2));
        let text = format!(
            "{HEADER}\n{} done\nnot a line\n{} 2 zz\n{} 2 0000000000000004\n{} 70 00\n",
            hex(&good),
            hex(&[1; 20]),
            hex(&[2; 20]), // bit past the end
            hex(&[3; 20]), // wrong length
        );
        let r = Record::parse(&text);
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[&good], FileState::Finished);
        // Not a record at all.
        assert!(Record::parse("hello\n").files.is_empty());
        assert!(Record::parse("").files.is_empty());
    }

    #[test]
    fn saves_by_replacing() {
        let dir = std::env::temp_dir().join(format!("sir-resume-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(Record::load(&dir).files.is_empty());
        let mut r = Record::default();
        r.mark_chunk([7; 20], 10, 3);
        r.save(&dir).unwrap();
        r.mark_finished([8; 20]);
        r.save(&dir).unwrap();
        assert_eq!(Record::load(&dir), r);
        assert!(!dir.join(".sirresume.tmp").exists());
        Record::remove(&dir);
        assert!(!Record::path(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}

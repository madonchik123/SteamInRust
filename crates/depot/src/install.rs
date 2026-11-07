//! Installing or updating depots into a game folder.
//!
//! - Files that are already correct are skipped.
//! - On updates, chunks that didn't change are copied from the installed file
//!   instead of downloaded (the old manifest says where they are).
//! - Chunks download in parallel; each is verified (SHA-1) before it's written.
//! - New files are written to `*.sirpart` and renamed when complete, so an
//!   interrupted install never leaves a half-written game file.

use crate::cdn::Cdn;
use crate::manifest::{Chunk, FileEntry, Manifest};
use crate::{Error, Result, chunk};
use futures_util::StreamExt;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub struct DepotJob {
    pub depot: u32,
    pub manifest: Manifest,
    pub key: Vec<u8>,
    /// The currently installed version, if any (enables chunk reuse).
    pub old: Option<Manifest>,
}

/// Live progress, shared with the UI.
#[derive(Debug, Default)]
pub struct Progress {
    /// Bytes that must come over the network.
    pub download_total: AtomicU64,
    pub downloaded: AtomicU64,
    /// Bytes that end up on disk (including reused/skipped).
    pub disk_total: AtomicU64,
    pub disk_done: AtomicU64,
    pub files_total: AtomicUsize,
    pub files_done: AtomicUsize,
    pub paused: AtomicBool,
    pub cancel: AtomicBool,
    /// KB/s, 0 = unlimited.
    pub limit_kbps: AtomicU64,
    /// Parallel chunk downloads.
    pub concurrency: AtomicU32,
    pub phase: std::sync::Mutex<Phase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Preparing,
    Checking,
    Downloading,
    Finishing,
    Done,
}

impl Progress {
    pub fn set_phase(&self, p: Phase) {
        *self.phase.lock().unwrap() = p;
    }
    pub fn phase(&self) -> Phase {
        *self.phase.lock().unwrap()
    }
    pub fn fraction(&self) -> f32 {
        let t = self.disk_total.load(Ordering::Relaxed).max(1);
        self.disk_done.load(Ordering::Relaxed) as f32 / t as f32
    }
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub files: usize,
    pub skipped: usize,
    pub reused_bytes: u64,
    pub downloaded_bytes: u64,
    pub removed: usize,
}

/// One unit of work: produce `chunk` for `file` (download or copy).
struct Work {
    file: usize,
    depot: u32,
    key: Arc<Vec<u8>>,
    chunk: Chunk,
    /// Where an identical chunk sits in the currently installed file.
    reuse_from: Option<(PathBuf, u64)>,
}

fn sha1_file(path: &Path) -> Option<Vec<u8>> {
    let mut f = fs::File::open(path).ok()?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().to_vec())
}

fn part_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".sirpart");
    PathBuf::from(s)
}

pub async fn install(cdn: Arc<Cdn>, dir: &Path, jobs: Vec<DepotJob>, progress: Arc<Progress>) -> Result<Report> {
    let mut report = Report::default();
    progress.set_phase(Phase::Checking);
    fs::create_dir_all(dir)?;

    // Plan: which files need work, and where each chunk comes from.
    let mut files: Vec<(PathBuf, FileEntry)> = Vec::new();
    let mut work: Vec<Work> = Vec::new();
    for job in &jobs {
        let key = Arc::new(job.key.clone());
        let old_files: HashMap<&str, &FileEntry> = job.old.iter().flat_map(|m| &m.files).map(|f| (f.name.as_str(), f)).collect();
        // Index of every chunk in the installed version, for reuse.
        let mut old_chunks: HashMap<&[u8], (&str, u64)> = HashMap::new();
        for f in job.old.iter().flat_map(|m| &m.files) {
            for c in &f.chunks {
                old_chunks.entry(c.sha.as_slice()).or_insert((f.name.as_str(), c.offset));
            }
        }
        for f in &job.manifest.files {
            let path = dir.join(&f.name);
            if f.directory {
                fs::create_dir_all(&path)?;
                continue;
            }
            if f.link.is_some() {
                continue;
            }
            progress.disk_total.fetch_add(f.size, Ordering::Relaxed);
            report.files += 1;
            // Unchanged since the installed version and the size matches: trust it.
            let same_as_old = old_files.get(f.name.as_str()).is_some_and(|o| o.sha == f.sha);
            let size_ok = fs::metadata(&path).map(|m| m.len() == f.size).unwrap_or(false);
            if size_ok && (same_as_old || sha1_file(&path).is_some_and(|h| f.sha.is_empty() || h == f.sha)) {
                report.skipped += 1;
                progress.disk_done.fetch_add(f.size, Ordering::Relaxed);
                continue;
            }
            let idx = files.len();
            for c in &f.chunks {
                let reuse_from = old_chunks.get(c.sha.as_slice()).map(|(name, off)| (dir.join(name), *off)).filter(|(p, _)| p.exists());
                if reuse_from.is_none() {
                    progress.download_total.fetch_add(c.compressed as u64, Ordering::Relaxed);
                }
                work.push(Work { file: idx, depot: job.depot, key: key.clone(), chunk: c.clone(), reuse_from });
            }
            files.push((path, f.clone()));
        }
    }
    progress.files_total.store(files.len(), Ordering::Relaxed);

    // Pre-allocate partial files.
    for (path, f) in &files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let part = OpenOptions::new().create(true).write(true).truncate(false).open(part_path(path))?;
        part.set_len(f.size)?;
    }
    let remaining: Arc<Vec<AtomicUsize>> = Arc::new(files.iter().map(|(_, f)| AtomicUsize::new(f.chunks.len())).collect());
    // Empty files are complete immediately.
    for (i, (path, f)) in files.iter().enumerate() {
        if f.chunks.is_empty() {
            finish_file(path)?;
            progress.files_done.fetch_add(1, Ordering::Relaxed);
            remaining[i].store(usize::MAX, Ordering::Relaxed);
        }
    }

    progress.set_phase(Phase::Downloading);
    let files = Arc::new(files);
    let reused = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let concurrency = progress.concurrency.load(Ordering::Relaxed).clamp(1, 64) as usize;
    let results: Vec<Result<()>> = futures_util::stream::iter(work.into_iter().map(|w| {
        let (cdn, files, remaining, progress, reused) = (cdn.clone(), files.clone(), remaining.clone(), progress.clone(), reused.clone());
        async move {
            loop {
                if progress.cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                if !progress.paused.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let data = match &w.reuse_from {
                Some((src, off)) => match read_range(src, *off, w.chunk.size as usize) {
                    Some(d) if chunk::verify(&d, &w.chunk.sha) => {
                        reused.fetch_add(d.len() as u64, Ordering::Relaxed);
                        d
                    }
                    _ => {
                        progress.download_total.fetch_add(w.chunk.compressed as u64, Ordering::Relaxed);
                        download(&cdn, &w, &progress, started).await?
                    }
                },
                None => download(&cdn, &w, &progress, started).await?,
            };
            let (path, _) = &files[w.file];
            write_at(&part_path(path), w.chunk.offset, &data)?;
            progress.disk_done.fetch_add(data.len() as u64, Ordering::Relaxed);
            if remaining[w.file].fetch_sub(1, Ordering::AcqRel) == 1 {
                finish_file(path)?;
                progress.files_done.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }
    }))
    .buffer_unordered(concurrency)
    .collect()
    .await;
    if let Some(e) = results.into_iter().find_map(|r| r.err()) {
        return Err(e);
    }

    // Remove files the new version no longer has.
    progress.set_phase(Phase::Finishing);
    for job in &jobs {
        if let Some(old) = &job.old {
            let keep: std::collections::HashSet<&str> = job.manifest.files.iter().map(|f| f.name.as_str()).collect();
            for f in old.files.iter().filter(|f| !f.directory && !keep.contains(f.name.as_str())) {
                if fs::remove_file(dir.join(&f.name)).is_ok() {
                    report.removed += 1;
                }
            }
        }
    }
    report.reused_bytes = reused.load(Ordering::Relaxed);
    report.downloaded_bytes = progress.downloaded.load(Ordering::Relaxed);
    progress.set_phase(Phase::Done);
    Ok(report)
}

async fn download(cdn: &Cdn, w: &Work, progress: &Progress, started: Instant) -> Result<Vec<u8>> {
    let mut last = None;
    for _ in 0..3 {
        let raw = cdn.chunk(w.depot, &w.chunk.sha).await?;
        let len = raw.len() as u64;
        match chunk::decode(&raw, &w.key) {
            Ok(data) if chunk::verify(&data, &w.chunk.sha) => {
                let total = progress.downloaded.fetch_add(len, Ordering::Relaxed) + len;
                throttle(progress, total, started).await;
                return Ok(data);
            }
            Ok(_) => last = Some(Error::Format(format!("chunk {} failed verification", chunk::hex(&w.chunk.sha)))),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or(Error::NoServers))
}

/// Keep the average rate under the user's limit.
async fn throttle(progress: &Progress, total: u64, started: Instant) {
    let limit = progress.limit_kbps.load(Ordering::Relaxed);
    if limit == 0 {
        return;
    }
    let should_take = Duration::from_secs_f64(total as f64 / (limit as f64 * 1024.0));
    let took = started.elapsed();
    if should_take > took {
        tokio::time::sleep(should_take - took).await;
    }
}

fn read_range(path: &Path, offset: u64, len: usize) -> Option<Vec<u8>> {
    let mut f = fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn write_at(path: &Path, offset: u64, data: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new().write(true).open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    f.write_all(data)?;
    Ok(())
}

fn finish_file(path: &Path) -> Result<()> {
    let part = part_path(path);
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&part, path)?;
    Ok(())
}

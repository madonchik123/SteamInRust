//! Installing or updating depots into a game folder.
//!
//! - Files that are already correct are skipped.
//! - On updates, chunks that didn't change are copied from the installed file
//!   instead of downloaded (the old manifest says where they are).
//! - Chunks download in parallel; each is verified (SHA-1) before it's written.
//! - New files are written to `*.sirpart` and renamed when complete, so an
//!   interrupted install never leaves a half-written game file.
//! - Every couple of seconds the chunks that reached the disk are noted in a
//!   resume record (see [`crate::resume`]), so a restart, even after the app
//!   was killed, only redoes the last few seconds.

use crate::cdn::Cdn;
use crate::manifest::{Chunk, FileEntry, Manifest};
use crate::resume::{self, FileState, Fingerprint, Record};
use crate::{Error, Result, chunk};
use futures_util::StreamExt;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
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
    /// Stop without cleaning up (another download goes first; this one
    /// continues from what's on disk later).
    pub stop: AtomicBool,
    /// The download ended (done, failed or cancelled): progress updates stop.
    pub ended: AtomicBool,
    /// Bytes actually written to disk (not files found already complete),
    /// for an honest disk speed.
    pub written: AtomicU64,
    /// KB/s, 0 = unlimited.
    pub limit_kbps: AtomicU64,
    /// Parallel chunk downloads.
    pub concurrency: AtomicU32,
    pub phase: std::sync::Mutex<Phase>,
    /// Things worth telling the user that don't stop the download,
    /// e.g. a part of the game this account doesn't include.
    pub notes: std::sync::Mutex<Vec<String>>,
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
    /// Already on disk from an earlier, interrupted attempt.
    pub resumed_bytes: u64,
}

/// How often finished chunks are noted in the resume record.
const SAVE_EVERY: Duration = Duration::from_secs(2);

/// One unit of work: produce `chunk` for `file` (download or copy).
struct Work {
    file: usize,
    /// Position in the file's chunk list (for the resume record).
    index: usize,
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

fn is_symlink_or_reparse(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn is_windows_reserved(seg: &str) -> bool {
    let stem = seg.split('.').next().unwrap_or("");
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "CONIN$"
            | "CONOUT$"
            | "COM0"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT0"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return true;
    }
    if stem.len() > 3
        && stem.is_char_boundary(3)
        && (stem[..3].eq_ignore_ascii_case("COM") || stem[..3].eq_ignore_ascii_case("LPT"))
        && matches!(&stem[3..], "\u{00b9}" | "\u{00b2}" | "\u{00b3}")
    {
        return true;
    }
    false
}

fn is_valid_segment(seg: &str) -> bool {
    if seg.is_empty() || seg == "." || seg == ".." {
        return false;
    }
    if seg.contains(['/', '\\', '\0']) || seg.chars().any(|c| (c as u32) < 0x20) {
        return false;
    }
    // Names only Windows can't hold (a trailing dot or space, a drive colon,
    // CON/NUL/COM1...): fine for Mac and Linux depots, which do ship them.
    if cfg!(windows) && (seg.ends_with('.') || seg.ends_with(' ') || seg.contains(':') || is_windows_reserved(seg)) {
        return false;
    }
    let lower = seg.to_ascii_lowercase();
    if lower.ends_with(".sirpart") || lower == ".sirresume" || lower == ".sirresume.tmp" {
        return false;
    }
    let mut comps = Path::new(seg).components();
    matches!((comps.next(), comps.next()), (Some(Component::Normal(s)), None) if !s.is_empty())
}

/// Validate a single folder name (e.g. `<library>/steamapps/common/<folder>`).
pub fn validate_folder_name(name: &str) -> Result<String> {
    if !is_valid_segment(name) {
        return Err(Error::Format(format!("unsafe folder name: {name:?}")));
    }
    Ok(name.to_string())
}

/// Validate a relative manifest path (`\` normalized to `/`), allowing only
/// non-empty `Component::Normal` segments without reserved names or `.sirpart`.
pub fn validate_rel_path(rel: &str) -> Result<PathBuf> {
    if rel.is_empty() || rel.contains('\0') {
        return Err(Error::Format(format!("unsafe manifest path: {rel:?}")));
    }
    let norm = rel.replace('\\', "/");
    let mut out = PathBuf::new();
    for seg in norm.split('/') {
        if !is_valid_segment(seg) {
            return Err(Error::Format(format!("unsafe manifest path: {rel:?}")));
        }
        out.push(seg);
    }
    if out.as_os_str().is_empty()
        || !out.components().all(|c| matches!(c, Component::Normal(s) if !s.is_empty()))
    {
        return Err(Error::Format(format!("unsafe manifest path: {rel:?}")));
    }
    Ok(out)
}

/// Verify `dir` (if it already exists) is a real directory, not a symlink/reparse point.
pub fn ensure_real_dir(dir: &Path) -> Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(m) if is_symlink_or_reparse(&m) => {
            Err(Error::Format(format!("refusing symlink directory: {}", dir.display())))
        }
        Ok(m) if !m.is_dir() => Err(Error::Format(format!("not a directory: {}", dir.display()))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Io(e)),
    }
}

fn ensure_not_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if is_symlink_or_reparse(&m) => {
            Err(Error::Format(format!("refusing symlink target: {}", path.display())))
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Io(e)),
    }
}

fn resolve_inside_opts(dir: &Path, rel: &str, allow_leaf_symlink: bool) -> Result<PathBuf> {
    let rel_path = validate_rel_path(rel)?;
    ensure_real_dir(dir)?;
    let mut cur = dir.to_path_buf();
    let mut comps = rel_path.components().peekable();
    while let Some(c) = comps.next() {
        cur.push(c);
        let is_last = comps.peek().is_none();
        match fs::symlink_metadata(&cur) {
            Ok(m) => {
                if is_symlink_or_reparse(&m) && !(allow_leaf_symlink && is_last) {
                    return Err(Error::Format(format!("refusing symlink path component in {rel:?}")));
                }
                if !is_last && !m.is_dir() {
                    return Err(Error::Format(format!("non-directory ancestor in {rel:?}")));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                for rest in comps {
                    cur.push(rest);
                }
                break;
            }
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(cur)
}

/// Validate `rel` and check via `symlink_metadata` that `dir` and every
/// existing component inside `dir` along `rel` is free of symlinks/reparse points.
pub fn resolve_inside(dir: &Path, rel: &str) -> Result<PathBuf> {
    resolve_inside_opts(dir, rel, false)
}

#[cfg(unix)]
fn validate_symlink_target(dir: &Path, link_path: &Path, target: &str) -> Result<()> {
    if target.is_empty() || target.contains('\0') {
        return Err(Error::Format(format!("unsafe symlink target: {target:?}")));
    }
    let norm = target.replace('\\', "/");
    if norm.starts_with('/') {
        return Err(Error::Format(format!("refusing absolute symlink target: {target:?}")));
    }
    let rel_link = link_path.strip_prefix(dir).map_err(|_| Error::Format("symlink outside dir".into()))?;
    let mut stack: Vec<String> = rel_link
        .parent()
        .map(|p| p.components().filter_map(|c| match c { Component::Normal(s) => Some(s.to_string_lossy().into_owned()), _ => None }).collect())
        .unwrap_or_default();
    for seg in norm.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.pop().is_none() {
                    return Err(Error::Format(format!("symlink target escapes install dir: {target:?}")));
                }
            }
            s => stack.push(s.to_string()),
        }
    }
    Ok(())
}

pub async fn install(cdn: Arc<Cdn>, dir: &Path, jobs: Vec<DepotJob>, progress: Arc<Progress>) -> Result<Report> {
    let mut report = Report::default();
    progress.set_phase(Phase::Checking);
    ensure_real_dir(dir)?;
    for job in &jobs {
        for f in &job.manifest.files {
            validate_rel_path(&f.name)?;
        }
    }
    fs::create_dir_all(dir)?;
    ensure_real_dir(dir)?;
    ensure_not_symlink(&Record::path(dir))?;
    ensure_not_symlink(&dir.join(".sirresume.tmp"))?;

    // A file can be in more than one depot (a language or platform depot
    // replacing a base file). Like Steam, the later depot's copy wins; the
    // earlier one is skipped, so the file is never written (and its
    // temporary file renamed) twice.
    let key_of = |name: &str| name.replace('\\', "/").to_lowercase();
    let mut owner: HashMap<String, usize> = HashMap::new();
    for (i, job) in jobs.iter().enumerate() {
        for f in job.manifest.files.iter().filter(|f| !f.directory) {
            owner.insert(key_of(&f.name), i);
        }
    }

    // What an earlier, interrupted attempt already finished. Only entries
    // that still match what's on disk are carried over.
    let saved = Record::load(dir);
    let mut record = Record::default();

    // Plan: which files need work, and where each chunk comes from.
    let mut files: Vec<(PathBuf, String, FileEntry)> = Vec::new();
    let mut prints: Vec<Fingerprint> = Vec::new();
    // Chunks still to write, per file.
    let mut todo: Vec<usize> = Vec::new();
    let mut work: Vec<Work> = Vec::new();
    for (job_index, job) in jobs.iter().enumerate() {
        let key = Arc::new(job.key.clone());
        let mut old_files: HashMap<String, &FileEntry> = HashMap::new();
        // Index of every verified chunk in the installed version, for reuse.
        let mut old_chunks: HashMap<&[u8], (PathBuf, u64)> = HashMap::new();
        for f in job.old.iter().flat_map(|m| &m.files).filter(|f| !f.directory && f.link.is_none()) {
            let Ok(old_path) = resolve_inside(dir, &f.name) else { continue };
            let is_reg = fs::symlink_metadata(&old_path)
                .map(|m| !is_symlink_or_reparse(&m) && m.is_file())
                .unwrap_or(false);
            if !is_reg {
                continue;
            }
            old_files.insert(key_of(&f.name), f);
            for c in &f.chunks {
                if c.sha.len() == 20 {
                    old_chunks.entry(c.sha.as_slice()).or_insert_with(|| (old_path.clone(), c.offset));
                }
            }
        }
        for f in &job.manifest.files {
            let path = resolve_inside(dir, &f.name)?;
            if f.directory {
                fs::create_dir_all(&path)?;
                ensure_real_dir(&path)?;
                continue;
            }
            if f.link.is_some() || owner.get(&key_of(&f.name)) != Some(&job_index) {
                continue;
            }
            progress.disk_total.fetch_add(f.size, Ordering::Relaxed);
            report.files += 1;
            let fp = resume::fingerprint(f);
            // Unchanged since the installed version, or finished by an
            // earlier attempt, and the size matches: trust it.
            let same_as_old = old_files.get(&key_of(&f.name)).is_some_and(|o| f.sha.len() == 20 && o.sha == f.sha);
            let finished_before = f.sha.len() == 20 && saved.files.get(&fp) == Some(&FileState::Finished);
            let size_ok = fs::symlink_metadata(&path)
                .map(|m| !is_symlink_or_reparse(&m) && m.is_file() && m.len() == f.size)
                .unwrap_or(false);
            if size_ok && (same_as_old || finished_before || sha1_file(&path).is_some_and(|h| f.sha.len() == 20 && h == f.sha)) {
                report.skipped += 1;
                progress.disk_done.fetch_add(f.size, Ordering::Relaxed);
                record.mark_finished(fp);
                continue;
            }
            // Chunks an earlier attempt wrote to the unfinished file.
            let part_ok = fs::symlink_metadata(part_path(&path))
                .is_ok_and(|m| !is_symlink_or_reparse(&m) && m.is_file() && m.len() == f.size);
            let done = match saved.files.get(&fp) {
                Some(FileState::Partial(b)) if part_ok && b.len() == f.chunks.len() => Some(b.clone()),
                _ => None,
            };
            if let Some(b) = &done {
                record.files.insert(fp, FileState::Partial(b.clone()));
            }
            let idx = files.len();
            let mut left = 0;
            for (index, c) in f.chunks.iter().enumerate() {
                if done.as_ref().is_some_and(|b| b.get(index)) {
                    progress.disk_done.fetch_add(c.size as u64, Ordering::Relaxed);
                    report.resumed_bytes += c.size as u64;
                    continue;
                }
                left += 1;
                let reuse_from = if c.sha.len() == 20 {
                    old_chunks.get(c.sha.as_slice()).cloned()
                } else {
                    None
                };
                if reuse_from.is_none() {
                    progress.download_total.fetch_add(c.compressed as u64, Ordering::Relaxed);
                }
                work.push(Work { file: idx, index, depot: job.depot, key: key.clone(), chunk: c.clone(), reuse_from });
            }
            files.push((path, f.name.clone(), f.clone()));
            prints.push(fp);
            todo.push(left);
        }
    }
    progress.files_total.store(files.len(), Ordering::Relaxed);

    // Pre-allocate partial files.
    for (path, rel, f) in &files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let checked = resolve_inside(dir, rel)?;
        let part = part_path(&checked);
        ensure_not_symlink(&part)?;
        let file = OpenOptions::new().create(true).write(true).truncate(false).open(&part)?;
        file.set_len(f.size)?;
    }
    let remaining: Arc<Vec<AtomicUsize>> = Arc::new(todo.iter().map(|&n| AtomicUsize::new(n)).collect());
    // Empty files, and files an earlier attempt wrote completely, are done.
    for (i, (path, _, _)) in files.iter().enumerate() {
        if todo[i] == 0 {
            finish_file(path)?;
            progress.files_done.fetch_add(1, Ordering::Relaxed);
            remaining[i].store(usize::MAX, Ordering::Relaxed);
            record.mark_finished(prints[i]);
        }
    }
    if report.resumed_bytes > 0 {
        tracing::info!("resuming: {} MB already on disk from the last attempt", report.resumed_bytes / 1_048_576);
    }
    let checkpoint = Arc::new(Checkpoint::new(dir, record, &files, prints));
    // Saved now so entries that no longer apply are dropped right away.
    checkpoint.save(true);
    let saver = {
        let checkpoint = checkpoint.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SAVE_EVERY).await;
                let checkpoint = checkpoint.clone();
                let _ = tokio::task::spawn_blocking(move || checkpoint.save(false)).await;
            }
        })
    };

    progress.set_phase(Phase::Downloading);
    let files = Arc::new(files);
    let reused = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let concurrency = progress.concurrency.load(Ordering::Relaxed).clamp(1, 64) as usize;
    let results: Vec<Result<()>> = futures_util::stream::iter(work.into_iter().map(|w| {
        let (cdn, files, remaining, progress, reused, checkpoint) = (cdn.clone(), files.clone(), remaining.clone(), progress.clone(), reused.clone(), checkpoint.clone());
        async move {
            loop {
                if progress.cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                if progress.stop.load(Ordering::Relaxed) {
                    return Err(Error::Stopped);
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
            let (path, _, _) = &files[w.file];
            write_at(&part_path(path), w.chunk.offset, &data)?;
            checkpoint.wrote(w.file, w.index);
            progress.disk_done.fetch_add(data.len() as u64, Ordering::Relaxed);
            progress.written.fetch_add(data.len() as u64, Ordering::Relaxed);
            if remaining[w.file].fetch_sub(1, Ordering::AcqRel) == 1 {
                finish_file(path)?;
                checkpoint.finished(w.file);
                progress.files_done.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }
    }))
    .buffer_unordered(concurrency)
    .collect()
    .await;
    saver.abort();
    if let Some(e) = results.into_iter().find_map(|r| r.err()) {
        // Keep what made it to disk for the next attempt (a cancelled
        // download is cleaned up by the caller instead).
        if !matches!(e, Error::Cancelled) {
            let checkpoint = checkpoint.clone();
            let _ = tokio::task::spawn_blocking(move || checkpoint.save(false)).await;
        }
        checkpoint.close();
        return Err(e);
    }

    // Remove files the new version no longer has.
    progress.set_phase(Phase::Finishing);
    #[cfg(unix)]
    unix_finish(dir, &jobs)?;
    for job in &jobs {
        if let Some(old) = &job.old {
            let keep: std::collections::HashSet<String> = job.manifest.files.iter().map(|f| key_of(&f.name)).collect();
            for f in old.files.iter().filter(|f| !f.directory && !keep.contains(&key_of(&f.name))) {
                if let Ok(path) = resolve_inside(dir, &f.name)
                    && fs::remove_file(path).is_ok()
                {
                    report.removed += 1;
                }
            }
        }
    }
    // Complete: nothing left to resume.
    checkpoint.close();
    Record::remove(dir);
    report.reused_bytes = reused.load(Ordering::Relaxed);
    report.downloaded_bytes = progress.downloaded.load(Ordering::Relaxed);
    progress.set_phase(Phase::Done);
    Ok(report)
}

/// Keeps the resume record up to date while chunks are written.
struct Checkpoint {
    dir: PathBuf,
    /// Per file: the unfinished file, the finished one, fingerprint, chunk count.
    files: Vec<(PathBuf, PathBuf, Fingerprint, usize)>,
    /// Written, but not yet flushed to disk and noted.
    pending: std::sync::Mutex<Pending>,
    /// The record as saved. Its lock is held for a whole save, so saves
    /// never overlap; `None` once the download ended (no more saves).
    record: std::sync::Mutex<Option<Record>>,
}

#[derive(Default)]
struct Pending {
    chunks: Vec<(usize, usize)>,
    finished: Vec<usize>,
}

impl Checkpoint {
    fn new(dir: &Path, record: Record, files: &[(PathBuf, String, FileEntry)], prints: Vec<Fingerprint>) -> Checkpoint {
        let files = files.iter().zip(prints).map(|((path, _, f), fp)| (part_path(path), path.clone(), fp, f.chunks.len())).collect();
        Checkpoint { dir: dir.to_path_buf(), files, pending: Default::default(), record: std::sync::Mutex::new(Some(record)) }
    }

    /// Chunk `index` of file `file` was written (not necessarily flushed yet).
    fn wrote(&self, file: usize, index: usize) {
        self.pending.lock().unwrap().chunks.push((file, index));
    }

    /// File `file` is complete and has its final name.
    fn finished(&self, file: usize) {
        self.pending.lock().unwrap().finished.push(file);
    }

    /// Flush the files written since the last save, then note their chunks.
    /// Anything that can't be flushed is left out (downloaded again later).
    fn save(&self, always: bool) {
        let mut guard = self.record.lock().unwrap();
        let Some(record) = guard.as_mut() else { return };
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        if pending.chunks.is_empty() && pending.finished.is_empty() && !always {
            return;
        }
        let touched: std::collections::BTreeSet<usize> = pending.chunks.iter().map(|c| c.0).chain(pending.finished.iter().copied()).collect();
        let flushed: std::collections::HashSet<usize> = touched
            .into_iter()
            .filter(|&i| {
                let (part, done, ..) = &self.files[i];
                // A file can get its final name between being written and now.
                [part, done].iter().any(|p| {
                    !fs::symlink_metadata(p).is_ok_and(|m| is_symlink_or_reparse(&m))
                        && OpenOptions::new().write(true).open(p).and_then(|f| f.sync_data()).is_ok()
                })
            })
            .collect();
        for (file, index) in pending.chunks.into_iter().filter(|c| flushed.contains(&c.0)) {
            let (_, _, fp, n) = self.files[file];
            record.mark_chunk(fp, n, index);
        }
        for file in pending.finished.into_iter().filter(|f| flushed.contains(f)) {
            record.mark_finished(self.files[file].2);
        }
        if let Err(e) = record.save(&self.dir) {
            tracing::warn!("couldn't save download progress: {e}");
        }
    }

    /// No more saves (waits for one in progress).
    fn close(&self) {
        *self.record.lock().unwrap() = None;
    }
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
    let meta = fs::symlink_metadata(path).ok()?;
    if is_symlink_or_reparse(&meta) || !meta.is_file() {
        return None;
    }
    let mut f = fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn write_at(path: &Path, offset: u64, data: &[u8]) -> Result<()> {
    ensure_not_symlink(path)?;
    let mut f = OpenOptions::new().write(true).open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    f.write_all(data)?;
    Ok(())
}

/// What Windows doesn't need: links inside the game (macOS app bundles are
/// full of them) and permission to run its programs.
#[cfg(unix)]
fn unix_finish(dir: &Path, jobs: &[DepotJob]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for f in jobs.iter().flat_map(|j| &j.manifest.files) {
        if let Some(target) = &f.link {
            let path = resolve_inside_opts(dir, &f.name, true)?;
            let target = target.replace('\\', "/");
            validate_symlink_target(dir, &path, &target)?;
            if fs::read_link(&path).is_ok_and(|t| t == Path::new(&target)) {
                continue;
            }
            if let Ok(m) = fs::symlink_metadata(&path) {
                if m.is_dir() && !m.file_type().is_symlink() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
                ensure_real_dir(parent)?;
            }
            std::os::unix::fs::symlink(&target, &path)?;
        } else if !f.directory && (f.executable || is_bundle_program(&f.name)) {
            let path = resolve_inside(dir, &f.name)?;
            if let Ok(m) = fs::symlink_metadata(&path)
                && !is_symlink_or_reparse(&m)
                && m.is_file()
            {
                let mode = m.permissions().mode();
                if mode & 0o111 != 0o111 {
                    fs::set_permissions(&path, fs::Permissions::from_mode(mode | 0o755))?;
                }
            }
        }
    }
    Ok(())
}

/// The program inside a macOS app bundle (`X.app/Contents/MacOS/X`); some
/// depots don't flag it as executable.
#[cfg(unix)]
fn is_bundle_program(name: &str) -> bool {
    let name = name.replace('\\', "/");
    name.split('/').collect::<Vec<_>>().windows(3).any(|w| w[0].ends_with(".app") && w[1] == "Contents" && w[2] == "MacOS")
}

fn finish_file(path: &Path) -> Result<()> {
    let part = part_path(path);
    ensure_not_symlink(&part)?;
    ensure_not_symlink(path)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&part, path)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::manifest::{FileEntry, Manifest};

    fn entry(name: &str, executable: bool, link: Option<&str>) -> FileEntry {
        FileEntry { name: name.into(), size: 0, sha: Vec::new(), chunks: Vec::new(), directory: false, executable, install_script: false, link: link.map(String::from) }
    }

    #[test]
    fn finds_bundle_programs() {
        assert!(is_bundle_program("Human.app/Contents/MacOS/Human"));
        assert!(!is_bundle_program("Human.app/Contents/Resources/icon.icns"));
        assert!(!is_bundle_program("game.sh"));
    }

    #[test]
    fn links_and_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sir-unix-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("G.app/Contents/MacOS")).unwrap();
        fs::create_dir_all(dir.join("Lib.framework/Versions/A")).unwrap();
        for f in ["G.app/Contents/MacOS/G", "run.sh", "data.txt"] {
            fs::write(dir.join(f), b"x").unwrap();
            fs::set_permissions(dir.join(f), fs::Permissions::from_mode(0o644)).unwrap();
        }
        let manifest = Manifest {
            files: vec![
                entry("G.app/Contents/MacOS/G", false, None),
                entry("run.sh", true, None),
                entry("data.txt", false, None),
                entry("Lib.framework/Versions/Current", false, Some("A")),
            ],
            ..Default::default()
        };
        let jobs = vec![DepotJob { depot: 1, manifest, key: Vec::new(), old: None }];
        unix_finish(&dir, &jobs).unwrap();
        // Twice is fine (updates run it again).
        unix_finish(&dir, &jobs).unwrap();
        let mode = |f: &str| fs::metadata(dir.join(f)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode("G.app/Contents/MacOS/G"), 0o755);
        assert_eq!(mode("run.sh"), 0o755);
        assert_eq!(mode("data.txt"), 0o644);
        assert_eq!(fs::read_link(dir.join("Lib.framework/Versions/Current")).unwrap(), Path::new("A"));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod resume_tests {
    use super::*;

    const CHUNK: usize = 1000;

    fn chunk_data(i: usize) -> Vec<u8> {
        (0..CHUNK).map(|b| (b * 7 + i * 31) as u8).collect()
    }

    fn sha(data: &[u8]) -> Vec<u8> {
        Sha1::digest(data).to_vec()
    }

    fn entry(name: &str, chunks: &[usize]) -> FileEntry {
        let data: Vec<u8> = chunks.iter().flat_map(|&i| chunk_data(i)).collect();
        FileEntry {
            name: name.into(),
            size: data.len() as u64,
            sha: sha(&data),
            chunks: chunks
                .iter()
                .enumerate()
                .map(|(n, &i)| Chunk { sha: sha(&chunk_data(i)), offset: (n * CHUNK) as u64, size: CHUNK as u32, compressed: CHUNK as u32 })
                .collect(),
            directory: false,
            executable: false,
            install_script: false,
            link: None,
        }
    }

    /// A game of one file made of chunks 0, 1 and 2. There's no network: a
    /// chunk can only come from `old.bin` (the "installed version"), which
    /// holds the chunks listed in `have`.
    fn job(dir: &Path, have: &[usize]) -> DepotJob {
        let old = entry("old.bin", have);
        let data: Vec<u8> = have.iter().flat_map(|&i| chunk_data(i)).collect();
        fs::write(dir.join("old.bin"), data).unwrap();
        let manifest = Manifest { files: vec![entry("game.bin", &[0, 1, 2])], ..Default::default() };
        DepotJob { depot: 1, manifest, key: Vec::new(), old: Some(Manifest { files: vec![old], ..Default::default() }) }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sir-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn run(dir: &Path, have: &[usize]) -> Result<Report> {
        install(Arc::new(Cdn::new(Vec::new())), dir, vec![job(dir, have)], Arc::new(Progress::default())).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_interrupted_download_resumes_where_it_was() {
        let dir = temp("resume-run");
        // Chunk 2 can't be had: the download stops, chunks 0 and 1 are on disk.
        assert!(run(&dir, &[0, 1]).await.is_err());
        let fp = resume::fingerprint(&entry("game.bin", &[0, 1, 2]));
        let FileState::Partial(bits) = &Record::load(&dir).files[&fp] else { panic!("no record") };
        assert_eq!((bits.get(0), bits.get(1), bits.get(2)), (true, true, false));

        // Next time only chunk 2 is fetched; the rest is taken as done.
        let r = run(&dir, &[2]).await.unwrap();
        assert_eq!(r.resumed_bytes, 2 * CHUNK as u64);
        assert_eq!(r.reused_bytes, CHUNK as u64);
        let want: Vec<u8> = (0..3).flat_map(chunk_data).collect();
        assert_eq!(fs::read(dir.join("game.bin")).unwrap(), want);
        // Finished: nothing left behind.
        assert!(!Record::path(&dir).exists());
        assert!(!dir.join("game.bin.sirpart").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_record_without_its_file_is_ignored() {
        let dir = temp("stale");
        assert!(run(&dir, &[0, 1]).await.is_err());
        // The unfinished file went away (cleaned up, say): start over.
        fs::remove_file(dir.join("game.bin.sirpart")).unwrap();
        let r = run(&dir, &[0, 1, 2]).await.unwrap();
        assert_eq!(r.resumed_bytes, 0);
        assert_eq!(r.reused_bytes, 3 * CHUNK as u64);
        let want: Vec<u8> = (0..3).flat_map(chunk_data).collect();
        assert_eq!(fs::read(dir.join("game.bin")).unwrap(), want);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_only_chunks_that_reached_the_disk() {
        let dir = temp("checkpoint");
        let files = vec![
            (dir.join("a.bin"), "a.bin".to_string(), entry("a.bin", &[0, 1])),
            (dir.join("gone.bin"), "gone.bin".to_string(), entry("gone.bin", &[0])),
        ];
        let prints: Vec<Fingerprint> = files.iter().map(|(_, _, f)| resume::fingerprint(f)).collect();
        fs::write(dir.join("a.bin.sirpart"), vec![0u8; 2 * CHUNK]).unwrap();
        let cp = Checkpoint::new(&dir, Record::default(), &files, prints.clone());
        cp.wrote(0, 1);
        // Its file can't be opened, so it can't have been flushed.
        cp.wrote(1, 0);
        cp.save(false);
        let saved = Record::load(&dir);
        let FileState::Partial(bits) = &saved.files[&prints[0]] else { panic!() };
        assert!(bits.get(1) && !bits.get(0));
        assert!(!saved.files.contains_key(&prints[1]));
        // Once closed, nothing more is written.
        cp.close();
        cp.wrote(0, 0);
        cp.save(true);
        assert_eq!(Record::load(&dir), saved);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    fn empty_file(name: &str) -> FileEntry {
        FileEntry {
            name: name.into(),
            size: 0,
            sha: Sha1::digest(b"").to_vec(),
            chunks: vec![],
            directory: false,
            executable: false,
            install_script: false,
            link: None,
        }
    }

    #[tokio::test]
    async fn rejects_traversal_reserved_and_sirpart_manifest_names() {
        let root = std::env::temp_dir().join(format!("sir-aur02-trav-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("game");
        fs::create_dir_all(&dir).unwrap();
        let cdn = Arc::new(Cdn::new(vec![]));

        let mut bad_names = vec![
            "../escaped.txt",
            "sub/../../escaped.txt",
            r"..\escaped.txt",
            "./dot.txt",
            "sub/./dot.txt",
            "/abs_escaped.txt",
            r"\abs_escaped.txt",
            "bad\0name.txt",
            "game.exe.sirpart",
            "GAME.EXE.SIRPART",
            ".sirresume",
            ".sirresume.tmp",
        ];
        // Only Windows can't hold these (Mac and Linux depots ship them).
        if cfg!(windows) {
            bad_names.extend([r"C:\evil.dll", "C:evil.dll", "CON", "sub/NUL.txt", "com1.bin", "trailing.", "trailing "]);
        }
        for bad in bad_names {
            let m = Manifest { depot: 1, gid: 1, files: vec![empty_file(bad)], ..Default::default() };
            let job = DepotJob { depot: 1, manifest: m, key: vec![0; 32], old: None };
            let res = install(cdn.clone(), &dir, vec![job], Arc::new(Progress::default())).await;
            assert!(res.is_err(), "expected {bad:?} to be rejected");
        }
        assert!(!root.join("escaped.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escape_on_install() {
        let root = std::env::temp_dir().join(format!("sir-aur02-sym-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("game");
        let outside = root.join("outside");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("sub")).unwrap();
        let cdn = Arc::new(Cdn::new(vec![]));

        let m = Manifest { depot: 1, gid: 1, files: vec![empty_file("sub/pwn.txt")], ..Default::default() };
        let job = DepotJob { depot: 1, manifest: m, key: vec![0; 32], old: None };
        let res = install(cdn, &dir, vec![job], Arc::new(Progress::default())).await;
        assert!(res.is_err(), "expected symlink ancestor write to be rejected");
        assert!(!outside.join("pwn.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }
}

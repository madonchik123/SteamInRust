//! Moving an installed game to another library folder, like Steam's "Move
//! install folder".
//!
//! The rule: at every moment one complete, working copy of the game exists
//! and its app manifest points at it. So:
//!
//! 1. Everything is copied into a hidden folder in the destination library
//!    (`steamapps/.sir-move-<app>`), with a small journal saying what's
//!    going on. The original isn't touched.
//! 2. Every copy is read back and compared with the original by SHA-1, and
//!    the original is checked again for changes (a game or updater that
//!    started meanwhile).
//! 3. The copies are renamed into place. The destination's app manifest is
//!    written last, in one atomic step: that's the moment the game has moved.
//! 4. Only then are the original manifest and files deleted.
//!
//! If the app or the PC stops at any point, [`recover`] (run at the next
//! start) reads the journal: before step 3 finished the copies are thrown
//! away and the game stays where it was; after it, the old files are removed.

use crate::manage::{self, Progress};
use model::{AppId, InstalledGame};
use sha1::{Digest, Sha1};
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::SystemTime;

const STAGING_PREFIX: &str = ".sir-move-";
const JOURNAL: &str = "journal.txt";
/// Room left on the drive after a move, for the file system's own needs.
const SPARE_BYTES: u64 = 64 << 20;

/// Why a game can't be moved, or why a move stopped. In every case but
/// [`MoveError::Interrupted`] the game is still complete where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    /// The destination is the library the game is already in (or inside it).
    SameFolder,
    /// The game is being updated or isn't fully installed.
    NotReady,
    /// Another move of this game is running.
    AlreadyMoving,
    /// A file is open in another program (usually: the game is running).
    InUse(String),
    NoSpace { need: u64, free: u64 },
    /// The destination already has a non-empty folder for this game.
    FolderExists(PathBuf),
    /// The destination already has this game installed.
    AlreadyThere,
    /// A link inside the game would break if it moved.
    Links(String),
    /// A file changed while it was being moved.
    Changed(String),
    /// A copy didn't match its original.
    Mismatch(String),
    /// An earlier move's leftovers are still being cleaned up.
    Leftover,
    Cancelled,
    /// Only in tests: the "power cut" of a crash test.
    Interrupted,
    Io(String),
}

impl std::fmt::Display for MoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SameFolder => write!(f, "the game is already in this folder"),
            Self::NotReady => write!(f, "the game is being updated or isn't fully installed"),
            Self::AlreadyMoving => write!(f, "this game is already being moved"),
            Self::InUse(p) => write!(f, "{p} is in use"),
            Self::NoSpace { need, free } => write!(f, "needs {need} bytes, {free} free"),
            Self::FolderExists(p) => write!(f, "{} already exists", p.display()),
            Self::AlreadyThere => write!(f, "the game is already installed there"),
            Self::Links(p) => write!(f, "{p} is a link that can't be moved"),
            Self::Changed(p) => write!(f, "{p} changed during the move"),
            Self::Mismatch(p) => write!(f, "the copy of {p} doesn't match"),
            Self::Leftover => write!(f, "an earlier move is still being cleaned up"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Interrupted => write!(f, "interrupted"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MoveError {}

fn io(path: &Path, e: std::io::Error) -> MoveError {
    MoveError::Io(format!("{}: {e}", path.display()))
}

/// What a running move is doing, for its progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Preparing = 0,
    Copying = 1,
    Verifying = 2,
    /// Switching over. Can't be cancelled any more.
    Finishing = 3,
    /// The game has moved; the old files are being deleted.
    Cleaning = 4,
}

/// Shared progress of a running move. The bar counts every byte twice: once
/// copied and once verified.
#[derive(Debug, Default)]
pub struct MoveProgress {
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    pub cancel: AtomicBool,
    phase: AtomicU8,
}

impl MoveProgress {
    pub fn fraction(&self) -> f32 {
        let total = self.total_bytes.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.done_bytes.load(Ordering::Relaxed) as f64 / total as f64).clamp(0.0, 1.0) as f32
    }

    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::Relaxed) {
            1 => Phase::Copying,
            2 => Phase::Verifying,
            3 => Phase::Finishing,
            4 => Phase::Cleaning,
            _ => Phase::Preparing,
        }
    }

    fn set_phase(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Games being moved or recovered right now. Launching, updating and
/// verifying them is refused until they're done.
static MOVING: Mutex<BTreeSet<AppId>> = Mutex::new(BTreeSet::new());

/// True while this game's files are being moved.
pub fn is_moving(app: AppId) -> bool {
    MOVING.lock().unwrap_or_else(|e| e.into_inner()).contains(&app)
}

struct Claim(AppId);

impl Claim {
    fn take(app: AppId) -> Option<Self> {
        MOVING.lock().unwrap_or_else(|e| e.into_inner()).insert(app).then_some(Self(app))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        MOVING.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// The folders that make up a game in a library. The game's own folder and
/// its Workshop items must move; Proton's prefix (saves on Linux) and shader
/// cache follow unless the destination already has its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Root {
    Game,
    Workshop,
    Compat,
    Shaders,
}

impl Root {
    const ALL: [Root; 4] = [Root::Game, Root::Workshop, Root::Compat, Root::Shaders];

    fn name(self) -> &'static str {
        match self {
            Root::Game => "game",
            Root::Workshop => "workshop",
            Root::Compat => "compatdata",
            Root::Shaders => "shadercache",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }

    /// The folder holding this root's per-game folders.
    fn parent(self, library: &Path) -> PathBuf {
        let apps = library.join("steamapps");
        match self {
            Root::Game => apps.join("common"),
            Root::Workshop => apps.join("workshop").join("content"),
            Root::Compat => apps.join("compatdata"),
            Root::Shaders => apps.join("shadercache"),
        }
    }

    fn path(self, library: &Path, app: AppId, install_dir: &str) -> PathBuf {
        match self {
            Root::Game => self.parent(library).join(install_dir),
            _ => self.parent(library).join(app.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Dir,
    File { len: u64, modified: Option<SystemTime> },
    Link(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    rel: PathBuf,
    kind: Kind,
}

/// A checked move, ready to run.
#[derive(Debug, Clone)]
pub struct Plan {
    pub game: InstalledGame,
    pub dest: PathBuf,
    /// Everything that moves: the game, Workshop items and so on.
    pub bytes: u64,
    pub files: usize,
    pub free: u64,
    install_dir: String,
    roots: Vec<(Root, Vec<Entry>)>,
    manifest: Vec<u8>,
    workshop_acf: Option<Vec<u8>>,
}

fn manifest_path(library: &Path, app: AppId) -> PathBuf {
    library.join("steamapps").join(format!("appmanifest_{app}.acf"))
}

fn workshop_acf_path(library: &Path, app: AppId) -> PathBuf {
    library.join("steamapps").join("workshop").join(format!("appworkshop_{app}.acf"))
}

fn staging_dir(library: &Path, app: AppId) -> PathBuf {
    library.join("steamapps").join(format!("{STAGING_PREFIX}{app}"))
}

/// Same folder, also through different spellings or links.
fn same_folder(a: &Path, b: &Path) -> bool {
    if crate::same_path(a, b) {
        return true;
    }
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

fn dir_has_content(dir: &Path) -> bool {
    match fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() && !crate::is_symlink_or_reparse(&m) => fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(true),
        Ok(_) => true,
        Err(_) => false,
    }
}

/// Where a link points, in terms of the moved folder: `Some` with the
/// folder-relative path when it stays inside.
fn link_inside(root: &Path, rel: &Path, target: &Path) -> Option<PathBuf> {
    let base = if target.is_absolute() {
        target.strip_prefix(root).ok()?.to_path_buf()
    } else {
        rel.parent().unwrap_or(Path::new("")).join(target)
    };
    let mut out = PathBuf::new();
    for c in base.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Every file, folder and link under `root`, without following links.
/// A link that would break by moving (absolute into the folder, or relative
/// out of it) stops the move.
fn scan(root: &Path) -> Result<Vec<Entry>, MoveError> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let dir = root.join(&rel);
        for e in fs::read_dir(&dir).map_err(|e| io(&dir, e))? {
            let e = e.map_err(|e| io(&dir, e))?;
            let path = e.path();
            let child = rel.join(e.file_name());
            let m = fs::symlink_metadata(&path).map_err(|e| io(&path, e))?;
            if crate::is_symlink_or_reparse(&m) {
                let target = fs::read_link(&path).map_err(|_| MoveError::Links(path.display().to_string()))?;
                let inside = link_inside(root, &child, &target);
                if inside.is_some() == target.is_absolute() {
                    return Err(MoveError::Links(path.display().to_string()));
                }
                out.push(Entry { rel: child, kind: Kind::Link(target) });
            } else if m.is_dir() {
                out.push(Entry { rel: child.clone(), kind: Kind::Dir });
                stack.push(child);
            } else {
                out.push(Entry { rel: child, kind: Kind::File { len: m.len(), modified: m.modified().ok() } });
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// A file another program has open. On Windows that's a running game's
/// program and libraries; elsewhere files aren't locked, so this says no
/// (the app checks for a running game itself).
#[cfg(windows)]
fn file_in_use(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    // Opening doesn't change the file: nothing is written.
    matches!(fs::OpenOptions::new().read(true).write(true).share_mode(0).open(path), Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION))
}

#[cfg(not(windows))]
fn file_in_use(_path: &Path) -> bool {
    false
}

fn check_not_in_use(root: &Path, entries: &[Entry]) -> Result<(), MoveError> {
    for e in entries.iter().filter(|e| matches!(e.kind, Kind::File { .. })) {
        let path = root.join(&e.rel);
        if file_in_use(&path) {
            return Err(MoveError::InUse(path.display().to_string()));
        }
    }
    Ok(())
}

/// Only "installed" and "installed, update waiting" are safe: other flags
/// mean an update is part-way through changing the files.
fn state_is_movable(manifest: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(manifest) else { return false };
    let Ok(doc) = vdf::parse(text) else { return false };
    matches!(doc.obj("AppState").and_then(|a| a.u64("StateFlags")), Some(4 | 6))
}

/// Check that `game` can move to the library `dest`, and list what moves.
/// Reads the whole game folder, so run it off the UI thread.
pub fn plan(game: &InstalledGame, dest: &Path) -> Result<Plan, MoveError> {
    let app = game.app_id;
    crate::validate_common_child(&game.library.join("steamapps").join("common"), &game.install_dir).map_err(MoveError::Io)?;
    let install_dir = game.install_dir.file_name().and_then(|n| n.to_str()).ok_or_else(|| MoveError::Io("the game's folder name isn't valid".into()))?.to_string();
    if same_folder(&game.library, dest) {
        return Err(MoveError::SameFolder);
    }
    if let (Ok(d), Ok(g)) = (dest.canonicalize(), game.install_dir.canonicalize())
        && d.starts_with(&g)
    {
        return Err(MoveError::SameFolder);
    }
    let path = manifest_path(&game.library, app);
    let manifest = fs::read(&path).map_err(|e| io(&path, e))?;
    if !state_is_movable(&manifest) {
        return Err(MoveError::NotReady);
    }
    let m = fs::symlink_metadata(&game.install_dir).map_err(|e| io(&game.install_dir, e))?;
    if crate::is_symlink_or_reparse(&m) || !m.is_dir() {
        return Err(MoveError::Links(game.install_dir.display().to_string()));
    }
    if manifest_path(dest, app).exists() {
        return Err(MoveError::AlreadyThere);
    }
    if fs::symlink_metadata(staging_dir(dest, app)).is_ok() {
        return Err(MoveError::Leftover);
    }
    let apps = dest.join("steamapps");
    fs::create_dir_all(&apps).map_err(|e| io(&apps, e))?;

    let mut roots = Vec::new();
    for root in Root::ALL {
        let src = root.path(&game.library, app, &install_dir);
        let Ok(m) = fs::symlink_metadata(&src) else { continue };
        if crate::is_symlink_or_reparse(&m) || !m.is_dir() {
            match root {
                Root::Game | Root::Workshop => return Err(MoveError::Links(src.display().to_string())),
                _ => continue,
            }
        }
        let dst = root.path(dest, app, &install_dir);
        if dir_has_content(&dst) {
            match root {
                Root::Game | Root::Workshop => return Err(MoveError::FolderExists(dst)),
                // The destination has its own: leave this one where it is.
                _ => continue,
            }
        }
        let entries = scan(&src)?;
        check_not_in_use(&src, &entries)?;
        roots.push((root, entries));
    }
    let (mut bytes, mut files) = (0, 0);
    for (_, entries) in &roots {
        for e in entries {
            if let Kind::File { len, .. } = e.kind {
                bytes += len;
                files += 1;
            }
        }
    }
    let free = fs4::available_space(dest).map_err(|e| io(dest, e))?;
    if free < bytes.saturating_add(SPARE_BYTES) {
        return Err(MoveError::NoSpace { need: bytes, free });
    }
    let workshop_acf = fs::read(workshop_acf_path(&game.library, app)).ok();
    Ok(Plan { game: game.clone(), dest: dest.to_path_buf(), bytes, files, free, install_dir, roots, manifest, workshop_acf })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Copying,
    /// Folders are being renamed into place.
    Placing,
    /// The destination manifest is written: the game lives there.
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Journal {
    source: PathBuf,
    install_dir: String,
    stage: Stage,
    roots: Vec<String>,
}

impl Journal {
    fn write(&self, staging: &Path) -> Result<(), MoveError> {
        let stage = match self.stage {
            Stage::Copying => "copying",
            Stage::Placing => "placing",
            Stage::Committed => "committed",
        };
        let text = format!("source={}\ninstalldir={}\nstage={stage}\nroots={}\n", self.source.display(), self.install_dir, self.roots.join(","));
        write_atomic(&staging.join(JOURNAL), text.as_bytes())
    }

    fn read(staging: &Path) -> Option<Self> {
        let text = fs::read_to_string(staging.join(JOURNAL)).ok()?;
        let value = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('='));
        let stage = match value("stage")? {
            "copying" => Stage::Copying,
            "placing" => Stage::Placing,
            "committed" => Stage::Committed,
            _ => return None,
        };
        let install_dir = value("installdir")?.to_string();
        if !crate::is_valid_folder_name(&install_dir) {
            return None;
        }
        Some(Self { source: PathBuf::from(value("source")?), install_dir, stage, roots: value("roots")?.split(',').filter(|r| !r.is_empty()).map(String::from).collect() })
    }
}

/// Write a whole file in one step: readers see the old one or the new one.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), MoveError> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.sir-tmp"));
    let written = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(io(path, e));
    }
    Ok(())
}

/// Clear read-only flags under `dir` (so it can be deleted), not following links.
fn clear_readonly(dir: &Path) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(m) = fs::symlink_metadata(&p) else { continue };
            if crate::is_symlink_or_reparse(&m) {
                continue;
            }
            if m.is_dir() {
                stack.push(p);
            } else if m.permissions().readonly() {
                let mut perms = m.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                let _ = fs::set_permissions(&p, perms);
            }
        }
    }
}

fn remove_tree(dir: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(m) if crate::is_symlink_or_reparse(&m) => crate::unlink_symlink_or_junction(dir, &m),
        Ok(_) => {
            clear_readonly(dir);
            fs::remove_dir_all(dir)
        }
    }
}

fn make_link(target: &Path, at: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, at)
    }
    #[cfg(windows)]
    {
        let full = at.parent().unwrap_or(Path::new("")).join(target);
        if full.is_dir() { std::os::windows::fs::symlink_dir(target, at) } else { std::os::windows::fs::symlink_file(target, at) }
    }
}

/// Copy one file, returning the SHA-1 of what was read from the original.
fn copy_file(src: &Path, dst: &Path, expected: u64, modified: Option<SystemTime>, buf: &mut [u8], p: &MoveProgress) -> Result<[u8; 20], MoveError> {
    let mut input = fs::File::open(src).map_err(|e| {
        if file_in_use(src) { MoveError::InUse(src.display().to_string()) } else { io(src, e) }
    })?;
    let perms = input.metadata().map_err(|e| io(src, e))?.permissions();
    let mut out = fs::File::create(dst).map_err(|e| io(dst, e))?;
    let mut hasher = Sha1::new();
    let mut copied = 0u64;
    loop {
        if p.cancelled() {
            return Err(MoveError::Cancelled);
        }
        let n = input.read(buf).map_err(|e| io(src, e))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| io(dst, e))?;
        hasher.update(&buf[..n]);
        copied += n as u64;
        p.done_bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    if copied != expected {
        return Err(MoveError::Changed(src.display().to_string()));
    }
    if let Some(t) = modified {
        let _ = out.set_modified(t);
    }
    // On disk before the original can be deleted, even if the power goes.
    out.sync_all().map_err(|e| io(dst, e))?;
    drop(out);
    fs::set_permissions(dst, perms).map_err(|e| io(dst, e))?;
    Ok(hasher.finalize().into())
}

fn hash_file(path: &Path, buf: &mut [u8], p: &MoveProgress) -> Result<[u8; 20], MoveError> {
    let mut f = fs::File::open(path).map_err(|e| io(path, e))?;
    let mut hasher = Sha1::new();
    loop {
        if p.cancelled() {
            return Err(MoveError::Cancelled);
        }
        let n = f.read(buf).map_err(|e| io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        p.done_bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(hasher.finalize().into())
}

/// Points of a move a crash test can stop at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
enum Step {
    Copied,
    Verified,
    Placing,
    PlacedOne,
    Placed,
    AcfWritten,
    Committed,
    SourceManifestGone,
    SourceDeleted,
}

/// How a finished move went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The game moved, but this old folder couldn't be fully deleted (it's
    /// tried again at the next start).
    pub leftovers: Option<PathBuf>,
}

/// Move the game. Blocks until done: run it off the UI thread.
pub fn run(plan: &Plan, p: &MoveProgress) -> Result<Outcome, MoveError> {
    let Some(_claim) = Claim::take(plan.game.app_id) else { return Err(MoveError::AlreadyMoving) };
    execute(plan, p, &mut |_| false)
}

fn execute(plan: &Plan, p: &MoveProgress, at: &mut dyn FnMut(Step) -> bool) -> Result<Outcome, MoveError> {
    let app = plan.game.app_id;
    let staging = staging_dir(&plan.dest, app);
    if fs::symlink_metadata(&staging).is_ok() {
        return Err(MoveError::Leftover);
    }
    fs::create_dir_all(&staging).map_err(|e| io(&staging, e))?;
    p.total_bytes.store(plan.bytes.saturating_mul(2), Ordering::Relaxed);
    p.done_bytes.store(0, Ordering::Relaxed);
    let mut journal = Journal {
        source: plan.game.library.clone(),
        install_dir: plan.install_dir.clone(),
        stage: Stage::Copying,
        roots: plan.roots.iter().map(|(r, _)| r.name().to_string()).collect(),
    };
    match copy_verify_place(plan, p, &staging, &mut journal, at) {
        Ok(()) => {}
        // A crash test's "power cut": nothing more runs.
        Err(MoveError::Interrupted) => return Err(MoveError::Interrupted),
        Err(e) => {
            undo(plan, &staging, journal.stage);
            return Err(e);
        }
    }
    finish(plan, p, &staging, &mut journal, at)
}

/// Everything up to and including the commit. Any error here leaves the
/// original as it was.
fn copy_verify_place(plan: &Plan, p: &MoveProgress, staging: &Path, journal: &mut Journal, at: &mut dyn FnMut(Step) -> bool) -> Result<(), MoveError> {
    let app = plan.game.app_id;
    macro_rules! step {
        ($s:expr) => {
            if at($s) {
                return Err(MoveError::Interrupted);
            }
        };
    }
    journal.write(staging)?;
    p.set_phase(Phase::Copying);
    let mut buf = vec![0u8; 1 << 20];
    let mut hashes: Vec<Vec<[u8; 20]>> = Vec::new();
    for (root, entries) in &plan.roots {
        let src = root.path(&plan.game.library, app, &plan.install_dir);
        let dst = staging.join(root.name());
        fs::create_dir_all(&dst).map_err(|e| io(&dst, e))?;
        let mut sums = Vec::new();
        for e in entries {
            if p.cancelled() {
                return Err(MoveError::Cancelled);
            }
            let (from, to) = (src.join(&e.rel), dst.join(&e.rel));
            match &e.kind {
                Kind::Dir => fs::create_dir_all(&to).map_err(|e| io(&to, e))?,
                Kind::Link(target) => {
                    if let Some(parent) = to.parent() {
                        fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
                    }
                    make_link(target, &to).map_err(|_| MoveError::Links(from.display().to_string()))?;
                }
                Kind::File { len, modified } => {
                    if let Some(parent) = to.parent() {
                        fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
                    }
                    sums.push(copy_file(&from, &to, *len, *modified, &mut buf, p)?);
                }
            }
        }
        hashes.push(sums);
    }
    step!(Step::Copied);

    // Read every copy back and compare.
    p.set_phase(Phase::Verifying);
    for ((root, entries), sums) in plan.roots.iter().zip(&hashes) {
        let dst = staging.join(root.name());
        let files = entries.iter().filter(|e| matches!(e.kind, Kind::File { .. }));
        for (e, sum) in files.zip(sums) {
            let to = dst.join(&e.rel);
            if hash_file(&to, &mut buf, p)? != *sum {
                return Err(MoveError::Mismatch(e.rel.display().to_string()));
            }
        }
    }
    // Nothing may have changed since the copy: no game started, no updater.
    for (root, entries) in &plan.roots {
        let src = root.path(&plan.game.library, app, &plan.install_dir);
        let now = scan(&src)?;
        if now != *entries {
            let what = now.iter().zip(entries.iter()).find(|(a, b)| a != b).map(|(a, _)| a.rel.display().to_string()).unwrap_or_else(|| src.display().to_string());
            return Err(MoveError::Changed(what));
        }
        check_not_in_use(&src, entries)?;
    }
    let manifest = manifest_path(&plan.game.library, app);
    if fs::read(&manifest).map_err(|e| io(&manifest, e))? != plan.manifest {
        return Err(MoveError::Changed(manifest.display().to_string()));
    }
    if p.cancelled() {
        return Err(MoveError::Cancelled);
    }
    step!(Step::Verified);

    // Switch over. From here on the move isn't cancelled any more.
    p.set_phase(Phase::Finishing);
    journal.stage = Stage::Placing;
    journal.write(staging)?;
    step!(Step::Placing);
    for (root, _) in &plan.roots {
        let dst = root.path(&plan.dest, app, &plan.install_dir);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        if dir_has_content(&dst) {
            return Err(MoveError::FolderExists(dst));
        }
        let _ = fs::remove_dir(&dst);
        let from = staging.join(root.name());
        fs::rename(&from, &dst).map_err(|e| io(&dst, e))?;
        step!(Step::PlacedOne);
    }
    step!(Step::Placed);
    if let Some(acf) = &plan.workshop_acf {
        let path = workshop_acf_path(&plan.dest, app);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        write_atomic(&path, acf)?;
    }
    step!(Step::AcfWritten);
    // The commit: one atomic write makes the destination the game's home.
    write_atomic(&manifest_path(&plan.dest, app), &plan.manifest)?;
    journal.stage = Stage::Committed;
    Ok(())
}

/// Take back whatever was put in the destination. Only called before the commit.
fn undo(plan: &Plan, staging: &Path, stage: Stage) {
    if stage != Stage::Copying {
        for (root, _) in &plan.roots {
            let _ = remove_tree(&root.path(&plan.dest, plan.game.app_id, &plan.install_dir));
        }
        if plan.workshop_acf.is_some() {
            let _ = fs::remove_file(workshop_acf_path(&plan.dest, plan.game.app_id));
        }
    }
    let _ = remove_tree(staging);
}

/// The game lives at the destination now: delete the original.
fn finish(plan: &Plan, p: &MoveProgress, staging: &Path, journal: &mut Journal, at: &mut dyn FnMut(Step) -> bool) -> Result<Outcome, MoveError> {
    // If this write fails the destination's manifest still marks the commit.
    let _ = journal.write(staging);
    if at(Step::Committed) {
        return Err(MoveError::Interrupted);
    }
    p.set_phase(Phase::Cleaning);
    let app = plan.game.app_id;
    let source_manifest = manifest_path(&plan.game.library, app);
    let manifest_gone = match fs::remove_file(&source_manifest) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    };
    if at(Step::SourceManifestGone) {
        return Err(MoveError::Interrupted);
    }
    let roots: Vec<Root> = plan.roots.iter().map(|(r, _)| *r).collect();
    let leftovers = if manifest_gone { delete_source(&plan.game.library, app, &plan.install_dir, &roots) } else { Some(source_manifest) };
    if at(Step::SourceDeleted) {
        return Err(MoveError::Interrupted);
    }
    // Kept when something's left: the next start tries again.
    if leftovers.is_none() {
        let _ = remove_tree(staging);
    }
    Ok(Outcome { leftovers })
}

/// Delete the moved folders from the old library. Returns the first folder
/// that couldn't be fully removed.
fn delete_source(library: &Path, app: AppId, install_dir: &str, roots: &[Root]) -> Option<PathBuf> {
    let mut left = None;
    for root in roots {
        let dir = root.path(library, app, install_dir);
        if manage::remove_contained_dir(&root.parent(library), &dir, &Progress::default()).is_err() || fs::symlink_metadata(&dir).is_ok() {
            left.get_or_insert(dir);
        }
    }
    if roots.contains(&Root::Workshop) {
        let _ = fs::remove_file(workshop_acf_path(library, app));
    }
    left
}

/// What [`recover`] did about a move that was cut off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    pub app: AppId,
    /// The move had finished switching over: the game is in `library`.
    /// Otherwise it stayed in the old one.
    pub moved: bool,
    pub library: PathBuf,
}

/// Finish or undo moves that were cut off (the app or PC stopped). Run once
/// at start, off the UI thread, with every library folder known.
pub fn recover(libraries: &[PathBuf]) -> Vec<Recovered> {
    let mut done = Vec::new();
    for lib in libraries {
        let Ok(rd) = fs::read_dir(lib.join("steamapps")) else { continue };
        let found: Vec<(PathBuf, AppId)> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name();
                let app = name.to_str()?.strip_prefix(STAGING_PREFIX)?.parse().ok()?;
                Some((e.path(), app))
            })
            .collect();
        for (staging, app) in found {
            // A move running right now isn't cut off.
            let Some(_claim) = Claim::take(app) else { continue };
            if let Some(r) = recover_one(lib, &staging, app, libraries) {
                done.push(r);
            }
        }
    }
    done
}

fn recover_one(lib: &Path, staging: &Path, app: AppId, libraries: &[PathBuf]) -> Option<Recovered> {
    let Some(journal) = Journal::read(staging) else {
        // Stopped before the journal was written: only partial copies here.
        let _ = remove_tree(staging);
        return None;
    };
    let roots: Vec<Root> = journal.roots.iter().filter_map(|r| Root::parse(r)).collect();
    let dest_has_it = manifest_path(lib, app).is_file() && Root::Game.path(lib, app, &journal.install_dir).is_dir();
    let source_known = libraries.iter().any(|l| crate::same_path(l, &journal.source)) && !crate::same_path(&journal.source, lib);

    if journal.stage != Stage::Copying && dest_has_it {
        // Past the commit: finish deleting the original.
        if !source_known {
            tracing::warn!("move of {app}: old library {} isn't known, leaving its files", journal.source.display());
            let _ = remove_tree(staging);
            return Some(Recovered { app, moved: true, library: lib.to_path_buf() });
        }
        let source_manifest = manifest_path(&journal.source, app);
        let manifest_gone = match fs::remove_file(&source_manifest) {
            Ok(()) => true,
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
        let left = if manifest_gone { delete_source(&journal.source, app, &journal.install_dir, &roots) } else { Some(source_manifest) };
        match left {
            None => {
                let _ = remove_tree(staging);
            }
            Some(p) => tracing::warn!("move of {app}: couldn't delete {} yet", p.display()),
        }
        return Some(Recovered { app, moved: true, library: lib.to_path_buf() });
    }

    // Before the commit: the original is the game. Take back what was placed,
    // but only if the original really is still there.
    if journal.stage == Stage::Placing {
        let intact = manifest_path(&journal.source, app).is_file() && Root::Game.path(&journal.source, app, &journal.install_dir).is_dir();
        if !intact {
            tracing::warn!("move of {app}: neither copy is complete, leaving the files in {} for repair", lib.display());
            return None;
        }
        for root in &roots {
            let _ = remove_tree(&root.path(lib, app, &journal.install_dir));
        }
        if roots.contains(&Root::Workshop) && workshop_acf_path(&journal.source, app).is_file() {
            let _ = fs::remove_file(workshop_acf_path(lib, app));
        }
    }
    let _ = remove_tree(staging);
    Some(Recovered { app, moved: false, library: journal.source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::InstallState;
    use std::sync::atomic::AtomicU32;

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Fake {
        base: PathBuf,
        from: PathBuf,
        to: PathBuf,
        game: InstalledGame,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = remove_tree(&self.base);
        }
    }

    const MANIFEST: &str = "\"AppState\"\n{\n\t\"appid\"\t\"10\"\n\t\"name\"\t\"Game\"\n\t\"StateFlags\"\t\"4\"\n\t\"installdir\"\t\"Game\"\n\t\"SizeOnDisk\"\t\"1000\"\n}\n";

    /// A pretend library with one game, Workshop items and a Proton prefix.
    fn fake() -> Fake {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("sir-relocate-{}-{n}", std::process::id()));
        let _ = remove_tree(&base);
        let (from, to) = (base.join("From"), base.join("To"));
        let apps = from.join("steamapps");
        let dir = apps.join("common").join("Game");
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::create_dir_all(dir.join("empty")).unwrap();
        fs::write(dir.join("game.exe"), vec![7u8; 300_000]).unwrap();
        fs::write(dir.join("bin").join("data.pak"), b"some game data").unwrap();
        let ro = dir.join("bin").join("readonly.cfg");
        fs::write(&ro, b"keep me").unwrap();
        let mut perms = fs::metadata(&ro).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&ro, perms).unwrap();
        let ws = apps.join("workshop");
        fs::create_dir_all(ws.join("content").join("10").join("555")).unwrap();
        fs::write(ws.join("content").join("10").join("555").join("mod.dat"), b"workshop item").unwrap();
        fs::write(ws.join("appworkshop_10.acf"), "\"AppWorkshop\" { \"WorkshopItemsInstalled\" { \"555\" { \"size\" \"13\" } } }").unwrap();
        fs::create_dir_all(apps.join("compatdata").join("10").join("pfx")).unwrap();
        fs::write(apps.join("compatdata").join("10").join("pfx").join("save.dat"), b"save").unwrap();
        fs::write(apps.join("appmanifest_10.acf"), MANIFEST).unwrap();
        fs::create_dir_all(to.join("steamapps")).unwrap();
        let game = InstalledGame {
            app_id: 10,
            name: "Game".into(),
            install_dir: dir,
            library: from.clone(),
            size_on_disk: 1000,
            last_updated: None,
            last_played: None,
            playtime_minutes: None,
            state: InstallState::Installed,
            last_update_bytes: None,
        };
        Fake { base, from, to, game }
    }

    /// True when `lib` holds the complete game, byte for byte.
    fn complete_in(lib: &Path) -> bool {
        let apps = lib.join("steamapps");
        let dir = apps.join("common").join("Game");
        fs::read_to_string(apps.join("appmanifest_10.acf")).is_ok_and(|m| m == MANIFEST)
            && fs::read(dir.join("game.exe")).is_ok_and(|b| b == vec![7u8; 300_000])
            && fs::read(dir.join("bin").join("data.pak")).is_ok_and(|b| b == b"some game data")
            && fs::read(dir.join("bin").join("readonly.cfg")).is_ok_and(|b| b == b"keep me")
            && dir.join("empty").is_dir()
            && fs::read(apps.join("workshop").join("content").join("10").join("555").join("mod.dat")).is_ok_and(|b| b == b"workshop item")
            && apps.join("workshop").join("appworkshop_10.acf").is_file()
            && fs::read(apps.join("compatdata").join("10").join("pfx").join("save.dat")).is_ok_and(|b| b == b"save")
    }

    /// True when nothing of the game is left in `lib`.
    fn gone_from(lib: &Path) -> bool {
        let apps = lib.join("steamapps");
        !apps.join("appmanifest_10.acf").exists()
            && !apps.join("common").join("Game").exists()
            && !apps.join("workshop").join("content").join("10").exists()
            && !apps.join("workshop").join("appworkshop_10.acf").exists()
            && !apps.join("compatdata").join("10").exists()
    }

    #[test]
    fn moves_a_game_with_workshop_items_and_saves() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        assert_eq!(plan.files, 5);
        assert!(plan.bytes >= 300_000);
        let p = MoveProgress::default();
        assert_eq!(run(&plan, &p).unwrap(), Outcome::default());
        assert!(complete_in(&f.to));
        assert!(gone_from(&f.from));
        assert!(!staging_dir(&f.to, 10).exists());
        assert!(fs::metadata(f.to.join("steamapps/common/Game/bin/readonly.cfg")).unwrap().permissions().readonly(), "keeps read-only files read-only");
        assert_eq!(p.done_bytes.load(Ordering::Relaxed), plan.bytes * 2);
        assert!(!is_moving(10));
        // The library now finds it in the new folder.
        let moved = crate::game_in_library(&f.to, 10).unwrap();
        assert_eq!(moved.install_dir, f.to.join("steamapps").join("common").join("Game"));
        assert!(crate::game_in_library(&f.from, 10).is_none());
    }

    #[test]
    fn keeps_file_times() {
        let f = fake();
        let src = f.game.install_dir.join("bin").join("data.pak");
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
        fs::OpenOptions::new().write(true).open(&src).unwrap().set_modified(old).unwrap();
        let plan = plan(&f.game, &f.to).unwrap();
        run(&plan, &MoveProgress::default()).unwrap();
        let t = fs::metadata(f.to.join("steamapps/common/Game/bin/data.pak")).unwrap().modified().unwrap();
        assert_eq!(t, old);
    }

    /// Stop at every step, as if the power went, then start again: the game
    /// is complete in exactly one library, and usable before recovery too.
    #[test]
    fn a_cut_off_move_always_leaves_a_working_game() {
        use Step::*;
        for stop in [Copied, Verified, Placing, PlacedOne, Placed, AcfWritten, Committed, SourceManifestGone, SourceDeleted] {
            let f = fake();
            let plan = plan(&f.game, &f.to).unwrap();
            let r = execute(&plan, &MoveProgress::default(), &mut |s| s == stop);
            assert_eq!(r, Err(MoveError::Interrupted), "{stop:?}");
            // Before any recovery: a manifest points at complete files.
            let usable = (crate::game_in_library(&f.from, 10).is_some() && complete_in(&f.from)) || (crate::game_in_library(&f.to, 10).is_some() && complete_in(&f.to));
            assert!(usable, "no working copy after stopping at {stop:?}");

            let libs = vec![f.from.clone(), f.to.clone()];
            let rec = recover(&libs);
            assert_eq!(rec.len(), 1, "{stop:?}");
            let committed = matches!(stop, Committed | SourceManifestGone | SourceDeleted);
            assert_eq!(rec[0].moved, committed, "{stop:?}");
            if committed {
                assert!(complete_in(&f.to), "{stop:?}");
                assert!(gone_from(&f.from), "{stop:?}");
            } else {
                assert!(complete_in(&f.from), "{stop:?}");
                assert!(gone_from(&f.to), "{stop:?}");
            }
            assert!(!staging_dir(&f.to, 10).exists(), "{stop:?}");
            assert!(recover(&libs).is_empty(), "second run has nothing to do");
        }
    }

    #[test]
    fn a_crash_while_copying_can_be_tried_again() {
        let f = fake();
        let first = plan(&f.game, &f.to).unwrap();
        let _ = execute(&first, &MoveProgress::default(), &mut |s| s == Step::Copied);
        assert_eq!(plan(&f.game, &f.to).unwrap_err(), MoveError::Leftover);
        recover(&[f.from.clone(), f.to.clone()]);
        let again = plan(&f.game, &f.to).unwrap();
        run(&again, &MoveProgress::default()).unwrap();
        assert!(complete_in(&f.to) && gone_from(&f.from));
    }

    #[test]
    fn cancelling_leaves_the_game_untouched() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        let p = MoveProgress::default();
        p.cancel.store(true, Ordering::Relaxed);
        assert_eq!(run(&plan, &p), Err(MoveError::Cancelled));
        assert!(complete_in(&f.from));
        assert!(gone_from(&f.to));
        assert!(!staging_dir(&f.to, 10).exists());
    }

    #[test]
    fn cancelling_during_verification_cleans_up() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        let p = MoveProgress::default();
        let r = execute(&plan, &p, &mut |s| {
            if s == Step::Copied {
                p.cancel.store(true, Ordering::Relaxed);
            }
            false
        });
        assert_eq!(r, Err(MoveError::Cancelled));
        assert!(complete_in(&f.from));
        assert!(gone_from(&f.to));
        assert!(!staging_dir(&f.to, 10).exists());
    }

    #[test]
    fn a_file_changing_during_the_move_stops_it() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        let changed = f.game.install_dir.join("bin").join("data.pak");
        let r = execute(&plan, &MoveProgress::default(), &mut |s| {
            if s == Step::Copied {
                fs::write(&changed, b"the game wrote this").unwrap();
            }
            false
        });
        assert!(matches!(r, Err(MoveError::Changed(_))), "{r:?}");
        assert_eq!(fs::read(&changed).unwrap(), b"the game wrote this");
        assert!(gone_from(&f.to));
        assert!(f.from.join("steamapps/appmanifest_10.acf").exists());
    }

    #[test]
    fn a_new_file_during_the_move_stops_it() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        let added = f.game.install_dir.join("new_save.dat");
        let r = execute(&plan, &MoveProgress::default(), &mut |s| {
            if s == Step::Copied {
                fs::write(&added, b"new").unwrap();
            }
            false
        });
        assert!(matches!(r, Err(MoveError::Changed(_))), "{r:?}");
        assert!(added.exists());
        assert!(gone_from(&f.to));
    }

    #[test]
    fn a_damaged_copy_stops_the_move() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        let staged = staging_dir(&f.to, 10).join("game").join("bin").join("data.pak");
        let r = execute(&plan, &MoveProgress::default(), &mut |s| {
            if s == Step::Copied {
                fs::write(&staged, b"some game dat!").unwrap();
            }
            false
        });
        assert!(matches!(r, Err(MoveError::Mismatch(_))), "{r:?}");
        assert!(complete_in(&f.from));
        assert!(gone_from(&f.to));
    }

    #[test]
    fn an_updating_game_cant_move() {
        let f = fake();
        let path = f.from.join("steamapps").join("appmanifest_10.acf");
        for flags in ["1026", "1", "516"] {
            fs::write(&path, MANIFEST.replace("\"StateFlags\"\t\"4\"", &format!("\"StateFlags\"\t\"{flags}\""))).unwrap();
            assert_eq!(plan(&f.game, &f.to).unwrap_err(), MoveError::NotReady, "flags {flags}");
        }
        // "Installed, update waiting" is complete, so it can.
        fs::write(&path, MANIFEST.replace("\"StateFlags\"\t\"4\"", "\"StateFlags\"\t\"6\"")).unwrap();
        assert!(plan(&f.game, &f.to).is_ok());
    }

    #[test]
    fn an_update_starting_during_the_move_stops_it() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        let manifest = f.from.join("steamapps").join("appmanifest_10.acf");
        let r = execute(&plan, &MoveProgress::default(), &mut |s| {
            if s == Step::Copied {
                fs::write(&manifest, MANIFEST.replace("\"StateFlags\"\t\"4\"", "\"StateFlags\"\t\"1026\"")).unwrap();
            }
            false
        });
        assert!(matches!(r, Err(MoveError::Changed(_))), "{r:?}");
        assert!(gone_from(&f.to));
    }

    #[test]
    fn refuses_bad_destinations() {
        let f = fake();
        assert_eq!(plan(&f.game, &f.from).unwrap_err(), MoveError::SameFolder);
        assert_eq!(plan(&f.game, &f.game.install_dir.join("bin")).unwrap_err(), MoveError::SameFolder);

        let taken = f.to.join("steamapps").join("common").join("Game");
        fs::create_dir_all(&taken).unwrap();
        fs::write(taken.join("other.txt"), b"not ours").unwrap();
        assert!(matches!(plan(&f.game, &f.to), Err(MoveError::FolderExists(_))));
        assert_eq!(fs::read(taken.join("other.txt")).unwrap(), b"not ours");
        fs::remove_dir_all(&taken).unwrap();

        // An empty leftover folder is fine.
        fs::create_dir_all(&taken).unwrap();
        assert!(plan(&f.game, &f.to).is_ok());
        fs::remove_dir_all(&taken).unwrap();

        fs::write(f.to.join("steamapps").join("appmanifest_10.acf"), MANIFEST).unwrap();
        assert_eq!(plan(&f.game, &f.to).unwrap_err(), MoveError::AlreadyThere);
    }

    #[test]
    fn keeps_a_destination_prefix_it_found_there() {
        let f = fake();
        let theirs = f.to.join("steamapps").join("compatdata").join("10");
        fs::create_dir_all(&theirs).unwrap();
        fs::write(theirs.join("other.dat"), b"theirs").unwrap();
        let plan = plan(&f.game, &f.to).unwrap();
        run(&plan, &MoveProgress::default()).unwrap();
        assert_eq!(fs::read(theirs.join("other.dat")).unwrap(), b"theirs");
        assert!(!theirs.join("pfx").exists());
        assert!(f.from.join("steamapps/compatdata/10/pfx/save.dat").exists(), "the old prefix isn't deleted when it didn't move");
    }

    #[test]
    fn refuses_a_game_outside_common() {
        let f = fake();
        let mut bad = f.game.clone();
        bad.install_dir = f.from.join("steamapps").join("common").join("..").join("Game");
        assert!(matches!(plan(&bad, &f.to), Err(MoveError::Io(_))));
    }

    #[test]
    fn recovery_leaves_other_folders_alone() {
        let f = fake();
        let other = f.to.join("steamapps").join("common").join("Other Game");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("x"), b"x").unwrap();
        fs::create_dir_all(f.to.join("steamapps").join(".sir-move-notanumber")).unwrap();
        // A cut-off move with no journal: only its own folder goes.
        let staging = staging_dir(&f.to, 42);
        fs::create_dir_all(staging.join("game")).unwrap();
        assert!(recover(&[f.from.clone(), f.to.clone()]).is_empty());
        assert!(!staging.exists());
        assert!(other.join("x").exists());
        assert!(f.to.join("steamapps").join(".sir-move-notanumber").exists());
        assert!(complete_in(&f.from));
    }

    #[test]
    fn recovery_wont_delete_from_an_unknown_library() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        let _ = execute(&plan, &MoveProgress::default(), &mut |s| s == Step::Committed);
        // Only the destination is listed: the old library isn't touched.
        let rec = recover(std::slice::from_ref(&f.to));
        assert_eq!(rec.len(), 1);
        assert!(rec[0].moved);
        assert!(f.from.join("steamapps/appmanifest_10.acf").exists());
        assert!(f.game.install_dir.join("game.exe").exists());
    }

    #[test]
    fn recovery_keeps_the_copy_if_the_original_is_gone() {
        let f = fake();
        let plan = plan(&f.game, &f.to).unwrap();
        let _ = execute(&plan, &MoveProgress::default(), &mut |s| s == Step::Placed);
        // Someone deleted the original meanwhile: the copy is all there is.
        remove_tree(&f.game.install_dir).unwrap();
        recover(&[f.from.clone(), f.to.clone()]);
        assert!(f.to.join("steamapps/common/Game/game.exe").exists());
    }

    #[test]
    fn a_game_being_moved_is_marked() {
        let c = Claim::take(4242).unwrap();
        assert!(is_moving(4242));
        assert!(Claim::take(4242).is_none());
        drop(c);
        assert!(!is_moving(4242));
    }

    #[cfg(unix)]
    #[test]
    fn keeps_relative_links_and_refuses_ones_that_would_break() {
        let f = fake();
        let dir = f.game.install_dir.clone();
        std::os::unix::fs::symlink("data.pak", dir.join("bin").join("link.pak")).unwrap();
        let p = plan(&f.game, &f.to).unwrap();
        run(&p, &MoveProgress::default()).unwrap();
        let moved = f.to.join("steamapps/common/Game/bin/link.pak");
        assert_eq!(fs::read_link(&moved).unwrap(), PathBuf::from("data.pak"));
        assert_eq!(fs::read(&moved).unwrap(), b"some game data");

        let g = fake();
        std::os::unix::fs::symlink(g.game.install_dir.join("game.exe"), g.game.install_dir.join("abs")).unwrap();
        assert!(matches!(plan(&g.game, &g.to), Err(MoveError::Links(_))));
    }

    #[test]
    fn link_targets_are_judged_correctly() {
        let root = Path::new("/lib/game");
        let rel = Path::new("bin/link");
        assert_eq!(link_inside(root, rel, Path::new("data")), Some(PathBuf::from("bin/data")));
        assert_eq!(link_inside(root, rel, Path::new("../top")), Some(PathBuf::from("top")));
        assert_eq!(link_inside(root, rel, Path::new("../../outside")), None);
    }

    #[test]
    fn journal_round_trip() {
        let f = fake();
        let dir = f.base.join("j");
        fs::create_dir_all(&dir).unwrap();
        let j = Journal { source: f.from.clone(), install_dir: "Game".into(), stage: Stage::Placing, roots: vec!["game".into(), "workshop".into()] };
        j.write(&dir).unwrap();
        assert_eq!(Journal::read(&dir), Some(j));
        fs::write(dir.join(JOURNAL), "source=x\ninstalldir=..\nstage=placing\nroots=game\n").unwrap();
        assert_eq!(Journal::read(&dir), None, "an unsafe folder name is refused");
    }
}

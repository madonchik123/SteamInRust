//! Steam Cloud: keeps a game's saves the same on every PC, like Steam does.
//!
//! Two kinds of games:
//! - games that save through Steam's API keep files in
//!   `<Steam>/userdata/<account>/<app>/remote/`;
//! - Auto-Cloud games save into their own folders, which their app info
//!   lists (e.g. `%WinMyDocuments%My Games/Foo/*.sav`).
//!
//! Before a game starts, newer cloud files are downloaded; after it closes,
//! changed files are uploaded. A small record of the last sync tells
//! "changed here", "changed in the cloud" and "changed in both" (a conflict
//! the player decides) apart. Local files are backed up before replacing.

use model::{AppId, CloudConfig, CloudConflict, CloudReport};
use net::Connection;
use proto::*;
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// What to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Look only: report what would change.
    Check,
    /// Before playing: bring down newer cloud files.
    Download,
    /// After playing: send up changed files.
    Upload,
    /// Both directions.
    Sync,
}

/// Everything a sync needs to know about the game on this PC.
#[derive(Debug, Clone)]
pub struct Target {
    pub app_id: AppId,
    pub steam_id: u64,
    pub install_dir: PathBuf,
    pub steam_root: Option<PathBuf>,
    pub config: Option<CloudConfig>,
    pub cell_id: u32,
}

impl Target {
    fn account(&self) -> u32 {
        (self.steam_id & 0xFFFF_FFFF) as u32
    }

    /// Where API-saved files live.
    fn remote_dir(&self) -> PathBuf {
        let root = self.steam_root.clone().unwrap_or_else(|| crate::store::config_dir().join("steam"));
        root.join("userdata").join(self.account().to_string()).join(self.app_id.to_string()).join("remote")
    }

    /// Folder for a Steam root name like "WinMyDocuments" or "%GameInstall%".
    fn root_dir(&self, root: &str) -> Option<PathBuf> {
        let key = root.trim_matches('%').to_lowercase();
        let user = directories::UserDirs::new();
        let base = directories::BaseDirs::new();
        let home = base.as_ref().map(|b| b.home_dir().to_path_buf());
        Some(match key.as_str() {
            "gameinstall" => self.install_dir.clone(),
            "steamclouddocuments" => self.remote_dir(),
            "winmydocuments" if cfg!(target_os = "windows") => user.as_ref()?.document_dir()?.to_path_buf(),
            "winappdatalocal" if cfg!(target_os = "windows") => base.as_ref()?.data_local_dir().to_path_buf(),
            "winappdataroaming" if cfg!(target_os = "windows") => base.as_ref()?.data_dir().to_path_buf(),
            "winappdatalocallow" if cfg!(target_os = "windows") => home?.join("AppData").join("LocalLow"),
            "winsavedgames" if cfg!(target_os = "windows") => home?.join("Saved Games"),
            "winpictures" if cfg!(target_os = "windows") => user.as_ref()?.picture_dir()?.to_path_buf(),
            "winvideos" if cfg!(target_os = "windows") => user.as_ref()?.video_dir()?.to_path_buf(),
            "winmusic" if cfg!(target_os = "windows") => user.as_ref()?.audio_dir()?.to_path_buf(),
            // Steam's macOS roots.
            "machome" if cfg!(target_os = "macos") => home?,
            "macappsupport" if cfg!(target_os = "macos") => base.as_ref()?.data_dir().to_path_buf(),
            "macdocuments" if cfg!(target_os = "macos") => user.as_ref()?.document_dir()?.to_path_buf(),
            "macdesktop" if cfg!(target_os = "macos") => user.as_ref()?.desktop_dir()?.to_path_buf(),
            "maccaches" if cfg!(target_os = "macos") => base.as_ref()?.cache_dir().to_path_buf(),
            // Steam's Linux roots.
            "linuxhome" if cfg!(target_os = "linux") => home?,
            "linuxxdgdatahome" if cfg!(target_os = "linux") => base.as_ref()?.data_dir().to_path_buf(),
            "linuxxdgconfighome" if cfg!(target_os = "linux") => base.as_ref()?.config_dir().to_path_buf(),
            "linuxxdgcachehome" if cfg!(target_os = "linux") => base.as_ref()?.cache_dir().to_path_buf(),
            _ => return None,
        })
    }

    /// Steam's placeholders in Auto-Cloud paths.
    fn expand(&self, path: &str) -> String {
        path.replace("{64BitSteamID}", &self.steam_id.to_string()).replace("{Steam3AccountID}", &self.account().to_string())
    }

    fn resolve_cloud_target(&self, name: &str) -> Option<(PathBuf, Vec<String>)> {
        if name.is_empty() || name.contains('\0') {
            return None;
        }
        if let Some(rest) = name.strip_prefix('%') {
            let end = rest.find('%')?;
            let raw_root = &rest[..end];
            let after_root = &rest[end + 1..];
            if raw_root.is_empty() {
                return None;
            }
            let cfg = self.config.as_ref()?;
            if cfg.save_files.is_empty() {
                return None;
            }
            cfg.save_files.iter().find_map(|rule| self.matches_save_rule(raw_root, after_root, rule))
        } else {
            let segs = safe_rel_segments(name)?;
            Some((self.remote_dir(), segs))
        }
    }

    fn matches_save_rule(&self, raw_root: &str, after_root: &str, rule: &model::SaveRule) -> Option<(PathBuf, Vec<String>)> {
        if !canonical_root(raw_root).eq_ignore_ascii_case(&canonical_root(&rule.root)) {
            return None;
        }
        let base = self.root_dir(&rule.root)?;
        safe_rel_segments(after_root)?;
        let segs = safe_rel_segments(&self.expand(after_root))?;
        let rule_sub = rule_dir(&self.expand(&rule.path));
        let rule_segs = if rule_sub.is_empty() { Vec::new() } else { safe_rel_segments(&rule_sub)? };
        let file_dir = &segs[..segs.len() - 1];
        let file_name = &segs[segs.len() - 1];
        if file_dir.len() < rule_segs.len() {
            return None;
        }
        if !rule.recursive && file_dir.len() != rule_segs.len() {
            return None;
        }
        if !file_dir[..rule_segs.len()].iter().zip(&rule_segs).all(|(a, b)| a.eq_ignore_ascii_case(b)) {
            return None;
        }
        if !glob(&rule.pattern, file_name) {
            return None;
        }
        let mut out_segs = rule_segs;
        out_segs.extend_from_slice(&segs[out_segs.len()..]);
        Some((base, out_segs))
    }

    /// Local file for a cloud name ("%Root%dir/file" or a plain API name),
    /// enforcing active `SaveRule`s and rejecting traversal or symlinks.
    pub fn local_path(&self, name: &str) -> Option<PathBuf> {
        let (base, segs) = self.resolve_cloud_target(name)?;
        resolve_under_root(&base, &segs)
    }

    /// Files on this PC that belong in the cloud: everything in the API
    /// folder plus files matching the Auto-Cloud rules. Cloud name -> path.
    fn scan_local(&self) -> BTreeMap<String, PathBuf> {
        let mut out = BTreeMap::new();
        let remote = self.remote_dir();
        if let Some(canon_remote) = resolve_scan_root(&remote) {
            for file in walk(&remote, true) {
                if let Ok(rel) = file.strip_prefix(&remote) {
                    let key = slashes(rel);
                    if safe_rel_segments(&key).is_some()
                        && file.canonicalize().is_ok_and(|c| c.starts_with(&canon_remote))
                    {
                        out.insert(key, file);
                    }
                }
            }
        }
        for rule in self.config.iter().flat_map(|c| c.save_files.iter()) {
            let Some(base) = self.root_dir(&rule.root) else { continue };
            let sub = self.expand(&rule_dir(&rule.path));
            let dir = if sub.is_empty() {
                let Some(_) = resolve_scan_root(&base) else { continue };
                base
            } else {
                let Some(sub_segs) = safe_rel_segments(&sub) else { continue };
                let Some(d) = resolve_scan_dir(&base, &sub_segs) else { continue };
                d
            };
            let Some(canon_dir) = resolve_scan_root(&dir) else { continue };
            for file in walk(&dir, rule.recursive) {
                let Some(fname) = file.file_name().and_then(|n| n.to_str()) else { continue };
                if !glob(&rule.pattern, fname) {
                    continue;
                }
                let Ok(rel) = file.strip_prefix(&dir) else { continue };
                let rel_str = slashes(rel);
                if safe_rel_segments(&rel_str).is_none()
                    || !file.canonicalize().is_ok_and(|c| c.starts_with(&canon_dir))
                {
                    continue;
                }
                let mut name = format!("%{}%", canonical_root(&rule.root));
                if !sub.is_empty() {
                    name.push_str(&rule_dir(&rule.path));
                    name.push('/');
                }
                name.push_str(&rel_str);
                out.insert(name, file);
            }
        }
        out
    }
}

/// An Auto-Cloud rule's folder with forward slashes and no slash at either
/// end: `KnightsEnd\Saved\SaveGames\` -> `KnightsEnd/Saved/SaveGames`.
/// (Trimming before converting left a trailing `\`, so files were named
/// `SaveGames//file.sav` in the cloud instead of Steam's `SaveGames/file.sav`.)
fn rule_dir(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_string()
}

fn slashes(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

fn is_symlink_or_reparse(meta: &std::fs::Metadata) -> bool {
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

fn is_valid_cloud_segment(seg: &str) -> bool {
    if seg.is_empty() || seg == "." || seg == ".." || seg.ends_with('.') || seg.ends_with(' ') {
        return false;
    }
    if seg.contains(['/', '\\', ':', '\0']) || seg.chars().any(|c| (c as u32) < 0x20) {
        return false;
    }
    if seg.to_ascii_lowercase().ends_with(".sircloud") || is_windows_reserved(seg) {
        return false;
    }
    let mut comps = Path::new(seg).components();
    matches!((comps.next(), comps.next()), (Some(Component::Normal(s)), None) if !s.is_empty())
}

fn safe_rel_segments(rel: &str) -> Option<Vec<String>> {
    if rel.is_empty() || rel.contains('\0') {
        return None;
    }
    let norm = rel.replace('\\', "/");
    let mut out = Vec::new();
    for seg in norm.split('/') {
        if !is_valid_cloud_segment(seg) {
            return None;
        }
        out.push(seg.to_string());
    }
    (!out.is_empty()).then_some(out)
}

fn resolve_scan_root(base: &Path) -> Option<PathBuf> {
    let m = std::fs::symlink_metadata(base).ok()?;
    if is_symlink_or_reparse(&m) || !m.is_dir() {
        return None;
    }
    base.canonicalize().ok()
}

fn resolve_scan_dir(base: &Path, segs: &[String]) -> Option<PathBuf> {
    let m = std::fs::symlink_metadata(base).ok()?;
    if is_symlink_or_reparse(&m) || !m.is_dir() {
        return None;
    }
    let mut cur = base.to_path_buf();
    for seg in segs {
        cur.push(seg);
        let dm = std::fs::symlink_metadata(&cur).ok()?;
        if is_symlink_or_reparse(&dm) || !dm.is_dir() {
            return None;
        }
    }
    Some(cur)
}

fn resolve_under_root(base: &Path, segs: &[String]) -> Option<PathBuf> {
    if segs.is_empty() {
        return None;
    }
    match std::fs::symlink_metadata(base) {
        Ok(m) if is_symlink_or_reparse(&m) || !m.is_dir() => return None,
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return None,
    }
    let mut cur = base.to_path_buf();
    for (idx, seg) in segs.iter().enumerate() {
        cur.push(seg);
        let is_last = idx + 1 == segs.len();
        match std::fs::symlink_metadata(&cur) {
            Ok(m) => {
                if is_symlink_or_reparse(&m) {
                    return None;
                }
                if !is_last && !m.is_dir() {
                    return None;
                }
                if is_last && m.is_dir() {
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                for rest in &segs[idx + 1..] {
                    cur.push(rest);
                }
                break;
            }
            Err(_) => return None,
        }
    }
    Some(cur)
}

fn prepare_parent_under_root(base: &Path, segs: &[String]) -> Result<PathBuf, String> {
    if segs.is_empty() {
        return Err("empty cloud path".into());
    }
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let base_meta = std::fs::symlink_metadata(base).map_err(|e| e.to_string())?;
    if is_symlink_or_reparse(&base_meta) || !base_meta.is_dir() {
        return Err("refusing symlink save root".into());
    }
    let mut cur = base.to_path_buf();
    for seg in &segs[..segs.len() - 1] {
        cur.push(seg);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) => {
                if is_symlink_or_reparse(&m) || !m.is_dir() {
                    return Err("refusing symlink or non-directory component in cloud path".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&cur).map_err(|e| e.to_string())?;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    cur.push(&segs[segs.len() - 1]);
    if let Ok(m) = std::fs::symlink_metadata(&cur)
        && (is_symlink_or_reparse(&m) || m.is_dir())
    {
        return Err("refusing symlink or directory cloud target".into());
    }
    Ok(cur)
}

fn cloud_tmp_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".sircloud");
    PathBuf::from(s)
}

/// Steam's spelling of root names in cloud file names.
fn canonical_root(root: &str) -> String {
    match root.trim_matches('%').to_lowercase().as_str() {
        "gameinstall" => "GameInstall".into(),
        "steamclouddocuments" => "SteamCloudDocuments".into(),
        "winmydocuments" => "WinMyDocuments".into(),
        "winappdatalocal" => "WinAppDataLocal".into(),
        "winappdataroaming" => "WinAppDataRoaming".into(),
        "winappdatalocallow" => "WinAppDataLocalLow".into(),
        "winsavedgames" => "WinSavedGames".into(),
        "winpictures" => "WinPictures".into(),
        "winvideos" => "WinVideos".into(),
        "winmusic" => "WinMusic".into(),
        "linuxhome" => "LinuxHome".into(),
        "linuxxdgdatahome" => "LinuxXdgDataHome".into(),
        "linuxxdgconfighome" => "LinuxXdgConfigHome".into(),
        "linuxxdgcachehome" => "LinuxXdgCacheHome".into(),
        "machome" => "MacHome".into(),
        "macappsupport" => "MacAppSupport".into(),
        "macdocuments" => "MacDocuments".into(),
        "macdesktop" => "MacDesktop".into(),
        "maccaches" => "MacCaches".into(),
        _ => root.trim_matches('%').to_string(),
    }
}

fn walk(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(meta) = std::fs::symlink_metadata(dir) else { return out };
    if is_symlink_or_reparse(&meta) || !meta.is_dir() {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for e in entries.flatten() {
        let p = e.path();
        let Ok(em) = std::fs::symlink_metadata(&p) else { continue };
        if is_symlink_or_reparse(&em) {
            continue;
        }
        if em.is_dir() {
            if recursive {
                out.extend(walk(&p, true));
            }
        } else if em.is_file() {
            out.push(p);
        }
    }
    out
}

/// `*` and `?` wildcards, case-insensitive (Windows file names).
pub fn glob(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha1(data: &[u8]) -> Vec<u8> {
    Sha1::digest(data).to_vec()
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What each file was at the last successful sync (cloud name -> sha1 hex).
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Snapshot {
    files: BTreeMap<String, String>,
}

fn snapshot_path(t: &Target) -> PathBuf {
    crate::store::config_dir().join("cloud").join(t.account().to_string()).join(format!("{}.json", t.app_id))
}

fn load_snapshot(t: &Target) -> Snapshot {
    let snap: Snapshot = std::fs::read_to_string(snapshot_path(t)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    // Older records may hold `dir//file` names (see `rule_dir`); the clean
    // name wins when both are there.
    let mut files = BTreeMap::new();
    for (k, v) in snap.files.iter().filter(|(k, _)| clean_name(k) != **k) {
        files.insert(clean_name(k), v.clone());
    }
    for (k, v) in snap.files.into_iter().filter(|(k, _)| clean_name(k) == *k) {
        files.insert(k, v);
    }
    Snapshot { files }
}

/// A cloud file name with doubled slashes collapsed (`dir//file` -> `dir/file`),
/// so files stored under the old names match the ones on this PC.
fn clean_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out
}

fn save_snapshot(t: &Target, s: &Snapshot) {
    let p = snapshot_path(t);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(p, text);
    }
}

/// Copy a local file aside before it's replaced. Kept per app and time.
fn backup(t: &Target, name: &str, path: &Path, stamp: u64) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if is_symlink_or_reparse(&m) || !m.is_file() => {
            return Err("refusing to back up symlink or non-regular file".into());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    }
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect();
    let dir = crate::store::config_dir().join("cloud-backups").join(t.app_id.to_string()).join(stamp.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::copy(path, dir.join(safe)).map(|_| ()).map_err(|e| e.to_string())
}

pub fn backups_dir(app_id: AppId) -> PathBuf {
    crate::store::config_dir().join("cloud-backups").join(app_id.to_string())
}

struct RemoteFile {
    /// The name as stored in the cloud (may differ from the clean key).
    raw: String,
    sha: String,
    time: u64,
    size: u64,
}

/// One planned action for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Same,
    Download,
    Upload,
    Conflict,
    /// Only in the cloud's history (deleted there); leave it.
    Skip,
}

fn decide(local: Option<&str>, remote: Option<&str>, last: Option<&str>) -> Plan {
    match (local, remote) {
        (None, Some(_)) => Plan::Download,
        (Some(_), None) if last.is_some() => Plan::Skip,
        (Some(_), None) => Plan::Upload,
        (Some(l), Some(r)) if l == r => Plan::Same,
        (Some(l), Some(r)) => match last {
            Some(s) if s == l => Plan::Download,
            Some(s) if s == r => Plan::Upload,
            _ => Plan::Conflict,
        },
        (None, None) => Plan::Skip,
    }
}

/// Run a sync. `keep_local` / `keep_cloud` resolve earlier conflicts.
pub async fn sync(conn: &Connection, t: &Target, mode: Mode, keep_local: &[String], keep_cloud: &[String]) -> CloudReport {
    let mut report = CloudReport { app_id: t.app_id, finished: now(), quota_bytes: t.config.as_ref().map(|c| c.quota).unwrap_or(0), ..Default::default() };
    // What's in the cloud.
    let list: CCloudGetAppFileChangelistResponse = match conn
        .call("Cloud.GetAppFileChangelist#1", &CCloudGetAppFileChangelistRequest { appid: Some(t.app_id), synced_change_number: Some(0) })
        .await
    {
        Ok(r) => r,
        Err(e) => {
            report.errors.push((String::new(), format!("Couldn't reach Steam Cloud: {e}")));
            return report;
        }
    };
    let mut remote: HashMap<String, RemoteFile> = HashMap::new();
    for f in &list.files {
        if f.persist_state() != ECloudStoragePersistState::KECloudStoragePersistStatePersisted {
            continue;
        }
        let prefix = f.path_prefix_index.and_then(|i| list.path_prefixes.get(i as usize)).cloned().unwrap_or_default();
        let name = format!("{prefix}{}", f.file_name());
        report.cloud_bytes += f.raw_file_size() as u64;
        let key = clean_name(&name);
        // A file under its clean name beats one under an old `//` name.
        if remote.get(&key).is_some_and(|r| r.raw == key) {
            continue;
        }
        remote.insert(key, RemoteFile { raw: name, sha: hex(f.sha_file()), time: f.time_stamp(), size: f.raw_file_size() as u64 });
    }
    report.cloud_files = remote.len() as u32;
    if let Ok(q) = conn.call::<_, CCloudClientGetAppQuotaUsageResponse>("Cloud.ClientGetAppQuotaUsage#1", &CCloudClientGetAppQuotaUsageRequest { appid: Some(t.app_id) }).await {
        if q.max_num_bytes() > 0 {
            report.quota_bytes = q.max_num_bytes();
        }
        report.cloud_bytes = report.cloud_bytes.max(q.existing_bytes());
    }

    // What's on this PC.
    let mut local: BTreeMap<String, PathBuf> = t.scan_local();
    for name in remote.keys() {
        if !local.contains_key(name) {
            if let Some(p) = t.local_path(name) {
                local.insert(name.clone(), p);
            }
        }
    }
    let mut snap = load_snapshot(t);
    let stamp = now();
    let mut uploads: Vec<(String, PathBuf, Vec<u8>)> = Vec::new();

    for (name, path) in &local {
        let data = std::fs::read(path).ok();
        let local_sha = data.as_deref().map(|d| hex(&sha1(d)));
        let r = remote.get(name);
        let mut plan = decide(local_sha.as_deref(), r.map(|r| r.sha.as_str()), snap.files.get(name).map(|s| s.as_str()));
        if plan == Plan::Conflict {
            if keep_local.contains(name) {
                plan = Plan::Upload;
            } else if keep_cloud.contains(name) {
                plan = Plan::Download;
            }
        }
        match plan {
            Plan::Same => {
                report.up_to_date += 1;
                if let Some(sha) = local_sha {
                    snap.files.insert(name.clone(), sha);
                }
            }
            Plan::Skip => {}
            Plan::Conflict => {
                let r = r.expect("conflict needs a cloud file");
                report.conflicts.push(CloudConflict { name: name.clone(), local_time: mtime(path), cloud_time: r.time, local_size: data.as_ref().map(|d| d.len() as u64).unwrap_or(0), cloud_size: r.size });
            }
            Plan::Download if matches!(mode, Mode::Download | Mode::Sync) => match download(conn, t, r.map(|r| r.raw.as_str()).unwrap_or(name), path, stamp).await {
                Ok(sha) => {
                    report.downloaded += 1;
                    snap.files.insert(name.clone(), sha);
                }
                Err(e) => report.errors.push((name.clone(), e)),
            },
            Plan::Upload if matches!(mode, Mode::Upload | Mode::Sync) => {
                if let Some(d) = data {
                    uploads.push((name.clone(), path.clone(), d));
                }
            }
            // Check mode, or the other direction: just count.
            Plan::Download | Plan::Upload => {}
        }
    }
    if !uploads.is_empty() {
        match upload(conn, t, &uploads).await {
            Ok(done) => {
                for (name, sha) in done {
                    report.uploaded += 1;
                    snap.files.insert(name, sha);
                }
            }
            Err(e) => report.errors.push((String::new(), e)),
        }
    }
    if mode != Mode::Check {
        save_snapshot(t, &snap);
    }
    report.finished = now();
    report
}

/// Fetch one file from the cloud into place (backing up what was there).
async fn download(conn: &Connection, t: &Target, name: &str, path: &Path, stamp: u64) -> Result<String, String> {
    let clean = clean_name(name);
    let (base, segs) = t.resolve_cloud_target(&clean).ok_or_else(|| "Cloud file is not allowed by save rules".to_string())?;
    let expected = resolve_under_root(&base, &segs).ok_or_else(|| "Cloud file path traverses a symlink".to_string())?;
    if expected != path {
        return Err("Cloud file target path mismatch".into());
    }
    let r: CCloudClientFileDownloadResponse = conn
        .call("Cloud.ClientFileDownload#1", &CCloudClientFileDownloadRequest { appid: Some(t.app_id), filename: Some(name.to_string()), ..Default::default() })
        .await
        .map_err(|e| e.to_string())?;
    if r.encrypted() {
        return Err("This file is encrypted by Steam and can only be synced by the official client".into());
    }
    let url = format!("{}://{}{}", if r.use_https() { "https" } else { "http" }, r.url_host(), r.url_path());
    let mut req = http().get(url);
    for h in &r.request_headers {
        req = req.header(h.name(), h.value());
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Steam Cloud answered {}", resp.status()));
    }
    let mut data = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
    // Compressed files come as a one-file ZIP.
    if r.file_size() != r.raw_file_size() && data.starts_with(b"PK") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
        let mut entry = zip.by_index(0).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut out).map_err(|e| e.to_string())?;
        data = out;
    }
    let sha = sha1(&data);
    if !r.sha_file().is_empty() && r.sha_file() != sha.as_slice() {
        return Err("The downloaded file didn't match Steam's checksum".into());
    }
    backup(t, name, path, stamp)?;
    let verified = prepare_parent_under_root(&base, &segs)?;
    let tmp = cloud_tmp_path(&verified);
    if let Ok(m) = std::fs::symlink_metadata(&tmp)
        && (is_symlink_or_reparse(&m) || m.is_dir())
    {
        return Err("refusing symlink temporary cloud file".into());
    }
    std::fs::write(&tmp, &data).map_err(|e| e.to_string())?;
    if let Ok(m) = std::fs::symlink_metadata(&verified)
        && (is_symlink_or_reparse(&m) || m.is_dir())
    {
        let _ = std::fs::remove_file(&tmp);
        return Err("refusing symlink cloud target".into());
    }
    std::fs::rename(&tmp, &verified).map_err(|e| e.to_string())?;
    if r.time_stamp() > 0 {
        if let Ok(f) = std::fs::File::options().write(true).open(&verified) {
            let _ = f.set_modified(UNIX_EPOCH + Duration::from_secs(r.time_stamp()));
        }
    }
    Ok(hex(&sha))
}

/// Send changed files up in one batch. Returns (name, sha) of each sent file.
async fn upload(conn: &Connection, t: &Target, files: &[(String, PathBuf, Vec<u8>)]) -> Result<Vec<(String, String)>, String> {
    let machine = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "SteamInRust".into());
    let batch: CCloudBeginAppUploadBatchResponse = conn
        .call(
            "Cloud.BeginAppUploadBatch#1",
            &CCloudBeginAppUploadBatchRequest {
                appid: Some(t.app_id),
                machine_name: Some(machine),
                files_to_upload: files.iter().map(|f| f.0.clone()).collect(),
                files_to_delete: Vec::new(),
                client_id: Some(now()),
                app_build_id: Some(0),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut done = Vec::new();
    let mut failed = None;
    for (name, path, data) in files {
        let sha = sha1(data);
        let begin: Result<CCloudClientBeginFileUploadResponse, _> = conn
            .call(
                "Cloud.ClientBeginFileUpload#1",
                &CCloudClientBeginFileUploadRequest {
                    appid: Some(t.app_id),
                    file_size: Some(data.len() as u32),
                    raw_file_size: Some(data.len() as u32),
                    file_sha: Some(sha.clone()),
                    time_stamp: Some(mtime(path)),
                    filename: Some(name.clone()),
                    cell_id: Some(t.cell_id),
                    can_encrypt: Some(false),
                    is_shared_file: Some(false),
                    upload_batch_id: batch.batch_id,
                    ..Default::default()
                },
            )
            .await;
        let ok = match begin {
            Ok(b) => send_blocks(&b, data).await,
            Err(e) => Err(e.to_string()),
        };
        let commit: Result<CCloudClientCommitFileUploadResponse, _> = conn
            .call(
                "Cloud.ClientCommitFileUpload#1",
                &CCloudClientCommitFileUploadRequest { transfer_succeeded: Some(ok.is_ok()), appid: Some(t.app_id), file_sha: Some(sha.clone()), filename: Some(name.clone()) },
            )
            .await;
        match (ok, commit) {
            (Ok(()), Ok(c)) if c.file_committed() => done.push((name.clone(), hex(&sha))),
            (Err(e), _) => failed = Some(format!("{name}: {e}")),
            (_, Err(e)) => failed = Some(format!("{name}: {e}")),
            _ => failed = Some(format!("{name}: Steam didn't accept the file")),
        }
    }
    let _ = conn
        .call::<_, CCloudCompleteAppUploadBatchResponse>(
            "Cloud.CompleteAppUploadBatchBlocking#1",
            &CCloudCompleteAppUploadBatchRequest { appid: Some(t.app_id), batch_id: batch.batch_id, batch_eresult: Some(if failed.is_none() { 1 } else { 2 }) },
        )
        .await;
    match failed {
        Some(e) if done.is_empty() => Err(e),
        _ => Ok(done),
    }
}

/// Upload the file body to wherever Steam said, block by block.
async fn send_blocks(b: &CCloudClientBeginFileUploadResponse, data: &[u8]) -> Result<(), String> {
    for block in &b.block_requests {
        let url = format!("{}://{}{}", if block.use_https() { "https" } else { "http" }, block.url_host(), block.url_path());
        // Steam's EHTTPMethod: 3 POST, 4 PUT.
        let mut req = match block.http_method() {
            3 => http().post(url),
            _ => http().put(url),
        };
        for h in &block.request_headers {
            req = req.header(h.name(), h.value());
        }
        let body = if !block.explicit_body_data().is_empty() {
            block.explicit_body_data().to_vec()
        } else {
            let start = block.block_offset() as usize;
            let end = (start + block.block_length() as usize).min(data.len());
            data.get(start..end).ok_or("bad block range")?.to_vec()
        };
        let resp = req.body(body).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("upload answered {}", resp.status()));
        }
    }
    Ok(())
}

fn http() -> &'static reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| reqwest::Client::builder().timeout(Duration::from_secs(120)).build().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_double_slash_names_match() {
        assert_eq!(clean_name("%WinAppDataLocal%KnightsEnd/Saved/SaveGames//Save02.sav"), "%WinAppDataLocal%KnightsEnd/Saved/SaveGames/Save02.sav");
        assert_eq!(clean_name("profile.dat"), "profile.dat");
        assert_eq!(clean_name("a///b"), "a/b");
    }

    #[test]
    fn rule_folders_have_single_slashes() {
        assert_eq!(rule_dir(r"KnightsEnd\Saved\SaveGames\"), "KnightsEnd/Saved/SaveGames");
        assert_eq!(rule_dir("/Saves/"), "Saves");
        assert_eq!(rule_dir(""), "");
    }

    #[test]
    fn decides_like_steam() {
        // Never synced: missing locally -> download; only local -> upload.
        assert_eq!(decide(None, Some("a"), None), Plan::Download);
        assert_eq!(decide(Some("a"), None, None), Plan::Upload);
        // Changed only in the cloud / only here.
        assert_eq!(decide(Some("a"), Some("b"), Some("a")), Plan::Download);
        assert_eq!(decide(Some("b"), Some("a"), Some("a")), Plan::Upload);
        // Changed in both, or no record: ask.
        assert_eq!(decide(Some("b"), Some("c"), Some("a")), Plan::Conflict);
        assert_eq!(decide(Some("b"), Some("c"), None), Plan::Conflict);
        assert_eq!(decide(Some("a"), Some("a"), None), Plan::Same);
    }

    #[test]
    fn matches_patterns_and_paths() {
        assert!(glob("*.sav", "slot1.SAV"));
        assert!(glob("save?.dat", "save3.dat"));
        assert!(!glob("*.sav", "slot1.bak"));
        let cfg = CloudConfig {
            quota: 1000,
            max_files: 10,
            save_files: vec![model::SaveRule { root: "GameInstall".into(), path: "saves".into(), pattern: "*.sav".into(), recursive: false }],
        };
        let t = Target { app_id: 10, steam_id: 76561197960265729, install_dir: PathBuf::from("C:/Games/X"), steam_root: Some(PathBuf::from("C:/Steam")), config: Some(cfg), cell_id: 0 };
        assert_eq!(t.local_path("%GameInstall%saves/a.sav").unwrap(), PathBuf::from("C:/Games/X").join("saves").join("a.sav"));
        assert_eq!(t.local_path("profile.dat").unwrap(), PathBuf::from("C:/Steam/userdata/1/10/remote/profile.dat"));
        assert_eq!(t.expand("{Steam3AccountID}/x"), "1/x");
    }

    #[test]
    fn enforces_save_rules_and_rejects_traversal() {
        let base_target = |config| Target {
            app_id: 10,
            steam_id: 76561197960265729,
            install_dir: PathBuf::from("/tmp/sir-cloud-game"),
            steam_root: Some(PathBuf::from("/tmp/sir-cloud-steam")),
            config,
            cell_id: 0,
        };

        let t_none = base_target(None);
        assert!(t_none.local_path("%GameInstall%version.dll").is_none());
        assert!(t_none.local_path("profile.dat").is_some());
        assert!(t_none.local_path("../escape.dat").is_none());
        assert!(t_none.local_path("").is_none());

        let t_empty = base_target(Some(CloudConfig { quota: 1000, max_files: 10, save_files: vec![] }));
        assert!(t_empty.local_path("%GameInstall%saves/a.sav").is_none());

        let cfg = CloudConfig {
            quota: 1000,
            max_files: 10,
            save_files: vec![
                model::SaveRule { root: "GameInstall".into(), path: "Saves/{Steam3AccountID}".into(), pattern: "*.sav".into(), recursive: false },
                model::SaveRule { root: "GameInstall".into(), path: "Profiles".into(), pattern: "*.cfg".into(), recursive: true },
            ],
        };
        let t_ruled = base_target(Some(cfg));
        assert_eq!(
            t_ruled.local_path("%GameInstall%Saves/1/slot1.sav").unwrap(),
            PathBuf::from("/tmp/sir-cloud-game/Saves/1/slot1.sav")
        );
        assert_eq!(
            t_ruled.local_path("%GameInstall%Saves/{Steam3AccountID}/slot1.sav").unwrap(),
            PathBuf::from("/tmp/sir-cloud-game/Saves/1/slot1.sav")
        );
        assert_eq!(
            t_ruled.local_path("%GameInstall%Profiles/deep/sub/user.cfg").unwrap(),
            PathBuf::from("/tmp/sir-cloud-game/Profiles/deep/sub/user.cfg")
        );
        assert!(t_ruled.local_path("%GameInstall%version.dll").is_none());
        assert!(t_ruled.local_path("%GameInstall%Saves/1/evil.dll").is_none());
        assert!(t_ruled.local_path("%GameInstall%Saves/1/sub/slot1.sav").is_none());
        assert!(t_ruled.local_path("%GameInstall%Saves/2/slot1.sav").is_none());
        assert!(t_ruled.local_path("%GameInstall%Saves/1/../slot1.sav").is_none());
        assert!(t_ruled.local_path("%LinuxHome%.bashrc").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_intermediate_symlinks_under_save_root() {
        let root = std::env::temp_dir().join(format!("sir-cloud-sym-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("game");
        let outside = root.join("outside");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, game.join("saves")).unwrap();

        let t = Target {
            app_id: 10,
            steam_id: 76561197960265729,
            install_dir: game,
            steam_root: Some(root.join("steam")),
            config: Some(CloudConfig {
                quota: 1000,
                max_files: 10,
                save_files: vec![model::SaveRule { root: "GameInstall".into(), path: "saves".into(), pattern: "*.sav".into(), recursive: false }],
            }),
            cell_id: 0,
        };
        assert!(t.local_path("%GameInstall%saves/slot1.sav").is_none());
        assert!(prepare_parent_under_root(&t.install_dir, &["saves".into(), "slot1.sav".into()]).is_err());
        let sym_file = outside.join("real.txt");
        std::fs::write(&sym_file, b"secret").unwrap();
        std::os::unix::fs::symlink(&sym_file, t.install_dir.join("link.sav")).unwrap();
        assert!(backup(&t, "link.sav", &t.install_dir.join("link.sav"), 1).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}

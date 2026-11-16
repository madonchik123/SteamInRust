//! A disk cache for every picture the app shows (store art, library art,
//! avatars, item icons, screenshots).
//!
//! It's an egui bytes loader registered in front of the normal HTTP one, so
//! all existing image code uses it without changes. A picture is downloaded
//! once, then read from disk on later starts (and offline). After a few days
//! it's revalidated in the background (ETag / Last-Modified) while the saved
//! copy keeps showing. Downloads run a few at a time, newest request first
//! (what's on screen now, not what was scrolled past), the folder is capped,
//! and the oldest pictures go first.

use egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Pictures older than this are checked again (in the background).
const FRESH_FOR: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// Cache size limit; trimmed back to 80% of it, oldest first.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Downloads at once. HTTP/2 carries them over a few connections, so this
/// is far below what makes Steam's CDN drop connections.
const PARALLEL: usize = 16;

#[derive(Clone)]
enum Entry {
    Pending,
    Ready(Bytes, Option<String>),
    Failed(String),
}

#[derive(Serialize, Deserialize, Default)]
struct Meta {
    mime: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
}

pub struct ImageCache {
    dir: PathBuf,
    rt: tokio::runtime::Handle,
    http: reqwest::Client,
    slots: Arc<Gate>,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

/// Lets `n` downloads run at once; the others wait, and a freed slot goes to
/// the one that asked most recently. Pictures are asked for as they're drawn,
/// so after scrolling, what's on screen loads before what scrolled past.
struct Gate {
    state: Mutex<(usize, Vec<tokio::sync::oneshot::Sender<()>>)>,
}

struct Slot(Arc<Gate>);

impl Gate {
    fn new(n: usize) -> Arc<Gate> {
        Arc::new(Gate { state: Mutex::new((n, Vec::new())) })
    }

    async fn acquire(self: &Arc<Self>) -> Slot {
        let rx = {
            let mut s = self.state.lock().unwrap();
            if s.0 > 0 {
                s.0 -= 1;
                return Slot(self.clone());
            }
            let (tx, rx) = tokio::sync::oneshot::channel();
            s.1.push(tx);
            rx
        };
        let _ = rx.await;
        Slot(self.clone())
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut s = self.0.state.lock().unwrap();
        // Hand the slot to the newest waiter still there.
        while let Some(tx) = s.1.pop() {
            if tx.send(()).is_ok() {
                return;
            }
        }
        s.0 += 1;
    }
}

/// Where pictures are kept: `<cache dir>/images`.
pub fn dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "SteamInRust", "SteamInRust").map(|d| d.cache_dir().join("images")).unwrap_or_else(|| PathBuf::from("cache/images"))
}

/// Total size of the saved pictures.
pub fn size() -> u64 {
    std::fs::read_dir(dir()).map(|r| r.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
}

/// Delete every saved picture (Settings → Storage).
pub fn clear(ctx: &egui::Context) {
    let _ = std::fs::remove_dir_all(dir());
    ctx.forget_all_images();
}

impl ImageCache {
    pub fn install(ctx: &egui::Context, rt: tokio::runtime::Handle) {
        let dir = dir();
        let _ = std::fs::create_dir_all(&dir);
        let http = reqwest::Client::builder()
            .user_agent(concat!("SteamInRust/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            // Now and then a picture stalls; give up on it sooner and try
            // again rather than hold a download slot for half a minute.
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        // Keep the folder under its limit (in the background, at start).
        let trim_dir = dir.clone();
        rt.spawn_blocking(move || trim(&trim_dir));
        ctx.add_bytes_loader(Arc::new(Self { dir, rt, http, slots: Gate::new(PARALLEL), entries: Default::default() }));
    }

    fn key(uri: &str) -> String {
        Sha256::digest(uri.as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect()
    }
}

fn read_meta(path: &PathBuf) -> Meta {
    std::fs::read_to_string(path.with_extension("json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn age(path: &PathBuf) -> Option<Duration> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok())
}

impl BytesLoader for ImageCache {
    fn id(&self) -> &str {
        egui::generate_loader_id!(ImageCache)
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !(uri.starts_with("https://") || uri.starts_with("http://")) {
            return Err(LoadError::NotSupported);
        }
        if let Some(e) = self.entries.lock().unwrap().get(uri).cloned() {
            return match e {
                Entry::Pending => Ok(BytesPoll::Pending { size: None }),
                Entry::Ready(bytes, mime) => Ok(BytesPoll::Ready { size: None, bytes, mime }),
                Entry::Failed(err) => Err(LoadError::Loading(err)),
            };
        }
        self.entries.lock().unwrap().insert(uri.to_string(), Entry::Pending);
        let (uri, ctx) = (uri.to_string(), ctx.clone());
        let path = self.dir.join(Self::key(&uri));
        let (http, slots, entries) = (self.http.clone(), self.slots.clone(), self.entries.clone());
        self.rt.spawn(async move {
            let done = |e: Entry| {
                entries.lock().unwrap().insert(uri.clone(), e);
                ctx.request_repaint();
            };
            // 1. A saved copy shows right away (even if it's old, or offline).
            let saved = std::fs::read(&path).ok();
            let meta = read_meta(&path);
            if let Some(bytes) = &saved {
                done(Entry::Ready(Bytes::from(bytes.clone()), meta.mime.clone()));
                if age(&path).is_some_and(|a| a < FRESH_FOR) {
                    // Touch-free: fresh enough, nothing else to do.
                    return;
                }
            }
            // 2. Download, or revalidate an old copy.
            let _slot = slots.acquire().await;
            let request = || {
                let mut req = http.get(&uri);
                if saved.is_some() {
                    if let Some(etag) = &meta.etag {
                        req = req.header(reqwest::header::IF_NONE_MATCH, etag);
                    }
                    if let Some(lm) = &meta.last_modified {
                        req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm);
                    }
                }
                req.send()
            };
            // One retry for a stalled or dropped request.
            let sent = match request().await {
                Err(e) if e.is_timeout() || e.is_connect() || e.is_request() => request().await,
                r => r,
            };
            match sent {
                Ok(resp) if resp.status() == reqwest::StatusCode::NOT_MODIFIED => {
                    // Still current: mark it fresh again.
                    if let Ok(f) = std::fs::File::options().append(true).open(&path) {
                        let _ = f.set_modified(SystemTime::now());
                    }
                }
                Ok(resp) if resp.status().is_success() => {
                    let header = |name| resp.headers().get(name).and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok()).map(String::from);
                    let meta = Meta { mime: header(reqwest::header::CONTENT_TYPE), etag: header(reqwest::header::ETAG), last_modified: header(reqwest::header::LAST_MODIFIED) };
                    match resp.bytes().await {
                        Ok(bytes) => {
                            let tmp = path.with_extension("part");
                            if std::fs::write(&tmp, &bytes).is_ok() {
                                let _ = std::fs::rename(&tmp, &path);
                                let _ = std::fs::write(path.with_extension("json"), serde_json::to_string(&meta).unwrap_or_default());
                            }
                            done(Entry::Ready(Bytes::from(bytes.to_vec()), meta.mime));
                        }
                        Err(e) if saved.is_none() => done(Entry::Failed(e.to_string())),
                        Err(_) => {}
                    }
                }
                Ok(resp) if saved.is_none() => done(Entry::Failed(format!("HTTP {}", resp.status()))),
                Err(e) if saved.is_none() => done(Entry::Failed(e.to_string())),
                // Offline or the server hiccuped: keep showing the saved copy.
                _ => {}
            }
        });
        Ok(BytesPoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.entries.lock().unwrap().remove(uri);
    }

    fn forget_all(&self) {
        self.entries.lock().unwrap().clear();
    }

    fn byte_size(&self) -> usize {
        self.entries.lock().unwrap().values().map(|e| if let Entry::Ready(b, _) = e { b.len() } else { 0 }).sum()
    }
}

/// Delete the oldest pictures until the folder is under 80% of the limit.
fn trim(dir: &PathBuf) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter(|e| e.path().extension().is_none())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= MAX_BYTES {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= MAX_BYTES / 10 * 8 {
            break;
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("json"));
        total = total.saturating_sub(len);
    }
}

#[cfg(test)]
mod gate_tests {
    use super::Gate;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn newest_waiter_goes_first() {
        let gate = Gate::new(1);
        let first = gate.acquire().await;
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = Vec::new();
        for i in 0..3 {
            let (gate, order) = (gate.clone(), order.clone());
            tasks.push(tokio::spawn(async move {
                let _slot = gate.acquire().await;
                order.lock().unwrap().push(i);
            }));
            // Let each one start waiting before the next.
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        }
        drop(first);
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec![2, 1, 0]);
    }
}

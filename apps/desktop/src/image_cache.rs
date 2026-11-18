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

/// egui keeps a decoded copy of every picture (4 bytes a pixel) after it's on
/// the graphics card, where it's never used again; that was ~30 MB on the
/// library alone. Drop those copies every few seconds. Only still images:
/// animated GIF and WebP decode their frames from it as they play.
pub fn trim_decoded(ctx: &egui::Context) {
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap();
        if last.is_some_and(|l| l.elapsed() < Duration::from_secs(5)) {
            return;
        }
        *last = Some(std::time::Instant::now());
    }
    for loader in ctx.loaders().image.lock().iter() {
        let ours = loader.id().ends_with("ImageCrateLoader") || loader.id().ends_with("SizedLoader");
        if ours && loader.byte_size() > 4 * 1024 * 1024 && !loader.has_pending() {
            loader.forget_all();
        }
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
        let http = build_http_client();
        // Keep the folder under its limit (in the background, at start).
        let trim_dir = dir.clone();
        rt.spawn_blocking(move || trim(&trim_dir));
        ctx.add_bytes_loader(Arc::new(Self { dir, rt, http, slots: Gate::new(PARALLEL), entries: Default::default() }));
        // Still pictures decoded at the size they're drawn at (sized_images.rs).
        crate::sized_images::SizedLoader::install(ctx);
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
        if let Err(err) = validate_image_url(uri) {
            return Err(LoadError::Loading(err));
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
                Ok(resp) if resp.remote_addr().is_some_and(|a| !is_public_ip(a.ip())) => {
                    if saved.is_none() {
                        done(Entry::Failed("blocked non-public image host".into()));
                    }
                }
                Ok(resp) if resp.status() == reqwest::StatusCode::NOT_MODIFIED => {
                    // Still current: mark it fresh again.
                    if let Ok(f) = std::fs::File::options().append(true).open(&path) {
                        let _ = f.set_modified(SystemTime::now());
                    }
                }
                Ok(resp) if resp.status().is_success() => {
                    let header = |name| resp.headers().get(name).and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok()).map(String::from);
                    let meta = Meta { mime: header(reqwest::header::CONTENT_TYPE), etag: header(reqwest::header::ETAG), last_modified: header(reqwest::header::LAST_MODIFIED) };
                    match read_image_bytes(resp).await {
                        Ok(bytes) => {
                            let tmp = path.with_extension("part");
                            if std::fs::write(&tmp, &bytes).is_ok() {
                                let _ = std::fs::rename(&tmp, &path);
                                let _ = std::fs::write(path.with_extension("json"), serde_json::to_string(&meta).unwrap_or_default());
                            }
                            done(Entry::Ready(Bytes::from(bytes), meta.mime));
                        }
                        Err(e) if saved.is_none() => done(Entry::Failed(e)),
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

const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

fn is_public_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || a == 0
        || (a == 100 && (b & 0b1100_0000) == 64)
        || (a == 198 && (b & 0b1111_1110) == 18)
        || a >= 240)
}

fn is_public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_public_ipv4(v4),
        std::net::IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return false;
            }
            if let Some(v4) = v6.to_ipv4_mapped().or_else(|| v6.to_ipv4()) {
                return is_public_ipv4(v4);
            }
            let segs = v6.segments();
            !((segs[0] & 0xfe00) == 0xfc00
                || (segs[0] & 0xffc0) == 0xfe80
                || (segs[0] == 0x2001 && segs[1] == 0x0db8))
        }
    }
}

fn validate_parsed_image_url(url: &reqwest::Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("unsupported image scheme".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("image URLs cannot carry credentials".into());
    }
    let Some(host) = url.host_str() else {
        return Err("image URL missing host".into());
    };
    let clean = host.trim_end_matches('.');
    let unbracketed = clean.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(clean);
    if let Ok(ip) = unbracketed.parse::<std::net::IpAddr>() {
        if !is_public_ip(ip) {
            return Err("blocked non-public IP address".into());
        }
        return Ok(());
    }
    let lower = unbracketed.to_ascii_lowercase();
    if lower.is_empty()
        || lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
    {
        return Err("blocked local image hostname".into());
    }
    Ok(())
}

fn resolve_public_addrs(host: &str) -> Result<Vec<std::net::SocketAddr>, String> {
    use std::net::ToSocketAddrs;
    let mut out = Vec::new();
    for addr in (host, 0).to_socket_addrs().map_err(|e| e.to_string())? {
        if !is_public_ip(addr.ip()) {
            return Err("blocked non-public DNS resolution".into());
        }
        out.push(addr);
    }
    if out.is_empty() {
        return Err("DNS returned no addresses".into());
    }
    Ok(out)
}

struct PublicDnsResolver;

impl reqwest::dns::Resolve for PublicDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = tokio::task::spawn_blocking(move || resolve_public_addrs(&host))
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
                .map_err(|e| Box::new(std::io::Error::new(std::io::ErrorKind::PermissionDenied, e)) as Box<dyn std::error::Error + Send + Sync>)?;
            let iter: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok(iter)
        })
    }
}

fn build_http_client() -> reqwest::Client {
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many redirects");
        }
        if let Err(err) = validate_parsed_image_url(attempt.url()) {
            return attempt.error(err);
        }
        attempt.follow()
    });
    reqwest::Client::builder()
        .user_agent(concat!("SteamInRust/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        // Now and then a picture stalls; give up on it sooner and try
        // again rather than hold a download slot for half a minute.
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(10))
        .redirect(redirect)
        .dns_resolver(Arc::new(PublicDnsResolver))
        .build()
        .unwrap_or_default()
}

fn validate_image_url(uri: &str) -> Result<reqwest::Url, String> {
    if uri.bytes().any(|b| b < 0x20 || b == 0x7f) || uri.contains('\\') {
        return Err("invalid character in image URL".into());
    }
    let url = reqwest::Url::parse(uri).map_err(|e| e.to_string())?;
    validate_parsed_image_url(&url)?;
    Ok(url)
}

async fn read_image_bytes(mut resp: reqwest::Response) -> Result<Vec<u8>, String> {
    if resp.content_length().is_some_and(|len| len > MAX_IMAGE_BYTES as u64) {
        return Err("image exceeds 16 MiB limit".into());
    }
    let cap = resp.content_length().map(|n| (n as usize).min(MAX_IMAGE_BYTES)).unwrap_or(0);
    let mut buf = Vec::with_capacity(cap);
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if buf.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
            return Err("image exceeds 16 MiB limit".into());
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[cfg(test)]
mod ssrf_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn rejects_ssrf_private_loopback_and_link_local_urls() {
        for bad in [
            "http://127.0.0.1/a.png",
            "https://127.0.1.1:8080/a.png",
            "http://[::1]/a.png",
            "http://[::ffff:127.0.0.1]/a.png",
            "http://localhost/a.png",
            "https://sub.localhost/a.png",
            "http://169.254.169.254/latest/meta-data/",
            "http://[fe80::1]/a.png",
            "http://10.0.0.1/a.png",
            "http://172.16.0.1/a.png",
            "http://192.168.1.1/a.png",
            "http://[fc00::1]/a.png",
            "http://[fd12:3456::1]/a.png",
            "http://0.0.0.0/a.png",
        ] {
            assert!(validate_image_url(bad).is_err(), "must reject SSRF url {bad}");
        }
        assert!(validate_image_url("https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/440/header.jpg").is_ok());
    }

    #[tokio::test]
    async fn blocks_redirect_ssrf_and_caps_streaming_body_at_16_mib() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = sock.read(&mut req);
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 17825792\r\nConnection: close\r\n\r\n");
            let block = vec![0u8; 1024 * 1024];
            for _ in 0..17 {
                if sock.write_all(&block).is_err() {
                    break;
                }
            }
        });
        let raw_client = reqwest::Client::new();
        let resp = raw_client.get(format!("http://{addr}/bomb.png")).send().await.unwrap();
        assert!(read_image_bytes(resp).await.is_err(), "image > 16 MiB must be rejected");
        tokio::task::spawn_blocking(move || srv.join().unwrap()).await.unwrap();

        let chunked = TcpListener::bind("127.0.0.1:0").unwrap();
        let caddr = chunked.local_addr().unwrap();
        let csrv = std::thread::spawn(move || {
            let (mut sock, _) = chunked.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = sock.read(&mut req);
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
            let chunk_hdr = format!("{:x}\r\n", 1024 * 1024);
            let block = vec![b'a'; 1024 * 1024];
            for _ in 0..17 {
                if sock.write_all(chunk_hdr.as_bytes()).is_err()
                    || sock.write_all(&block).is_err()
                    || sock.write_all(b"\r\n").is_err()
                {
                    break;
                }
            }
            let _ = sock.write_all(b"0\r\n\r\n");
        });
        let resp = raw_client.get(format!("http://{caddr}/chunked.png")).send().await.unwrap();
        assert!(read_image_bytes(resp).await.is_err(), "chunked image > 16 MiB must be rejected");
        drop(raw_client);
        tokio::task::spawn_blocking(move || csrv.join().unwrap()).await.unwrap();

        let redir = TcpListener::bind("127.0.0.1:0").unwrap();
        let raddr = redir.local_addr().unwrap();
        let rsrv = std::thread::spawn(move || {
            let (mut sock, _) = redir.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = sock.read(&mut req);
            let _ = sock.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        });
        let cache_http = build_http_client();
        let err = cache_http.get(format!("http://{raddr}/redir")).send().await.unwrap_err();
        assert!(err.is_redirect(), "redirect to link-local IP must be blocked, got {err}");
        tokio::task::spawn_blocking(move || rsrv.join().unwrap()).await.unwrap();

        assert!(validate_image_url("http://2130706433/a.png").is_err());
        assert!(validate_image_url("http://0177.0.0.1/a.png").is_err());
        assert!(resolve_public_addrs("127.0.0.1").is_err());
        assert!(resolve_public_addrs("169.254.169.254").is_err());
    }
}

//! Native trailer playback: no web view, no external player.
//!
//! - Streams HLS (fMP4 segments, what Steam serves for trailers) or plain MP4
//!   (Steam's microtrailers) over HTTP, a few segments ahead.
//! - Decodes H.264 and AAC with Windows Media Foundation (built into Windows).
//! - Plays audio through WASAPI; its clock drives A/V sync (a wall clock when
//!   there's no audio).
//! - Hands frames to egui as a texture; [`widget`] draws a Steam-like player.
//!
//! Threads: a tokio task downloads, one thread decodes video into a short
//! frame queue, one decodes and plays audio. The UI picks the frame for the
//! current clock in [`Player::poll`] and asks egui to repaint when the next
//! one is due, so a paused player costs nothing.

pub mod hls;
pub mod mp4;
pub mod widget;
mod yuv;

#[cfg(windows)]
mod engine;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
mod wasapi;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// What to play.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// HLS master or media playlist.
    Hls(String),
    /// A plain MP4 file.
    Mp4(String),
}

impl Source {
    pub fn url(&self) -> &str {
        match self {
            Source::Hls(u) | Source::Mp4(u) => u,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub autoplay: bool,
    pub muted: bool,
    /// 0..=1
    pub volume: f32,
    pub looping: bool,
    /// Starting quality: the smallest variant at least this wide (pixels).
    pub width: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { autoplay: true, muted: false, volume: 0.7, looping: false, width: 1280 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing to show yet.
    Loading,
    /// Waiting for data mid-playback.
    Buffering,
    Playing,
    Paused,
    Ended,
    Failed,
}

/// One quality of an HLS stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quality {
    pub width: u32,
    pub height: u32,
}

/// How long the UI may go without polling before playback pauses itself
/// (page hidden, window minimized).
const UNSEEN: Duration = Duration::from_millis(500);
/// Download this far ahead of the play position.
const AHEAD: f64 = 30.0;
/// Decoded frames kept ready.
const MAX_FRAMES: usize = 4;

pub(crate) struct Seg {
    pub url: String,
    pub start: f64,
    pub dur: f64,
}

/// The segments of one stream.
pub(crate) struct Plan {
    pub init: Option<String>,
    pub segs: Vec<Seg>,
    /// A whole plain MP4 file as the only segment.
    pub single: bool,
}

impl Plan {
    fn from_media(m: hls::Media) -> Self {
        let mut t = 0.0;
        let segs = m
            .segments
            .into_iter()
            .map(|(url, dur)| {
                let s = Seg { url, start: t, dur };
                t += dur;
                s
            })
            .collect();
        Plan { init: m.init, segs, single: false }
    }
    pub fn index_at(&self, t: f64) -> usize {
        if self.single {
            return 0;
        }
        self.segs.iter().rposition(|s| s.start <= t + 1e-3).unwrap_or(0)
    }
    pub fn init_url(&self) -> &str {
        self.init.as_deref().unwrap_or_else(|| self.segs.first().map(|s| s.url.as_str()).unwrap_or(""))
    }
    fn end(&self) -> f64 {
        self.segs.last().map(|s| s.start + s.dur).unwrap_or(0.0)
    }
}

pub(crate) struct Frame {
    pub pts: f64,
    pub image: egui::ColorImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Audio {
    /// Not known yet whether there's audio.
    Pending,
    /// The audio thread runs the clock.
    On,
    /// No audio: the UI runs a wall clock.
    Off,
}

pub(crate) struct St {
    /// Bumped on every seek / quality switch; workers restart when it changes.
    pub epoch: u64,
    pub target: f64,
    /// Playlists loaded.
    pub ready: bool,
    pub video: Option<Arc<Plan>>,
    pub audio_plan: Option<Arc<Plan>>,
    pub duration: f64,
    pub variants: Vec<hls::Variant>,
    pub variant: usize,
    /// None = pick by `width`.
    pub manual: Option<usize>,
    pub width: u32,
    pub cache: HashMap<String, Arc<Vec<u8>>>,

    pub frames: VecDeque<Frame>,
    /// A frame was decoded since the last restart.
    pub has_frame: bool,
    pub video_eos: bool,

    pub audio: Audio,
    pub audio_eos: bool,

    /// Clock: `clock` seconds at `clock_at`, advancing if `running`.
    pub clock: f64,
    pub clock_at: Instant,
    pub running: bool,

    pub paused: bool,
    pub auto_paused: bool,
    pub last_poll: Instant,
    pub volume: f32,
    pub muted: bool,
    pub looping: bool,
    pub error: Option<String>,
}

impl St {
    /// Start over from `t`: workers drop what they have and seek.
    pub fn restart(&mut self, t: f64) {
        self.epoch += 1;
        self.target = t.max(0.0);
        if self.duration > 0.0 {
            self.target = self.target.min((self.duration - 0.05).max(0.0));
        }
        self.frames.clear();
        self.has_frame = false;
        self.video_eos = false;
        self.audio_eos = false;
        self.clock = self.target;
        self.clock_at = Instant::now();
        self.running = false;
    }
}

pub(crate) struct Shared {
    pub m: Mutex<St>,
    pub cv: Condvar,
    pub stop: AtomicBool,
    pub epoch: AtomicU64,
    pub ctx: egui::Context,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, St> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Relaxed)
    }
    /// Wake workers and the UI after a state change.
    pub fn notify(&self) {
        self.cv.notify_all();
    }
    pub fn repaint(&self) {
        self.ctx.request_repaint();
    }
    pub fn wait<'a>(&self, g: MutexGuard<'a, St>, ms: u64) -> MutexGuard<'a, St> {
        self.cv.wait_timeout(g, Duration::from_millis(ms)).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0)
    }
    /// Downloaded bytes of `url`, waiting for them. None if stopped, or if
    /// `epoch` is given and playback restarted meanwhile.
    pub fn wait_data(&self, url: &str, epoch: Option<u64>) -> Option<Arc<Vec<u8>>> {
        let mut st = self.lock();
        loop {
            if self.stopped() || epoch.is_some_and(|g| g != st.epoch) {
                return None;
            }
            if let Some(d) = st.cache.get(url) {
                return Some(d.clone());
            }
            st = self.wait(st, 100);
        }
    }
    pub fn fail(&self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::warn!("video: {msg}");
        self.lock().error.get_or_insert(msg);
        self.notify();
        self.repaint();
    }
}

/// A playing (or paused) video. Dropping it stops everything.
pub struct Player {
    sh: Arc<Shared>,
    tex: Option<egui::TextureHandle>,
    /// Generation of the frame in `tex`.
    shown_epoch: u64,
    shown_pts: f64,
    pos: f64,
    status: Status,
    ended_flag: bool,
    /// Full screen requested (the widget shows it).
    pub fullscreen: bool,
    pub(crate) ui: widget::UiState,
}

impl Player {
    pub fn new(rt: &tokio::runtime::Handle, ctx: &egui::Context, source: Source, opts: Options) -> Self {
        let now = Instant::now();
        let st = St {
            epoch: 0,
            target: 0.0,
            ready: false,
            video: None,
            audio_plan: None,
            duration: 0.0,
            variants: Vec::new(),
            variant: 0,
            manual: None,
            width: opts.width,
            cache: HashMap::new(),
            frames: VecDeque::new(),
            has_frame: false,
            video_eos: false,
            audio: Audio::Pending,
            audio_eos: false,
            clock: 0.0,
            clock_at: now,
            running: false,
            paused: !opts.autoplay,
            auto_paused: false,
            last_poll: now,
            volume: opts.volume.clamp(0.0, 1.0),
            muted: opts.muted,
            looping: opts.looping,
            error: None,
        };
        let sh = Arc::new(Shared { m: Mutex::new(st), cv: Condvar::new(), stop: AtomicBool::new(false), epoch: AtomicU64::new(0), ctx: ctx.clone() });
        rt.spawn(download(sh.clone(), source));
        #[cfg(windows)]
        {
            let (a, b) = (sh.clone(), sh.clone());
            let ok = std::thread::Builder::new().name("video-decode".into()).spawn(move || engine::video_thread(a)).is_ok()
                && std::thread::Builder::new().name("video-audio".into()).spawn(move || engine::audio_thread(b)).is_ok();
            if !ok {
                sh.fail("could not start decoder threads");
            }
        }
        #[cfg(not(windows))]
        sh.fail("video playback needs Windows Media Foundation");
        Self { sh, tex: None, shown_epoch: u64::MAX, shown_pts: 0.0, pos: 0.0, status: Status::Loading, ended_flag: false, fullscreen: false, ui: Default::default() }
    }

    /// Advance playback to now: picks the frame for the clock, uploads it and
    /// schedules the next repaint. Call once per frame while visible.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut st = self.sh.lock();
        let away = now.duration_since(st.last_poll) > UNSEEN;
        st.last_poll = now;
        st.auto_paused = false;
        let want = !st.paused;

        // The clock.
        let mut clock = st.clock;
        match st.audio {
            Audio::On => {
                if st.running {
                    clock += now.duration_since(st.clock_at).as_secs_f64().min(0.06);
                }
            }
            Audio::Off => {
                if st.running {
                    if !away {
                        clock += now.duration_since(st.clock_at).as_secs_f64();
                    }
                    let starved = st.frames.is_empty() && !st.video_eos;
                    st.clock = clock;
                    st.clock_at = now;
                    if !want || starved {
                        st.running = false;
                    }
                } else if want && st.ready && (st.has_frame || st.video_eos) && !st.frames.is_empty() {
                    st.running = true;
                    st.clock_at = now;
                }
            }
            Audio::Pending => {}
        }
        if st.duration > 0.0 {
            clock = clock.min(st.duration);
        }

        // The frame for this moment: the newest one that is due. Right after a
        // restart show the first one at once (a preview while paused/buffering).
        let mut pick = None;
        let fresh = self.shown_epoch != st.epoch;
        while let Some(f) = st.frames.front() {
            if fresh && pick.is_none() || f.pts <= clock + 0.004 {
                pick = st.frames.pop_front();
                if fresh {
                    break;
                }
            } else {
                break;
            }
        }
        if pick.is_some() {
            self.sh.notify();
        }
        let epoch = st.epoch;
        let next_due = st.frames.front().map(|f| f.pts - clock);

        let ended = st.ready && st.audio != Audio::Pending && st.video_eos && st.frames.is_empty() && (st.audio == Audio::Off || st.audio_eos);
        if ended && st.looping && st.error.is_none() {
            st.restart(0.0);
            self.sh.epoch.store(st.epoch, Ordering::Relaxed);
            self.sh.notify();
        } else if ended && !self.ended_flag && self.status != Status::Ended {
            tracing::debug!("video: ended at {clock:.2}");
            self.ended_flag = true;
        }
        self.pos = if ended { st.duration.max(clock) } else { clock };
        let stuck = !st.running && st.frames.is_empty() && !ended;
        self.status = if st.error.is_some() && (!st.has_frame || stuck) {
            Status::Failed
        } else if ended && !st.looping {
            Status::Ended
        } else if self.tex.is_none() {
            Status::Loading
        } else if st.paused {
            Status::Paused
        } else if !st.running && st.error.is_none() {
            Status::Buffering
        } else {
            Status::Playing
        };
        drop(st);

        if let Some(f) = pick {
            self.shown_epoch = epoch;
            self.shown_pts = f.pts;
            match &mut self.tex {
                Some(t) => t.set(f.image, egui::TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("video-frame", f.image, egui::TextureOptions::LINEAR)),
            }
        }

        let after = match self.status {
            Status::Playing => Some(next_due.unwrap_or(0.01).clamp(0.004, 0.05)),
            Status::Loading | Status::Buffering => Some(0.1),
            _ => None,
        };
        if let Some(s) = after {
            let d = Duration::from_secs_f64(s);
            // The main window's pass also paints the full-screen window.
            ctx.request_repaint_after_for(d, egui::ViewportId::ROOT);
        }
    }

    /// The current picture and its size in pixels.
    pub fn texture(&self) -> Option<(egui::TextureId, egui::Vec2)> {
        self.tex.as_ref().map(|t| (t.id(), t.size_vec2()))
    }

    pub fn status(&self) -> Status {
        self.status
    }
    /// Seconds into the video.
    pub fn position(&self) -> f64 {
        self.pos
    }
    pub fn duration(&self) -> f64 {
        self.sh.lock().duration
    }
    /// False once it's known there's no sound (or no audio device).
    pub fn has_audio(&self) -> bool {
        self.sh.lock().audio != Audio::Off
    }
    pub fn error(&self) -> Option<String> {
        self.sh.lock().error.clone()
    }

    /// End of the downloaded stretch from the play position, in seconds.
    pub fn buffered(&self) -> f64 {
        let st = self.sh.lock();
        let Some(v) = st.video.clone() else { return 0.0 };
        if v.single {
            return if st.cache.contains_key(&v.segs[0].url) { st.duration } else { 0.0 };
        }
        let mut end = self.pos;
        let mut i = v.index_at(self.pos);
        while let Some(s) = v.segs.get(i) {
            let have_audio = st.audio_plan.as_ref().is_none_or(|a| a.segs.get(a.index_at(s.start + 0.01)).is_none_or(|x| st.cache.contains_key(&x.url)));
            if !st.cache.contains_key(&s.url) || !have_audio {
                break;
            }
            end = s.start + s.dur;
            i += 1;
        }
        end
    }

    /// True once, when playback reached the end (not when looping).
    pub fn take_ended(&mut self) -> bool {
        std::mem::take(&mut self.ended_flag)
    }

    pub fn is_paused(&self) -> bool {
        self.sh.lock().paused
    }
    pub fn play(&mut self) {
        tracing::debug!("video: play ({:?})", self.status);
        let mut st = self.sh.lock();
        if self.status == Status::Ended {
            st.restart(0.0);
            self.sh.epoch.store(st.epoch, Ordering::Relaxed);
        }
        st.paused = false;
        drop(st);
        self.sh.notify();
        self.sh.repaint();
    }
    pub fn pause(&mut self) {
        tracing::debug!("video: pause");
        self.sh.lock().paused = true;
        self.sh.notify();
        self.sh.repaint();
    }
    pub fn toggle(&mut self) {
        if self.status == Status::Ended || self.is_paused() { self.play() } else { self.pause() }
    }
    pub fn seek(&mut self, t: f64) {
        tracing::debug!("video: seek to {t:.2}");
        let mut st = self.sh.lock();
        st.restart(t);
        self.sh.epoch.store(st.epoch, Ordering::Relaxed);
        drop(st);
        self.ended_flag = false;
        self.pos = t;
        self.sh.notify();
        self.sh.repaint();
    }
    pub fn volume(&self) -> f32 {
        self.sh.lock().volume
    }
    pub fn muted(&self) -> bool {
        self.sh.lock().muted
    }
    pub fn set_volume(&mut self, v: f32) {
        self.sh.lock().volume = v.clamp(0.0, 1.0);
    }
    pub fn set_muted(&mut self, m: bool) {
        self.sh.lock().muted = m;
    }
    pub fn set_looping(&mut self, l: bool) {
        self.sh.lock().looping = l;
    }

    /// Available qualities, smallest first (empty for plain MP4).
    pub fn qualities(&self) -> Vec<Quality> {
        self.sh.lock().variants.iter().map(|v| Quality { width: v.width, height: v.height }).collect()
    }
    /// (current variant index, chosen manually?)
    pub fn quality(&self) -> (usize, bool) {
        let st = self.sh.lock();
        (st.variant, st.manual.is_some())
    }
    /// Pick a quality, or None for automatic (by the displayed width).
    pub fn set_quality(&mut self, q: Option<usize>) {
        self.sh.lock().manual = q;
    }
    /// The width the video is shown at, in physical pixels (automatic quality).
    pub fn set_display_width(&mut self, px: u32) {
        let mut st = self.sh.lock();
        if st.width != px {
            st.width = px;
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.sh.stop.store(true, Ordering::Relaxed);
        self.sh.notify();
    }
}

/// The smallest variant at least `width` wide (the largest if none is).
fn pick_variant(vs: &[hls::Variant], width: u32) -> usize {
    vs.iter().position(|v| v.width >= width.saturating_sub(16)).unwrap_or(vs.len().saturating_sub(1))
}

// ---------------------------------------------------------------- download

/// Byte-range part size: big segments come in parallel parts, which is
/// much faster when the CDN or a proxy limits each connection.
const PART: u64 = 384 * 1024;
/// Segments downloaded at the same time.
const PARALLEL: usize = 3;

/// One request (optionally a byte range), retried. Returns the body and the
/// full size when the server answered with a range.
async fn fetch(http: &reqwest::Client, sh: &Shared, url: &str, range: Option<(u64, u64)>) -> Result<(Vec<u8>, Option<u64>), String> {
    let mut last = String::new();
    for attempt in 0..4u32 {
        if sh.stopped() {
            return Err("stopped".into());
        }
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400 << attempt)).await;
        }
        let mut req = http.get(url);
        if let Some((a, b)) = range {
            req = req.header(reqwest::header::RANGE, format!("bytes={a}-{b}"));
        }
        match req.send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => {
                let total = if r.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                    r.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit('/').next()).and_then(|t| t.parse().ok())
                } else {
                    None
                };
                match r.bytes().await {
                    Ok(b) => return Ok((b.to_vec(), total)),
                    Err(e) => last = e.to_string(),
                }
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// A whole file: the first part tells the size, the rest come in parallel.
async fn get(http: reqwest::Client, sh: Arc<Shared>, url: String) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let (mut body, total) = fetch(&http, &sh, &url, Some((0, PART - 1))).await?;
    if let Some(total) = total.filter(|t| *t > PART) {
        let mut parts = Vec::new();
        let mut a = PART;
        while a < total {
            let b = (a + PART).min(total) - 1;
            let (h, s, u) = (http.clone(), sh.clone(), url.clone());
            parts.push(tokio::spawn(async move { fetch(&h, &s, &u, Some((a, b))).await.map(|r| r.0) }));
            a = b + 1;
        }
        body.reserve(total as usize - body.len());
        for p in parts {
            body.extend(p.await.map_err(|e| e.to_string())??);
        }
    }
    tracing::debug!("video: {} bytes in {:?}: {url}", body.len(), started.elapsed());
    Ok(body)
}

async fn text(http: &reqwest::Client, sh: &Arc<Shared>, url: &str) -> Result<String, String> {
    get(http.clone(), sh.clone(), url.to_string()).await.map(|b| String::from_utf8_lossy(&b).into_owned())
}

async fn media_plan(http: &reqwest::Client, sh: &Arc<Shared>, url: &str) -> Result<Plan, String> {
    Ok(Plan::from_media(hls::parse_media(&text(http, sh, url).await?, url)))
}

/// Load the playlists, then keep the next stretch of segments downloaded.
async fn download(sh: Arc<Shared>, source: Source) {
    // One client for all players: later trailers reuse warm connections.
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let http = HTTP.get_or_init(|| reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(15)).build().unwrap_or_default()).clone();
    if let Err(e) = load(&http, &sh, &source).await {
        if !sh.stopped() {
            sh.fail(e);
        }
        return;
    }
    let mut inflight = tokio::task::JoinSet::new();
    let mut busy: HashMap<String, tokio::task::AbortHandle> = HashMap::new();
    while !sh.stopped() {
        // Quality switch.
        let switch = {
            let st = sh.lock();
            // Automatic quality only goes up (e.g. for full screen): going
            // back down would rebuffer for nothing.
            let want = st.manual.unwrap_or_else(|| pick_variant(&st.variants, st.width).max(st.variant)).min(st.variants.len().saturating_sub(1));
            (!st.variants.is_empty() && want != st.variant).then(|| (want, st.variants[want].url.clone()))
        };
        if let Some((i, url)) = switch {
            match media_plan(&http, &sh, &url).await {
                Ok(plan) => {
                    let mut st = sh.lock();
                    let old: Vec<String> = st.video.iter().flat_map(|p| p.segs.iter().map(|s| s.url.clone()).chain(p.init.clone())).collect();
                    for u in old {
                        st.cache.remove(&u);
                    }
                    st.video = Some(Arc::new(plan));
                    st.variant = i;
                    let at = st.clock;
                    st.restart(at);
                    sh.epoch.store(st.epoch, Ordering::Relaxed);
                    drop(st);
                    sh.notify();
                }
                Err(e) => tracing::warn!("video: quality switch: {e}"),
            }
            continue;
        }

        // What to fetch next, and what to keep.
        let (todo, keep) = {
            let st = sh.lock();
            let pos = st.clock.max(st.target.min(st.clock + AHEAD));
            let mut todo: Vec<(f64, String)> = Vec::new();
            let mut keep: HashSet<String> = HashSet::new();
            for plan in [&st.video, &st.audio_plan].into_iter().flatten() {
                if let Some(i) = &plan.init {
                    keep.insert(i.clone());
                    if !st.cache.contains_key(i) {
                        todo.push((-1.0, i.clone()));
                    }
                }
                let i0 = plan.index_at(pos);
                if let Some(prev) = i0.checked_sub(1).and_then(|i| plan.segs.get(i)) {
                    keep.insert(prev.url.clone());
                }
                for s in plan.segs.iter().skip(i0) {
                    if s.start > pos + AHEAD && keep.len() > 3 {
                        break;
                    }
                    keep.insert(s.url.clone());
                    if !st.cache.contains_key(&s.url) {
                        todo.push((s.start, s.url.clone()));
                    }
                }
            }
            todo.sort_by(|a, b| a.0.total_cmp(&b.0));
            todo.dedup_by(|a, b| a.1 == b.1);
            (todo, keep)
        };
        sh.lock().cache.retain(|k, _| keep.contains(k));
        // Drop downloads nobody needs any more (after a seek), start new ones.
        busy.retain(|u, h| {
            let k = keep.contains(u);
            if !k {
                h.abort();
            }
            k
        });
        let free = PARALLEL.saturating_sub(busy.len());
        let start: Vec<String> = todo.into_iter().map(|(_, u)| u).filter(|u| !busy.contains_key(u)).take(free).collect();
        for url in start {
            let (h, s, u) = (http.clone(), sh.clone(), url.clone());
            let handle = inflight.spawn(async move {
                let r = get(h, s, u.clone()).await;
                (u, r)
            });
            busy.insert(url, handle);
        }
        if inflight.is_empty() {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        let Ok(Some(Ok((url, r)))) = tokio::time::timeout(Duration::from_millis(250), inflight.join_next()).await else { continue };
        busy.remove(&url);
        let mut st = sh.lock();
        match r {
            Ok(bytes) => {
                st.cache.insert(url, Arc::new(bytes));
            }
            Err(e) if !sh.stopped() => {
                st.error.get_or_insert(format!("download failed: {e}"));
            }
            Err(_) => {}
        }
        let failed = st.error.is_some();
        drop(st);
        sh.notify();
        sh.repaint();
        if failed {
            tracing::warn!("video: giving up on downloads");
            return;
        }
    }
}

async fn load(http: &reqwest::Client, sh: &Arc<Shared>, source: &Source) -> Result<(), String> {
    let (video, audio, variants, variant) = match source {
        Source::Mp4(url) => {
            let plan = Arc::new(Plan { init: None, segs: vec![Seg { url: url.clone(), start: 0.0, dur: 0.0 }], single: true });
            (plan.clone(), Some(plan), Vec::new(), 0)
        }
        Source::Hls(url) => {
            let body = text(http, sh, url).await?;
            if hls::is_master(&body) {
                let variants = hls::parse_master(&body, url);
                if variants.is_empty() {
                    return Err("empty playlist".into());
                }
                let i = {
                    let st = sh.lock();
                    st.manual.unwrap_or_else(|| pick_variant(&variants, st.width)).min(variants.len() - 1)
                };
                let (video, audio) = tokio::join!(media_plan(http, sh, &variants[i].url), async {
                    match &variants[i].audio {
                        Some(a) => Some(media_plan(http, sh, a).await),
                        None => None,
                    }
                });
                let video = video?;
                let audio = audio.transpose()?.map(Arc::new);
                let video = Arc::new(video);
                let audio = audio.or_else(|| Some(video.clone()));
                (video, audio, variants, i)
            } else {
                let plan = Arc::new(Plan::from_media(hls::parse_media(&body, url)));
                (plan.clone(), Some(plan), Vec::new(), 0)
            }
        }
    };
    let mut st = sh.lock();
    if !video.single {
        st.duration = video.end();
    }
    st.video = Some(video);
    st.audio_plan = audio;
    st.variants = variants;
    st.variant = variant;
    st.ready = true;
    drop(st);
    sh.notify();
    Ok(())
}

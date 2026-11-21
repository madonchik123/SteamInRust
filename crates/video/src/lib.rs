//! Native video playback: no web view, no external player.
//!
//! - Streams HLS (fMP4 segments, what Steam serves for trailers), live HLS
//!   (Steam broadcasts) or plain MP4 (Steam's microtrailers) over HTTP, a few
//!   segments ahead.
//! - Decodes H.264 and AAC with what the system has built in: Media
//!   Foundation on Windows, VideoToolbox and AudioToolbox on macOS.
//! - Plays audio through WASAPI (Windows) or the default output audio unit
//!   (macOS); its clock drives A/V sync (a wall clock when there's no audio).
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

#[cfg(any(windows, target_os = "macos"))]
mod engine;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
mod wasapi;
#[cfg(target_os = "macos")]
mod apple;

// The decoders and the sound output, whichever this system has.
#[cfg(windows)]
use {mf as codec, wasapi as sound};
#[cfg(target_os = "macos")]
use {apple as codec, apple as sound};

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
    /// A live HLS stream's master playlist (a Steam broadcast): its segment
    /// list moves forward and playback starts near the newest segment.
    Live(String),
}

impl Source {
    pub fn url(&self) -> &str {
        match self {
            Source::Hls(u) | Source::Mp4(u) | Source::Live(u) => u,
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
/// Live: start this many segments behind the newest, so a late segment
/// doesn't stall playback (Steam's own player sits about as far back).
const LIVE_BACK: usize = 3;
/// Live: download this far ahead at most (keeps memory small).
const LIVE_AHEAD: f64 = 10.0;
/// Live: this many failed playlist or segment loads in a row and the stream
/// counts as lost (the page then asks Steam what happened to it).
const LIVE_FAILS: u32 = 4;
/// Live: "at the live edge" within this many seconds of the joining point
/// (Steam's lists move in steps of a few seconds).
const LIVE_SLACK: f64 = 6.0;
/// Live: wait this long before asking for a segment that wasn't there.
const LIVE_RETRY: Duration = Duration::from_millis(700);
/// Live: give up on one request after this long (a hung connection would
/// otherwise freeze the picture until the normal read timeout).
const LIVE_TIMEOUT: Duration = Duration::from_secs(4);
/// Live: fallen this far behind without asking to (after a stall), jump
/// back to the live picture.
const LIVE_CATCH_UP: f64 = 8.0;
/// Live: this many quick video segments in a row before trying a higher
/// quality again.
const LIVE_QUICK: u32 = 20;

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
    /// The number of `segs[0]` (0 unless it's a live stream's moving list).
    pub first: u64,
    /// A live stream that is still going: more segments will come.
    pub live: bool,
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
        Plan { init: m.init, segs, single: false, first: 0, live: false }
    }
    /// A live playlist on the stream's own clock: segment `anchor.0` starts at
    /// `anchor.1` seconds, the others follow from their durations.
    fn from_live(m: hls::Media, anchor: (u64, f64)) -> Self {
        let nominal = if m.target > 0.0 { m.target } else { m.segments.first().map_or(2.0, |s| s.1) };
        let first = m.sequence;
        let mut t = match anchor.0.checked_sub(first) {
            // The anchor is in this list: count back with the real durations.
            Some(back) if (back as usize) < m.segments.len() => anchor.1 - m.segments[..back as usize].iter().map(|s| s.1).sum::<f64>(),
            Some(back) => anchor.1 - back as f64 * nominal,
            None => anchor.1 + (first - anchor.0) as f64 * nominal,
        };
        let live = !m.ended;
        let segs = m
            .segments
            .into_iter()
            .map(|(url, dur)| {
                let s = Seg { url, start: t, dur };
                t += dur;
                s
            })
            .collect();
        Plan { init: m.init, segs, single: false, first, live }
    }
    /// Where playback joins a live stream: a few segments behind the newest.
    fn live_point(&self) -> f64 {
        self.segs.len().checked_sub(LIVE_BACK + 1).and_then(|i| self.segs.get(i)).or(self.segs.first()).map_or(0.0, |s| s.start)
    }
    /// The number of the segment playing at `t`.
    pub fn number_at(&self, t: f64) -> u64 {
        self.first + self.index_at(t) as u64
    }
    /// The segment with this number, if it's in the list.
    pub fn seg(&self, number: u64) -> Option<&Seg> {
        self.segs.get(usize::try_from(number.checked_sub(self.first)?).ok()?)
    }
    /// The number after the last segment.
    pub fn end_number(&self) -> u64 {
        self.first + if self.single { 1 } else { self.segs.len() as u64 }
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
    /// Playing a live stream.
    pub live: bool,
    /// Live: too many loads failed; the stream may be over.
    pub live_lost: bool,
    /// Live: the viewer paused or moved back, so being behind is on purpose.
    pub behind_on_purpose: bool,
    /// Live: don't pick a quality above this one (the connection was too
    /// slow for it).
    pub cap: Option<usize>,
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
            live: matches!(source, Source::Live(_)),
            live_lost: false,
            behind_on_purpose: false,
            cap: None,
        };
        let sh = Arc::new(Shared { m: Mutex::new(st), cv: Condvar::new(), stop: AtomicBool::new(false), epoch: AtomicU64::new(0), ctx: ctx.clone() });
        rt.spawn(download(sh.clone(), source));
        #[cfg(any(windows, target_os = "macos"))]
        {
            let (a, b) = (sh.clone(), sh.clone());
            let ok = std::thread::Builder::new().name("video-decode".into()).spawn(move || engine::video_thread(a)).is_ok()
                && std::thread::Builder::new().name("video-audio".into()).spawn(move || engine::audio_thread(b)).is_ok();
            if !ok {
                sh.fail("could not start decoder threads");
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        sh.fail("video playback isn't available on this system yet");
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
                // Same size: write into the picture already on the graphics
                // card instead of replacing it whole every frame (the OpenGL
                // driver then doesn't set up new storage 30 times a second).
                Some(t) if t.size() == f.image.size => t.set_partial([0, 0], f.image, egui::TextureOptions::LINEAR),
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
        if self.status == Status::Ended && !st.live {
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
        let mut st = self.sh.lock();
        st.paused = true;
        st.behind_on_purpose = true;
        drop(st);
        self.sh.notify();
        self.sh.repaint();
    }
    pub fn toggle(&mut self) {
        if self.status == Status::Ended || self.is_paused() { self.play() } else { self.pause() }
    }
    pub fn seek(&mut self, t: f64) {
        tracing::debug!("video: seek to {t:.2}");
        let mut st = self.sh.lock();
        // A live stream can only go back as far as Steam still keeps it.
        let t = match st.video.as_ref().filter(|p| st.live && !p.segs.is_empty()) {
            Some(p) => {
                let t = t.clamp(p.segs[0].start + 0.5, p.live_point().max(p.segs[0].start + 0.5));
                st.behind_on_purpose = p.live_point() - t >= LIVE_SLACK;
                t
            }
            None => t,
        };
        st.restart(t);
        self.sh.epoch.store(st.epoch, Ordering::Relaxed);
        drop(st);
        self.ended_flag = false;
        self.pos = t;
        self.sh.notify();
        self.sh.repaint();
    }

    /// Playing a live stream (a broadcast).
    pub fn is_live(&self) -> bool {
        self.sh.lock().live
    }
    /// Live: seconds behind the point where playback joins the stream
    /// (0 = watching live).
    pub fn live_delay(&self) -> f64 {
        let st = self.sh.lock();
        st.video.as_ref().filter(|_| st.live).map_or(0.0, |p| (p.live_point() - self.pos).max(0.0))
    }
    /// Live: playing, and close enough to the newest video to call it live.
    pub fn at_live_edge(&self) -> bool {
        !self.is_paused() && self.live_delay() < LIVE_SLACK
    }
    /// Live: jump to the newest video and keep playing.
    pub fn go_live(&mut self) {
        let edge = {
            let st = self.sh.lock();
            st.video.as_ref().filter(|_| st.live).map(|p| p.live_point())
        };
        if let Some(edge) = edge {
            self.seek(edge);
            self.play();
        }
    }
    /// Live: the stream stopped loading. Ask Steam whether it ended.
    pub fn live_lost(&self) -> bool {
        self.sh.lock().live_lost
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
/// Maximum size for a single downloaded segment or playlist/MP4 asset (256 MiB).
const MAX_MEDIA_BYTES: usize = 256 * 1024 * 1024;
/// Maximum concurrent byte-range requests across a single `get()`.
const MAX_RANGE_CONCURRENCY: usize = 6;

/// One request (optionally a byte range), retried. Returns the body and the
/// full size when the server answered with a range.
async fn fetch(http: &reqwest::Client, sh: &Shared, url: &str, range: Option<(u64, u64)>) -> Result<(Vec<u8>, Option<u64>), String> {
    let mut last = String::new();
    // A live segment is only useful for a moment: don't keep retrying it.
    let tries = if sh.lock().live { 2 } else { 4u32 };
    for attempt in 0..tries {
        if sh.stopped() {
            return Err("stopped".into());
        }
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400 << attempt)).await;
        }
        let mut req = http.get(url);
        if tries == 2 {
            req = req.timeout(LIVE_TIMEOUT);
        }
        if let Some((a, b)) = range {
            req = req.header(reqwest::header::RANGE, format!("bytes={a}-{b}"));
        }
        match req.send().await.and_then(|r| r.error_for_status()) {
            Ok(mut r) => {
                if r.content_length().is_some_and(|n| n > MAX_MEDIA_BYTES as u64) {
                    return Err("video response exceeds 256 MiB limit".into());
                }
                let total = if r.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                    r.headers()
                        .get(reqwest::header::CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.rsplit('/').next())
                        .and_then(|t| t.parse::<u64>().ok())
                } else {
                    None
                };
                if total.is_some_and(|t| t > MAX_MEDIA_BYTES as u64) {
                    return Err("Content-Range total exceeds 256 MiB limit".into());
                }
                let mut body = Vec::new();
                loop {
                    match r.chunk().await {
                        Ok(Some(chunk)) => {
                            if body.len().saturating_add(chunk.len()) > MAX_MEDIA_BYTES {
                                return Err("video response exceeds 256 MiB limit".into());
                            }
                            body.extend_from_slice(&chunk);
                        }
                        Ok(None) => return Ok((body, total)),
                        Err(e) => {
                            last = e.to_string();
                            break;
                        }
                    }
                }
            }
            // Gone for good (a live segment that aged out, a broadcast that ended).
            Err(e) if tries == 2 && e.status().is_some_and(|c| c == reqwest::StatusCode::NOT_FOUND || c == reqwest::StatusCode::GONE) => return Err(e.to_string()),
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
        if total > MAX_MEDIA_BYTES as u64 {
            return Err("Content-Range total exceeds 256 MiB limit".into());
        }
        let sem = Arc::new(tokio::sync::Semaphore::new(MAX_RANGE_CONCURRENCY));
        let mut parts = Vec::new();
        let mut a = PART;
        while a < total {
            let b = (a + PART).min(total) - 1;
            let (h, s, u, sem) = (http.clone(), sh.clone(), url.clone(), sem.clone());
            parts.push(tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.map_err(|e| e.to_string())?;
                fetch(&h, &s, &u, Some((a, b))).await.map(|r| r.0)
            }));
            a = b + 1;
        }
        body.reserve((total as usize).saturating_sub(body.len()).min(MAX_MEDIA_BYTES));
        for p in parts {
            let chunk = p.await.map_err(|e| e.to_string())??;
            if body.len().saturating_add(chunk.len()) > MAX_MEDIA_BYTES {
                return Err("video response exceeds 256 MiB limit".into());
            }
            body.extend(chunk);
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
    let mut feed = match load(&http, &sh, &source).await {
        Ok(f) => f,
        Err(e) => {
            if !sh.stopped() {
                sh.fail(e);
            }
            return;
        }
    };
    let ahead = if feed.is_some() { LIVE_AHEAD } else { AHEAD };
    let mut inflight = tokio::task::JoinSet::new();
    let mut busy: HashMap<String, (tokio::task::AbortHandle, Instant)> = HashMap::new();
    // Live: segments that just failed, and when to try them again.
    let mut cooling: HashMap<String, Instant> = HashMap::new();
    while !sh.stopped() {
        if let Some(f) = feed.as_mut() {
            f.tick(&http, &sh).await;
        }
        // Quality switch.
        let switch = {
            let st = sh.lock();
            // Automatic quality only goes up (e.g. for full screen): going
            // back down would rebuffer for nothing.
            let auto = pick_variant(&st.variants, st.width).max(st.variant).min(st.cap.unwrap_or(usize::MAX));
            let want = st.manual.unwrap_or(auto).min(st.variants.len().saturating_sub(1));
            (!st.variants.is_empty() && want != st.variant).then(|| (want, st.variants[want].url.clone()))
        };
        if let Some((i, url)) = switch {
            let plan = match feed.as_mut() {
                Some(f) => f.switch(&http, &sh, url).await,
                None => media_plan(&http, &sh, &url).await,
            };
            match plan {
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
            let pos = st.clock.max(st.target.min(st.clock + ahead));
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
                    if s.start > pos + ahead && keep.len() > 3 {
                        break;
                    }
                    keep.insert(s.url.clone());
                    // Live: the newest listed segment usually isn't on the
                    // CDN yet. Leave it until playback gets close to it.
                    let too_new = plan.live && plan.segs.last().is_some_and(|l| std::ptr::eq(l, s)) && s.start > pos + 1.0;
                    if !too_new && !st.cache.contains_key(&s.url) {
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
        busy.retain(|u, (h, _)| {
            let k = keep.contains(u);
            if !k {
                h.abort();
            }
            k
        });
        let free = PARALLEL.saturating_sub(busy.len());
        let now = Instant::now();
        cooling.retain(|u, at| *at > now && keep.contains(u));
        let start: Vec<String> = todo.into_iter().map(|(_, u)| u).filter(|u| !busy.contains_key(u) && !cooling.contains_key(u)).take(free).collect();
        for url in start {
            let (h, s, u) = (http.clone(), sh.clone(), url.clone());
            let handle = inflight.spawn(async move {
                let r = get(h, s, u.clone()).await;
                (u, r)
            });
            busy.insert(url, (handle, Instant::now()));
        }
        if inflight.is_empty() {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        let Ok(Some(Ok((url, r)))) = tokio::time::timeout(Duration::from_millis(250), inflight.join_next()).await else { continue };
        let took = busy.remove(&url).map(|(_, at)| at.elapsed().as_secs_f64());
        if let Some(f) = feed.as_mut() {
            let error = r.as_ref().err().filter(|_| !sh.stopped());
            f.loaded(&sh, &url, took, error.map(String::as_str));
            if error.is_some() {
                // Try it again in a moment.
                cooling.insert(url, Instant::now() + LIVE_RETRY);
                continue;
            }
        }
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

async fn load(http: &reqwest::Client, sh: &Arc<Shared>, source: &Source) -> Result<Option<LiveFeed>, String> {
    let (video, audio, variants, variant) = match source {
        Source::Live(url) => return LiveFeed::open(http, sh, url).await.map(Some),
        Source::Mp4(url) => {
            let plan = Arc::new(Plan { init: None, segs: vec![Seg { url: url.clone(), start: 0.0, dur: 0.0 }], single: true, first: 0, live: false });
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
    Ok(None)
}

/// Keeps a live stream's segment lists current.
struct LiveFeed {
    video_url: String,
    audio_url: Option<String>,
    /// A known segment on the stream's clock: (number, seconds).
    video_anchor: (u64, f64),
    audio_anchor: (u64, f64),
    /// The longest segment.
    target: f64,
    last: Instant,
    refresh: Option<tokio::task::JoinHandle<Result<(hls::Media, Option<hls::Media>), String>>>,
    /// Failed loads in a row.
    fails: u32,
    /// Failed loads in a row of a segment playback is waiting for.
    due_fails: u32,
    /// Video segments in a row that took longer to load than to play.
    slow: u32,
    /// Video segments in a row that loaded with lots of time to spare.
    quick: u32,
}

impl Drop for LiveFeed {
    fn drop(&mut self) {
        if let Some(r) = self.refresh.take() {
            r.abort();
        }
    }
}

/// Where a segment really starts on the stream's clock (its own timestamps).
/// Numbers and timestamps line up only roughly, and audio and video differ.
async fn anchor_of(http: &reqwest::Client, sh: &Arc<Shared>, m: &hls::Media, number: u64, video: bool) -> Result<(u64, f64), String> {
    let init_url = m.init.clone().ok_or("live stream without an init segment")?;
    let i = usize::try_from(number.saturating_sub(m.sequence)).unwrap_or(usize::MAX);
    let seg_url = m.segments.get(i).ok_or("live stream without segments")?.0.clone();
    let (init, seg) = tokio::try_join!(get(http.clone(), sh.clone(), init_url.clone()), get(http.clone(), sh.clone(), seg_url.clone()))?;
    let movie = mp4::parse_movie(&init)?;
    let track = if video { movie.video() } else { movie.audio() }.ok_or("live stream track missing")?;
    let start = mp4::parse_fragment(&seg, track).first().map(|s| track.pts(s)).ok_or("live segment without samples")?;
    // Already here: don't download them again.
    let mut st = sh.lock();
    st.cache.insert(init_url, Arc::new(init));
    st.cache.insert(seg_url, Arc::new(seg));
    Ok((number, start))
}

impl LiveFeed {
    async fn open(http: &reqwest::Client, sh: &Arc<Shared>, url: &str) -> Result<LiveFeed, String> {
        let body = text(http, sh, url).await?;
        if !hls::is_master(&body) {
            return Err("not a live stream's master playlist".into());
        }
        let variants = hls::parse_master(&body, url);
        if variants.is_empty() {
            return Err("empty playlist".into());
        }
        let i = {
            let st = sh.lock();
            st.manual.unwrap_or_else(|| pick_variant(&variants, st.width)).min(variants.len() - 1)
        };
        let video_url = variants[i].url.clone();
        let audio_url = variants[i].audio.clone();
        let (v, a) = fetch_lists(http.clone(), sh.clone(), video_url.clone(), audio_url.clone()).await?;
        let join = v.sequence + v.segments.len().saturating_sub(LIVE_BACK + 1) as u64;
        let video_anchor = anchor_of(http, sh, &v, join, true).await?;
        // The audio list can be a segment ahead of or behind the video's.
        let audio_anchor = match &a {
            Some(a) => {
                let last = a.sequence + (a.segments.len() as u64).saturating_sub(1);
                match anchor_of(http, sh, a, join.clamp(a.sequence, last), false).await {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::info!("video: live audio: {e}");
                        video_anchor
                    }
                }
            }
            None => video_anchor,
        };
        let target = if v.target > 0.0 { v.target } else { 2.0 };
        let video = Arc::new(Plan::from_live(v, video_anchor));
        let audio = a.map(|a| Arc::new(Plan::from_live(a, audio_anchor)));
        let mut st = sh.lock();
        let at = video.seg(video_anchor.0).map_or_else(|| video.live_point(), |s| s.start);
        st.video = Some(video);
        st.audio_plan = audio;
        st.variants = variants;
        st.variant = i;
        st.restart(at);
        sh.epoch.store(st.epoch, Ordering::Relaxed);
        st.ready = true;
        drop(st);
        sh.notify();
        Ok(LiveFeed { video_url, audio_url, video_anchor, audio_anchor, target, last: Instant::now(), refresh: None, fails: 0, due_fails: 0, slow: 0, quick: 0 })
    }

    /// Reload the lists when it's time and apply new ones. Never waits for
    /// the network, so downloads keep flowing.
    async fn tick(&mut self, http: &reqwest::Client, sh: &Arc<Shared>) {
        if self.refresh.as_ref().is_some_and(|r| r.is_finished()) {
            if let Some(r) = self.refresh.take() {
                match r.await {
                    Ok(Ok((v, a))) => {
                        self.fails = 0;
                        self.apply(sh, v, a);
                    }
                    Ok(Err(e)) => {
                        tracing::debug!("video: live playlist: {e}");
                        self.failed(sh);
                    }
                    Err(_) => self.failed(sh),
                }
            }
        }
        // A paused player only needs to know where the stream has got to.
        let paused = sh.lock().paused;
        let every = if paused { 5.0 } else { self.target.clamp(1.0, 6.0) };
        if self.refresh.is_none() && self.last.elapsed().as_secs_f64() >= every {
            self.last = Instant::now();
            self.refresh = Some(tokio::spawn(fetch_lists(http.clone(), sh.clone(), self.video_url.clone(), self.audio_url.clone())));
        }
    }

    fn failed(&mut self, sh: &Shared) {
        self.fails += 1;
        if self.fails >= LIVE_FAILS {
            let mut st = sh.lock();
            if !st.live_lost {
                tracing::info!("video: live stream lost");
                st.live_lost = true;
                drop(st);
                sh.notify();
                sh.repaint();
            }
        }
    }

    fn apply(&mut self, sh: &Shared, v: hls::Media, a: Option<hls::Media>) {
        let ended = v.ended;
        let video = Arc::new(Plan::from_live(v, self.video_anchor));
        let audio = a.map(|a| Arc::new(Plan::from_live(a, self.audio_anchor)));
        // Keep the clock exact from what's listed now, so rounding can't add up.
        if let Some(s) = video.segs.first() {
            self.video_anchor = (video.first, s.start);
        }
        if let Some(s) = audio.as_ref().and_then(|p| p.segs.first().map(|s| (p.first, s.start))) {
            self.audio_anchor = s;
        }
        let mut st = sh.lock();
        let (start, join) = (video.segs.first().map_or(0.0, |s| s.start), video.live_point());
        st.video = Some(video);
        if audio.is_some() {
            st.audio_plan = audio;
        }
        // Fell out of what Steam still keeps (a long stall): rejoin. A paused
        // player stays on its picture until it's played again.
        let stalled_behind = !st.behind_on_purpose && st.clock < join - LIVE_CATCH_UP;
        if !ended && !st.paused && (st.clock < start - 0.5 || stalled_behind) {
            tracing::debug!("video: behind the live window, rejoining");
            st.behind_on_purpose = false;
            st.restart(join);
            sh.epoch.store(st.epoch, Ordering::Relaxed);
        }
        drop(st);
        sh.notify();
        sh.repaint();
    }

    /// The list of another quality, on the same clock.
    async fn switch(&mut self, http: &reqwest::Client, sh: &Arc<Shared>, url: String) -> Result<Plan, String> {
        let m = hls::parse_media(&text(http, sh, &url).await?, &url);
        self.video_url = url;
        self.slow = 0;
        Ok(Plan::from_live(m, self.video_anchor))
    }

    /// A segment finished loading, or failed with `error`.
    fn loaded(&mut self, sh: &Shared, url: &str, took: Option<f64>, error: Option<&str>) {
        let mut st = sh.lock();
        if let Some(e) = error {
            tracing::debug!("video: live segment: {e}");
            // The newest segments often aren't on this CDN server yet: that
            // only matters once playback actually needs the segment.
            let clock = st.clock;
            let due = [&st.video, &st.audio_plan].into_iter().flatten().flat_map(|p| p.segs.iter()).find(|s| s.url == url).is_some_and(|s| s.start <= clock + 1.0);
            if !due {
                return;
            }
            self.due_fails += 1;
            if self.due_fails < 3 {
                return;
            }
            // That moment is gone: pick up again from the newest video.
            self.due_fails = 0;
            if let Some(join) = st.video.as_ref().map(|p| p.live_point()) {
                st.restart(join);
                sh.epoch.store(st.epoch, Ordering::Relaxed);
            }
            drop(st);
            self.failed(sh);
            sh.notify();
            return;
        }
        self.fails = 0;
        self.due_fails = 0;
        if !st.video.as_ref().is_some_and(|p| p.segs.iter().any(|s| s.url == url)) {
            return;
        }
        // Two video segments in a row slower than real time: step down a
        // quality (only on automatic).
        self.slow = if took.is_some_and(|s| s > self.target) { self.slow + 1 } else { 0 };
        if self.slow >= 2 && st.manual.is_none() && st.variant > 0 {
            self.slow = 0;
            st.cap = Some(st.variant - 1);
            tracing::info!("video: connection too slow, lowering the quality");
        }
        // Fast again for a good while: allow one quality more.
        self.quick = if took.is_some_and(|s| s < self.target * 0.4) { self.quick + 1 } else { 0 };
        if self.quick >= LIVE_QUICK {
            self.quick = 0;
            st.cap = st.cap.map(|c| c + 1).filter(|c| *c + 1 < st.variants.len());
        }
    }
}

/// Both of a live stream's playlists.
async fn fetch_lists(http: reqwest::Client, sh: Arc<Shared>, video: String, audio: Option<String>) -> Result<(hls::Media, Option<hls::Media>), String> {
    let v = text(&http, &sh, &video).await?;
    let a = match &audio {
        Some(u) => Some(hls::parse_media(&text(&http, &sh, u).await?, u)),
        None => None,
    };
    Ok((hls::parse_media(&v, &video), a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn live_media(sequence: u64, count: usize) -> hls::Media {
        hls::Media {
            init: Some("init".into()),
            segments: (0..count as u64).map(|i| (format!("seg{}", sequence + i), 2.0)).collect(),
            sequence,
            target: 2.0,
            ended: false,
        }
    }

    #[test]
    fn live_plan_keeps_numbers_and_times_as_the_window_moves() {
        // Segment 105 really starts at 1000.5 s on the stream's clock.
        let p = Plan::from_live(live_media(100, 10), (105, 1000.5));
        assert_eq!((p.first, p.end_number()), (100, 110));
        assert!(p.live);
        assert_eq!(p.seg(105).unwrap().start, 1000.5);
        assert_eq!(p.seg(100).unwrap().start, 990.5);
        assert!(p.seg(99).is_none() && p.seg(110).is_none());
        assert_eq!(p.number_at(1001.0), 105);
        // Joins three segments behind the newest (109).
        assert_eq!(p.live_point(), p.seg(106).unwrap().start);

        // Later the list has moved past the anchor: the same segment keeps its time.
        let later = Plan::from_live(live_media(108, 10), (105, 1000.5));
        assert_eq!(later.seg(108).unwrap().start, 1006.5);
        assert_eq!(later.seg(117).unwrap().start, 1024.5);
    }

    #[test]
    fn live_plan_knows_when_the_broadcast_ended() {
        let mut m = live_media(7, 3);
        m.ended = true;
        let p = Plan::from_live(m, (7, 14.0));
        assert!(!p.live);
        assert_eq!(p.end_number(), 10);
    }

    #[test]
    fn a_short_live_list_joins_at_its_start() {
        let p = Plan::from_live(live_media(5, 2), (5, 10.0));
        assert_eq!(p.live_point(), 10.0);
    }
    use std::net::TcpListener;

    fn test_shared() -> Arc<Shared> {
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
            width: 1280,
            cache: HashMap::new(),
            frames: VecDeque::new(),
            has_frame: false,
            video_eos: false,
            audio: Audio::Pending,
            audio_eos: false,
            clock: 0.0,
            clock_at: now,
            running: false,
            paused: true,
            auto_paused: false,
            last_poll: now,
            volume: 0.7,
            muted: false,
            looping: false,
            error: None,
            live: false,
            live_lost: false,
            behind_on_purpose: false,
            cap: None,
        };
        Arc::new(Shared {
            m: Mutex::new(st),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            ctx: egui::Context::default(),
        })
    }

    #[tokio::test]
    async fn content_range_underflow_and_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut sock, _) = listener.accept().unwrap();
                let mut req = [0u8; 2048];
                let n = sock.read(&mut req).unwrap_or(0);
                let req_s = String::from_utf8_lossy(&req[..n]);
                let first_line = req_s.lines().next().unwrap_or("");
                if first_line.contains("/huge") {
                    let hdr = "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-3/300000000\r\nContent-Length: 4\r\nConnection: close\r\n\r\n";
                    let _ = sock.write_all(hdr.as_bytes());
                    let _ = sock.write_all(b"abcd");
                } else if req_s.to_ascii_lowercase().contains("range: bytes=0-") {
                    let hdr = "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-499999/450000\r\nContent-Length: 500000\r\nConnection: close\r\n\r\n";
                    let _ = sock.write_all(hdr.as_bytes());
                    let _ = sock.write_all(&vec![b'a'; 500_000]);
                } else {
                    let hdr = "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 393216-449999/450000\r\nContent-Length: 56784\r\nConnection: close\r\n\r\n";
                    let _ = sock.write_all(hdr.as_bytes());
                    let _ = sock.write_all(&vec![b'b'; 56_784]);
                }
            }
        });

        let http = reqwest::Client::new();
        let sh = test_shared();
        let r1 = get(http.clone(), sh.clone(), format!("http://{addr}/underflow")).await;
        assert!(r1.is_ok(), "unexpected error on short Content-Range total: {r1:?}");

        let r2 = get(http, sh, format!("http://{addr}/huge")).await;
        assert!(r2.is_err(), "Content-Range total > 256 MiB must be rejected");
        srv.join().unwrap();
    }
}

//! Pictures on the graphics card, let go of when they're no longer drawn.
//!
//! egui's own texture cache keeps every picture it ever showed until the app
//! closes: every store capsule, screenshot, avatar and library cover you
//! scrolled past, and every frame of every animated avatar. With the OpenGL
//! renderer the driver keeps a copy of each in the app's memory too, so a
//! long session grew by hundreds of megabytes. This one (tried before egui's)
//! frees a picture that hasn't been drawn for [`KEEP`] seconds of use, and the
//! longest-unused ones once the total passes [`BUDGET`]. Drawn again later,
//! a picture comes back from the disk cache in a moment.

use egui::load::{ImagePoll, Loaders, SizedTexture, TextureLoadResult, TextureLoader, TexturePoll};
use egui::{Context, SizeHint, TextureHandle, TextureOptions, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Seconds (of the app being used, not idle) a picture stays after it was
/// last drawn.
const KEEP: f64 = 30.0;
/// Above this many bytes of pictures, the longest-unused go first.
const BUDGET: usize = 160 * 1024 * 1024;
/// Pictures drawn this recently are on screen: never freed for the budget.
const ON_SCREEN: f64 = 1.0;

struct Entry {
    handle: TextureHandle,
    size: Vec2,
    bytes: usize,
    /// [`State::clock`] when it was last drawn.
    used: f64,
}

#[derive(Default)]
struct State {
    cache: HashMap<(String, TextureOptions), Entry>,
    /// Seconds the app has been in use: advances with frames, at most a
    /// quarter second at a time, so pictures on screen aren't freed while
    /// the app sits idle (nothing is redrawn then).
    clock: f64,
    last_tick: Option<Instant>,
    last_sweep: f64,
}

#[derive(Default)]
pub struct TextureCache {
    state: Mutex<State>,
}

static INSTALLED: OnceLock<Arc<TextureCache>> = OnceLock::new();

impl TextureCache {
    pub fn install(ctx: &Context) {
        let cache = INSTALLED.get_or_init(Default::default).clone();
        ctx.add_texture_loader(cache);
    }

    /// Free what's no longer drawn. `loaders`: also forget their copies.
    fn sweep(&self, now: f64, loaders: Option<&Loaders>) -> usize {
        let freed: Vec<String> = {
            let mut st = self.state.lock().unwrap();
            st.last_sweep = now;
            let mut freed = Vec::new();
            st.cache.retain(|(uri, _), e| {
                let keep = now - e.used <= KEEP;
                if !keep {
                    freed.push(uri.clone());
                }
                keep
            });
            let mut total: usize = st.cache.values().map(|e| e.bytes).sum();
            if total > BUDGET {
                let mut old: Vec<(f64, (String, TextureOptions), usize)> =
                    st.cache.iter().filter(|(_, e)| now - e.used > ON_SCREEN).map(|(k, e)| (e.used, k.clone(), e.bytes)).collect();
                old.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (_, key, bytes) in old {
                    if total <= BUDGET {
                        break;
                    }
                    st.cache.remove(&key);
                    total -= bytes;
                    freed.push(key.0);
                }
            }
            if freed.is_empty() {
                return 0;
            }
            // A plain address none of whose sizes or frames is kept any more:
            // the loaders' decoded frames and downloaded bytes go too.
            let live: HashSet<&str> = st.cache.keys().map(|(u, _)| base(u)).collect();
            let mut out: Vec<String> = freed.iter().filter(|u| !live.contains(base(u))).map(|u| base(u).to_string()).collect();
            out.extend(freed.iter().cloned());
            out.sort();
            out.dedup();
            out
        };
        let n = freed.len();
        if let Some(loaders) = loaders {
            let image = loaders.image.lock();
            let bytes = loaders.bytes.lock();
            for uri in &freed {
                for l in image.iter() {
                    l.forget(uri);
                }
                for l in bytes.iter() {
                    l.forget(uri);
                }
            }
        }
        n
    }

    fn total_bytes(&self) -> usize {
        self.state.lock().unwrap().cache.values().map(|e| e.bytes).sum()
    }
}

/// The plain address of a picture: without a frame number (`<url>#3`, how
/// egui names the frames of an animation) or a drawn size (`sized_url`).
fn base(uri: &str) -> &str {
    if let Some((url, ..)) = ui::widgets::parse_sized(uri) {
        return url;
    }
    match uri.rsplit_once('#') {
        Some((url, frame)) if !frame.is_empty() && frame.bytes().all(|b| b.is_ascii_digit()) => url,
        _ => uri,
    }
}

/// Called once a frame: advance the clock and, every couple of seconds,
/// free pictures that aren't drawn any more.
pub fn tick(ctx: &Context) {
    let Some(cache) = INSTALLED.get() else { return };
    let now = {
        let mut st = cache.state.lock().unwrap();
        let t = Instant::now();
        let dt = st.last_tick.map(|l| t.duration_since(l).as_secs_f64()).unwrap_or(0.0);
        st.last_tick = Some(t);
        st.clock += dt.min(0.25);
        if st.clock - st.last_sweep < 2.0 {
            return;
        }
        st.clock
    };
    let loaders = ctx.loaders();
    cache.sweep(now, Some(&loaders));
}

impl TextureLoader for TextureCache {
    fn id(&self) -> &str {
        egui::generate_loader_id!(TextureCache)
    }

    fn load(&self, ctx: &Context, uri: &str, options: TextureOptions, size_hint: SizeHint) -> TextureLoadResult {
        // SVG icons are drawn at many sizes; egui's own cache handles those.
        let path = uri.split(['?', '#']).next().unwrap_or(uri);
        if path.to_ascii_lowercase().ends_with(".svg") {
            return Err(egui::load::LoadError::NotSupported);
        }
        let key = (uri.to_string(), options);
        {
            let mut st = self.state.lock().unwrap();
            let clock = st.clock;
            if let Some(e) = st.cache.get_mut(&key) {
                e.used = clock;
                return Ok(TexturePoll::Ready { texture: SizedTexture::new(e.handle.id(), e.size) });
            }
        }
        match ctx.try_load_image(uri, size_hint)? {
            ImagePoll::Pending { size } => Ok(TexturePoll::Pending { size }),
            ImagePoll::Ready { image } => {
                let size = image.source_size;
                let bytes = image.pixels.len() * 4;
                let handle = ctx.load_texture(uri, image, options);
                let texture = SizedTexture::new(handle.id(), size);
                let mut st = self.state.lock().unwrap();
                let used = st.clock;
                st.cache.insert(key, Entry { handle, size, bytes, used });
                Ok(TexturePoll::Ready { texture })
            }
        }
    }

    fn forget(&self, uri: &str) {
        self.state.lock().unwrap().cache.retain(|(u, _), _| u != uri);
    }

    fn forget_all(&self) {
        self.state.lock().unwrap().cache.clear();
    }

    fn byte_size(&self) -> usize {
        self.total_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(cache: &TextureCache, ctx: &Context, uri: &str, side: usize, used: f64) {
        let image = egui::ColorImage::filled([side, side], egui::Color32::RED);
        let handle = ctx.load_texture(uri, image, TextureOptions::default());
        cache.state.lock().unwrap().cache.insert((uri.to_string(), TextureOptions::default()), Entry { handle, size: Vec2::splat(side as f32), bytes: side * side * 4, used });
    }

    fn kept(cache: &TextureCache) -> Vec<String> {
        let mut v: Vec<String> = cache.state.lock().unwrap().cache.keys().map(|k| k.0.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn frees_pictures_not_drawn_for_a_while() {
        let ctx = Context::default();
        let cache = TextureCache::default();
        put(&cache, &ctx, "https://x/old.jpg", 8, 0.0);
        put(&cache, &ctx, "https://x/new.jpg", 8, 50.0);
        let textures = ctx.tex_manager().read().num_allocated();
        assert_eq!(cache.sweep(60.0, None), 1);
        assert_eq!(kept(&cache), ["https://x/new.jpg"]);
        // Its texture is gone from the graphics card too.
        assert_eq!(ctx.tex_manager().read().num_allocated(), textures - 1);
    }

    #[test]
    fn over_budget_the_longest_unused_go_but_not_whats_on_screen() {
        let ctx = Context::default();
        let cache = TextureCache::default();
        // 2048² × 4 = 16 MB each; 12 of them = 192 MB, over the 160 MB budget.
        for i in 0..12 {
            put(&cache, &ctx, &format!("https://x/{i:02}.jpg"), 2048, 20.0 + i as f64);
        }
        cache.sweep(32.0, None);
        assert!(cache.total_bytes() <= BUDGET);
        let left = kept(&cache);
        assert_eq!(left.len(), 10);
        assert!(!left.contains(&"https://x/00.jpg".to_string()) && !left.contains(&"https://x/01.jpg".to_string()));
        // All drawn just now (on screen): nothing is freed, even over budget.
        let cache = TextureCache::default();
        for i in 0..12 {
            put(&cache, &ctx, &format!("https://x/{i:02}.jpg"), 2048, 32.0);
        }
        assert_eq!(cache.sweep(32.5, None), 0);
    }

    /// Remembers what it was told to forget.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl egui::load::ImageLoader for Recorder {
        fn id(&self) -> &str {
            "Recorder"
        }
        fn load(&self, _: &Context, _: &str, _: SizeHint) -> egui::load::ImageLoadResult {
            Err(egui::load::LoadError::NotSupported)
        }
        fn forget(&self, uri: &str) {
            self.0.lock().unwrap().push(uri.to_string());
        }
        fn forget_all(&self) {}
        fn byte_size(&self) -> usize {
            0
        }
    }

    #[test]
    fn an_animation_no_longer_drawn_is_forgotten_whole() {
        let ctx = Context::default();
        let recorder = Arc::new(Recorder::default());
        ctx.add_image_loader(recorder.clone());
        let cache = TextureCache::default();
        // Every frame of a.gif is old; b.gif still plays (one old frame).
        put(&cache, &ctx, "https://x/a.gif#0", 4, 0.0);
        put(&cache, &ctx, "https://x/a.gif#1", 4, 0.0);
        put(&cache, &ctx, "https://x/b.gif#0", 4, 0.0);
        put(&cache, &ctx, "https://x/b.gif#1", 4, 59.0);
        cache.sweep(60.0, Some(&ctx.loaders()));
        let forgot = recorder.0.lock().unwrap().clone();
        // a.gif's decoded frames go; b.gif's stay (it's still drawn).
        assert!(forgot.contains(&"https://x/a.gif".to_string()));
        assert!(!forgot.contains(&"https://x/b.gif".to_string()));
        assert_eq!(kept(&cache), ["https://x/b.gif#1"]);
    }

    #[test]
    fn plain_address_of_frames_and_sizes() {
        assert_eq!(base("https://x/a.gif#12"), "https://x/a.gif");
        assert_eq!(base("https://x/a.gif"), "https://x/a.gif");
        assert_eq!(base("https://x/page#top"), "https://x/page#top");
        let sized = ui::widgets::sized_url("https://x/a.jpg", Vec2::new(180.0, 270.0), true);
        assert_eq!(base(&sized), "https://x/a.jpg");
    }
}

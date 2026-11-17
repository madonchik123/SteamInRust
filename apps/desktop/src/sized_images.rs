//! Still pictures decoded at the size they're drawn at, on a few threads.
//!
//! egui's own image loader decodes every picture in full (a 600×900 library
//! capsule is 2.2 MB as a texture, drawn at ~190×280) and starts a thread
//! per picture, so opening a page full of art decoded dozens at once. The
//! widgets ask for `<url>#sir-fit=<cover|contain>,<w>x<h>` (see
//! `ui::widgets::sized_url`); this loader reads the picture's bytes through
//! the normal (disk-cached) bytes loader, decodes and shrinks it on a small
//! pool, and keeps the result only until it's on the GPU.

use egui::ColorImage;
use egui::load::{BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Decoded pictures nobody picked up for this long are dropped.
const UNCLAIMED_FOR: Duration = Duration::from_secs(10);

enum Entry {
    Pending,
    Ready(Arc<ColorImage>, Instant),
    Failed(String),
}

struct Job {
    uri: String,
    bytes: egui::load::Bytes,
    cover: bool,
    w: u32,
    h: u32,
    ctx: egui::Context,
}

pub struct SizedLoader {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
    jobs: Mutex<Sender<Job>>,
    swept: Mutex<Instant>,
}

impl SizedLoader {
    pub fn install(ctx: &egui::Context) {
        let entries: Arc<Mutex<HashMap<String, Entry>>> = Default::default();
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        // A few decoders: enough to keep up with scrolling, few enough that a
        // page of art doesn't hold dozens of full-size pictures at once.
        let threads = std::thread::available_parallelism().map(|n| (n.get() / 2).clamp(1, 3)).unwrap_or(2);
        for i in 0..threads {
            let (rx, entries) = (rx.clone(), entries.clone());
            let _ = std::thread::Builder::new().name(format!("picture-decode-{i}")).spawn(move || worker(&rx, &entries));
        }
        ctx.add_image_loader(Arc::new(SizedLoader { entries, jobs: Mutex::new(tx), swept: Mutex::new(Instant::now()) }));
    }

    /// Drop decoded pictures that were never uploaded (scrolled past while decoding).
    fn sweep(&self) {
        let mut swept = self.swept.lock().unwrap();
        if swept.elapsed() < Duration::from_secs(5) {
            return;
        }
        *swept = Instant::now();
        self.entries.lock().unwrap().retain(|_, e| !matches!(e, Entry::Ready(_, at) if at.elapsed() > UNCLAIMED_FOR));
    }
}

fn worker(rx: &Mutex<Receiver<Job>>, entries: &Mutex<HashMap<String, Entry>>) {
    loop {
        let Ok(job) = rx.lock().unwrap().recv() else { return };
        let result = decode(&job.bytes, job.cover, job.w, job.h);
        let mut map = entries.lock().unwrap();
        // Forgotten meanwhile (another page): don't keep it.
        if matches!(map.get(&job.uri), Some(Entry::Pending)) {
            map.insert(job.uri, match result {
                Ok(image) => Entry::Ready(Arc::new(image), Instant::now()),
                Err(e) => Entry::Failed(e),
            });
            drop(map);
            job.ctx.request_repaint();
        }
    }
}

/// Decode, then shrink so the picture still covers (or fits in) `w`×`h`.
fn decode(bytes: &[u8], cover: bool, w: u32, h: u32) -> Result<ColorImage, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let (iw, ih) = (img.width().max(1), img.height().max(1));
    let (sx, sy) = (w as f32 / iw as f32, h as f32 / ih as f32);
    let scale = if cover { sx.max(sy) } else { sx.min(sy) };
    let img = if scale < 0.9 {
        let (nw, nh) = (((iw as f32 * scale).ceil() as u32).max(1), ((ih as f32 * scale).ceil() as u32).max(1));
        img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

impl ImageLoader for SizedLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(SizedLoader)
    }

    fn load(&self, ctx: &egui::Context, uri: &str, _: SizeHint) -> ImageLoadResult {
        let Some((url, cover, w, h)) = ui::widgets::parse_sized(uri) else {
            return Err(LoadError::NotSupported);
        };
        self.sweep();
        match self.entries.lock().unwrap().get(uri) {
            Some(Entry::Pending) => return Ok(ImagePoll::Pending { size: None }),
            Some(Entry::Ready(image, _)) => return Ok(ImagePoll::Ready { image: image.clone() }),
            Some(Entry::Failed(e)) => return Err(LoadError::Loading(e.clone())),
            None => {}
        }
        match ctx.try_load_bytes(url) {
            Ok(BytesPoll::Ready { bytes, .. }) => {
                self.entries.lock().unwrap().insert(uri.to_string(), Entry::Pending);
                // The decoder has its own copy now; the disk cache has the file.
                for loader in ctx.loaders().bytes.lock().iter() {
                    loader.forget(url);
                }
                let job = Job { uri: uri.to_string(), bytes, cover, w, h, ctx: ctx.clone() };
                if self.jobs.lock().unwrap().send(job).is_err() {
                    self.entries.lock().unwrap().insert(uri.to_string(), Entry::Failed("no decoder".into()));
                }
                Ok(ImagePoll::Pending { size: None })
            }
            Ok(BytesPoll::Pending { size }) => Ok(ImagePoll::Pending { size }),
            Err(e) => Err(e),
        }
    }

    fn forget(&self, uri: &str) {
        self.entries.lock().unwrap().remove(uri);
    }

    fn forget_all(&self) {
        self.entries.lock().unwrap().clear();
    }

    fn byte_size(&self) -> usize {
        self.entries.lock().unwrap().values().map(|e| if let Entry::Ready(i, _) = e { i.pixels.len() * 4 } else { 0 }).sum()
    }

    fn has_pending(&self) -> bool {
        self.entries.lock().unwrap().values().any(|e| matches!(e, Entry::Pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn shrinks_to_the_drawn_size() {
        let capsule = jpeg(600, 900);
        // Cover 192×384: the width needs 192, the height 384 → 256×384.
        assert_eq!(decode(&capsule, true, 192, 384).unwrap().size, [256, 384]);
        // Contain in 192×384: the width limits → 192×288.
        assert_eq!(decode(&capsule, false, 192, 384).unwrap().size, [192, 288]);
        // Drawn about as large as it is: kept whole.
        assert_eq!(decode(&capsule, true, 600, 900).unwrap().size, [600, 900]);
        assert_eq!(decode(&capsule, true, 2048, 2048).unwrap().size, [600, 900]);
    }

    #[test]
    fn sized_urls_round_trip() {
        let url = "https://shared.steamstatic.com/store_item_assets/steam/apps/10/library_600x900.jpg?t=1";
        let sized = ui::widgets::sized_url(url, egui::vec2(180.0, 270.0), true);
        assert_eq!(ui::widgets::parse_sized(&sized), Some((url, true, 192, 384)));
        // Animations and huge draws stay plain.
        assert_eq!(ui::widgets::sized_url("https://x/a.gif", egui::vec2(10.0, 10.0), true), "https://x/a.gif");
        assert_eq!(ui::widgets::sized_url(url, egui::vec2(4000.0, 100.0), true), url);
    }
}

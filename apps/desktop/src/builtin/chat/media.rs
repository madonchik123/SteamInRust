//! Pictures and videos in messages, and pictures you're about to send.
//!
//! - Pictures and videos from Steam's own servers show right away. Ones from
//!   any other site wait behind a "Load preview" box: loading them would tell
//!   that site your IP address, so nothing is asked of it until you click.
//! - An MP4 link plays inside the chat with the app's own player (one at a
//!   time; it stops when the chat stops being drawn).
//! - A picture pasted with Ctrl+V, dropped on the chat or picked with "+"
//!   is shown first and sent only after "Send".

use egui::{Align2, ColorImage, Rect, Sense, Ui, vec2};
use i18n::t;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ui::{Theme, icons, widgets as w};

/// Pictures and videos from other sites the player said to load (this run).
static ALLOWED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// The site a link points to, lower case and without a port ("i.imgur.com").
pub(crate) fn host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    host.split(':').next().unwrap_or_default().to_ascii_lowercase()
}

/// Is this one of Steam's own servers (safe to load without asking)?
pub(crate) fn is_steam(url: &str) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    let h = host(url);
    const STEAM: [&str; 5] = ["steamstatic.com", "steamusercontent.com", "steampowered.com", "steamcommunity.com", "steamchina.com"];
    const AKAMAI: [&str; 3] = ["steamcdn-a.akamaihd.net", "steamuserimages-a.akamaihd.net", "steamcommunity-a.akamaihd.net"];
    STEAM.iter().any(|d| h == *d || h.ends_with(&format!(".{d}"))) || AKAMAI.contains(&h.as_str())
}

/// May this picture or video be loaded now?
pub(crate) fn may_load(url: &str) -> bool {
    is_steam(url) || ALLOWED.lock().ok().is_some_and(|g| g.as_ref().is_some_and(|s| s.contains(url)))
}

/// The player said to load it.
pub(crate) fn allow(url: &str) {
    if let Ok(mut g) = ALLOWED.lock() {
        g.get_or_insert_with(HashSet::new).insert(url.to_string());
    }
}

/// A video link.
pub(crate) fn is_video(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    [".mp4", ".m4v", ".webm", ".mov"].iter().any(|e| path.ends_with(e))
}

/// A video the app's own player can show (H.264 in MP4).
pub(crate) fn plays_here(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    path.ends_with(".mp4") || path.ends_with(".m4v")
}

/// The box shown in place of a picture or video from another site. True
/// when "Load preview" was clicked.
pub(crate) fn placeholder(ui: &mut Ui, t: &Theme, url: &str, video: bool) -> bool {
    let width = ui.available_width().min(t.dense(360.0));
    let site = host(url);
    let mut load = false;
    let r = w::component_frame(t, &t.components.card).inner_margin(egui::Margin::same(t.spacing.sm as i8)).show(ui, |ui| {
        ui.set_width((width - t.spacing.lg).max(0.0));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(if video { icons::PLAY } else { icons::IMAGE }).font(w::font(t.dense(28.0))).color(t.text_dim));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let head = if video { t!("Video from {site}", site = site) } else { t!("Picture from {site}", site = site) };
                ui.add(egui::Label::new(crate::builtin::friends::strong(t, head, t.text)).truncate());
                full_address(ui, t, url);
                ui.add(egui::Label::new(w::body_s(t, t!("Not from Steam. Loading it lets that site see your IP address."), t.text_dim)).wrap());
                ui.horizontal(|ui| {
                    load = w::button(ui, t, Some(icons::EYE), t!("Load preview"), w::Kind::Secondary, w::Size::Sm).clicked();
                    if w::button(ui, t, Some(icons::EXTERNAL), t!("Open in browser"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        crate::links::open_http(url);
                    }
                });
            });
        });
    });
    link_menu(&r.response, ui, t, url, None);
    if load {
        allow(url);
    }
    load
}

/// The whole address, wrapped and selectable, so it can be checked.
fn full_address(ui: &mut Ui, t: &Theme, url: &str) {
    ui.add(egui::Label::new(address(url, egui::FontId::new(t.typography.body_s.size, t.typography.body_s.weight.family()), t.text_faint)).wrap());
}

/// An address laid out to wrap at any letter (they rarely have spaces),
/// filling each line instead of jumping to the next one.
pub(crate) fn address(text: &str, font: egui::FontId, color: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap.break_anywhere = true;
    job
}

/// A picture that didn't load: a small box saying so, with its link.
pub(crate) fn failed(ui: &mut Ui, t: &Theme, url: &str) {
    let width = ui.available_width().min(t.dense(360.0));
    let r = w::component_frame(t, &t.components.card).inner_margin(egui::Margin::same(t.spacing.sm as i8)).show(ui, |ui| {
        ui.set_width((width - t.spacing.lg).max(0.0));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(icons::IMAGE).font(w::font(t.dense(28.0))).color(t.text_faint));
            ui.vertical(|ui| {
                ui.label(w::body_m(t, t!("Couldn't load the picture"), t.text_dim));
                    full_address(ui, t, url);
                if w::button(ui, t, Some(icons::EXTERNAL), t!("Open in browser"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    crate::links::open_http(url);
                }
            });
        });
    });
    link_menu(&r.response, ui, t, url, None);
}

/// "Open", "Open in browser" and "Copy link" on a right-click. `open` is
/// what "Open" does in the app (Store pages, profiles…), when it does more
/// than the browser would.
pub(crate) fn link_menu(resp: &egui::Response, ui: &Ui, t: &Theme, url: &str, mut open: Option<&mut dyn FnMut()>) {
    resp.context_menu(|ui| {
        ui.set_min_width(t.dense(200.0));
        if let Some(open) = open.as_mut() {
            if w::menu_item(ui, t, icons::LINK, t!("Open")).clicked() {
                open();
                ui.close();
            }
        }
        if w::menu_item(ui, t, icons::EXTERNAL, t!("Open in browser")).clicked() {
            crate::links::open_http(url);
            ui.close();
        }
        if w::menu_item(ui, t, icons::COPY, t!("Copy link")).clicked() {
            ui.ctx().copy_text(url.to_string());
            ui.close();
        }
    });
    let _ = ui;
}

// ---------- the video playing in a chat ----------

struct Playing {
    url: String,
    player: video::Player,
    drawn: Instant,
}

thread_local! {
    static PLAYING: RefCell<Option<Playing>> = const { RefCell::new(None) };
}

/// Is this video the one playing?
pub(crate) fn is_playing(url: &str) -> bool {
    PLAYING.with(|p| p.borrow().as_ref().is_some_and(|p| p.url == url))
}

/// Start this video (stopping any other).
pub(crate) fn play(ctx: &egui::Context, url: &str) {
    let Some(rt) = super::cards::runtime() else { return };
    allow(url);
    // Stop it once the chat is no longer drawn (closed, another page…).
    let swept = egui::Id::unique("chat-video-sweep");
    if !ctx.data(|d| d.get_temp::<bool>(swept).unwrap_or(false)) {
        ctx.data_mut(|d| d.insert_temp(swept, true));
        ctx.on_end_pass("chat-video-sweep", Arc::new(|_| sweep()));
    }
    let mut opts = video::widget::options(ctx);
    opts.autoplay = true;
    let player = video::Player::new(&rt, ctx, video::Source::Mp4(url.to_string()), opts);
    PLAYING.with(|p| *p.borrow_mut() = Some(Playing { url: url.to_string(), player, drawn: Instant::now() }));
}

/// Stop the video.
pub(crate) fn stop() {
    PLAYING.with(|p| p.borrow_mut().take());
}

fn sweep() {
    PLAYING.with(|p| {
        // Busy: the player's own full-screen window is drawing inside it.
        let Ok(mut p) = p.try_borrow_mut() else { return };
        if p.as_ref().is_some_and(|v| v.drawn.elapsed() > Duration::from_millis(600) && !v.player.fullscreen) {
            *p = None;
        }
    });
}

/// A video in a message: a box with a play button, the player once started.
pub(crate) fn video(ui: &mut Ui, t: &Theme, url: &str) {
    let width = ui.available_width().min(t.dense(400.0));
    if !plays_here(url) {
        // WebM and the like: the browser plays those.
        let r = w::component_frame(t, &t.components.card).inner_margin(egui::Margin::same(t.spacing.sm as i8)).show(ui, |ui| {
            ui.set_width((width - t.spacing.lg).max(0.0));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icons::PLAY).font(w::font(t.dense(28.0))).color(t.text_dim));
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(crate::builtin::friends::strong(t, t!("Video from {site}", site = host(url)), t.text)).truncate());
                    full_address(ui, t, url);
                    if w::button(ui, t, Some(icons::EXTERNAL), t!("Open in browser"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        crate::links::open_http(url);
                    }
                });
            });
        });
        link_menu(&r.response, ui, t, url, None);
        return;
    }
    if !may_load(url) {
        if placeholder(ui, t, url, true) {
            play(ui.ctx(), url);
        }
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(width, (width * 0.5625).round()), Sense::click());
    let shown = PLAYING.with(|p| {
        let mut p = p.borrow_mut();
        match p.as_mut().filter(|v| v.url == url) {
            Some(v) => {
                v.drawn = Instant::now();
                let _ = video::widget::show(ui, t, rect, &mut v.player, "", &host(url));
                if let Some(e) = v.player.error() {
                    ui.painter().rect_filled(rect, t.corner_small(), egui::Color32::from_black_alpha(200));
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, t!("Couldn't play this video: {error}", error = e), w::font(t.typography.body_s.size), t.text_dim);
                }
                true
            }
            None => false,
        }
    });
    if !shown {
        // Not started: a dark box with a play button.
        ui.painter().rect_filled(rect, t.corner_small(), egui::Color32::from_gray(16));
        let hot = resp.hovered();
        let c = rect.center();
        let rad = t.dense(26.0);
        ui.painter().circle_filled(c, rad, if hot { t.accent } else { egui::Color32::from_black_alpha(170) });
        w::icon_centered(ui.painter(), c, icons::PLAY, rad, egui::Color32::WHITE);
        ui.painter().text(rect.left_bottom() + vec2(t.spacing.sm, -t.spacing.sm), Align2::LEFT_BOTTOM, host(url), w::font(t.typography.body_s.size), t.text_dim);
        if resp.clone().on_hover_text(t!("Play video")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            play(ui.ctx(), url);
        }
    }
    link_menu(&resp, ui, t, url, None);
}

// ---------- pictures about to be sent ----------

/// A picture waiting for "Send": its file (picked or dropped) or what was
/// pasted, and the preview.
#[derive(Clone)]
pub(crate) struct Outgoing {
    pub path: Option<PathBuf>,
    pub pasted: Option<Arc<ColorImage>>,
    preview: Option<egui::TextureHandle>,
    tried: bool,
}

impl Outgoing {
    pub(crate) fn file(path: PathBuf) -> Self {
        Self { path: Some(path), pasted: None, preview: None, tried: false }
    }
    pub(crate) fn pasted(image: Arc<ColorImage>) -> Self {
        Self { path: None, pasted: Some(image), preview: None, tried: false }
    }

    /// The file to upload (a pasted picture is saved as PNG first).
    pub(crate) fn into_file(self) -> Result<PathBuf, String> {
        match (self.path, self.pasted) {
            (Some(p), _) => Ok(p),
            (None, Some(img)) => save_png(&img),
            _ => Err("nothing to send".into()),
        }
    }

    fn texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.preview.is_none() && !self.tried {
            self.tried = true;
            let img = match (&self.pasted, &self.path) {
                (Some(img), _) => Some((**img).clone()),
                (None, Some(p)) => decode_small(p),
                _ => None,
            };
            self.preview = img.map(|i| ctx.load_texture("chat-outgoing", i, egui::TextureOptions::LINEAR));
        }
        self.preview.clone()
    }
}

/// A picture with an extension Steam's chat takes.
pub(crate) fn is_picture_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "gif", "webp"].contains(&e.to_ascii_lowercase().as_str()))
}

/// A picture pasted with Ctrl+V or dropped on the chat this frame.
pub(crate) fn incoming(ui: &Ui, pasting: bool, dropping: bool) -> Option<Outgoing> {
    ui.input(|i| {
        if pasting {
            for e in &i.events {
                if let egui::Event::PasteImage(img) = e {
                    return Some(Outgoing::pasted(img.clone()));
                }
            }
        }
        if dropping {
            for f in &i.raw.dropped_files {
                let p = f.path();
                if p.is_absolute() && is_picture_file(p) {
                    return Some(Outgoing::file(p.to_path_buf()));
                }
            }
        }
        None
    })
}

/// Files are being dragged over the window.
pub(crate) fn dragging_files(ui: &Ui) -> bool {
    ui.input(|i| !i.raw.hovered_files.is_empty())
}

/// What "Send this picture?" answered.
pub(crate) enum Answer {
    Send,
    Cancel,
}

/// "Send this picture?" with the picture, Send and Cancel (Esc cancels).
pub(crate) fn confirm(ui: &mut Ui, t: &Theme, out: &mut Outgoing) -> Option<Answer> {
    let screen = ui.ctx().content_rect();
    let mut answer = None;
    let tex = out.texture(ui.ctx());
    let modal = egui::Modal::new(egui::Id::unique("chat-send-picture")).show(ui.ctx(), |ui| {
        let max = vec2((screen.width() - 96.0).clamp(200.0, 560.0), (screen.height() - 220.0).clamp(120.0, 420.0));
        ui.set_width(max.x);
        ui.label(w::heading_s(t, t!("Send this picture?"), t.text));
        ui.add_space(t.spacing.sm);
        match &tex {
            Some(tex) => {
                let s = tex.size_vec2();
                let k = (max.x / s.x).min(max.y / s.y).min(1.0);
                ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), s * k)).corner_radius(t.corner_small()));
            }
            None => {
                let (r, _) = ui.allocate_exact_size(vec2(max.x, max.y * 0.6), Sense::hover());
                ui.painter().rect_filled(r, t.corner_small(), t.surface);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, t!("No preview"), w::font(t.typography.body_m.size), t.text_dim);
            }
        }
        if let Some(name) = out.path.as_ref().and_then(|p| p.file_name()) {
            ui.label(w::body_s(t, name.to_string_lossy(), t.text_dim));
        }
        ui.add_space(t.spacing.md);
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::SEND), t!("Send"), w::Kind::Primary, w::Size::Sm).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                answer = Some(Answer::Send);
            }
            if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                answer = Some(Answer::Cancel);
            }
        });
    });
    if answer.is_none() && modal.should_close() {
        answer = Some(Answer::Cancel);
    }
    answer
}

/// A picture file, at most 960 pixels wide or tall.
pub(crate) fn decode_small(path: &Path) -> Option<ColorImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 << 20);
    let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    reader.limits(limits);
    let img = reader.decode().ok()?.thumbnail(960, 960).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// Save a pasted picture as a PNG in the temp folder (clearing out ones
/// from earlier days), for the upload.
pub(crate) fn save_png(img: &ColorImage) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("steaminrust-chat");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Ok(list) = std::fs::read_dir(&dir) {
        for e in list.flatten() {
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).is_some_and(|d| d > Duration::from_secs(86_400));
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let [w_, h_] = img.size;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let rgba = image::RgbaImage::from_raw(w_ as u32, h_ as u32, bytes).ok_or("the picture's size doesn't match")?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    let path = dir.join(format!("pasted-{stamp}.png"));
    rgba.save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(path)
}

/// A dashed frame over the chat while a file is dragged over the window.
pub(crate) fn drop_hint(ui: &Ui, t: &Theme, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(rect, t.corner(), t.bg.gamma_multiply(0.85));
    p.rect_stroke(rect.shrink(4.0), t.corner(), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, t!("Drop a picture to send it"), w::font(t.typography.body_l.size), t.text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_hosts_load_others_wait() {
        assert_eq!(host("https://User@I.Imgur.com:443/a.png?x"), "i.imgur.com");
        assert!(is_steam("https://images.steamusercontent.com/ugc/1/2/"));
        assert!(is_steam("https://shared.akamai.steamstatic.com/a.jpg"));
        assert!(is_steam("https://steamcdn-a.akamaihd.net/a.jpg"));
        assert!(!is_steam("http://images.steamusercontent.com/ugc/1/2/"));
        assert!(!is_steam("https://steamstatic.com.evil.example/a.jpg"));
        assert!(!is_steam("https://evilsteamstatic.com/a.jpg"));
        assert!(!is_steam("https://i.imgur.com/a.png"));
        assert!(!may_load("https://example.com/never-allowed.png"));
        allow("https://example.com/allowed.png");
        assert!(may_load("https://example.com/allowed.png"));
        assert!(is_video("https://x.example/clip.MP4?t=1") && plays_here("https://x.example/clip.mp4"));
        assert!(is_video("https://x.example/clip.webm") && !plays_here("https://x.example/clip.webm"));
    }

    #[test]
    fn pasted_picture_saves_as_png() {
        let img = ColorImage::from_rgba_unmultiplied([2, 1], &[255, 0, 0, 255, 0, 255, 0, 255]);
        let path = save_png(&img).unwrap();
        let back = image::open(&path).unwrap().to_rgba8();
        assert_eq!((back.width(), back.height()), (2, 1));
        assert_eq!(back.get_pixel(1, 0).0, [0, 255, 0, 255]);
        let _ = std::fs::remove_file(path);
    }
}

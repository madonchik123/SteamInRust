//! Reusable widgets. Everything takes the [`Theme`] explicitly so plugins,
//! extensions and built-in pages all look the same.

use crate::Theme;
use crate::theme::{bold, medium, semibold, type_scale};
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2, load::TexturePoll, pos2, vec2,
};

// ---------- text ----------

pub fn text(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color)
}
pub fn text_medium(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color).family(medium())
}
pub fn text_semibold(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color).family(semibold())
}
pub fn text_bold(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color).family(bold())
}

// The style guide's type scale ([`crate::theme::type_scale`]) as text helpers.

/// Heading Large: 26 px bold.
pub fn heading_l(s: impl Into<String>, color: Color32) -> RichText {
    text_bold(s, type_scale::HEADING_L, color)
}
/// Heading Medium: 22 px bold.
pub fn heading_m(s: impl Into<String>, color: Color32) -> RichText {
    text_bold(s, type_scale::HEADING_M, color)
}
/// Heading Small: 18 px bold.
pub fn heading_s(s: impl Into<String>, color: Color32) -> RichText {
    text_bold(s, type_scale::HEADING_S, color)
}
/// Body Large: 16 px regular.
pub fn body_l(s: impl Into<String>, color: Color32) -> RichText {
    text(s, type_scale::BODY_L, color)
}
/// Body Medium: 14 px regular.
pub fn body_m(s: impl Into<String>, color: Color32) -> RichText {
    text(s, type_scale::BODY_M, color)
}
/// Body Small: 12 px regular.
pub fn body_s(s: impl Into<String>, color: Color32) -> RichText {
    text(s, type_scale::BODY_S, color)
}

/// Height of the app's top bar (Figma `333:2374`, "Rectangle 68").
pub const TOP_BAR_HEIGHT: f32 = 59.0;
/// Height of a page's own bar under it (Figma `375:2459`, "library header").
pub const PAGE_BAR_HEIGHT: f32 = 57.0;

/// Draw an icon glyph centered on `center` by its visible outline (text
/// centering uses the line box, which leaves glyphs off-center). A play
/// triangle is nudged right a little so it looks centered in a circle.
pub fn icon_centered(painter: &egui::Painter, center: egui::Pos2, icon: &str, size: f32, color: Color32) {
    let g = painter.layout_no_wrap(icon.to_string(), font(size), color);
    let b = g.mesh_bounds;
    if !b.is_positive() {
        painter.text(center, Align2::CENTER_CENTER, icon, font(size), color);
        return;
    }
    let nudge = if icon == crate::icons::PLAY { b.width() * 0.08 } else { 0.0 };
    painter.galley(center - b.center().to_vec2() + vec2(nudge, 0.0), g, color);
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn font_medium(size: f32) -> FontId {
    FontId::new(size, medium())
}
pub fn font_semibold(size: f32) -> FontId {
    FontId::new(size, semibold())
}
pub fn font_bold(size: f32) -> FontId {
    FontId::new(size, bold())
}

/// Linear blend between two colors.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

// ---------- painting ----------

/// Four-corner gradient fill (no rounding).
pub fn gradient(painter: &egui::Painter, rect: Rect, a: Color32, b: Color32, horizontal: bool) {
    let mut mesh = egui::Mesh::default();
    let (c0, c1, c2, c3) = if horizontal { (a, b, b, a) } else { (a, a, b, b) };
    mesh.colored_vertex(rect.left_top(), c0);
    mesh.colored_vertex(rect.right_top(), c1);
    mesh.colored_vertex(rect.right_bottom(), c2);
    mesh.colored_vertex(rect.left_bottom(), c3);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// A radial gradient filling `rect`: `inner` at `center`, blending to
/// `outer` at `radius` and beyond (as CSS `radial-gradient`). Drawn as a
/// fine grid of shaded cells, so it stays smooth at any size.
pub fn radial_gradient(painter: &egui::Painter, rect: Rect, center: egui::Pos2, radius: f32, inner: Color32, outer: Color32) {
    if !rect.is_positive() {
        return;
    }
    let (cols, rows) = (32u32, 20u32);
    let mut mesh = egui::Mesh::default();
    let radius = radius.max(1.0);
    for j in 0..=rows {
        for i in 0..=cols {
            let p = pos2(rect.left() + rect.width() * i as f32 / cols as f32, rect.top() + rect.height() * j as f32 / rows as f32);
            let k = ((p - center).length() / radius).clamp(0.0, 1.0);
            mesh.colored_vertex(p, mix(inner, outer, k));
        }
    }
    for j in 0..rows {
        for i in 0..cols {
            let a = j * (cols + 1) + i;
            let b = a + 1;
            let c = a + cols + 1;
            let d = c + 1;
            mesh.add_triangle(a, b, d);
            mesh.add_triangle(a, d, c);
        }
    }
    painter.add(mesh);
}

/// Fade `rect` from transparent (top) to `color` (bottom), starting at `from` (0..1).
pub fn fade_down(ui: &Ui, rect: Rect, color: Color32, from: f32) {
    let top = rect.top() + rect.height() * from;
    let r = Rect::from_min_max(pos2(rect.left(), top), rect.max);
    gradient(&ui.painter_at(rect), r, Color32::TRANSPARENT, color, false);
}

/// Fade from `color` (left) to transparent at `to` (0..1).
pub fn fade_right(ui: &Ui, rect: Rect, color: Color32, to: f32) {
    let r = Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * to, rect.bottom()));
    gradient(&ui.painter_at(rect), r, color, Color32::TRANSPARENT, true);
}

/// How a remote image is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageState {
    Loading,
    Ready,
    /// Failed for good (after a few retries), e.g. the image doesn't exist.
    Missing,
}

/// Pixel sizes still pictures are decoded at (the app's sized image
/// loader): a few steps, so a picture drawn at slightly different sizes
/// shares one texture.
const STEPS: [u32; 11] = [64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048];
const SIZED: &str = "#sir-fit=";

fn step(px: f32) -> Option<u32> {
    STEPS.iter().copied().find(|s| *s as f32 >= px.ceil())
}

/// `url` with the size it's drawn at (`px`, in pixels), so the picture is
/// decoded at that size instead of in full: `<url>#sir-fit=<cover|contain>,<w>x<h>`.
/// Animations, unknown formats and very large draws keep the plain URL.
pub fn sized_url(url: &str, px: Vec2, cover: bool) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    let still = [".jpg", ".jpeg", ".png"].iter().any(|e| path.ends_with(e));
    match (still, step(px.x), step(px.y)) {
        (true, Some(w), Some(h)) if url.starts_with("http") && !url.contains('#') => format!("{url}{SIZED}{},{w}x{h}", if cover { "cover" } else { "contain" }),
        _ => url.to_string(),
    }
}

/// The parts of a [`sized_url`]: (plain URL, cover, width, height).
pub fn parse_sized(uri: &str) -> Option<(&str, bool, u32, u32)> {
    let (url, spec) = uri.split_once(SIZED)?;
    let (mode, size) = spec.split_once(',')?;
    let (w, h) = size.split_once('x')?;
    Some((url, mode == "cover", w.parse().ok()?, h.parse().ok()?))
}

/// Plain URLs known missing for good, and ones asked for (see [`image_state`]).
static SEEN: std::sync::Mutex<Option<std::collections::HashMap<String, ImageState>>> = std::sync::Mutex::new(None);

fn note_state(url: &str, state: ImageState) {
    let mut guard = SEEN.lock().unwrap();
    let map = guard.get_or_insert_with(Default::default);
    if map.get(url) != Some(&state) {
        map.insert(url.to_string(), state);
    }
}

/// Failed loads: url -> (attempts, when the last one failed).
static FAILED: std::sync::Mutex<Option<std::collections::HashMap<String, (u32, f64)>>> = std::sync::Mutex::new(None);
const RETRIES: u32 = 3;

/// Load an image, retrying failed downloads a few times with a pause.
/// Steam's image servers sometimes drop a connection when many images load
/// at once; without this a dropped one would stay blank.
fn load(ui: &Ui, url: &str, size: Vec2, cover: bool) -> (ImageState, Option<egui::load::SizedTexture>) {
    let (state, texture) = load_sized(ui, url, size, cover);
    if !url.is_empty() {
        note_state(url, state);
    }
    (state, texture)
}

fn load_sized(ui: &Ui, url: &str, size: Vec2, cover: bool) -> (ImageState, Option<egui::load::SizedTexture>) {
    if url.is_empty() {
        return (ImageState::Missing, None);
    }
    let uri = sized_url(url, size * ui.ctx().pixels_per_point(), cover);
    let img = egui::Image::new(uri.clone());
    match img.load_for_size(ui.ctx(), size) {
        Ok(TexturePoll::Ready { texture }) => (ImageState::Ready, Some(texture)),
        Ok(TexturePoll::Pending { .. }) => (ImageState::Loading, None),
        Err(_) => {
            let now = ui.input(|i| i.time);
            let mut guard = FAILED.lock().unwrap();
            let map = guard.get_or_insert_with(Default::default);
            let entry = map.entry(url.to_string()).or_insert((0, now));
            if entry.0 >= RETRIES {
                return (ImageState::Missing, None);
            }
            // Wait 2 s, 4 s, 6 s between attempts.
            if now - entry.1 > 2.0 * (entry.0 + 1) as f64 {
                entry.0 += 1;
                entry.1 = now;
                ui.ctx().forget_image(url);
                if uri != url {
                    ui.ctx().forget_image(&uri);
                }
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            (ImageState::Loading, None)
        }
    }
}

/// Whether an image loaded, is loading, or is missing. For a picture drawn
/// before, what that draw found (no extra texture at another size).
pub fn image_state(ui: &Ui, url: &str) -> ImageState {
    if let Some(state) = SEEN.lock().unwrap().as_ref().and_then(|m| m.get(url).copied()) {
        return state;
    }
    load(ui, url, Vec2::splat(64.0), false).0
}

/// Draw a remote image covering `rect` (center-cropped). False while loading.
pub fn paint_cover(ui: &Ui, rect: Rect, url: &str, radius: CornerRadius) -> bool {
    let (_, Some(texture)) = load(ui, url, rect.size(), true) else {
        return false;
    };
    let tex = texture.size;
    let (ta, ra) = (tex.x / tex.y, rect.width() / rect.height());
    let uv = if ta > ra {
        let w = ra / ta;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ta / ra;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    };
    egui::Image::from_texture(texture).uv(uv).corner_radius(radius).paint_at(ui, rect);
    true
}

/// Draw the first of `urls` that exists, covering `rect`. A later URL is only
/// fetched once the earlier ones failed for good, not while they're still
/// loading, so a fallback never competes with the image it stands in for.
pub fn paint_cover_first(ui: &Ui, rect: Rect, urls: &[&str], radius: CornerRadius) -> bool {
    for url in urls {
        match load(ui, url, rect.size(), true) {
            (_, Some(_)) => return paint_cover(ui, rect, url, radius),
            (ImageState::Missing, _) => continue,
            _ => return false,
        }
    }
    false
}

/// Draw a remote image as large as fits in `rect`, anchored by `align`.
pub fn paint_contain(ui: &Ui, rect: Rect, url: &str, align: Align2) -> bool {
    let (_, Some(texture)) = load(ui, url, rect.size(), false) else {
        return false;
    };
    let scale = (rect.width() / texture.size.x).min(rect.height() / texture.size.y);
    let target = align.align_size_within_rect(texture.size * scale, rect);
    egui::Image::from_texture(texture).paint_at(ui, target);
    true
}

/// Animated loading placeholder.
pub fn skeleton(ui: &Ui, t: &Theme, rect: Rect, radius: CornerRadius) {
    let time = ui.input(|i| i.time) as f32;
    let k = (time * 2.2 + rect.left() * 0.004).sin() * 0.5 + 0.5;
    ui.painter().rect_filled(rect, radius, mix(t.surface, t.surface_hover, k));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

/// Placeholder with a title, for apps that have no art.
pub fn placeholder(ui: &Ui, t: &Theme, rect: Rect, title: &str, radius: CornerRadius) {
    let p = ui.painter();
    p.rect_filled(rect, radius, t.surface_hover);
    gradient(&ui.painter_at(rect.shrink(1.0)), rect.shrink(1.0), t.accent.gamma_multiply(0.10), t.accent_2.gamma_multiply(0.05), false);
    let galley = p.layout(title.to_string(), font_semibold(15.0), t.text_dim, rect.width() - 24.0);
    p.galley(rect.center() - galley.size() / 2.0, galley, t.text_dim);
}

/// Image if loaded, shimmer while loading, a plain tile if it's missing.
pub fn image_or_skeleton(ui: &Ui, t: &Theme, rect: Rect, url: &str, radius: CornerRadius) {
    if !paint_cover(ui, rect, url, radius) {
        if image_state(ui, url) == ImageState::Missing {
            placeholder(ui, t, rect, "", radius);
        } else {
            skeleton(ui, t, rect, radius);
        }
    }
}

/// Image if loaded; otherwise a titled placeholder once it's known missing.
pub fn image_or_title(ui: &Ui, t: &Theme, rect: Rect, url: &str, title: &str, radius: CornerRadius) {
    if !paint_cover(ui, rect, url, radius) {
        if image_state(ui, url) == ImageState::Missing {
            placeholder(ui, t, rect, title, radius);
        } else {
            skeleton(ui, t, rect, radius);
        }
    }
}

// ---------- containers ----------

/// A content block: flat, rounded, no border (the library's "Friends who
/// play", "Achievements" and "Activity" boxes, Figma `523:6593`).
pub fn card(t: &Theme) -> Frame {
    Frame::new().fill(t.surface).corner_radius(t.corner()).inner_margin(Margin::same(22))
}

pub fn panel(t: &Theme) -> Frame {
    Frame::new().fill(t.raised).corner_radius(t.corner_small()).inner_margin(Margin::same(12))
}

// ---------- buttons ----------

/// Button styles, each matching a frame of the style guide (Figma page
/// `10:5`). `docs/redesign.md` lists the node IDs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// "primary button" (`84:791`): Color / Secondary, Color / Primary on hover.
    Primary,
    /// "secondary button" (`84:787`): Bg / Highlight, Bg / Hover on hover.
    Secondary,
    /// "tertiary button 2" (`400:13118`): an outline only, firmer on hover.
    Tertiary,
    /// No fill until hovered, for toolbars and dense rows.
    Ghost,
    /// "Play button" (`335:1183`): Accent / Green with dark text.
    Play,
    /// "install button" (`375:4453`): Color / Primary with dark text.
    Install,
    /// "pause button" (`379:8204`): Bg / Hover.
    Pause,
    /// Red, for actions that remove or can't be undone.
    Danger,
}

/// Button heights: 32, 40 and 53 px (53 is the style guide's size).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    Sm,
    Md,
    Lg,
}

/// The one button. `icon` is a glyph from [`crate::icons`] (`icons::PLAY`
/// and `icons::PAUSE` are drawn as solid shapes, like the design's).
pub fn button(ui: &mut Ui, t: &Theme, icon: Option<&str>, label: &str, kind: Kind, size: Size) -> Response {
    let (h, fs, pad) = match size {
        Size::Sm => (32.0, 13.0, 14.0),
        Size::Md => (40.0, 15.0, 20.0),
        Size::Lg => (53.0, type_scale::HEADING_S, 30.0),
    };
    button_sized(ui, t, icon, label, kind, h, fs, pad)
}

/// [`button`] at any height, `fs` the text size and `pad` the space on
/// each side (the main CTA's button is 44 px tall, for example).
#[allow(clippy::too_many_arguments)]
pub fn button_sized(ui: &mut Ui, t: &Theme, icon: Option<&str>, label: &str, kind: Kind, h: f32, fs: f32, pad: f32) -> Response {
    let enabled = ui.is_enabled();
    // Fill, fill on hover, text, text on hover, outline.
    let white = |c: Color32| mix(c, Color32::WHITE, 0.10);
    let (bg, bg_hover, fg, fg_hover, outline) = match kind {
        Kind::Primary => (t.secondary, t.accent, t.text_on(t.secondary), t.text_on(t.secondary), None),
        Kind::Secondary => (t.raised, t.surface_hover, t.text, t.text, None),
        Kind::Tertiary => (Color32::TRANSPARENT, Color32::TRANSPARENT, t.text, t.text, Some((t.text_dim.gamma_multiply(0.2), t.text_dim.gamma_multiply(0.4)))),
        Kind::Ghost => (Color32::TRANSPARENT, t.surface_hover, t.text_dim, t.text, None),
        Kind::Play => (t.play, white(t.play), t.text_on(t.play), t.text_on(t.play), None),
        // The design turns the label light on hover.
        Kind::Install => (t.accent, white(t.accent), t.text_on(t.accent), Color32::from_rgb(0xF3, 0xF3, 0xF3), None),
        Kind::Pause => (t.surface_hover, white(t.tertiary), t.text, t.text, None),
        Kind::Danger => (t.danger.gamma_multiply(0.18), t.danger.gamma_multiply(0.3), t.danger, t.danger, Some((t.danger.gamma_multiply(0.4), t.danger.gamma_multiply(0.6)))),
    };
    let bold_text = matches!(kind, Kind::Primary | Kind::Play | Kind::Install | Kind::Pause);
    let fid = if bold_text { font_bold(fs) } else { font_medium(fs) };
    let galley = ui.painter().layout_no_wrap(label.to_string(), fid, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { fs * 1.1 } else { 0.0 };
    let gap = if icon.is_some() && !label.is_empty() { (fs * 0.55).round() } else { 0.0 };
    let w = if label.is_empty() { h } else { galley.size().x + icon_w + gap + pad * 2.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() && enabled, 0.12);
        let pressed = resp.is_pointer_button_down_on() && enabled;
        let mut fill = mix(bg, bg_hover, hover);
        if pressed {
            fill = mix(fill, Color32::BLACK, 0.14);
        }
        let mut color = mix(fg, fg_hover, hover);
        if !enabled {
            fill = fill.gamma_multiply(0.45);
            color = color.gamma_multiply(0.55);
        }
        let r = t.corner_small();
        let p = ui.painter();
        p.rect_filled(rect, r, fill);
        if let Some((a, b)) = outline {
            p.rect_stroke(rect, r, Stroke::new(1.0, mix(a, b, hover)), StrokeKind::Inside);
        }
        let x0 = rect.center().x - (icon_w + gap + if label.is_empty() { 0.0 } else { galley.size().x }) / 2.0;
        if let Some(i) = icon {
            let c = pos2(x0 + icon_w / 2.0, rect.center().y);
            paint_icon(p, c, i, fs * 1.1, color);
        }
        if !label.is_empty() {
            p.galley(pos2(x0 + icon_w + gap, rect.center().y - galley.size().y / 2.0), galley, color);
        }
    }
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default })
}

/// Draw an icon glyph centered on `c`, `size` px tall. Play and pause are
/// drawn as solid shapes (the font only has outlines).
pub fn paint_icon(p: &egui::Painter, c: egui::Pos2, icon: &str, size: f32, color: Color32) {
    if icon == crate::icons::PLAY {
        // The design's play arrow: a solid triangle, a little right of center.
        let h = size * 0.62;
        let w = h * 0.84;
        let x = c.x - w * 0.42;
        p.add(egui::Shape::convex_polygon(vec![pos2(x, c.y - h / 2.0), pos2(x + w, c.y), pos2(x, c.y + h / 2.0)], color, Stroke::NONE));
    } else if icon == crate::icons::PAUSE {
        let h = size * 0.62;
        let bw = h * 0.3;
        for dx in [-h * 0.25, h * 0.25] {
            p.rect_filled(Rect::from_center_size(pos2(c.x + dx, c.y), vec2(bw, h)), CornerRadius::same(1), color);
        }
    } else {
        icon_centered(p, c, icon, size * 0.9, color);
    }
}

/// The big green Play button (Figma `335:1183`).
pub fn play_button(ui: &mut Ui, t: &Theme, label: &str) -> Response {
    button(ui, t, Some(crate::icons::PLAY), label, Kind::Play, Size::Lg)
}

/// The blue Install button (Figma `375:4453`).
pub fn install_button(ui: &mut Ui, t: &Theme, label: &str) -> Response {
    button(ui, t, Some(crate::icons::DOWNLOAD), label, Kind::Install, Size::Lg)
}

/// The Pause button for a running download (Figma `379:8204`).
pub fn pause_button(ui: &mut Ui, t: &Theme, label: &str) -> Response {
    button(ui, t, Some(crate::icons::PAUSE), label, Kind::Pause, Size::Lg)
}

/// A count badge (Figma `447:3845`): a 16 px circle in the accent color
/// with the number in dark text, centered on `c`.
pub fn badge(p: &egui::Painter, t: &Theme, c: egui::Pos2, n: usize) {
    let label = if n > 99 { "99+".to_string() } else { n.to_string() };
    let g = p.layout_no_wrap(label, font_semibold(11.0), t.text_on(t.accent));
    let w = (g.size().x + 8.0).max(16.0);
    p.rect_filled(Rect::from_center_size(c, vec2(w, 16.0)), CornerRadius::same(8), t.accent);
    p.galley(c - g.size() / 2.0, g, t.text_on(t.accent));
}

/// Square icon button with an optional count badge.
pub fn icon_button(ui: &mut Ui, t: &Theme, icon: &str, tooltip: &str, active: bool, badge_count: usize) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let enabled = ui.is_enabled();
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() && enabled, 0.12);
    let p = ui.painter();
    let bg = if active { t.surface_hover } else { mix(Color32::TRANSPARENT, t.surface_hover, hover) };
    p.rect_filled(rect, t.corner_small(), bg);
    let color = if active { t.text } else { mix(t.text_dim, t.text, hover) };
    let color = if enabled { color } else { color.gamma_multiply(0.45) };
    p.text(rect.center(), Align2::CENTER_CENTER, icon, font(18.0), color);
    if badge_count > 0 {
        badge(p, t, rect.right_top() + vec2(-7.0, 7.0), badge_count);
    }
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The top bar's Friends and Notifications buttons (Figma `447:3849`,
/// `766:10160`): a 56×37 block in Bg / Highlight with a dim icon and a
/// count badge on its top-right corner.
pub fn bar_button(ui: &mut Ui, t: &Theme, icon: &str, tooltip: &str, active: bool, badge_count: usize) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(56.0, 37.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    let fill = if active { t.surface_hover } else { mix(t.raised, t.surface_hover, hover) };
    p.rect_filled(rect, t.corner_small(), fill);
    icon_centered(p, rect.center(), icon, 22.0, if active { t.text } else { mix(t.text_dim, t.text, hover) });
    if badge_count > 0 {
        badge(p, t, pos2(rect.right() - 5.0, rect.top() + 6.0), badge_count);
    }
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small square button with an icon (Figma "icon box" `280:175` and
/// "icon select" `346:2627`): 40 px, a faint fill, the icon in Text / Dim.
/// For view switches and filters in a page's bar, and the actions row under
/// a game's art. `selected` marks the view that's showing.
pub fn icon_box(ui: &mut Ui, t: &Theme, icon: &str, tooltip: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    let k = if selected { 0.16 } else { 0.05 + 0.07 * hover };
    p.rect_filled(rect, t.corner_small(), t.text_dim.gamma_multiply(k));
    let color = if selected { t.text } else { mix(t.text_dim, t.text, hover) };
    icon_centered(p, rect.center(), icon, 22.0, color);
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Draw the filled heart, `size` px wide, centered on `c`.
pub fn paint_heart(p: &egui::Painter, c: egui::Pos2, size: f32, color: Color32) {
    // Two round lobes and a point, each drawn with smooth edges (a filled
    // curve had jagged, clipped-looking edges).
    let k = size / 18.0;
    let (r, d, tip) = (4.7 * k, 3.5 * k, 9.6 * k);
    // Centered: the shape spans from `-r` (lobe tops) to `tip`.
    let o = c - vec2(0.0, (tip - r) / 2.0 - 0.5);
    let left = o + vec2(-d, 0.0);
    let right = o + vec2(d, 0.0);
    let point = o + vec2(0.0, tip);
    // Where the line from the point touches each lobe, on its outer side.
    let touch = |c: egui::Pos2, side: f32| {
        let v = point - c;
        let a = (r / v.length()).acos() * side;
        let u = v.normalized();
        c + vec2(u.x * a.cos() - u.y * a.sin(), u.x * a.sin() + u.y * a.cos()) * r
    };
    p.circle_filled(left, r, color);
    p.circle_filled(right, r, color);
    p.add(egui::Shape::convex_polygon(vec![point, touch(left, 1.0), left, right, touch(right, -1.0)], color, Stroke::NONE));
}

/// The favorite button: an outlined heart, filled in red once the game is a
/// favorite, so you can see which it is at a glance.
pub fn heart_button(ui: &mut Ui, t: &Theme, on: bool, tooltip: &str) -> Response {
    if !on {
        return icon_button(ui, t, crate::icons::HEART, tooltip, false, 0);
    }
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect_filled(rect, t.corner_small(), t.surface_hover);
    paint_heart(p, rect.center(), 18.0 * (1.0 + 0.06 * hover), t.danger);
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The Wishlist button (Figma "wishlist" `280:345` and "wishlist page"
/// `302:1745`): an outlined heart on Bg / Highlight; once wishlisted, a
/// solid heart on Bg / Hover, turning red on hover to say a click removes
/// it. `label` adds text after the heart ("Wishlist", "Wishlisted").
pub fn wishlist_button(ui: &mut Ui, t: &Theme, wishlisted: bool, label: Option<&str>) -> Response {
    let h = 53.0;
    let galley = label.map(|l| ui.painter().layout_no_wrap(l.to_string(), font(type_scale::BODY_L), Color32::PLACEHOLDER));
    let w = match &galley {
        Some(g) => 19.0 + 24.0 + 10.0 + g.size().x + 22.0,
        None => 62.0,
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
        let fill = match wishlisted {
            false => mix(t.raised, t.surface_hover, hover),
            true => mix(t.surface_hover, t.danger, hover),
        };
        let p = ui.painter();
        p.rect_filled(rect, t.corner_small(), fill);
        let heart_c = if galley.is_some() { pos2(rect.left() + 19.0 + 12.0, rect.center().y) } else { rect.center() };
        if wishlisted {
            paint_heart(p, heart_c, 20.0, t.text);
        } else {
            icon_centered(p, heart_c, crate::icons::HEART, 22.0, t.text);
        }
        if let Some(g) = galley {
            p.galley(pos2(heart_c.x + 12.0 + 10.0, rect.center().y - g.size().y / 2.0), g, t.text);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row in a dropdown menu (Figma "Menu items" `18:248`): icon and label
/// across the menu's width, Bg / Hover behind the row under the pointer.
pub fn menu_item(ui: &mut Ui, t: &Theme, icon: &str, label: &str) -> Response {
    menu_item_tinted(ui, t, icon, label, None)
}

/// [`menu_item`] in its own color, e.g. red for "Uninstall…".
pub fn menu_item_tinted(ui: &mut Ui, t: &Theme, icon: &str, label: &str, tint: Option<Color32>) -> Response {
    let fs = 14.5;
    // As wide as the text needs, and the menu's set width; a right-click
    // menu with no width set offers ~400 px, which made every row that wide.
    let text_w = ui.painter().layout_no_wrap(label.to_string(), font(fs), Color32::PLACEHOLDER).size().x;
    let avail = ui.available_width();
    let width = (44.0 + text_w + 28.0).max(if avail > 360.0 { 220.0 } else { avail });
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.08);
        let p = ui.painter();
        if hover > 0.0 {
            p.rect_filled(rect, t.corner_small(), t.surface_hover.gamma_multiply(hover));
        }
        let y = rect.center().y;
        let (icon_c, text_c) = match tint {
            Some(c) => (c, c),
            None => (mix(t.text_dim, t.text, hover), t.text),
        };
        p.text(pos2(rect.left() + 20.0, y), egui::Align2::CENTER_CENTER, icon, font(15.0), icon_c);
        p.text(pos2(rect.left() + 40.0, y), egui::Align2::LEFT_CENTER, label, font(fs), text_c);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A menu row that opens a submenu (open it with egui's `SubMenu`): the
/// same look as [`menu_item`], with an arrow on the right.
pub fn submenu_item(ui: &mut Ui, t: &Theme, icon: &str, label: &str) -> Response {
    let r = menu_item(ui, t, icon, label);
    ui.painter().text(pos2(r.rect.right() - 12.0, r.rect.center().y), egui::Align2::CENTER_CENTER, crate::icons::CARET_RIGHT, font(12.0), t.text_faint);
    r
}

/// An all-caps tab: dim, brighter on hover, in the accent color once
/// selected (the top bar's look, at a smaller size for in-page tabs).
pub fn nav_tab(ui: &mut Ui, t: &Theme, label: &str, selected: bool, height: f32) -> Response {
    tab_caps(ui, t, label, selected, height, 13.0, 24.0)
}

/// A main section in the top bar (Figma `18:294` "store menu"): bold
/// all-caps 18 px, Text / Main, Color / Primary when it's the open one.
pub fn section_tab(ui: &mut Ui, t: &Theme, label: &str, selected: bool, height: f32) -> Response {
    section_tab_sized(ui, t, label, selected, height, 17.0)
}

/// [`section_tab`] at another text size (the top bar uses 14 on narrow windows).
pub fn section_tab_sized(ui: &mut Ui, t: &Theme, label: &str, selected: bool, height: f32, size: f32) -> Response {
    tab_caps(ui, t, label, selected, height, size, 0.0)
}

fn tab_caps(ui: &mut Ui, t: &Theme, label: &str, selected: bool, height: f32, fs: f32, pad: f32) -> Response {
    // Laid out without a color, so the state color below applies.
    let galley = ui.painter().layout_no_wrap(label.to_uppercase(), font_bold(fs), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + pad, height), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let sel = ui.ctx().animate_bool_with_time(resp.id.with("sel"), selected, 0.18);
    // The top bar's sections are white until opened; smaller tabs start dim.
    let rest = if pad == 0.0 { mix(t.text, t.text_dim, 0.12) } else { t.text_dim };
    let color = mix(mix(rest, t.text, hover), t.accent, sel);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A tab in a page's bar (Figma "menu" `82:322`): 45 px tall, 16 px text,
/// Color / Secondary behind the selected one, a light wash on hover.
pub fn sub_tab(ui: &mut Ui, t: &Theme, label: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), font(type_scale::BODY_L), Color32::PLACEHOLDER);
    // 29 px each side in the design; less when the bar is short of room.
    let pad = if ui.available_width() > 900.0 { 29.0 } else { 18.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + pad * 2.0, 45.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
        let fill = if selected { t.secondary } else { Color32::from_white_alpha(25).gamma_multiply(hover) };
        ui.painter().rect_filled(rect, t.corner_small(), fill);
        let color = if selected { t.text_on(t.secondary) } else { t.text };
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Rounded filter chip.
pub fn chip(ui: &mut Ui, t: &Theme, label: &str, selected: bool) -> Response {
    tinted_chip(ui, t, label, None, selected)
}

/// A chip in its own color, e.g. an item rarity: a colored dot and text,
/// and a tinted fill when selected.
pub fn tinted_chip(ui: &mut Ui, t: &Theme, label: &str, tint: Option<Color32>, selected: bool) -> Response {
    // The style guide's tag shape (`280:203`): a 30 px block, 13 px each
    // side; Bg / Highlight, Bg / Hover once selected.
    let galley = ui.painter().layout_no_wrap(label.to_string(), font(13.0), Color32::PLACEHOLDER);
    let dot = if tint.is_some() { 14.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 26.0 + dot, 30.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    let r = t.corner_small();
    if selected {
        p.rect_filled(rect, r, t.surface_hover);
        if let Some(c) = tint {
            p.rect_stroke(rect, r, Stroke::new(1.0, c.gamma_multiply(0.8)), StrokeKind::Inside);
        }
    } else {
        p.rect_filled(rect, r, mix(t.raised, t.surface_hover, hover * 0.6));
        if let Some(c) = tint {
            p.rect_stroke(rect, r, Stroke::new(1.0, c.gamma_multiply(0.35)), StrokeKind::Inside);
        }
    }
    let color = match (tint, selected) {
        (Some(c), _) => mix(c, t.text, if selected { 0.25 } else { 0.1 + hover * 0.2 }),
        (None, true) => t.text,
        (None, false) => mix(t.text_dim, t.text, hover),
    };
    if let Some(c) = tint {
        p.circle_filled(egui::pos2(rect.left() + 16.0, rect.center().y), 4.0, c);
    }
    let pos = egui::pos2(rect.left() + 13.0 + dot, rect.center().y - galley.size().y / 2.0);
    p.galley(pos, galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An on/off chip with a checkmark, for quick filters.
pub fn toggle_chip(ui: &mut Ui, t: &Theme, label: &str, on: &mut bool) -> Response {
    let text = if *on { format!("{}  {label}", crate::icons::CHECK) } else { label.to_string() };
    let mut r = chip(ui, t, &text, *on);
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r
}

/// Small non-interactive label pill.
pub fn pill(ui: &mut Ui, text_: &str, fg: Color32, bg: Color32) {
    let galley = ui.painter().layout_no_wrap(text_.to_string(), font_semibold(11.0), fg);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
}

/// A row of choices in one dark block, the selected one lit (the "sorting"
/// bar's look, Figma `457:12113`, compact). Returns true if it changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(t.chrome)
        .corner_radius(t.corner_small())
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (opt, label) in options {
                    let on = *value == *opt;
                    let galley = ui.painter().layout_no_wrap(label.to_string(), font(13.0), Color32::PLACEHOLDER);
                    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, 26.0), Sense::click());
                    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                    if on {
                        ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
                    } else if hover > 0.0 {
                        ui.painter().rect_filled(rect, t.corner_small(), t.raised.gamma_multiply(hover));
                    }
                    let c = if on { t.text } else { mix(t.text_dim, t.text, hover * 0.6) };
                    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, c);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                        *value = *opt;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// The search field (Figma "Search" `280:57`), 34 px tall: a dark block,
/// the hint and magnifier in Color / Secondary, a clear button once typed in.
pub fn search(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32) -> Response {
    search_sized(ui, t, value, hint, width, 34.0)
}

/// [`search`] at the page bar's height (41 px), as in the store and
/// library bars.
pub fn bar_search(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32) -> Response {
    search_sized(ui, t, value, hint, width, 41.0)
}

fn search_sized(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32, height: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(60.0), height), Sense::hover());
    let r = t.corner_small();
    ui.painter().rect_filled(rect, r, t.chrome.gamma_multiply(0.4));
    let clear_w = 30.0;
    let field = Rect::from_min_max(pos2(rect.left() + 16.0, rect.top()), pos2(rect.right() - clear_w, rect.bottom()));
    // `place`, not `put`: `put` would end the row at the text field,
    // so the next widget overlapped the box's right end.
    let resp = ui.place(
        field,
        egui::TextEdit::singleline(value)
            .hint_text(text(hint, 14.0, hint_color(t)))
            .font(font(14.0))
            .frame(Frame::NONE)
            .margin(vec2(0.0, 0.0))
            .align(Align2::LEFT_CENTER)
            .desired_width(field.width()),
    ).with_edit_menu();
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, r, Stroke::new(1.0, t.accent.gamma_multiply(0.6)), StrokeKind::Inside);
    }
    let c = pos2(rect.right() - 18.0, rect.center().y);
    if value.is_empty() {
        ui.painter().text(c, Align2::CENTER_CENTER, crate::icons::SEARCH, font(16.0), hint_color(t));
    } else {
        let x = ui.interact(Rect::from_center_size(c, Vec2::splat(22.0)), resp.id.with("clear"), Sense::click());
        ui.painter().text(c, Align2::CENTER_CENTER, crate::icons::X, font(13.0), if x.hovered() { t.text } else { t.text_dim });
        if x.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            value.clear();
        }
    }
    resp
}

/// The search hint's color: Color / Secondary, lifted a little on themes
/// where it would sink into the field.
fn hint_color(t: &Theme) -> Color32 {
    mix(t.secondary, t.text_dim, 0.25)
}

/// The states of a [`tag`] (Figma "tag" `280:203`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TagState {
    /// A game's tag: Bg / Highlight, dim text.
    Plain,
    /// A filter you can turn on: no fill, white text, the count dim.
    Unchecked,
    /// A filter that's on: Bg / Hover.
    Checked,
    /// The tag of the page you're on: Bg / Highlight, white text.
    Current,
    /// A filter in use, shown with a × to remove it: Bg / Hover.
    Filtered,
}

/// A tag (Figma "tag" `280:203`): 30 px tall, 12 px text, an optional count
/// after it ("5,000"), and a × for [`TagState::Filtered`].
pub fn tag(ui: &mut Ui, t: &Theme, label: &str, count: Option<&str>, state: TagState) -> Response {
    let fs = type_scale::BODY_S;
    let g = ui.painter().layout_no_wrap(label.to_string(), font(fs), Color32::PLACEHOLDER);
    let cg = count.map(|c| ui.painter().layout_no_wrap(c.to_string(), font(fs), Color32::PLACEHOLDER));
    let extra = match (&cg, state) {
        (_, TagState::Filtered) => 10.0 + 8.0,
        (Some(c), _) => 10.0 + c.size().x,
        (None, _) => 0.0,
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(13.0 + g.size().x + extra + 13.0, 30.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
        let (fill, fg) = match state {
            TagState::Plain => (mix(t.raised, t.surface_hover, hover), mix(t.text_dim, t.text, hover)),
            TagState::Unchecked => (t.raised.gamma_multiply(hover), t.text),
            TagState::Checked | TagState::Filtered => (t.surface_hover, t.text),
            TagState::Current => (t.raised, t.text),
        };
        let p = ui.painter();
        p.rect_filled(rect, t.corner_small(), fill);
        let y = rect.center().y;
        p.galley(pos2(rect.left() + 13.0, y - g.size().y / 2.0), g.clone(), fg);
        let x = rect.left() + 13.0 + g.size().x + 10.0;
        if state == TagState::Filtered {
            let c = pos2(x + 3.0, y);
            let s = Stroke::new(1.2, mix(t.text_dim, t.text, hover));
            p.line_segment([c - vec2(3.0, 3.0), c + vec2(3.0, 3.0)], s);
            p.line_segment([c + vec2(-3.0, 3.0), c + vec2(3.0, -3.0)], s);
        } else if let Some(c) = cg {
            p.galley(pos2(x, y - c.size().y / 2.0), c, t.text_dim.gamma_multiply(0.8));
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An on/off switch (Figma "toggle" `477:7301`): 63×30, the knob in
/// Color / Primary on Bg / Tertiary when on, Bg / Hover on Bg / Highlight
/// when off. Flips `on` when clicked.
pub fn toggle(ui: &mut Ui, t: &Theme, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(63.0, 30.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let k = ui.ctx().animate_bool_with_time(resp.id, *on, 0.14);
        let hover = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.1);
        let p = ui.painter();
        let track = mix(t.raised, t.tertiary, k);
        p.rect_filled(rect, CornerRadius::same(15), mix(track, Color32::WHITE, 0.04 * hover));
        let x = rect.left() + 15.0 + (rect.width() - 30.0) * k;
        p.circle_filled(pos2(x, rect.center().y), 10.0, mix(t.surface_hover, t.accent, k));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A [`toggle`] with a label before it, the switch at the row's right end.
pub fn toggle_row(ui: &mut Ui, t: &Theme, on: &mut bool, label: &str) -> Response {
    ui.horizontal(|ui| {
        ui.label(body_m(label, t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| toggle(ui, t, on)).inner
    })
    .inner
}

/// One choice in a "Sort by" row (Figma "sort filter" `296:579`): 39 px,
/// 16 px dim text on Bg / Main; Bg / Highlight on hover; Bg / Hover with
/// white text once selected.
pub fn sort_chip(ui: &mut Ui, t: &Theme, label: &str, selected: bool) -> Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), font(type_scale::BODY_L), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 40.0, 39.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
        let fill = if selected { t.surface_hover } else { mix(t.chrome, t.raised, hover) };
        ui.painter().rect_filled(rect, t.corner_small(), fill);
        ui.painter().galley(rect.center() - g.size() / 2.0, g, if selected { t.text } else { t.text_dim });
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "Sort by" and a row of [`sort_chip`]s. Returns true if the choice changed.
pub fn sort_by<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(body_l(i18n::t!("Sort by"), t.text_dim));
        ui.add_space(5.0);
        for (opt, label) in options {
            if sort_chip(ui, t, label, *value == *opt).clicked() && *value != *opt {
                *value = *opt;
                changed = true;
            }
        }
    });
    changed
}

/// A dropdown's button: the [`sort_chip`] look with a caret, at least
/// `min_width` wide. Open a menu from it with `egui::Popup::menu(&response)`
/// (its rows are [`menu_item`]s).
pub fn dropdown(ui: &mut Ui, t: &Theme, label: &str, min_width: f32) -> Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), font(type_scale::BODY_L), Color32::PLACEHOLDER);
    let w = (20.0 + g.size().x + 12.0 + 12.0 + 16.0).max(min_width);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 39.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
        let fill = if open { t.surface_hover } else { mix(t.chrome, t.raised, hover) };
        let p = ui.painter();
        p.rect_filled(rect, t.corner_small(), fill);
        let y = rect.center().y;
        p.galley(pos2(rect.left() + 20.0, y - g.size().y / 2.0), g, if open { t.text } else { mix(t.text_dim, t.text, hover) });
        let caret = if open { crate::icons::CARET_UP } else { crate::icons::CARET_DOWN };
        p.text(pos2(rect.right() - 20.0, y), Align2::CENTER_CENTER, caret, font(13.0), t.text_dim);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A review score bar (Figma "reviews" `289:270`), `width` wide and 23 px
/// tall: the positive share in Color / Primary and the negative share in
/// Accent / Red, with their percentages above. `positive` is 0–100.
pub fn review_bar(ui: &mut Ui, t: &Theme, positive: u8, width: f32) -> Response {
    let positive = positive.min(100);
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(60.0), 23.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fs = type_scale::BODY_S;
        let up = format!("{positive}%");
        let down = format!("{}%", 100 - positive);
        let gu = p.layout_no_wrap(up, font(fs), t.accent);
        let gd = p.layout_no_wrap(down, font(fs), t.danger);
        let gu_w = gu.size().x;
        p.galley(rect.left_top(), gu, t.accent);
        p.text(pos2(rect.left() + gu_w + 3.0, rect.top()), Align2::LEFT_TOP, crate::icons::THUMBS_UP, font(fs), t.accent.gamma_multiply(0.4));
        let thumb_w = 15.0;
        p.text(pos2(rect.right(), rect.top()), Align2::RIGHT_TOP, crate::icons::THUMBS_DOWN, font(fs), t.danger.gamma_multiply(0.4));
        p.galley(pos2(rect.right() - thumb_w - gd.size().x, rect.top()), gd, t.danger);
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 4.0), rect.right_bottom());
        let split = bar.left() + (bar.width() - 2.0) * positive as f32 / 100.0;
        let r = CornerRadius::same(2);
        if positive > 0 {
            p.rect_filled(Rect::from_min_max(bar.min, pos2(split, bar.bottom())), r, t.accent.gamma_multiply(0.8));
        }
        if positive < 100 {
            p.rect_filled(Rect::from_min_max(pos2(split + 2.0, bar.top()), bar.max), r, t.danger.gamma_multiply(0.8));
        }
    }
    resp
}

/// A small download ring (Figma "download status" `383:10133`): Bg / Hover
/// track, Color / Primary arc for `fraction` (0–1), centered on `c`.
pub fn progress_ring(p: &egui::Painter, t: &Theme, c: egui::Pos2, radius: f32, fraction: f32) {
    p.circle_filled(c, radius, t.surface_hover);
    let f = fraction.clamp(0.0, 1.0);
    if f <= 0.0 {
        return;
    }
    // A pie slice from 12 o'clock, clockwise.
    let steps = (48.0 * f).ceil().max(2.0) as usize;
    let mut pts = vec![c];
    for i in 0..=steps {
        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * f * i as f32 / steps as f32;
        pts.push(c + vec2(a.cos(), a.sin()) * radius);
    }
    let mut mesh = egui::Mesh::default();
    for q in &pts {
        mesh.colored_vertex(*q, t.accent);
    }
    for i in 1..pts.len() - 1 {
        mesh.add_triangle(0, i as u32, i as u32 + 1);
    }
    p.add(mesh);
}

/// Labeled text input used on forms.
pub fn field(ui: &mut Ui, t: &Theme, label: &str, value: &mut String, password: bool) -> Response {
    ui.label(text_medium(label, 12.5, t.text_dim));
    ui.add_space(-2.0);
    ui.add(
        egui::TextEdit::singleline(value)
            .password(password)
            .font(font(15.0))
            .desired_width(f32::INFINITY)
            .margin(vec2(12.0, 10.0)),
    ).with_edit_menu()
}

// ---------- people ----------

/// Round avatar with an optional presence dot.
pub fn avatar(ui: &mut Ui, t: &Theme, url: &str, size: f32, status: Option<Color32>) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let radius = CornerRadius::same((size / 2.0) as u8);
    if !paint_cover(ui, rect, url, radius) {
        ui.painter().circle_filled(rect.center(), size / 2.0, t.surface_hover);
    }
    if let Some(c) = status {
        let r = (size * 0.14).max(4.0);
        let center = rect.right_bottom() - Vec2::splat(r * 0.9);
        ui.painter().circle_filled(center, r + 2.0, t.chrome);
        ui.painter().circle_filled(center, r, c);
    }
    resp
}

// ---------- games ----------

/// Library capsule (2:3) with hover lift and installed marker.
pub fn game_tile(ui: &mut Ui, t: &Theme, url: &str, fallback_url: &str, title: &str, installed: bool, size: Vec2) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let r = t.corner_small();
    let img = rect.expand(hover * 3.0);
    if hover > 0.0 {
        ui.painter().rect_filled(img.translate(vec2(0.0, 8.0 * hover)), r, Color32::from_black_alpha((90.0 * hover) as u8));
    }
    if !paint_cover_first(ui, img, &[url, fallback_url], r) {
        placeholder(ui, t, img, title, r);
    }
    let p = ui.painter();
    if !installed {
        p.rect_filled(img, r, Color32::from_black_alpha(90));
    }
    if hover > 0.0 {
        p.rect_stroke(img, r, Stroke::new(1.5, t.accent.gamma_multiply(hover)), StrokeKind::Outside);
    }
    if installed {
        let c = img.left_bottom() + vec2(14.0, -14.0);
        p.circle_filled(c, 7.0, Color32::from_black_alpha(160));
        p.circle_filled(c, 4.0, t.play);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A compact price for cards and rows (36 px): the price alone, or the
/// style guide's discount look (Figma "price" `426:4419`) at a smaller size.
pub fn price_tag(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8) {
    price_block(ui, t, label, original, discount, 36.0, 14.0);
}

/// The price block at the style guide's size (Figma "price" `426:4419`,
/// 44 px): the price in bold 18 px; with a discount, a green "-34%" block,
/// then the old price struck through above the new price in green.
pub fn price(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8) {
    price_block(ui, t, label, original, discount, 44.0, type_scale::HEADING_S);
}

fn price_block(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8, h: f32, fs: f32) {
    if discount == 0 {
        let g = ui.painter().layout_no_wrap(label.to_string(), font_bold(fs), t.text);
        let (rect, _) = ui.allocate_exact_size(vec2(g.size().x, h), Sense::hover());
        ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.text);
        return;
    }
    // The design's numbers at 44 px: block 75×39 with 19 px text; prices 18 px.
    let k = h / 44.0;
    let pct = ui.painter().layout_no_wrap(format!("-{discount}%"), font_bold(fs * 1.054), t.text_on(t.discount));
    let new_color = if t.dark { mix(t.discount, t.play, 0.5) } else { t.play };
    let new = ui.painter().layout_no_wrap(label.to_string(), font_bold(fs), new_color);
    let old = original.map(|o| ui.painter().layout_no_wrap(o.to_string(), font_bold(fs * 0.78), t.text_dim));
    let block_w = (pct.size().x + 24.0 * k).max(75.0 * k);
    let gap = 10.0 * k;
    let price_w = new.size().x.max(old.as_ref().map(|g| g.size().x).unwrap_or(0.0));
    let (rect, _) = ui.allocate_exact_size(vec2(block_w + gap + price_w, h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter();
    let block = Rect::from_min_size(pos2(rect.left(), rect.center().y - 19.5 * k), vec2(block_w, 39.0 * k));
    p.rect_filled(block, CornerRadius::same(2), t.discount);
    p.galley(block.center() - pct.size() / 2.0, pct, t.text_on(t.discount));
    let x = block.right() + gap;
    match old {
        Some(old) => {
            let top = pos2(x, rect.top());
            let strike_y = top.y + old.size().y / 2.0;
            let old_w = old.size().x;
            p.galley(top, old, t.text_dim);
            p.line_segment([pos2(top.x, strike_y), pos2(top.x + old_w, strike_y)], Stroke::new(1.0, t.text_dim));
            p.galley(pos2(x, rect.bottom() - new.size().y), new, new_color);
        }
        None => {
            p.galley(pos2(x, rect.center().y - new.size().y / 2.0), new, new_color);
        }
    }
}

/// The store page's buy row (Figma "main-cta" `426:4355`): an optional
/// "Until October 19" note, the [`price`] block, and a 44 px primary button
/// (`button_label`, e.g. "Add to Cart"). Returns the button's response.
pub fn main_cta(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8, until: Option<&str>, button_label: &str) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 13.0;
        if let Some(u) = until.filter(|_| discount > 0) {
            ui.vertical(|ui| {
                ui.add_space(24.0);
                ui.label(body_s(u, t.text_dim));
            });
        }
        price(ui, t, label, original, discount);
        button_sized(ui, t, None, button_label, Kind::Primary, 44.0, type_scale::HEADING_S, 26.0)
    })
    .inner
}

// ---------- layout helpers ----------

/// A centered column at most `max` wide with at least `pad` on each side.
pub fn centered<R>(ui: &mut Ui, max: f32, pad: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let avail = ui.available_width();
    let width = (avail - pad * 2.0).min(max).max(100.0);
    let left = ((avail - width) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(left);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            add(ui)
        })
        .inner
    })
    .inner
}

pub fn page_title(ui: &mut Ui, t: &Theme, title: &str, subtitle: Option<&str>) {
    ui.label(text_bold(title, 28.0, t.text));
    if let Some(s) = subtitle {
        ui.label(text(s, 14.0, t.text_dim));
    }
    ui.add_space(14.0);
}

/// Section header; returns true if the action link was clicked.
pub fn section(ui: &mut Ui, t: &Theme, title: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(text_bold(title, 18.0, t.text));
        if let Some(a) = action {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add(egui::Label::new(text_medium(a, 13.0, t.text_dim)).sense(Sense::click()));
                if r.hovered() {
                    ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, t.text_dim));
                }
                clicked = r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            });
        }
    });
    ui.add_space(6.0);
    clicked
}

/// Small caps label used to group lists.
pub fn caption(ui: &mut Ui, t: &Theme, s: &str) {
    ui.label(caption_text(t, s));
}

pub fn caption_text(t: &Theme, s: &str) -> RichText {
    text_bold(s.to_uppercase(), 11.0, t.text_faint)
}

pub fn stat(ui: &mut Ui, t: &Theme, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(text_bold(label.to_uppercase(), 10.5, t.text_faint));
        ui.label(text_semibold(value, 15.0, t.text));
    });
}

pub fn loading(ui: &mut Ui, t: &Theme, label: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        ui.add(egui::Spinner::new().size(28.0).color(t.accent));
        ui.add_space(10.0);
        ui.label(text(label, 14.0, t.text_dim));
    });
}

pub fn empty(ui: &mut Ui, t: &Theme, icon: &str, title: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(70.0);
        ui.label(text(icon, 44.0, t.text_faint));
        ui.add_space(8.0);
        ui.label(text_semibold(title, 18.0, t.text));
        ui.label(text(body, 14.0, t.text_dim));
    });
}

/// Thin progress bar.
pub fn progress(ui: &mut Ui, t: &Theme, value: f32, width: f32) {
    // Never negative: egui aborts the whole app on a negative size.
    let width = width.max(0.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(3), t.surface_hover);
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * value.clamp(0.0, 1.0), rect.height()));
    ui.painter().rect_filled(fill, CornerRadius::same(3), t.accent);
}

/// The app's mark for its own bars and menus: the cut-out logo in one color
/// (white on dark themes, black on light ones), drawn sharp at any size.
pub fn logo(ui: &mut Ui, t: &Theme, size: f32) -> Response {
    use crate::brand::Logo;
    logo_as(ui, if t.dark { Logo::CutoutWhite } else { Logo::CutoutBlack }, size)
}

/// The app icon (the violet logo), for where the app introduces itself
/// (sign-in, About, updates).
pub fn app_icon(ui: &mut Ui, size: f32) -> Response {
    logo_as(ui, crate::brand::Logo::Purple, size)
}

/// One version of the logo, drawn sharp at any size.
pub fn logo_as(ui: &mut Ui, logo: crate::brand::Logo, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    logo.image().paint_at(ui, rect);
    resp
}

// ---------- choices ----------

/// Indicator + label row shared by [`radio`] and [`check`].
fn choice_row(ui: &mut Ui, label: impl Into<egui::WidgetText>, paint: impl FnOnce(&egui::Painter, Rect, f32)) -> Response {
    let galley = label.into().into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
    let size = vec2(18.0 + 10.0 + galley.size().x, galley.size().y.max(18.0) + 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let mark = Rect::from_center_size(egui::pos2(rect.left() + 9.0, rect.center().y), Vec2::splat(18.0));
    paint(ui.painter(), mark, hover);
    let color = ui.visuals().text_color();
    ui.painter().galley(egui::pos2(mark.right() + 10.0, rect.center().y - galley.size().y / 2.0), galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A clearly visible radio button, ringed like the [`check`] box.
pub fn radio(ui: &mut Ui, t: &Theme, selected: bool, label: impl Into<egui::WidgetText>) -> Response {
    choice_row(ui, label, |p, r, hover| {
        let ring = if selected { t.accent } else { mix(t.surface_hover, t.text_dim, hover) };
        p.circle_filled(r.center(), 9.0, t.raised);
        p.circle_stroke(r.center(), 8.5, Stroke::new(1.6, ring));
        if selected {
            p.circle_filled(r.center(), 4.5, t.accent);
        }
    })
}

/// Radio button that sets `current` to `value` when clicked.
pub fn radio_value<V: PartialEq>(ui: &mut Ui, t: &Theme, current: &mut V, value: V, label: impl Into<egui::WidgetText>) -> Response {
    let mut r = radio(ui, t, *current == value, label);
    if r.clicked() && *current != value {
        *current = value;
        r.mark_changed();
    }
    r
}

/// A check box with its label (Figma "Check box" `757:10005`): an outline
/// in Bg / Hover, Text / Dim on hover, and a Text / Dim box with a dark
/// check mark when on.
pub fn check(ui: &mut Ui, t: &Theme, on: &mut bool, label: impl Into<egui::WidgetText>) -> Response {
    let state = *on;
    let mut r = choice_row(ui, label, |p, r, hover| paint_check(p, t, r, state, hover));
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r
}

/// The check box alone (no label), `size` px square. Flips `on` when clicked.
pub fn checkbox(ui: &mut Ui, t: &Theme, on: &mut bool, size: f32) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    paint_check(ui.painter(), t, rect.shrink(size * 0.08), *on, hover);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_check(p: &egui::Painter, t: &Theme, r: Rect, on: bool, hover: f32) {
    let rr = CornerRadius::same(3);
    if on {
        p.rect_filled(r, rr, mix(t.text_dim, t.text, hover * 0.3));
        let s = Stroke::new((r.width() * 0.11).max(1.8), t.chrome);
        let (w, h) = (r.width(), r.height());
        let a = pos2(r.left() + w * 0.22, r.top() + h * 0.52);
        let b = pos2(r.left() + w * 0.42, r.top() + h * 0.72);
        let c = pos2(r.left() + w * 0.78, r.top() + h * 0.30);
        p.line_segment([a, b], s);
        p.line_segment([b, c], s);
    } else {
        p.rect_stroke(r, rr, Stroke::new(2.0, mix(t.surface_hover, t.text_dim, hover)), StrokeKind::Inside);
    }
}

/// A chip with a small picture (a game icon, an avatar) before the label.
pub fn image_chip(ui: &mut Ui, t: &Theme, image: Option<&str>, label: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), font(13.0), Color32::PLACEHOLDER);
    let pic = if image.is_some() { 26.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 26.0 + pic, 30.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let r = t.corner_small();
    let p = ui.painter();
    let fill = if selected { t.surface_hover } else { mix(t.raised, t.surface_hover, hover * 0.6) };
    p.rect_filled(rect, r, fill);
    if let Some(url) = image {
        let img = Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(20.0));
        if !paint_cover(ui, img, url, CornerRadius::same(4)) {
            skeleton(ui, t, img, CornerRadius::same(4));
        }
    }
    let color = if selected { t.text } else { mix(t.text_dim, t.text, hover) };
    ui.painter().galley(egui::pos2(rect.left() + 13.0 + pic, rect.center().y - galley.size().y / 2.0), galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------- Steam markup ----------

/// Lay out Steam's mix of BBCode and simple HTML (item descriptions, bios):
/// `[color=#ff9800]…[/color]`, `[b]`, `[i]`, `[u]`, `[url=…]…[/url]`,
/// `<br>`, `<font color="…">`. Unknown tags are dropped, text is kept.
pub fn markup(text: &str, size: f32, color: Color32, wrap_width: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut colors: Vec<Color32> = vec![color];
    let (mut bold_on, mut italic, mut underline) = (0u32, 0u32, 0u32);
    let parse_color = |v: &str| -> Option<Color32> {
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'').trim_start_matches('#');
        let n = u32::from_str_radix(v.get(..6)?, 16).ok()?;
        Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
    };
    let push = |job: &mut LayoutJob, s: &str, colors: &[Color32], b: u32, i: u32, u: u32| {
        if s.is_empty() {
            return;
        }
        let c = *colors.last().unwrap_or(&color);
        let mut f = TextFormat { font_id: if b > 0 { font_semibold(size) } else { font(size) }, color: c, italics: i > 0, ..Default::default() };
        if u > 0 {
            f.underline = Stroke::new(1.0, c);
        }
        job.append(&decode_entities(s), 0.0, f);
    };
    let mut rest = text;
    while !rest.is_empty() {
        let next = rest.find(['[', '<']);
        let Some(at) = next else {
            push(&mut job, rest, &colors, bold_on, italic, underline);
            break;
        };
        push(&mut job, &rest[..at], &colors, bold_on, italic, underline);
        let close_ch = if rest[at..].starts_with('[') { ']' } else { '>' };
        let Some(end) = rest[at..].find(close_ch) else {
            push(&mut job, &rest[at..], &colors, bold_on, italic, underline);
            break;
        };
        let tag = &rest[at + 1..at + end];
        let lower = tag.to_ascii_lowercase();
        let name = lower.trim_start_matches('/').split(['=', ' ']).next().unwrap_or_default().to_string();
        let closing = lower.starts_with('/');
        let known = matches!(name.as_str(), "color" | "font" | "b" | "strong" | "i" | "em" | "u" | "url" | "a" | "br" | "p" | "h1" | "h2" | "h3" | "list" | "li" | "*" | "span" | "div" | "img" | "noparse" | "hr" | "strike" | "spoiler" | "quote" | "code");
        if !known {
            // Not a tag: keep the bracket as text.
            push(&mut job, &rest[at..at + 1], &colors, bold_on, italic, underline);
            rest = &rest[at + 1..];
            continue;
        }
        match (name.as_str(), closing) {
            ("color" | "font", false) => {
                let value = tag.split_once('=').map(|x| x.1).unwrap_or_default();
                let value = value.trim_start_matches("color=").trim_start_matches("\"").to_string();
                colors.push(parse_color(&value).unwrap_or(*colors.last().unwrap_or(&color)));
            }
            ("color" | "font", true) => {
                if colors.len() > 1 {
                    colors.pop();
                }
            }
            ("b" | "strong" | "h1" | "h2" | "h3", false) => bold_on += 1,
            ("b" | "strong", true) => bold_on = bold_on.saturating_sub(1),
            ("h1" | "h2" | "h3", true) => {
                bold_on = bold_on.saturating_sub(1);
                push(&mut job, "\n", &colors, 0, 0, 0);
            }
            ("i" | "em", false) => italic += 1,
            ("i" | "em", true) => italic = italic.saturating_sub(1),
            ("u", false) => underline += 1,
            ("u", true) => underline = underline.saturating_sub(1),
            ("br" | "p" | "li" | "*" | "hr", _) | ("div", true) => push(&mut job, "\n", &colors, 0, 0, 0),
            _ => {}
        }
        rest = &rest[at + end + 1..];
    }
    job
}

fn decode_entities(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ").replace("&amp;", "&")
}

/// Show Steam markup as a wrapped label.
pub fn markup_label(ui: &mut Ui, text: &str, size: f32, color: Color32) -> Response {
    let job = markup(text, size, color, ui.available_width());
    ui.label(job)
}

#[cfg(test)]
mod markup_tests {
    #[test]
    fn reads_steam_markup() {
        let job = super::markup("Skin for [color=#ffdba5]Jacket[/color]. [b]Breaks[/b] into 1 x Cloth<br>Tradable", 12.0, egui::Color32::WHITE, 300.0);
        assert_eq!(job.text, "Skin for Jacket. Breaks into 1 x Cloth\nTradable");
        assert!(job.sections.iter().any(|s| s.format.color == egui::Color32::from_rgb(0xff, 0xdb, 0xa5)));
        // Brackets that aren't tags stay.
        assert_eq!(super::markup("[x] and [y]", 12.0, egui::Color32::WHITE, 100.0).text, "[x] and [y]");
    }
}

/// Right-click menu for text fields, like Steam's: Cut, Copy, Paste.
/// They act on the field through egui's own clipboard commands, so
/// selections and the cursor behave exactly as with Ctrl+X/C/V.
pub trait EditMenu {
    fn with_edit_menu(self) -> Self;
}

impl EditMenu for Response {
    fn with_edit_menu(self) -> Self {
        if self.secondary_clicked() {
            // The commands go to the focused field: this one.
            self.request_focus();
        }
        self.context_menu(|ui| {
            ui.set_min_width(160.0);
            let mut cmd = None;
            if ui.button(i18n::tr("Cut")).clicked() {
                cmd = Some(egui::ViewportCommand::RequestCut);
            }
            if ui.button(i18n::tr("Copy")).clicked() {
                cmd = Some(egui::ViewportCommand::RequestCopy);
            }
            if ui.button(i18n::tr("Paste")).clicked() {
                cmd = Some(egui::ViewportCommand::RequestPaste);
            }
            if let Some(c) = cmd {
                self.request_focus();
                ui.ctx().send_viewport_cmd(c);
                ui.close();
            }
        });
        self
    }
}

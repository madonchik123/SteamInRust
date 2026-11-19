//! Reusable widgets. Every one draws from the [`Theme`] it's given: its
//! component style (`t.components.*`) for shape, size, text and colors per
//! state, and the theme's tokens and text styles for the rest. Pages build
//! from these and theme values only, so a theme restyles everything.

use crate::Theme;
use crate::theme::{Component, Interact, Look, ResolvedFill, Fit, TextStyle, Weight, bold, medium, semibold};
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

/// Text in one of the theme's text styles.
pub fn styled(s: impl Into<String>, style: TextStyle, color: Color32) -> RichText {
    RichText::new(s).size(style.size).color(color).family(style.weight.family())
}

/// The theme's Heading Large (26 px bold in the default theme).
pub fn heading_l(t: &Theme, s: impl Into<String>, color: Color32) -> RichText {
    styled(s, t.typography.heading_l, color)
}
/// Heading Medium (22 px bold).
pub fn heading_m(t: &Theme, s: impl Into<String>, color: Color32) -> RichText {
    styled(s, t.typography.heading_m, color)
}
/// Heading Small (18 px bold).
pub fn heading_s(t: &Theme, s: impl Into<String>, color: Color32) -> RichText {
    styled(s, t.typography.heading_s, color)
}
/// Body Large (16 px).
pub fn body_l(t: &Theme, s: impl Into<String>, color: Color32) -> RichText {
    styled(s, t.typography.body_l, color)
}
/// Body Medium (14 px).
pub fn body_m(t: &Theme, s: impl Into<String>, color: Color32) -> RichText {
    styled(s, t.typography.body_m, color)
}
/// Body Small (12 px).
pub fn body_s(t: &Theme, s: impl Into<String>, color: Color32) -> RichText {
    styled(s, t.typography.body_s, color)
}

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

// ---------- drawing a component ----------

/// The pointer state of a widget, animated, for [`Theme::look`].
pub fn interact(ui: &Ui, resp: &Response, selected: bool) -> Interact {
    let disabled = !ui.is_enabled();
    Interact {
        hover: ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered() && !disabled, 0.12),
        pressed: resp.is_pointer_button_down_on() && !disabled,
        selected,
        disabled,
    }
}

/// Paint a fill (color, gradient or the theme's picture) into `rect`.
pub fn paint_fill(ui: &Ui, t: &Theme, rect: Rect, radius: CornerRadius, fill: &ResolvedFill) {
    let p = ui.painter();
    match fill {
        ResolvedFill::Color(c) => {
            if c.a() > 0 {
                p.rect_filled(rect, radius, *c);
            }
        }
        ResolvedFill::Linear { a, b, horizontal, end } => {
            let painter = ui.painter_at(rect);
            let end = end.clamp(0.05, 1.0);
            let split = if *horizontal {
                Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * end, rect.bottom()))
            } else {
                Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + rect.height() * end))
            };
            gradient(&painter, split, *a, *b, *horizontal);
            if end < 1.0 {
                let rest = if *horizontal { Rect::from_min_max(pos2(split.right(), rect.top()), rect.max) } else { Rect::from_min_max(pos2(rect.left(), split.bottom()), rect.max) };
                painter.rect_filled(rest, CornerRadius::ZERO, *b);
            }
        }
        ResolvedFill::Radial { a, b, center, end } => {
            let c = rect.min + vec2(center[0], center[1]);
            let far = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()].iter().map(|q| (*q - c).length()).fold(0.0, f32::max);
            radial_gradient(&ui.painter_at(rect), rect, c, far * end.clamp(0.05, 1.0), *a, *b);
        }
        ResolvedFill::Image { path, fit, tint } => {
            if let Some(uri) = crate::themes::image_uri(ui.ctx(), t, path) {
                let img = egui::Image::new(uri.clone());
                if let Ok(TexturePoll::Ready { texture }) = img.load_for_size(ui.ctx(), rect.size()) {
                    let tex = egui::Image::from_texture(texture).corner_radius(radius);
                    match fit {
                        Fit::Stretch => tex.paint_at(ui, rect),
                        Fit::Cover => {
                            let (ta, ra) = (texture.size.x / texture.size.y, rect.width() / rect.height());
                            let uv = if ta > ra {
                                let w = ra / ta;
                                Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
                            } else {
                                let h = ta / ra;
                                Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
                            };
                            tex.uv(uv).paint_at(ui, rect);
                        }
                        Fit::Tile => {
                            let size = texture.size.max(Vec2::splat(8.0));
                            let painter = ui.painter_at(rect);
                            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                            let mut y = rect.top();
                            while y < rect.bottom() {
                                let mut x = rect.left();
                                while x < rect.right() {
                                    painter.image(texture.id, Rect::from_min_size(pos2(x, y), size), uv, Color32::WHITE);
                                    x += size.x;
                                }
                                y += size.y;
                            }
                        }
                    }
                }
            }
            if let Some(c) = tint {
                p.rect_filled(rect, radius, *c);
            }
        }
    }
}

/// Paint a component's box: shadow, fill, outline (or the underline that
/// marks it selected, `selected` being 0..1).
pub fn paint_box(ui: &Ui, t: &Theme, rect: Rect, c: &Component, look: &Look, selected: f32) {
    let radius = t.radius_of(c);
    let shadow = crate::theme::shadow_of(t, &c.shadow);
    if shadow.color.a() > 0 {
        ui.painter().add(shadow.as_shape(rect, radius));
    }
    paint_fill(ui, t, rect, radius, &look.fill);
    if look.border.a() == 0 || c.border <= 0.0 {
        return;
    }
    match c.indicator {
        crate::theme::Indicator::Fill => {
            ui.painter().rect_stroke(rect, radius, Stroke::new(c.border, look.border), StrokeKind::Inside);
        }
        crate::theme::Indicator::Underline => {
            if selected > 0.0 {
                let pad = t.padding_of(c).x;
                let w = (rect.width() - pad * 2.0).max(8.0) * selected;
                let line = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 1.5), vec2(w, 3.0));
                ui.painter().rect_filled(line, CornerRadius::same(2), look.border);
            }
        }
    }
}

/// Allocate and draw a component with one line of text (and an optional
/// icon before it): the shared body of buttons, tabs, chips and tags.
/// `height` 0 takes the component's own.
#[allow(clippy::too_many_arguments)]
pub fn text_box(ui: &mut Ui, t: &Theme, c: &Component, icon: Option<&str>, label: &str, selected: bool, height: f32, font_id: Option<FontId>) -> Response {
    let fid = font_id.unwrap_or_else(|| t.font_of(c));
    let label = if c.uppercase { label.to_uppercase() } else { label.to_string() };
    let galley = ui.painter().layout_no_wrap(label.clone(), fid.clone(), Color32::PLACEHOLDER);
    let pad = t.padding_of(c);
    let h = if height > 0.0 { height } else { t.height_of(c, galley.size().y + pad.y * 2.0 + 10.0) };
    let icon_w = if icon.is_some() { fid.size * 1.1 } else { 0.0 };
    let gap = if icon.is_some() && !label.is_empty() { (fid.size * 0.55).round() } else { 0.0 };
    let w = if label.is_empty() { h } else { galley.size().x + icon_w + gap + pad.x * 2.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let s = interact(ui, &resp, selected);
        let look = t.look(c, s);
        let sel = ui.ctx().animate_bool_with_time(resp.id.with("sel"), selected, 0.18);
        paint_box(ui, t, rect, c, &look, sel);
        let x0 = rect.center().x - (icon_w + gap + if label.is_empty() { 0.0 } else { galley.size().x }) / 2.0;
        if let Some(i) = icon {
            paint_icon(ui.painter(), pos2(x0 + icon_w / 2.0, rect.center().y), i, fid.size * 1.1, look.icon);
        }
        if !label.is_empty() {
            ui.painter().galley(pos2(x0 + icon_w + gap, rect.center().y - galley.size().y / 2.0), galley, look.text);
        }
    }
    let enabled = ui.is_enabled();
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default })
}

/// A component's fill as one color (for frames that can only take a color).
pub fn solid_fill(t: &Theme, c: &Component) -> Color32 {
    match t.look(c, Interact::default()).fill {
        ResolvedFill::Color(c) => c,
        ResolvedFill::Linear { a, .. } | ResolvedFill::Radial { a, .. } => a,
        ResolvedFill::Image { .. } => t.surface,
    }
}

/// An egui frame in a component's style (fill, outline, radius, padding,
/// shadow). Pictures and gradients show as their main color.
pub fn component_frame(t: &Theme, c: &Component) -> Frame {
    let look = t.look(c, Interact::default());
    let pad = t.padding_of(c);
    let mut f = Frame::new()
        .fill(solid_fill(t, c))
        .corner_radius(t.radius_of(c))
        .inner_margin(Margin::symmetric(pad.x.round() as i8, pad.y.round() as i8))
        .shadow(crate::theme::shadow_of(t, &c.shadow));
    if look.border.a() > 0 && c.border > 0.0 {
        f = f.stroke(Stroke::new(c.border, look.border));
    }
    f
}

/// Paint the theme's background for an area of the app (`"store"`,
/// `"library"`, `"community"`, `"profile"`, anything else for plain pages).
pub fn paint_background(ui: &Ui, t: &Theme, rect: Rect, area: &str) {
    let b = &t.backgrounds;
    let fill = match area {
        "store" => &b.store,
        "library" => &b.library,
        "community" => &b.community,
        "profile" => &b.profile,
        _ => &b.page,
    };
    paint_fill(ui, t, rect, CornerRadius::ZERO, &t.fill(fill));
}

// ---------- painting ----------

/// Linear blend between two colors.
/// Works on see-through colors too (fading from `TRANSPARENT` to a color
/// gives that color at growing opacity, not a darker one).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    // Color32 keeps its channels premultiplied by alpha; blending them as
    // they are is the right blend.
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
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

/// Failed loads: url -> (attempts, when the last one was asked for).
static FAILED: std::sync::Mutex<Option<std::collections::HashMap<String, (u32, f64)>>> = std::sync::Mutex::new(None);

/// How a loader's error starts when the picture isn't there for good (a
/// 404, not a picture): it's shown as missing at once, so the widget's
/// fallback (another picture, or the title) shows instead of a long wait.
pub const GONE: &str = "gone: ";

/// Load an image. A picture that isn't there is missing at once; any other
/// failure (a dropped connection, a busy server, which happens when many
/// pictures load at once) is tried again with a growing pause, as long as
/// the picture is still drawn, so a hiccup never leaves it blank.
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
        Ok(TexturePoll::Pending { .. }) => {
            note_pending(ui);
            (ImageState::Loading, None)
        }
        Err(e) => {
            // Only a failed download is worth another try; anything else
            // (no such picture, not a picture, no loader for it) is final.
            let transient = matches!(&e, egui::load::LoadError::Loading(s) if !s.starts_with(GONE));
            if !transient {
                return (ImageState::Missing, None);
            }
            let now = ui.input(|i| i.time);
            let mut guard = FAILED.lock().unwrap();
            let map = guard.get_or_insert_with(Default::default);
            let entry = map.entry(url.to_string()).or_insert((0, now));
            // Wait 1 s, 2 s, 4 s… up to 30 s between attempts.
            if now - entry.1 > f64::from(1u32 << entry.0.min(5)).min(30.0) {
                entry.0 += 1;
                entry.1 = now;
                ui.ctx().forget_image(url);
                if uri != url {
                    ui.ctx().forget_image(&uri);
                }
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            note_pending(ui);
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

/// [`paint_cover_first`] fading out toward the bottom: solid down to
/// `solid` (0..1 of the height), clear at the bottom edge, so whatever is
/// behind shows through (a game's hero picture over the page's background).
pub fn paint_cover_fade(ui: &Ui, rect: Rect, urls: &[&str], solid: f32) -> bool {
    let mut texture = None;
    for url in urls {
        match load(ui, url, rect.size(), true) {
            (_, Some(tex)) => {
                texture = Some(tex);
                break;
            }
            (ImageState::Missing, _) => continue,
            _ => return false,
        }
    }
    let Some(texture) = texture else { return false };
    let (ta, ra) = (texture.size.x / texture.size.y, rect.width() / rect.height());
    let uv = if ta > ra {
        let w = ra / ta;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ta / ra;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    };
    let solid = solid.clamp(0.0, 1.0);
    // Three bands of the picture: solid, fading, and the rows' alphas.
    let stops = [(0.0, 255u8), (solid, 255), (solid + (1.0 - solid) * 0.5, 90), (1.0, 0)];
    let mut mesh = egui::Mesh::with_texture(texture.id);
    for (i, (f, a)) in stops.iter().enumerate() {
        let y = rect.top() + rect.height() * f;
        let v = uv.top() + uv.height() * f;
        let c = Color32::from_white_alpha(*a);
        mesh.vertices.push(egui::epaint::Vertex { pos: pos2(rect.left(), y), uv: pos2(uv.left(), v), color: c });
        mesh.vertices.push(egui::epaint::Vertex { pos: pos2(rect.right(), y), uv: pos2(uv.right(), v), color: c });
        if i > 0 {
            let b = (i as u32) * 2;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b + 1, b);
        }
    }
    ui.painter().with_clip_rect(rect).add(mesh);
    true
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

/// The frame in which a picture was last still on its way.
static LAST_PENDING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn note_pending(ui: &Ui) {
    LAST_PENDING.store(ui.ctx().cumulative_pass_nr(), std::sync::atomic::Ordering::Relaxed);
}

/// Loading placeholder. It pulses while pictures are still on their way;
/// once nothing is loading (a picture that never comes, data that's late)
/// it holds still, so an idle window stops drawing. New data or a picture
/// arriving wakes the window by itself.
pub fn skeleton(ui: &Ui, t: &Theme, rect: Rect, radius: CornerRadius) {
    let loading = ui.ctx().cumulative_pass_nr().saturating_sub(LAST_PENDING.load(std::sync::atomic::Ordering::Relaxed)) <= 2;
    let k = if loading {
        let time = ui.input(|i| i.time) as f32;
        // A slow pulse: 10 frames a second is smooth enough for it.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        (time * 2.2 + rect.left() * 0.004).sin() * 0.5 + 0.5
    } else {
        0.5
    };
    ui.painter().rect_filled(rect, radius, mix(t.surface, t.surface_hover, k));
}

/// Placeholder with a title, for apps that have no art.
pub fn placeholder(ui: &Ui, t: &Theme, rect: Rect, title: &str, radius: CornerRadius) {
    let p = ui.painter();
    p.rect_filled(rect, radius, t.surface_hover);
    gradient(&ui.painter_at(rect.shrink(1.0)), rect.shrink(1.0), t.accent.gamma_multiply(0.10), t.accent_2.gamma_multiply(0.05), false);
    let galley = p.layout(title.to_string(), FontId::new(t.typography.body_l.size - 1.0, Weight::Semibold.family()), t.text_dim, rect.width() - 24.0);
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

/// A content block in the theme's `card` style (flat and borderless in the
/// default theme, Figma `523:6593`).
pub fn card(t: &Theme) -> Frame {
    component_frame(t, &t.components.card)
}

/// A plain raised block (inputs' background color, small radius).
pub fn panel(t: &Theme) -> Frame {
    Frame::new().fill(t.raised).corner_radius(t.corner_small()).inner_margin(Margin::same(t.spacing.md as i8))
}

// ---------- buttons ----------

/// Button styles, each its own component in the theme (`button_primary`…).
/// In the default theme each matches a frame of the style guide (Figma page
/// `10:5`); `docs/redesign.md` lists the node IDs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// `button_primary`: the main action.
    Primary,
    /// `button_secondary`.
    Secondary,
    /// `button_tertiary`: an outline.
    Tertiary,
    /// `button_ghost`: no fill until hovered.
    Ghost,
    /// `button_play`.
    Play,
    /// `button_install`.
    Install,
    /// `button_pause`.
    Pause,
    /// `button_danger`: removing, things that can't be undone.
    Danger,
}

/// Button sizes: `Md` is the component's own height; `Sm` is 80% of it;
/// `Lg` is a third taller with the theme's `button_large` text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    Sm,
    Md,
    Lg,
}

/// The theme's style for a kind of button.
pub fn button_style(t: &Theme, kind: Kind) -> &Component {
    let c = &t.components;
    match kind {
        Kind::Primary => &c.button_primary,
        Kind::Secondary => &c.button_secondary,
        Kind::Tertiary => &c.button_tertiary,
        Kind::Ghost => &c.button_ghost,
        Kind::Play => &c.button_play,
        Kind::Install => &c.button_install,
        Kind::Pause => &c.button_pause,
        Kind::Danger => &c.button_danger,
    }
}

/// The one button. `icon` is a glyph from [`crate::icons`] (`icons::PLAY`
/// and `icons::PAUSE` are drawn as solid shapes).
pub fn button(ui: &mut Ui, t: &Theme, icon: Option<&str>, label: &str, kind: Kind, size: Size) -> Response {
    let c = button_style(t, kind);
    let base = t.font_of(c);
    let h = t.height_of(c, 40.0);
    let (h, fid, pad) = match size {
        Size::Sm => ((h * 0.8).round(), FontId::new((base.size - 2.0).max(10.0), base.family), 0.7),
        Size::Md => (h, base, 1.0),
        Size::Lg => ((h * 1.325).round(), FontId::new(t.typography.button_large.size, base.family), 1.5),
    };
    let mut c = c.clone();
    c.padding[0] *= pad;
    text_box(ui, t, &c, icon, label, false, h, Some(fid))
}

/// [`button`] at any height, `fs` the text size and `pad` the space on
/// each side.
#[allow(clippy::too_many_arguments)]
pub fn button_sized(ui: &mut Ui, t: &Theme, icon: Option<&str>, label: &str, kind: Kind, h: f32, fs: f32, pad: f32) -> Response {
    let mut c = button_style(t, kind).clone();
    c.padding[0] = pad;
    let fid = FontId::new(fs, t.font_of(&c).family);
    text_box(ui, t, &c, icon, label, false, h, Some(fid))
}

/// Draw an icon glyph centered on `c`, `size` px tall. Play and pause are
/// drawn as solid shapes (the font only has outlines).
pub fn paint_icon(p: &egui::Painter, c: egui::Pos2, icon: &str, size: f32, color: Color32) {
    if icon == crate::icons::PLAY {
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

/// The Play button, large (Figma `335:1183` in the default theme).
pub fn play_button(ui: &mut Ui, t: &Theme, label: &str) -> Response {
    button(ui, t, Some(crate::icons::PLAY), label, Kind::Play, Size::Lg)
}

/// The Install button, large (Figma `375:4453`).
pub fn install_button(ui: &mut Ui, t: &Theme, label: &str) -> Response {
    button(ui, t, Some(crate::icons::DOWNLOAD), label, Kind::Install, Size::Lg)
}

/// The Pause button for a running download, large (Figma `379:8204`).
pub fn pause_button(ui: &mut Ui, t: &Theme, label: &str) -> Response {
    button(ui, t, Some(crate::icons::PAUSE), label, Kind::Pause, Size::Lg)
}

/// A count badge in the theme's `badge` style, centered on `c`.
pub fn badge(p: &egui::Painter, t: &Theme, c: egui::Pos2, n: usize) {
    let style = &t.components.badge;
    let look = t.look(style, Interact::default());
    let label = if n > 99 { "99+".to_string() } else { n.to_string() };
    let g = p.layout_no_wrap(label, t.font_of(style), look.text);
    let h = t.height_of(style, 16.0);
    let w = (g.size().x + t.padding_of(style).x * 2.0).max(h);
    let rect = Rect::from_center_size(c, vec2(w, h));
    let fill = match look.fill {
        ResolvedFill::Color(c) => c,
        _ => t.accent,
    };
    p.rect_filled(rect, t.radius_of(style), fill);
    p.galley(c - g.size() / 2.0, g, look.text);
}

/// A square icon button (`icon_button`) with an optional count badge.
pub fn icon_button(ui: &mut Ui, t: &Theme, icon: &str, tooltip: &str, active: bool, badge_count: usize) -> Response {
    let c = &t.components.icon_button;
    let size = t.height_of(c, 36.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let look = t.look(c, interact(ui, &resp, active));
    paint_box(ui, t, rect, c, &look, 0.0);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, font(size * 0.5), look.icon);
    if badge_count > 0 {
        badge(ui.painter(), t, rect.right_top() + vec2(-7.0, 7.0), badge_count);
    }
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The top bar's Friends and Notifications buttons (`bar_button`; Figma
/// `447:3849`, `766:10160`), with a count badge on the top-right corner.
pub fn bar_button(ui: &mut Ui, t: &Theme, icon: &str, tooltip: &str, active: bool, badge_count: usize) -> Response {
    let c = &t.components.bar_button;
    let h = t.height_of(c, 37.0);
    let icon_size = (h * 0.6).round();
    let (rect, resp) = ui.allocate_exact_size(vec2(icon_size + t.padding_of(c).x * 2.0, h), Sense::click());
    let look = t.look(c, interact(ui, &resp, active));
    paint_box(ui, t, rect, c, &look, 0.0);
    icon_centered(ui.painter(), rect.center(), icon, icon_size, look.icon);
    if badge_count > 0 {
        badge(ui.painter(), t, pos2(rect.right() - 5.0, rect.top() + 6.0), badge_count);
    }
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small square button with an icon (`icon_box`; Figma `280:175`,
/// `346:2627`): view switches, filters, a game's action row. `selected`
/// marks the view that's showing.
pub fn icon_box(ui: &mut Ui, t: &Theme, icon: &str, tooltip: &str, selected: bool) -> Response {
    let c = &t.components.icon_box;
    let size = t.height_of(c, 40.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let look = t.look(c, interact(ui, &resp, selected));
    paint_box(ui, t, rect, c, &look, 0.0);
    icon_centered(ui.painter(), rect.center(), icon, (size * 0.55).round(), look.icon);
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

/// The favorite button: an outlined heart, filled in the danger color once
/// the game is a favorite.
pub fn heart_button(ui: &mut Ui, t: &Theme, on: bool, tooltip: &str) -> Response {
    if !on {
        return icon_button(ui, t, crate::icons::HEART, tooltip, false, 0);
    }
    let c = &t.components.icon_button;
    let size = t.height_of(c, 36.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let s = interact(ui, &resp, true);
    paint_box(ui, t, rect, c, &t.look(c, s), 0.0);
    paint_heart(ui.painter(), rect.center(), size * 0.5 * (1.0 + 0.06 * s.hover), t.danger);
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The Wishlist button (`wishlist`; Figma `280:345`, `302:1745`): an
/// outlined heart, solid once wishlisted. `label` adds text after it.
pub fn wishlist_button(ui: &mut Ui, t: &Theme, wishlisted: bool, label: Option<&str>) -> Response {
    wishlist_button_sized(ui, t, wishlisted, label, None)
}

/// [`wishlist_button`] at another height (the store's cards use 44 px).
pub fn wishlist_button_sized(ui: &mut Ui, t: &Theme, wishlisted: bool, label: Option<&str>, height: Option<f32>) -> Response {
    let c = &t.components.wishlist;
    let h = height.unwrap_or_else(|| t.height_of(c, 53.0));
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    let heart = (h * 0.42).round();
    let galley = label.map(|l| ui.painter().layout_no_wrap(l.to_string(), fid.clone(), Color32::PLACEHOLDER));
    let w = match &galley {
        Some(g) => pad + heart + 10.0 + g.size().x + pad + 3.0,
        None => h + 9.0,
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, interact(ui, &resp, wishlisted));
        paint_box(ui, t, rect, c, &look, 0.0);
        let heart_c = if galley.is_some() { pos2(rect.left() + pad + heart / 2.0, rect.center().y) } else { rect.center() };
        if wishlisted {
            paint_heart(ui.painter(), heart_c, heart * 0.9, look.icon);
        } else {
            icon_centered(ui.painter(), heart_c, crate::icons::HEART, heart, look.icon);
        }
        if let Some(g) = galley {
            ui.painter().galley(pos2(heart_c.x + heart / 2.0 + 10.0, rect.center().y - g.size().y / 2.0), g, look.text);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row in a dropdown menu (`menu_item`; Figma `18:248`): icon and label
/// across the menu's width.
pub fn menu_item(ui: &mut Ui, t: &Theme, icon: &str, label: &str) -> Response {
    menu_item_tinted(ui, t, icon, label, None)
}

/// [`menu_item`] in its own color, e.g. red for "Uninstall…".
pub fn menu_item_tinted(ui: &mut Ui, t: &Theme, icon: &str, label: &str, tint: Option<Color32>) -> Response {
    let c = &t.components.menu_item;
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    // As wide as the text needs, and the menu's set width; a right-click
    // menu with no width set offers ~400 px, which made every row that wide.
    let text_w = ui.painter().layout_no_wrap(label.to_string(), fid.clone(), Color32::PLACEHOLDER).size().x;
    let avail = ui.available_width();
    let width = (pad + 20.0 + 8.0 + text_w + 28.0).max(if avail > 360.0 { 220.0 } else { avail });
    let (rect, resp) = ui.allocate_exact_size(vec2(width, t.height_of(c, 38.0)), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, interact(ui, &resp, false));
        paint_box(ui, t, rect, c, &look, 0.0);
        let y = rect.center().y;
        let (icon_c, text_c) = match tint {
            Some(x) => (x, x),
            None => (look.icon, look.text),
        };
        ui.painter().text(pos2(rect.left() + pad, y), Align2::CENTER_CENTER, icon, font(fid.size + 0.5), icon_c);
        ui.painter().text(pos2(rect.left() + pad + 20.0, y), Align2::LEFT_CENTER, label, fid, text_c);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row across the whole width of a list or dropdown, in the `menu_item`
/// style, that can show as selected (the row the arrow keys are on, as in
/// the app-wide search's results). A long label ends in "…".
pub fn menu_row(ui: &mut Ui, t: &Theme, icon: &str, label: &str, selected: bool) -> Response {
    let c = &t.components.menu_item;
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), t.height_of(c, 38.0)), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, interact(ui, &resp, selected));
        paint_box(ui, t, rect, c, &look, if selected { 1.0 } else { 0.0 });
        let y = rect.center().y;
        ui.painter().text(pos2(rect.left() + pad, y), Align2::CENTER_CENTER, icon, font(fid.size + 0.5), look.icon);
        let x = rect.left() + pad + 20.0;
        let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), fid, look.text);
        job.wrap.max_width = (rect.right() - pad - x).max(10.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, y - g.size().y / 2.0), g, look.text);
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

/// A tab inside a page (`nav_tab`), `height` tall.
pub fn nav_tab(ui: &mut Ui, t: &Theme, label: &str, selected: bool, height: f32) -> Response {
    text_box(ui, t, &t.components.nav_tab, None, label, selected, height, None)
}

/// A main section in the top bar (`section_tab`; Figma `18:294`).
pub fn section_tab(ui: &mut Ui, t: &Theme, label: &str, selected: bool, height: f32) -> Response {
    text_box(ui, t, &t.components.section_tab, None, label, selected, height, None)
}

/// [`section_tab`] at another text size (the top bar shrinks it on narrow windows).
pub fn section_tab_sized(ui: &mut Ui, t: &Theme, label: &str, selected: bool, height: f32, size: f32) -> Response {
    let c = &t.components.section_tab;
    let fid = FontId::new(size, t.font_of(c).family);
    text_box(ui, t, c, None, label, selected, height, Some(fid))
}

/// A tab in a page bar (`page_tab`; Figma `82:322`).
pub fn sub_tab(ui: &mut Ui, t: &Theme, label: &str, selected: bool) -> Response {
    let mut c = t.components.page_tab.clone();
    // Less room at the sides when the bar is short of it.
    if ui.available_width() < 900.0 {
        c.padding[0] = (c.padding[0] * 0.62).round();
    }
    text_box(ui, t, &c, None, label, selected, 0.0, None)
}

/// A filter chip (`chip`).
pub fn chip(ui: &mut Ui, t: &Theme, label: &str, selected: bool) -> Response {
    tinted_chip(ui, t, label, None, selected)
}

/// A chip in its own color, e.g. an item rarity: a colored dot before the
/// label, and an outline in that color.
pub fn tinted_chip(ui: &mut Ui, t: &Theme, label: &str, tint: Option<Color32>, selected: bool) -> Response {
    let c = &t.components.chip;
    let Some(tint) = tint else {
        return text_box(ui, t, c, None, label, selected, 0.0, None);
    };
    let fid = t.font_of(c);
    let galley = ui.painter().layout_no_wrap(label.to_string(), fid, Color32::PLACEHOLDER);
    let pad = t.padding_of(c).x;
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + pad * 2.0 + 14.0, t.height_of(c, 30.0)), Sense::click());
    let s = interact(ui, &resp, selected);
    let look = t.look(c, s);
    paint_box(ui, t, rect, c, &look, 0.0);
    let r = t.radius_of(c);
    ui.painter().rect_stroke(rect, r, Stroke::new(1.0, tint.gamma_multiply(if selected { 0.8 } else { 0.35 })), StrokeKind::Inside);
    let color = mix(tint, t.text, if selected { 0.25 } else { 0.1 + s.hover * 0.2 });
    ui.painter().circle_filled(pos2(rect.left() + pad + 4.0, rect.center().y), 4.0, tint);
    ui.painter().galley(pos2(rect.left() + pad + 14.0, rect.center().y - galley.size().y / 2.0), galley, color);
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

/// A row of choices in one block (`segmented` around compact `sort`
/// chips). Returns true if the selection changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    let mut opt_style = t.components.sort.clone();
    opt_style.padding[0] = 11.0;
    let fid = FontId::new(t.typography.body_m.size - 1.0, t.font_of(&opt_style).family);
    component_frame(t, &t.components.segmented).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            for (opt, label) in options {
                let on = *value == *opt;
                let r = text_box(ui, t, &opt_style, None, label, on, t.dense(26.0), Some(fid.clone()));
                if r.clicked() && !on {
                    *value = *opt;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// A search field (`input`; Figma `280:57`), 34 px tall in the default
/// theme: the hint and magnifier in the input's `icon` color, a clear
/// button once typed in.
pub fn search(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32) -> Response {
    let h = t.height_of(&t.components.input, 34.0);
    search_sized(ui, t, value, hint, width, h)
}

/// [`search`] at the page bar's height (41 px in the default theme).
pub fn bar_search(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32) -> Response {
    let h = t.height_of(&t.components.input, 34.0) * 1.2;
    search_sized(ui, t, value, hint, width, h.round())
}

fn search_sized(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32, height: f32) -> Response {
    search_in(ui, t, &t.components.input, value, hint, width, height)
}

/// How wide a text field is: the width asked for (`f32::INFINITY` fills
/// the row), grown to fit its placeholder when the row has the room, so
/// placeholders are only shortened when there truly isn't space.
/// `reserved` is the room inside the field that isn't text (padding, icons).
fn field_width(ui: &Ui, fid: &FontId, hint: &str, width: f32, reserved: f32) -> f32 {
    let room = ui.available_width().max(0.0);
    let asked = if width.is_finite() { width } else { room };
    let hint_w = ui.painter().layout_no_wrap(hint.to_string(), fid.clone(), Color32::PLACEHOLDER).size().x;
    let needed = (hint_w + reserved + 4.0).ceil();
    asked.max(needed.min(room)).max(60.0)
}

/// A field's whole text on hover, when it doesn't fit: the placeholder
/// while it's empty, the typed text while it's not being edited.
/// Typed text that runs past the field's end (while it isn't being edited)
/// fades out there instead of being cut through a letter.
fn full_text_on_hover(ui: &Ui, resp: Response, fid: &FontId, value: &str, hint: &str, text_w: f32) -> Response {
    let shown = if value.is_empty() { hint } else { value };
    if shown.is_empty() || (!value.is_empty() && resp.has_focus()) {
        return resp;
    }
    let w = ui.painter().layout_no_wrap(shown.to_string(), fid.clone(), Color32::PLACEHOLDER).size().x;
    if w > text_w { resp.on_hover_text(shown.to_string()) } else { resp }
}

fn fade_overflow(ui: &Ui, t: &Theme, c: &Component, field: Rect, fid: &FontId, value: &str, focused: bool) {
    if value.is_empty() || focused {
        return;
    }
    let w = ui.painter().layout_no_wrap(value.to_string(), fid.clone(), Color32::PLACEHOLDER).size().x;
    if w <= field.width() {
        return;
    }
    let fill = solid_fill(t, c);
    let edge = Rect::from_min_max(pos2(field.right() - 28.0, field.top() + 2.0), pos2(field.right() + 1.0, field.bottom() - 2.0));
    gradient(ui.painter(), edge, fill.gamma_multiply(0.0), fill, true);
}

fn search_in(ui: &mut Ui, t: &Theme, c: &Component, value: &mut String, hint: &str, width: f32, height: f32) -> Response {
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    let width = field_width(ui, &fid, hint, width, pad + 38.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let field = Rect::from_min_max(pos2(rect.left() + pad, rect.top()), pos2(rect.right() - 30.0, rect.bottom()));
    // Painted before the field (it draws on top), in the unfocused look;
    // the focus outline goes on after.
    let rest = t.look(c, Interact::default());
    paint_box(ui, t, rect, c, &rest, 0.0);
    // `place`, not `put`: `put` would end the row at the text field,
    // so the next widget overlapped the box's right end.
    let resp = ui
        .place(
            field,
            egui::TextEdit::singleline(value)
                .hint_text(RichText::new(hint).font(fid.clone()).color(rest.icon))
                .font(fid.clone())
                .text_color(rest.text)
                .frame(Frame::NONE)
                .margin(vec2(0.0, 0.0))
                .align(Align2::LEFT_CENTER)
                .desired_width(field.width()),
        )
        .with_edit_menu();
    let resp = full_text_on_hover(ui, resp, &fid, value, hint, field.width());
    fade_overflow(ui, t, c, field, &fid, value, resp.has_focus());
    if resp.has_focus() {
        let focus = t.look(c, Interact { selected: true, ..Default::default() });
        if focus.border.a() > 0 {
            ui.painter().rect_stroke(rect, t.radius_of(c), Stroke::new(c.border.max(1.0), focus.border), StrokeKind::Inside);
        }
    }
    let ic = pos2(rect.right() - 18.0, rect.center().y);
    if value.is_empty() {
        ui.painter().text(ic, Align2::CENTER_CENTER, crate::icons::SEARCH, font(fid.size + 2.0), rest.icon);
    } else {
        let x = ui.interact(Rect::from_center_size(ic, Vec2::splat(22.0)), resp.id.with("clear"), Sense::click());
        ui.painter().text(ic, Align2::CENTER_CENTER, crate::icons::X, font(fid.size - 1.0), if x.hovered() { t.text } else { t.text_dim });
        if x.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            value.clear();
        }
    }
    resp
}

/// The states of a [`tag`] (Figma "tag" `280:203`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TagState {
    /// A game's tag: the `tag` style's `normal` state.
    Plain,
    /// A filter you can turn on: no fill, the `selected` text color.
    Unchecked,
    /// A filter that's on: `selected`.
    Checked,
    /// The tag of the page you're on: `normal` fill, `selected` text.
    Current,
    /// A filter in use, with a × to remove it: `selected`.
    Filtered,
}

/// A tag (`tag`; Figma `280:203`), with an optional count after it
/// ("5,000") and a × for [`TagState::Filtered`].
pub fn tag(ui: &mut Ui, t: &Theme, label: &str, count: Option<&str>, state: TagState) -> Response {
    let c = &t.components.tag;
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    let g = ui.painter().layout_no_wrap(label.to_string(), fid.clone(), Color32::PLACEHOLDER);
    let cg = count.map(|x| ui.painter().layout_no_wrap(x.to_string(), fid.clone(), Color32::PLACEHOLDER));
    let extra = match (&cg, state) {
        (_, TagState::Filtered) => 10.0 + 8.0,
        (Some(x), _) => 10.0 + x.size().x,
        (None, _) => 0.0,
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(pad + g.size().x + extra + pad, t.height_of(c, 30.0)), Sense::click());
    if ui.is_rect_visible(rect) {
        let s = interact(ui, &resp, matches!(state, TagState::Checked | TagState::Filtered));
        let mut look = t.look(c, s);
        let on = t.look(c, Interact { selected: true, ..s });
        match state {
            TagState::Unchecked => {
                look.fill = if s.hover > 0.0 { look.fill } else { ResolvedFill::Color(Color32::TRANSPARENT) };
                look.text = on.text;
            }
            TagState::Current => look.text = on.text,
            _ => {}
        }
        paint_box(ui, t, rect, c, &look, 0.0);
        let y = rect.center().y;
        ui.painter().galley(pos2(rect.left() + pad, y - g.size().y / 2.0), g.clone(), look.text);
        let x = rect.left() + pad + g.size().x + 10.0;
        if state == TagState::Filtered {
            let cx = pos2(x + 3.0, y);
            let st = Stroke::new(1.2, mix(t.text_dim, t.text, s.hover));
            ui.painter().line_segment([cx - vec2(3.0, 3.0), cx + vec2(3.0, 3.0)], st);
            ui.painter().line_segment([cx + vec2(-3.0, 3.0), cx + vec2(3.0, -3.0)], st);
        } else if let Some(cg) = cg {
            ui.painter().galley(pos2(x, y - cg.size().y / 2.0), cg, t.text_dim.gamma_multiply(0.8));
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An on/off switch (`toggle`; Figma `477:7301`): the track is the state's
/// fill, the knob its text color. Flips `on` when clicked.
pub fn toggle(ui: &mut Ui, t: &Theme, on: &mut bool) -> Response {
    let c = &t.components.toggle;
    let h = t.height_of(c, 30.0);
    let (rect, mut resp) = ui.allocate_exact_size(vec2((h * 2.1).round(), h), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let k = ui.ctx().animate_bool_with_time(resp.id, *on, 0.14);
        let s = interact(ui, &resp, false);
        let off = t.look(c, s);
        let onl = t.look(c, Interact { selected: true, ..s });
        let track = match (&off.fill, &onl.fill) {
            (ResolvedFill::Color(a), ResolvedFill::Color(b)) => mix(*a, *b, k),
            _ => t.raised,
        };
        let r = t.radius_of(c);
        ui.painter().rect_filled(rect, r, mix(track, Color32::WHITE, 0.04 * s.hover));
        let knob_r = h / 3.0;
        let x = rect.left() + h / 2.0 + (rect.width() - h) * k;
        ui.painter().circle_filled(pos2(x, rect.center().y), knob_r, mix(off.text, onl.text, k));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A [`toggle`] with a label before it, the switch at the row's right end.
pub fn toggle_row(ui: &mut Ui, t: &Theme, on: &mut bool, label: &str) -> Response {
    ui.horizontal(|ui| {
        ui.label(body_m(t, label, t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| toggle(ui, t, on)).inner
    })
    .inner
}

/// One choice in a "Sort by" row (`sort`; Figma `296:579`).
pub fn sort_chip(ui: &mut Ui, t: &Theme, label: &str, selected: bool) -> Response {
    text_box(ui, t, &t.components.sort, None, label, selected, 0.0, None)
}

/// "Sort by" and a row of [`sort_chip`]s. Returns true if the choice changed.
pub fn sort_by<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.xs + 1.0;
        ui.label(styled(i18n::t!("Sort by"), t.text_style(&t.components.sort.text), t.text_dim));
        ui.add_space(t.spacing.xs);
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
/// `min_width` wide. Open a menu from it with `egui::Popup::menu(&response)`.
pub fn dropdown(ui: &mut Ui, t: &Theme, label: &str, min_width: f32) -> Response {
    let c = &t.components.sort;
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    let g = ui.painter().layout_no_wrap(label.to_string(), fid, Color32::PLACEHOLDER);
    let w = (pad + g.size().x + 12.0 + 12.0 + 16.0).max(min_width);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, t.height_of(c, 39.0)), Sense::click());
    if ui.is_rect_visible(rect) {
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
        let look = t.look(c, interact(ui, &resp, open));
        paint_box(ui, t, rect, c, &look, 0.0);
        let y = rect.center().y;
        ui.painter().galley(pos2(rect.left() + pad, y - g.size().y / 2.0), g, look.text);
        let caret = if open { crate::icons::CARET_UP } else { crate::icons::CARET_DOWN };
        ui.painter().text(pos2(rect.right() - 20.0, y), Align2::CENTER_CENTER, caret, font(13.0), look.icon);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A review score bar (Figma "reviews" `289:270`), `width` wide: the
/// positive share in the accent, the negative in the danger color, with
/// their percentages above. `positive` is 0–100.
pub fn review_bar(ui: &mut Ui, t: &Theme, positive: u8, width: f32) -> Response {
    let positive = positive.min(100);
    let fs = t.typography.body_s.size;
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(60.0), fs + 11.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fid = t.typography.body_s.font();
        let gu = p.layout_no_wrap(format!("{positive}%"), fid.clone(), t.accent);
        let gd = p.layout_no_wrap(format!("{}%", 100 - positive), fid.clone(), t.danger);
        let gu_w = gu.size().x;
        p.galley(rect.left_top(), gu, t.accent);
        p.text(pos2(rect.left() + gu_w + 3.0, rect.top()), Align2::LEFT_TOP, crate::icons::THUMBS_UP, fid.clone(), t.accent.gamma_multiply(0.4));
        let thumb_w = fs + 3.0;
        p.text(pos2(rect.right(), rect.top()), Align2::RIGHT_TOP, crate::icons::THUMBS_DOWN, fid, t.danger.gamma_multiply(0.4));
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

/// A small download ring (Figma "download status" `383:10133`): the
/// `progress` style's track and filled colors, centered on `c`.
pub fn progress_ring(p: &egui::Painter, t: &Theme, c: egui::Pos2, radius: f32, fraction: f32) {
    let (track, done) = progress_colors(t);
    p.circle_filled(c, radius, track);
    let f = fraction.clamp(0.0, 1.0);
    if f <= 0.0 {
        return;
    }
    // A pie slice from 12 o'clock, clockwise.
    let steps = (48.0 * f).ceil().max(2.0) as usize;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, done);
    for i in 0..=steps {
        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * f * i as f32 / steps as f32;
        mesh.colored_vertex(c + vec2(a.cos(), a.sin()) * radius, done);
    }
    for i in 1..=steps as u32 {
        mesh.add_triangle(0, i, i + 1);
    }
    p.add(mesh);
}

fn progress_colors(t: &Theme) -> (Color32, Color32) {
    let c = &t.components.progress;
    let color = |l: Look| match l.fill {
        ResolvedFill::Color(x) => x,
        _ => t.accent,
    };
    (color(t.look(c, Interact::default())), color(t.look(c, Interact { selected: true, ..Default::default() })))
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

/// What a [`game_tile_info`] shows when the pointer is on it (Figma "game
/// card" `363:5954`, its hover state): the title, a button (Play, Install…)
/// and two facts with their icons.
#[derive(Clone, Copy, Default)]
pub struct TileInfo<'a> {
    /// The button's icon (`icons::PLAY`, `icons::DOWNLOAD`…); `None` for no button.
    pub action: Option<&'a str>,
    /// The button's tooltip.
    pub action_tip: &'a str,
    /// Last played ("21 Aug 2022").
    pub last_played: Option<&'a str>,
    /// Time played ("92 hours").
    pub playtime: Option<&'a str>,
}

/// A game's cover in the theme's `game_card` style (Figma "game card"
/// `363:5954`). Not-installed games are dimmed.
pub fn game_tile(ui: &mut Ui, t: &Theme, url: &str, fallback_url: &str, title: &str, installed: bool, size: Vec2) -> Response {
    game_tile_info(ui, t, url, fallback_url, title, installed, size, &TileInfo::default()).0
}

/// [`game_tile`] with what shows on hover: the picture dims, the title
/// appears at the top and a strip slides up from the bottom with the
/// button and the facts in `info`. Returns the card's response and whether
/// the button was clicked.
#[allow(clippy::too_many_arguments)]
pub fn game_tile_info(ui: &mut Ui, t: &Theme, url: &str, fallback_url: &str, title: &str, installed: bool, size: Vec2, info: &TileInfo) -> (Response, bool) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return (resp, false);
    }
    let c = &t.components.game_card;
    // The button on the card takes the pointer; the card stays "hovered".
    let on = resp.contains_pointer() && ui.is_enabled();
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), on, 0.2);
    let r = t.radius_of(c);
    if !paint_cover_first(ui, rect, &[url, fallback_url], r) {
        placeholder(ui, t, rect, title, r);
    }
    let p = ui.painter_at(rect);
    if !installed {
        p.rect_filled(rect, r, t.overlay_dark(1));
    }
    // Sized for the design's 191 × 282 card, smaller on smaller or wide cards.
    let k = (size.x / 191.0).min(size.y / 282.0).clamp(0.5, 1.4);
    let mut clicked = false;
    if hover > 0.0 {
        // Dim the picture, darker at the top for the title, tinted below.
        p.rect_filled(rect, r, t.overlay_dark(1).gamma_multiply(hover));
        gradient(&p, rect, t.overlay_dark(2).gamma_multiply(hover), t.accent_2.gamma_multiply(0.44 * hover), false);
        let mut job = egui::text::LayoutJob::simple(title.to_string(), t.typography.body_l.font(), t.text.gamma_multiply(hover), rect.width() - 18.0 * k);
        job.wrap.max_rows = 2;
        job.wrap.overflow_character = Some('…');
        p.galley(rect.min + vec2(9.0, 11.0) * k, p.layout_job(job), t.text);
        // The strip slides up from below the card.
        let has_facts = info.last_played.is_some() || info.playtime.is_some();
        if info.action.is_some() || has_facts {
            let strip_h = 74.0 * k;
            let top = rect.bottom() - strip_h * hover;
            let strip = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), strip_h));
            let mut x = strip.left() + 9.0 * k;
            if let Some(icon) = info.action {
                let b = Rect::from_min_size(pos2(x, strip.top() + 11.0 * k), Vec2::splat(51.0 * k));
                let br = ui.interact(b.intersect(rect), resp.id.with("action"), Sense::click());
                let ib = &t.components.icon_box;
                let look = t.look(ib, Interact { selected: true, ..interact(ui, &br, true) });
                p.rect_filled(b, t.radius_of(ib), match look.fill {
                    ResolvedFill::Color(c) => c,
                    _ => t.overlay_light(1),
                });
                paint_icon(&p, b.center(), icon, 34.0 * k, t.text.gamma_multiply(hover));
                clicked = br.on_hover_text(info.action_tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                x = b.right() + 10.0 * k;
            }
            let fid = t.typography.body_s.font();
            let icon_color = t.text_dim.gamma_multiply(0.5 + 0.5 * hover);
            let mut y = strip.top() + 14.0 * k;
            for (icon, value) in [(crate::icons::CALENDAR, info.last_played), (crate::icons::CLOCK, info.playtime)] {
                let Some(value) = value else { continue };
                let line = Rect::from_min_size(pos2(x, y), vec2(strip.right() - x - 6.0, 20.0 * k));
                icon_centered(&p, pos2(line.left() + 10.0 * k, line.center().y), icon, 16.0 * k, icon_color);
                let mut job = egui::text::LayoutJob::simple_singleline(value.to_string(), fid.clone(), t.text.gamma_multiply(hover));
                job.wrap = egui::text::TextWrapping::truncate_at_width(line.width() - 25.0 * k);
                let g = p.layout_job(job);
                p.galley(pos2(line.left() + 25.0 * k, line.center().y - g.size().y / 2.0), g, t.text);
                y += 26.0 * k;
            }
        }
    }
    let look = t.look(c, Interact { hover, ..Default::default() });
    if look.border.a() > 0 && c.border > 0.0 {
        ui.painter().rect_stroke(rect, r, Stroke::new(c.border, look.border), StrokeKind::Outside);
    }
    (resp.on_hover_cursor(egui::CursorIcon::PointingHand), clicked)
}

/// A game card in the theme's card style ([`crate::theme::CardStyle`]):
/// the cover art (2:3) or the wide header picture (460:215), `width` wide.
/// Library pages should use this so a theme's choice shows.
pub fn game_card(ui: &mut Ui, t: &Theme, cover_url: &str, header_url: &str, title: &str, installed: bool, width: f32) -> Response {
    game_card_info(ui, t, cover_url, header_url, title, installed, width, &TileInfo::default()).0
}

/// [`game_card`] with what shows on hover (see [`game_tile_info`]).
#[allow(clippy::too_many_arguments)]
pub fn game_card_info(ui: &mut Ui, t: &Theme, cover_url: &str, header_url: &str, title: &str, installed: bool, width: f32, info: &TileInfo) -> (Response, bool) {
    let size = game_card_size(t, width);
    match t.layout.game_card {
        crate::theme::CardStyle::Cover => game_tile_info(ui, t, cover_url, header_url, title, installed, size, info),
        crate::theme::CardStyle::Header => game_tile_info(ui, t, header_url, cover_url, title, installed, size, info),
    }
}

/// The size of a [`game_card`] `width` wide in the theme's card style.
pub fn game_card_size(t: &Theme, width: f32) -> Vec2 {
    match t.layout.game_card {
        crate::theme::CardStyle::Cover => vec2(width, (width * 1.5).round()),
        crate::theme::CardStyle::Header => vec2(width, (width * 215.0 / 460.0).round()),
    }
}

/// A compact price (cards and rows): the price alone, or the discount box,
/// the old price struck through and the new price.
pub fn price_tag(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8) {
    let size = t.typography.price.size * 0.78;
    price_block(ui, t, label, original, discount, (size * 2.57).round(), size);
}

/// The price block at full size (Figma "price" `426:4419`).
pub fn price(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8) {
    let size = t.typography.price.size;
    price_block(ui, t, label, original, discount, (size * 2.45).round(), size);
}

fn price_block(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8, h: f32, fs: f32) {
    let weight = t.typography.price.weight.family();
    if discount == 0 {
        let g = ui.painter().layout_no_wrap(label.to_string(), FontId::new(fs, weight), t.price);
        let (rect, _) = ui.allocate_exact_size(vec2(g.size().x, h), Sense::hover());
        ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.price);
        return;
    }
    let c = &t.components.price;
    let look = t.look(c, Interact::default());
    let k = h / 44.0;
    let pct = ui.painter().layout_no_wrap(format!("-{discount}%"), FontId::new(fs * 1.054, weight.clone()), look.text);
    let new = ui.painter().layout_no_wrap(label.to_string(), FontId::new(fs, weight.clone()), t.price_new);
    let old = original.map(|o| ui.painter().layout_no_wrap(o.to_string(), FontId::new(fs * 0.78, weight.clone()), t.price_old));
    let block_w = (pct.size().x + 24.0 * k).max(75.0 * k);
    let gap = 10.0 * k;
    let price_w = new.size().x.max(old.as_ref().map(|g| g.size().x).unwrap_or(0.0));
    let (rect, _) = ui.allocate_exact_size(vec2(block_w + gap + price_w, h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let block = Rect::from_min_size(pos2(rect.left(), rect.center().y - 19.5 * k), vec2(block_w, 39.0 * k));
    paint_box(ui, t, block, c, &look, 0.0);
    let p = ui.painter();
    p.galley(block.center() - pct.size() / 2.0, pct, look.text);
    let x = block.right() + gap;
    match old {
        Some(old) => {
            let top = pos2(x, rect.top());
            let strike_y = top.y + old.size().y / 2.0;
            let old_w = old.size().x;
            p.galley(top, old, t.price_old);
            p.line_segment([pos2(top.x, strike_y), pos2(top.x + old_w, strike_y)], Stroke::new(1.0, t.price_old));
            p.galley(pos2(x, rect.bottom() - new.size().y), new, t.price_new);
        }
        None => {
            p.galley(pos2(x, rect.center().y - new.size().y / 2.0), new, t.price_new);
        }
    }
}

/// The store page's buy row (Figma "main-cta" `426:4355`): an optional
/// "Until October 19" note, the [`price`] block, and the `button_cta`
/// button (`button_label`, e.g. "Add to Cart"). Returns the button's response.
pub fn main_cta(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8, until: Option<&str>, button_label: &str) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.md + 1.0;
        if let Some(u) = until.filter(|_| discount > 0) {
            ui.vertical(|ui| {
                ui.add_space(t.spacing.xl);
                ui.label(body_s(t, u, t.text_dim));
            });
        }
        price(ui, t, label, original, discount);
        text_box(ui, t, &t.components.button_cta, None, button_label, false, 0.0, None)
    })
    .inner
}

// ---------- layout helpers ----------

/// A centered column at most `max` wide with at least `pad` on each side.
pub fn centered<R>(ui: &mut Ui, max: f32, pad: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let avail = ui.available_width();
    let width = (avail - pad * 2.0).min(max).max(100.0);
    let left = ((avail - width) / 2.0).max(0.0);
    let gap = ui.spacing().item_spacing;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(left);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = gap;
            add(ui)
        })
        .inner
    })
    .inner
}

pub fn page_title(ui: &mut Ui, t: &Theme, title: &str, subtitle: Option<&str>) {
    ui.label(styled(title, TextStyle::new(t.typography.heading_l.size + 2.0, t.typography.heading_l.weight), t.text));
    if let Some(s) = subtitle {
        ui.label(body_m(t, s, t.text_dim));
    }
    ui.add_space(t.spacing.md + 2.0);
}

/// Section header; returns true if the action link was clicked.
pub fn section(ui: &mut Ui, t: &Theme, title: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(heading_s(t, title, t.text));
        if let Some(a) = action {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add(egui::Label::new(styled(a, TextStyle::new(t.typography.body_m.size - 1.0, Weight::Medium), t.text_dim)).sense(Sense::click()));
                if r.hovered() {
                    ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, t.text_dim));
                }
                clicked = r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            });
        }
    });
    ui.add_space(t.spacing.xs + 2.0);
    clicked
}

/// Small caps label used to group lists.
pub fn caption(ui: &mut Ui, t: &Theme, s: &str) {
    ui.label(caption_text(t, s));
}

pub fn caption_text(t: &Theme, s: &str) -> RichText {
    styled(s.to_uppercase(), t.typography.caption, t.text_faint)
}

pub fn stat(ui: &mut Ui, t: &Theme, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(styled(label.to_uppercase(), TextStyle::new(t.typography.caption.size - 0.5, t.typography.caption.weight), t.text_faint));
        ui.label(styled(value, TextStyle::new(t.typography.body_m.size + 1.0, Weight::Semibold), t.text));
    });
}

pub fn loading(ui: &mut Ui, t: &Theme, label: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        ui.add(egui::Spinner::new().size(28.0).color(t.accent));
        ui.add_space(10.0);
        ui.label(body_m(t, label, t.text_dim));
    });
}

pub fn empty(ui: &mut Ui, t: &Theme, icon: &str, title: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(70.0);
        ui.label(text(icon, 44.0, t.text_faint));
        ui.add_space(8.0);
        ui.label(styled(title, TextStyle::new(t.typography.heading_s.size, Weight::Semibold), t.text));
        ui.label(body_m(t, body, t.text_dim));
    });
}

/// Thin progress bar (`progress`: track `normal`, filled part `selected`).
pub fn progress(ui: &mut Ui, t: &Theme, value: f32, width: f32) {
    // Never negative: egui aborts the whole app on a negative size.
    let width = width.max(0.0);
    let c = &t.components.progress;
    let (rect, _) = ui.allocate_exact_size(vec2(width, t.height_of(c, 6.0)), Sense::hover());
    let (track, done) = progress_colors(t);
    let r = t.radius_of(c);
    ui.painter().rect_filled(rect, r, track);
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * value.clamp(0.0, 1.0), rect.height()));
    ui.painter().rect_filled(fill, r, done);
}

/// The app's mark for its own bars and menus: the theme's logo picture if
/// it has one, else the cut-out logo in one color (white on dark themes,
/// black on light ones), drawn sharp at any size.
pub fn logo(ui: &mut Ui, t: &Theme, size: f32) -> Response {
    use crate::brand::Logo;
    if let Some(uri) = t.images.logo.as_deref().and_then(|p| crate::themes::image_uri(ui.ctx(), t, p)) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        egui::Image::new(uri).fit_to_exact_size(rect.size()).paint_at(ui, rect);
        return resp;
    }
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
fn choice_row(ui: &mut Ui, t: &Theme, label: impl Into<egui::WidgetText>, paint: impl FnOnce(&egui::Painter, Rect, Interact)) -> Response {
    let galley = label.into().into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
    let box_size = t.dense(18.0);
    let size = vec2(box_size + 10.0 + galley.size().x, galley.size().y.max(box_size) + 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let s = interact(ui, &resp, false);
    let mark = Rect::from_center_size(egui::pos2(rect.left() + box_size / 2.0, rect.center().y), Vec2::splat(box_size));
    paint(ui.painter(), mark, s);
    let color = ui.visuals().text_color();
    ui.painter().galley(egui::pos2(mark.right() + 10.0, rect.center().y - galley.size().y / 2.0), galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A radio button in the check box's colors, with an accent dot.
pub fn radio(ui: &mut Ui, t: &Theme, selected: bool, label: impl Into<egui::WidgetText>) -> Response {
    choice_row(ui, t, label, |p, r, s| {
        let look = t.look(&t.components.check, Interact { selected: false, ..s });
        let ring = if selected { t.accent } else { look.border };
        p.circle_filled(r.center(), r.width() / 2.0, t.raised);
        p.circle_stroke(r.center(), r.width() / 2.0 - 0.5, Stroke::new(1.6, ring));
        if selected {
            p.circle_filled(r.center(), r.width() / 4.0, t.accent);
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

/// A check box with its label (`check`; Figma `757:10005`).
pub fn check(ui: &mut Ui, t: &Theme, on: &mut bool, label: impl Into<egui::WidgetText>) -> Response {
    let state = *on;
    let mut r = choice_row(ui, t, label, |p, r, s| paint_check(p, t, r, Interact { selected: state, ..s }));
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
    let s = interact(ui, &resp, *on);
    paint_check(ui.painter(), t, rect.shrink(size * 0.08), s);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_check(p: &egui::Painter, t: &Theme, r: Rect, s: Interact) {
    let c = &t.components.check;
    let look = t.look(c, s);
    let rr = t.radius_of(c);
    if let ResolvedFill::Color(f) = look.fill {
        p.rect_filled(r, rr, f);
    }
    if look.border.a() > 0 {
        p.rect_stroke(r, rr, Stroke::new(c.border.max(1.0), look.border), StrokeKind::Inside);
    }
    if s.selected {
        let st = Stroke::new((r.width() * 0.11).max(1.8), look.text);
        let (w, h) = (r.width(), r.height());
        let a = pos2(r.left() + w * 0.22, r.top() + h * 0.52);
        let b = pos2(r.left() + w * 0.42, r.top() + h * 0.72);
        let cc = pos2(r.left() + w * 0.78, r.top() + h * 0.30);
        p.line_segment([a, b], st);
        p.line_segment([b, cc], st);
    }
}

/// A chip with a small picture (a game icon, an avatar) before the label.
pub fn image_chip(ui: &mut Ui, t: &Theme, image: Option<&str>, label: &str, selected: bool) -> Response {
    let c = &t.components.chip;
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    let galley = ui.painter().layout_no_wrap(label.to_string(), fid, Color32::PLACEHOLDER);
    let pic = if image.is_some() { 26.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + pad * 2.0 + pic, t.height_of(c, 30.0)), Sense::click());
    let look = t.look(c, interact(ui, &resp, selected));
    paint_box(ui, t, rect, c, &look, 0.0);
    if let Some(url) = image {
        let img = Rect::from_center_size(egui::pos2(rect.left() + pad + 6.0, rect.center().y), Vec2::splat(20.0));
        if !paint_cover(ui, img, url, CornerRadius::same(4)) {
            skeleton(ui, t, img, CornerRadius::same(4));
        }
    }
    ui.painter().galley(egui::pos2(rect.left() + pad + pic, rect.center().y - galley.size().y / 2.0), galley, look.text);
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

    #[test]
    fn mixing_see_through_colors_keeps_them() {
        use egui::Color32;
        let veil = Color32::WHITE.gamma_multiply(0.1);
        // A hover from a clear fill ends exactly on the hover color...
        assert_eq!(super::mix(Color32::TRANSPARENT, veil, 1.0), veil);
        // ...and halfway is half as strong, not darker.
        let half = super::mix(Color32::TRANSPARENT, veil, 0.5);
        assert!(half.a() > 0 && half.r() >= half.a() - 1);
        // Solid colors blend as before.
        assert_eq!(super::mix(Color32::BLACK, Color32::WHITE, 0.5), Color32::from_rgb(128, 128, 128));
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

// ---------- the store ----------

/// Page dots under a carousel (`pager`; Figma `634:15220`): `n` dots
/// `width` wide, the current one filling up as `fill` goes from 0 to 1 (how
/// long until it moves on; 1 when it doesn't). Returns a dot that was clicked.
pub fn pager(ui: &mut Ui, t: &Theme, n: usize, current: usize, fill: f32, width: f32) -> Option<usize> {
    let c = &t.components.pager;
    let h = t.height_of(c, 8.0);
    let gap = t.spacing.sm + 2.0;
    let total = n as f32 * width + n.saturating_sub(1) as f32 * gap;
    let (rect, base) = ui.allocate_exact_size(vec2(ui.available_width().max(total), h.max(14.0)), Sense::hover());
    let mut picked = None;
    let left = rect.center().x - total / 2.0;
    for i in 0..n {
        let dot = Rect::from_min_size(pos2(left + i as f32 * (width + gap), rect.center().y - h / 2.0), vec2(width, h));
        let resp = ui.interact(dot.expand2(vec2(gap / 2.0, 4.0)), base.id.with(("pager", i)), Sense::click());
        let look = t.look(c, interact(ui, &resp, false));
        paint_box(ui, t, dot, c, &look, 0.0);
        if i == current {
            let on = t.look(c, Interact { selected: true, ..Default::default() });
            // Starts as a round dot and grows to the full width.
            let w = h + (width - h) * fill.clamp(0.0, 1.0);
            paint_box(ui, t, Rect::from_min_size(dot.min, vec2(w, h)), c, &on, 0.0);
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            picked = Some(i);
        }
    }
    picked
}

/// [`main_cta`] for a right-to-left row (the button is added first, so it
/// stays on the right): the price block, then the `button_cta` button.
pub fn main_cta_rtl(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8, button_label: &str) -> Response {
    let r = text_box(ui, t, &t.components.button_cta, None, button_label, false, 0.0, None);
    ui.add_space(t.spacing.md + 1.0 - ui.spacing().item_spacing.x);
    if !label.is_empty() {
        price(ui, t, label, original, discount);
    }
    r
}

// ---------- community ----------

/// A post in a feed (`post_card`; Figma `494:5547`).
pub fn post_card(t: &Theme) -> Frame {
    component_frame(t, &t.components.post_card)
}

/// The bar a feed's filters and sort sit in (`filter_bar`; Figma `489:5317`).
pub fn filter_bar(t: &Theme) -> Frame {
    component_frame(t, &t.components.filter_bar)
}

/// A small box under a post: an icon, an optional label before it and a
/// count after it (`stat_chip`; Figma `494:5552`, "Yes 👍 374"). Clickable
/// when the caller uses the response.
pub fn stat_chip(ui: &mut Ui, t: &Theme, icon: &str, label: Option<&str>, count: Option<&str>) -> Response {
    let c = &t.components.stat_chip;
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    let gap = t.spacing.xs + 1.0;
    let painter = ui.painter().clone();
    let lg = label.map(|l| painter.layout_no_wrap(l.to_string(), fid.clone(), Color32::PLACEHOLDER));
    let cg = count.map(|n| painter.layout_no_wrap(n.to_string(), fid.clone(), Color32::PLACEHOLDER));
    let icon_w = (fid.size * 1.4).round();
    let mut w = pad * 2.0 + icon_w;
    for g in lg.iter().chain(cg.iter()) {
        w += g.size().x + gap;
    }
    let h = t.height_of(c, 37.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, interact(ui, &resp, false));
        paint_box(ui, t, rect, c, &look, 0.0);
        let count_color = t.look(c, Interact { selected: true, ..Default::default() }).text;
        let mut x = rect.left() + pad;
        let y = rect.center().y;
        if let Some(g) = lg {
            let gw = g.size().x;
            ui.painter().galley(pos2(x, y - g.size().y / 2.0), g, look.text);
            x += gw + gap;
        }
        paint_icon(ui.painter(), pos2(x + icon_w / 2.0, y), icon, icon_w, look.icon);
        x += icon_w + gap;
        if let Some(g) = cg {
            ui.painter().galley(pos2(x, y - g.size().y / 2.0), g, count_color);
        }
    }
    resp
}

/// The big search box of a landing page (`search_large`; Figma `487:5333`),
/// 55 px tall in the default theme.
pub fn search_large(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32) -> Response {
    let c = &t.components.search_large;
    let h = t.height_of(c, 55.0);
    search_in(ui, t, c, value, hint, width, h)
}


/// Numbered paging (Figma `505:8934`): Prev, the first page, the pages
/// around this one, "…", the last page, Next, in the `sort` style. Returns
/// the page clicked.
pub fn page_numbers(ui: &mut Ui, t: &Theme, page: u32, pages: u32) -> Option<u32> {
    if pages <= 1 {
        return None;
    }
    let page = page.clamp(1, pages);
    let mut to = None;
    // 1 … 4 5 [6] 7 8 … 20
    let mut shown: Vec<u32> = vec![1, pages];
    for p in page.saturating_sub(2).max(1)..=(page + 2).min(pages) {
        shown.push(p);
    }
    if page <= 4 {
        shown.extend(1..=5.min(pages));
    }
    shown.sort_unstable();
    shown.dedup();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.xs;
        ui.add_enabled_ui(page > 1, |ui| {
            if sort_chip(ui, t, i18n::t!("Prev"), false).clicked() {
                to = Some(page - 1);
            }
        });
        ui.add_space(t.spacing.md);
        let mut last = 0;
        for p in shown {
            if p > last + 1 {
                ui.label(styled("…", t.text_style(&t.components.sort.text), t.text_dim));
            }
            if sort_chip(ui, t, &p.to_string(), p == page).clicked() && p != page {
                to = Some(p);
            }
            last = p;
        }
        ui.add_space(t.spacing.md);
        ui.add_enabled_ui(page < pages, |ui| {
            if sort_chip(ui, t, i18n::t!("Next"), false).clicked() {
                to = Some(page + 1);
            }
        });
    });
    to
}

/// A remote image covering `rect` (center-cropped), `opacity` strong at the
/// top and fading out to nothing at the bottom: art softly behind a page,
/// over whatever background the theme paints. False while loading.
pub fn paint_cover_fading(ui: &Ui, rect: Rect, url: &str, opacity: f32) -> bool {
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
    let top = Color32::WHITE.gamma_multiply(opacity.clamp(0.0, 1.0));
    let mut mesh = egui::Mesh::with_texture(texture.id);
    let v = |pos: egui::Pos2, uv: egui::Pos2, color: Color32| egui::epaint::Vertex { pos, uv, color };
    mesh.vertices.extend([
        v(rect.left_top(), uv.left_top(), top),
        v(rect.right_top(), uv.right_top(), top),
        v(rect.right_bottom(), uv.right_bottom(), Color32::TRANSPARENT),
        v(rect.left_bottom(), uv.left_bottom(), Color32::TRANSPARENT),
    ]);
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    ui.painter().add(egui::Shape::mesh(mesh));
    true
}

// ---------- rows, boxes and fields (profile and settings) ----------

/// A full-width row in a row component (`profile_row`, `settings_nav_item`):
/// an optional icon, the label on the left and `value` on the right, in
/// the state's `icon` color unless `value_color` says otherwise.
#[allow(clippy::too_many_arguments)]
pub fn list_row(ui: &mut Ui, t: &Theme, c: &Component, icon: Option<&str>, label: &str, value: &str, value_color: Option<Color32>, selected: bool) -> Response {
    let fid = t.font_of(c);
    let pad = t.padding_of(c);
    let h = t.height_of(c, fid.size + pad.y * 2.0 + 6.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(h), h), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, interact(ui, &resp, selected));
        paint_box(ui, t, rect, c, &look, 0.0);
        let mut x = rect.left() + pad.x;
        if let Some(i) = icon {
            paint_icon(ui.painter(), pos2(x + fid.size * 0.55, rect.center().y), i, fid.size * 1.1, look.icon);
            x += fid.size * 1.1 + (fid.size * 0.55).round();
        }
        let right = rect.right() - pad.x;
        let vg = (!value.is_empty()).then(|| ui.painter().layout_no_wrap(value.to_string(), fid.clone(), Color32::PLACEHOLDER));
        let vw = vg.as_ref().map(|g| g.size().x + pad.x).unwrap_or(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), fid.clone(), look.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - vw - x).max(10.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, look.text);
        if let Some(vg) = vg {
            ui.painter().galley(pos2(right - vg.size().x, rect.center().y - vg.size().y / 2.0), vg, value_color.unwrap_or(look.icon));
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A box in a component's style (`profile_box`, `settings_group`) with a
/// title in the component's text style and color, and a note on the
/// title's right. An empty title draws just the box.
pub fn titled_box<R>(ui: &mut Ui, t: &Theme, c: &Component, title: &str, note: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    component_frame(t, c)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() || !note.is_empty() {
                let look = t.look(c, Interact::default());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).font(t.font_of(c)).color(look.text));
                    if !note.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(body_s(t, note, look.text));
                        });
                    }
                });
            }
            add(ui)
        })
        .inner
}

/// A text field in the `input` style, `width` wide; more than one row
/// makes it a text area.
pub fn text_input(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32, rows: usize) -> Response {
    let c = &t.components.input;
    let fid = t.font_of(c);
    let pad = t.padding_of(c);
    let line = ui.fonts_mut(|f| f.row_height(&fid));
    let single = t.height_of(c, 34.0);
    let h = if rows <= 1 { single } else { (line * rows as f32 + (single - line).max(pad.x)).round() };
    let width = field_width(ui, &fid, hint, width, pad.x * 2.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    let rest = t.look(c, Interact::default());
    paint_box(ui, t, rect, c, &rest, 0.0);
    let inner = if rows <= 1 {
        Rect::from_min_max(pos2(rect.left() + pad.x, rect.top()), pos2(rect.right() - pad.x, rect.bottom()))
    } else {
        rect.shrink2(vec2(pad.x, ((single - line) / 2.0).max(4.0)))
    };
    let edit = if rows <= 1 { egui::TextEdit::singleline(value).align(Align2::LEFT_CENTER) } else { egui::TextEdit::multiline(value).desired_rows(rows) };
    let resp = ui
        .place(
            inner,
            edit.hint_text(RichText::new(hint).font(fid.clone()).color(rest.icon))
                .font(fid.clone())
                .text_color(rest.text)
                .frame(Frame::NONE)
                .margin(vec2(0.0, 0.0))
                .desired_width(inner.width()),
        )
        .with_edit_menu();
    let resp = if rows <= 1 { full_text_on_hover(ui, resp, &fid, value, hint, inner.width()) } else { resp };
    if rows <= 1 {
        fade_overflow(ui, t, c, inner, &fid, value, resp.has_focus());
    }
    if resp.has_focus() {
        let focus = t.look(c, Interact { selected: true, ..Default::default() });
        if focus.border.a() > 0 {
            ui.painter().rect_stroke(rect, t.radius_of(c), Stroke::new(c.border.max(1.0), focus.border), StrokeKind::Inside);
        }
    }
    resp
}

/// A dropdown field in the `input` style showing the chosen option, with
/// the options in a menu. True when the choice changed.
pub fn select<T: PartialEq + Clone>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, String)], width: f32) -> bool {
    let c = &t.components.input;
    let fid = t.font_of(c);
    let pad = t.padding_of(c);
    let shown = options.iter().find(|(v, _)| v == value).map(|(_, l)| l.clone()).unwrap_or_default();
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(80.0), t.height_of(c, 34.0)), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    if ui.is_rect_visible(rect) {
        let s = interact(ui, &resp, open);
        let look = t.look(c, Interact { selected: false, ..s });
        paint_box(ui, t, rect, c, &look, 0.0);
        if s.hover > 0.0 {
            ui.painter().rect_filled(rect, t.radius_of(c), Color32::WHITE.gamma_multiply(0.03 * s.hover));
        }
        let mut job = egui::text::LayoutJob::simple_singleline(shown.clone(), fid.clone(), look.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - pad.x * 2.0 - 20.0).max(10.0));
        if ui.painter().layout_no_wrap(shown.clone(), fid.clone(), Color32::PLACEHOLDER).size().x > rect.width() - pad.x * 2.0 - 20.0 {
            ui.data_mut(|d| d.insert_temp(resp.id.with("shortened"), shown.clone()));
        } else {
            ui.data_mut(|d| d.remove::<String>(resp.id.with("shortened")));
        }
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(rect.left() + pad.x, rect.center().y - g.size().y / 2.0), g, look.text);
        let caret = if open { crate::icons::CARET_UP } else { crate::icons::CARET_DOWN };
        ui.painter().text(pos2(rect.right() - pad.x, rect.center().y), Align2::RIGHT_CENTER, caret, font(fid.size), look.text);
    }
    let mut resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if let Some(full) = ui.data(|d| d.get_temp::<String>(resp.id.with("shortened"))) {
        resp = resp.on_hover_text(full);
    }
    let mut changed = false;
    egui::Popup::menu(&resp).width(rect.width()).show(|ui| {
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (v, label) in options {
                let on = v == value;
                let r = menu_item_tinted(ui, t, if on { crate::icons::CHECK } else { "" }, label, on.then_some(t.accent));
                if r.clicked() && !on {
                    *value = v.clone();
                    changed = true;
                }
            }
        });
    });
    changed
}

/// The small "i" after a setting's name, with its explanation on hover.
pub fn info_tip(ui: &mut Ui, t: &Theme, tip: &str) -> Response {
    let size = t.typography.body_l.size + 2.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let color = if resp.hovered() { t.text } else { t.text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, crate::icons::INFO, font(size - 2.0), color);
    resp.on_hover_text(tip)
}

// ---------- friends ----------

/// A friend's picture in the theme's `friend_avatar` style: a square with
/// an outline in their status color (the redesign), or round with a status
/// dot when the style has no `border` (Classic). `status` `None` draws it
/// plain.
pub fn presence_avatar(ui: &mut Ui, t: &Theme, url: &str, size: f32, status: Option<Color32>) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_presence_avatar(ui, t, rect, url, status);
    resp
}

/// [`presence_avatar`] painted into `rect`.
pub fn paint_presence_avatar(ui: &Ui, t: &Theme, rect: Rect, url: &str, status: Option<Color32>) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let c = &t.components.friend_avatar;
    let radius = t.radius_of(c);
    // A radius past half the size is a circle.
    let radius = CornerRadius::same(radius.nw.min((rect.width() / 2.0) as u8));
    if !paint_cover(ui, rect, url, radius) {
        ui.painter().rect_filled(rect, radius, t.surface_hover);
    }
    let Some(color) = status else { return };
    if c.border > 0.0 {
        ui.painter().rect_stroke(rect, radius, Stroke::new(c.border, color), StrokeKind::Inside);
    } else {
        let r = (rect.width() * 0.14).max(4.0);
        let center = rect.right_bottom() - Vec2::splat(r * 0.9);
        ui.painter().circle_filled(center, r + 2.0, t.chrome);
        ui.painter().circle_filled(center, r, color);
    }
}

/// A frame in component `c`'s style that lights up under the pointer (its
/// `hover` state) and can be clicked anywhere no other control is: post
/// cards, topic rows, Workshop rows. `id` must stay the same between
/// frames. Returns what `add` returned and the frame's response (hovered
/// while the pointer is anywhere on it; clicked on its own empty parts).
pub fn hover_frame<R>(ui: &mut Ui, t: &Theme, c: &Component, id: egui::Id, margin: Option<Margin>, add: impl FnOnce(&mut Ui) -> R) -> (R, Response) {
    // The frame's place from last frame, claimed before its contents so
    // they keep their own clicks.
    let last: Option<Rect> = ui.data(|d| d.get_temp(id));
    let behind = last.map(|r| ui.interact(r, id, Sense::click()));
    let hovered = behind.as_ref().is_some_and(|r| r.contains_pointer());
    let anim = ui.ctx().animate_bool_with_time(id.with("hover"), hovered, 0.12);
    let look = t.look(c, Interact { hover: anim, ..Default::default() });
    let mut frame = component_frame(t, c);
    if let ResolvedFill::Color(fill) = look.fill {
        frame = frame.fill(fill);
    }
    if look.border.a() > 0 && c.border > 0.0 {
        frame = frame.stroke(Stroke::new(c.border, look.border));
    }
    if let Some(m) = margin {
        frame = frame.inner_margin(m);
    }
    let inner = frame.show(ui, add);
    let rect = inner.response.rect;
    if last != Some(rect) {
        ui.data_mut(|d| d.insert_temp(id, rect));
        ui.ctx().request_repaint();
    }
    let resp = match behind {
        Some(r) => r.on_hover_cursor(egui::CursorIcon::PointingHand),
        None => ui.interact(Rect::NOTHING, id.with("first"), Sense::hover()),
    };
    (inner.inner, resp)
}

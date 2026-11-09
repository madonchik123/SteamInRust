//! Reusable widgets. Everything takes the [`Theme`] explicitly so plugins,
//! extensions and built-in pages all look the same.

use crate::Theme;
use crate::theme::{bold, medium, semibold};
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

/// Failed loads: url -> (attempts, when the last one failed).
static FAILED: std::sync::Mutex<Option<std::collections::HashMap<String, (u32, f64)>>> = std::sync::Mutex::new(None);
const RETRIES: u32 = 3;

/// Load an image, retrying failed downloads a few times with a pause.
/// Steam's image servers sometimes drop a connection when many images load
/// at once; without this a dropped one would stay blank.
fn load(ui: &Ui, url: &str, size: Vec2) -> (ImageState, Option<egui::load::SizedTexture>) {
    if url.is_empty() {
        return (ImageState::Missing, None);
    }
    let img = egui::Image::new(url.to_owned());
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
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            (ImageState::Loading, None)
        }
    }
}

/// Whether an image loaded, is loading, or is missing.
pub fn image_state(ui: &Ui, url: &str) -> ImageState {
    load(ui, url, Vec2::splat(64.0)).0
}

/// Draw a remote image covering `rect` (center-cropped). False while loading.
pub fn paint_cover(ui: &Ui, rect: Rect, url: &str, radius: CornerRadius) -> bool {
    let (_, Some(texture)) = load(ui, url, rect.size()) else {
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
        match load(ui, url, rect.size()) {
            (_, Some(_)) => return paint_cover(ui, rect, url, radius),
            (ImageState::Missing, _) => continue,
            _ => return false,
        }
    }
    false
}

/// Draw a remote image as large as fits in `rect`, anchored by `align`.
pub fn paint_contain(ui: &Ui, rect: Rect, url: &str, align: Align2) -> bool {
    let (_, Some(texture)) = load(ui, url, rect.size()) else {
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

pub fn card(t: &Theme) -> Frame {
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(t.corner())
        .inner_margin(Margin::same(18))
}

pub fn panel(t: &Theme) -> Frame {
    Frame::new().fill(t.raised).corner_radius(t.corner_small()).inner_margin(Margin::same(12))
}

// ---------- buttons ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Play,
    Danger,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Sm,
    Md,
    Lg,
}

/// The one button. `icon` is a glyph from [`crate::icons`].
pub fn button(ui: &mut Ui, t: &Theme, icon: Option<&str>, label: &str, kind: Kind, size: Size) -> Response {
    let (h, fs, pad) = match size {
        Size::Sm => (30.0, 13.0, 12.0),
        Size::Md => (38.0, 14.0, 16.0),
        Size::Lg => (48.0, 16.0, 24.0),
    };
    let text = match icon {
        Some(i) if !label.is_empty() => format!("{i}  {label}"),
        Some(i) => i.to_string(),
        None => label.to_string(),
    };
    let (bg, fg, border) = match kind {
        Kind::Primary => (t.accent, t.on_accent, Color32::TRANSPARENT),
        Kind::Play => (t.play, Color32::WHITE, Color32::TRANSPARENT),
        Kind::Danger => (t.danger.gamma_multiply(0.18), t.danger, t.danger.gamma_multiply(0.4)),
        Kind::Secondary => (t.raised, t.text, t.border),
        Kind::Ghost => (Color32::TRANSPARENT, t.text_dim, Color32::TRANSPARENT),
    };
    let galley = ui.painter().layout_no_wrap(text, font_semibold(fs), fg);
    let w = if label.is_empty() { h } else { galley.size().x + pad * 2.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() && enabled, 0.12);
        let pressed = resp.is_pointer_button_down_on();
        let fill = match kind {
            Kind::Ghost => mix(Color32::TRANSPARENT, t.surface_hover, hover),
            Kind::Secondary => mix(bg, t.surface_hover, hover),
            _ => mix(bg, mix(bg, Color32::WHITE, 0.14), hover),
        };
        let fill = if pressed { mix(fill, Color32::BLACK, 0.12) } else { fill };
        let fill = if enabled { fill } else { fill.gamma_multiply(0.45) };
        let r = t.corner_small();
        let p = ui.painter();
        if matches!(kind, Kind::Primary | Kind::Play) && enabled {
            p.rect_filled(rect.translate(vec2(0.0, 2.0)), r, bg.gamma_multiply(0.25 + 0.2 * hover));
        }
        p.rect_filled(rect, r, fill);
        if matches!(kind, Kind::Primary | Kind::Play) {
            // Soft top highlight for depth.
            let top = Rect::from_min_max(rect.min, pos2(rect.right(), rect.center().y));
            gradient(&ui.painter_at(top.shrink2(vec2(r.nw as f32 * 0.6, 0.0))), top, Color32::from_white_alpha(18), Color32::TRANSPARENT, false);
        }
        if border != Color32::TRANSPARENT {
            p.rect_stroke(rect, r, Stroke::new(1.0, border), StrokeKind::Inside);
        }
        let fg = if enabled { mix(fg, if kind == Kind::Ghost { t.text } else { fg }, hover) } else { fg.gamma_multiply(0.6) };
        p.galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Square icon button with optional red badge.
pub fn icon_button(ui: &mut Ui, t: &Theme, icon: &str, tooltip: &str, active: bool, badge: usize) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    let bg = if active { t.surface_hover } else { mix(Color32::TRANSPARENT, t.surface_hover, hover) };
    p.rect_filled(rect, t.corner_small(), bg);
    let color = if active { t.text } else { mix(t.text_dim, t.text, hover) };
    p.text(rect.center(), Align2::CENTER_CENTER, icon, font(18.0), color);
    if badge > 0 {
        let c = rect.right_top() + vec2(-8.0, 8.0);
        let label = if badge > 99 { "99+".to_string() } else { badge.to_string() };
        let w = (label.len() as f32 * 6.0 + 8.0).max(16.0);
        let b = Rect::from_center_size(c, vec2(w, 16.0));
        p.rect_filled(b, CornerRadius::same(8), t.danger);
        p.text(c, Align2::CENTER_CENTER, label, font_bold(10.0), Color32::WHITE);
    }
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Top navigation tab with an animated underline.
/// A row in a dropdown menu: icon and label, the full menu width, with a
/// highlight while hovered so it's clear which item a click will hit.
pub fn menu_item(ui: &mut Ui, t: &Theme, icon: &str, label: &str) -> Response {
    let width = ui.available_width().max(160.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.08);
        let p = ui.painter();
        if hover > 0.0 {
            p.rect_filled(rect, t.corner_small(), t.accent.gamma_multiply(0.16 * hover));
        }
        let y = rect.center().y;
        p.text(pos2(rect.left() + 18.0, y), egui::Align2::CENTER_CENTER, icon, font(14.0), mix(t.text_dim, t.accent, hover));
        p.text(pos2(rect.left() + 36.0, y), egui::Align2::LEFT_CENTER, label, font_medium(13.5), mix(t.text_dim, t.text, 0.6 + 0.4 * hover));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn nav_tab(ui: &mut Ui, t: &Theme, label: &str, selected: bool, height: f32) -> Response {
    // Laid out without a color, so the state color below applies (a fixed
    // color made every tab look the same, hovered or not).
    let galley = ui.painter().layout_no_wrap(label.to_uppercase(), font_bold(13.0), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, height), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let sel = ui.ctx().animate_bool_with_time(resp.id.with("sel"), selected, 0.18);
    let color = mix(mix(t.text_dim, t.text, hover), t.text, sel);
    let p = ui.painter();
    // A soft pill behind the hovered tab.
    if hover > 0.0 {
        let bg = Rect::from_center_size(rect.center(), vec2(rect.width() - 4.0, (galley.size().y + 14.0).min(rect.height())));
        p.rect_filled(bg, CornerRadius::same(6), t.surface_hover.gamma_multiply(hover * 0.8));
    }
    p.galley(rect.center() - galley.size() / 2.0, galley, color);
    // Accent underline for the open page; a faint one while hovering others.
    let underline = sel.max(hover * 0.45);
    if underline > 0.0 {
        let w = (rect.width() - 24.0) * underline.max(sel);
        let line = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 1.5), vec2(w, 3.0));
        p.rect_filled(line, CornerRadius::same(2), if sel > 0.0 { t.accent } else { t.text_faint.gamma_multiply(hover) });
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
    let galley = ui.painter().layout_no_wrap(label.to_string(), font_medium(13.0), Color32::PLACEHOLDER);
    let dot = if tint.is_some() { 14.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0 + dot, 30.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    let r = CornerRadius::same(15);
    let accent = tint.unwrap_or(t.accent);
    if selected {
        p.rect_filled(rect, r, accent.gamma_multiply(0.20));
        p.rect_stroke(rect, r, Stroke::new(1.0, accent.gamma_multiply(0.8)), StrokeKind::Inside);
    } else {
        p.rect_filled(rect, r, mix(t.raised, t.surface_hover, hover));
        p.rect_stroke(rect, r, Stroke::new(1.0, tint.map(|c| c.gamma_multiply(0.45)).unwrap_or(t.border)), StrokeKind::Inside);
    }
    let color = match (tint, selected) {
        (Some(c), _) => mix(c, t.text, if selected { 0.25 } else { 0.1 + hover * 0.2 }),
        (None, true) => t.accent,
        (None, false) => mix(t.text_dim, t.text, hover),
    };
    if let Some(c) = tint {
        p.circle_filled(egui::pos2(rect.left() + 16.0, rect.center().y), 4.0, c);
    }
    let pos = egui::pos2(rect.left() + 12.0 + dot, rect.center().y - galley.size().y / 2.0);
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

/// Pill-shaped segmented control. Returns true if the selection changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(t.raised)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(t.corner_small())
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (opt, label) in options {
                    let on = *value == *opt;
                    let galley = ui.painter().layout_no_wrap(label.to_string(), font_medium(13.0), Color32::PLACEHOLDER);
                    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, 26.0), Sense::click());
                    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                    if on {
                        ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
                    }
                    let c = if on { t.text } else { mix(t.text_faint, t.text_dim, hover) };
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

/// Compact pill search field (34 px tall) with a clear button.
pub fn search(ui: &mut Ui, t: &Theme, value: &mut String, hint: &str, width: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    let r = CornerRadius::same(17);
    ui.painter().rect_filled(rect, r, t.raised);
    ui.painter().text(rect.left_center() + vec2(15.0, 0.0), Align2::CENTER_CENTER, crate::icons::SEARCH, font(14.0), t.text_faint);
    let clear_w = if value.is_empty() { 0.0 } else { 26.0 };
    let field = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(rect.right() - 12.0 - clear_w, rect.bottom()));
    let resp = ui.put(
        field,
        egui::TextEdit::singleline(value)
            .hint_text(text(hint, 13.5, t.text_faint))
            .font(font(13.5))
            .frame(Frame::NONE)
            .margin(vec2(0.0, 0.0))
            .align(Align2::LEFT_CENTER)
            .desired_width(field.width()),
    );
    let border = if resp.has_focus() { t.accent.gamma_multiply(0.8) } else { t.border };
    ui.painter().rect_stroke(rect, r, Stroke::new(1.0, border), StrokeKind::Inside);
    if !value.is_empty() {
        let c = pos2(rect.right() - 20.0, rect.center().y);
        let x = ui.interact(Rect::from_center_size(c, Vec2::splat(20.0)), resp.id.with("clear"), Sense::click());
        ui.painter().text(c, Align2::CENTER_CENTER, crate::icons::X, font(12.0), if x.hovered() { t.text } else { t.text_faint });
        if x.clicked() {
            value.clear();
        }
    }
    resp
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
    )
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

/// Price with an optional discount badge and struck-through original.
pub fn price_tag(ui: &mut Ui, t: &Theme, label: &str, original: Option<&str>, discount: u8) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if discount > 0 {
            pill(ui, &format!("-{discount}%"), Color32::from_rgb(0xBE, 0xEE, 0x11), t.discount);
            if let Some(o) = original {
                ui.label(text(o, 12.0, t.text_faint).strikethrough());
            }
        }
        ui.label(text_semibold(label, 14.0, t.text));
    });
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
    ui.painter().rect_filled(rect, CornerRadius::same(3), t.raised);
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * value.clamp(0.0, 1.0), rect.height()));
    ui.painter().rect_filled(fill, CornerRadius::same(3), t.accent);
}

/// The app mark: a diamond drawn as a shape.
pub fn logo(ui: &mut Ui, t: &Theme, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let c = rect.center();
    let r = size / 2.0;
    let pts = |s: f32| vec![c - vec2(0.0, s), c + vec2(s, 0.0), c + vec2(0.0, s), c - vec2(s, 0.0)];
    ui.painter().add(egui::Shape::convex_polygon(pts(r), t.accent, Stroke::NONE));
    ui.painter().add(egui::Shape::convex_polygon(pts(r * 0.62), t.accent_2, Stroke::NONE));
    ui.painter().add(egui::Shape::convex_polygon(pts(r * 0.26), t.bg, Stroke::NONE));
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

/// A clearly visible radio button.
pub fn radio(ui: &mut Ui, t: &Theme, selected: bool, label: impl Into<egui::WidgetText>) -> Response {
    choice_row(ui, label, |p, r, hover| {
        let ring = if selected { t.accent } else { mix(t.text_faint, t.text_dim, hover) };
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

/// A clearly visible checkbox.
pub fn check(ui: &mut Ui, t: &Theme, on: &mut bool, label: impl Into<egui::WidgetText>) -> Response {
    let state = *on;
    let mut r = choice_row(ui, label, |p, r, hover| {
        let rr = CornerRadius::same(4);
        if state {
            p.rect_filled(r, rr, t.accent);
            let s = Stroke::new(2.0, t.on_accent);
            let a = egui::pos2(r.left() + 4.5, r.center().y + 0.5);
            let b = egui::pos2(r.left() + 7.8, r.bottom() - 5.0);
            let c = egui::pos2(r.right() - 4.0, r.top() + 5.0);
            p.line_segment([a, b], s);
            p.line_segment([b, c], s);
        } else {
            p.rect_filled(r, rr, t.raised);
            p.rect_stroke(r, rr, Stroke::new(1.6, mix(t.text_faint, t.text_dim, hover)), StrokeKind::Inside);
        }
    });
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r
}

/// A chip with a small picture (a game icon, an avatar) before the label.
pub fn image_chip(ui: &mut Ui, t: &Theme, image: Option<&str>, label: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), font_medium(13.0), Color32::PLACEHOLDER);
    let pic = if image.is_some() { 26.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0 + pic, 30.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let r = CornerRadius::same(15);
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, r, t.accent.gamma_multiply(0.18));
        p.rect_stroke(rect, r, Stroke::new(1.0, t.accent.gamma_multiply(0.7)), StrokeKind::Inside);
    } else {
        p.rect_filled(rect, r, mix(t.raised, t.surface_hover, hover));
        p.rect_stroke(rect, r, Stroke::new(1.0, t.border), StrokeKind::Inside);
    }
    if let Some(url) = image {
        let img = Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(20.0));
        if !paint_cover(ui, img, url, CornerRadius::same(4)) {
            skeleton(ui, t, img, CornerRadius::same(4));
        }
    }
    let color = if selected { t.accent } else { mix(t.text_dim, t.text, hover) };
    ui.painter().galley(egui::pos2(rect.left() + 12.0 + pic, rect.center().y - galley.size().y / 2.0), galley, color);
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

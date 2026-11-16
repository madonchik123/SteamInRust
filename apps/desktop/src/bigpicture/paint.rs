//! Drawing pieces for Big Picture: tiles, pill buttons, text that fits,
//! glows and the moving background.

use egui::epaint::Shadow;
use egui::text::LayoutJob;
use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Ui, UiBuilder, pos2, vec2};
use ui::{Theme, widgets as w};

use super::focus::Hit;

/// A child Ui over `rect` drawn at `opacity` (for fades).
pub fn faded(ui: &mut Ui, rect: Rect, opacity: f32) -> Ui {
    let mut c = ui.new_child(UiBuilder::new().max_rect(rect));
    c.multiply_opacity(opacity.clamp(0.0, 1.0));
    c
}

/// `rect` grown around its center by `factor`.
pub fn grow(rect: Rect, factor: f32) -> Rect {
    Rect::from_center_size(rect.center(), rect.size() * factor)
}

pub fn radius(r: f32) -> CornerRadius {
    CornerRadius::same(r.clamp(0.0, 255.0) as u8)
}

/// One line of text, cut with "…" to fit `max_width`. Returns its rect.
pub fn line(p: &Painter, pos: Pos2, align: Align2, text: &str, font: FontId, color: Color32, max_width: f32) -> Rect {
    let mut job = LayoutJob::simple(text.to_string(), font, color, max_width.max(1.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = p.layout_job(job);
    let rect = align.anchor_size(pos, galley.size());
    p.galley(rect.min, galley, color);
    rect
}

/// Wrapped text, at most `rows` lines. Returns its rect.
pub fn wrapped(p: &Painter, pos: Pos2, text: &str, font: FontId, color: Color32, max_width: f32, rows: usize) -> Rect {
    let mut job = LayoutJob::simple(text.to_string(), font, color, max_width.max(1.0));
    job.wrap.max_rows = rows;
    job.wrap.overflow_character = Some('…');
    let galley = p.layout_job(job);
    let rect = Rect::from_min_size(pos, galley.size());
    p.galley(pos, galley, color);
    rect
}

pub fn text_width(p: &Painter, text: &str, font: FontId) -> f32 {
    p.layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

/// A soft drop shadow / glow under a raised thing.
pub fn shadow(p: &Painter, rect: Rect, r: f32, s: f32, strength: f32, color: Color32) {
    if strength <= 0.01 {
        return;
    }
    let sh = Shadow {
        offset: [0, (12.0 * s * strength).clamp(0.0, 127.0) as i8],
        blur: (44.0 * s).clamp(0.0, 255.0) as u8,
        spread: (2.0 * s * strength).clamp(0.0, 255.0) as u8,
        color: color.gamma_multiply(strength),
    };
    p.add(sh.as_shape(rect, radius(r)));
}

/// A light sheen that sweeps across `rect` once, `e` going 0→1.
pub fn sheen(p: &Painter, rect: Rect, e: f32) {
    if !(0.0..1.0).contains(&e) {
        return;
    }
    let p = p.with_clip_rect(rect.intersect(p.clip_rect()));
    let w = rect.width().max(rect.height()) * 0.45;
    let x = rect.left() - w + (rect.width() + 2.0 * w) * e;
    let tilt = rect.height() * 0.5;
    let a = (std::f32::consts::PI * e).sin() * 0.16;
    let c = Color32::from_white_alpha((a * 255.0) as u8);
    let mut mesh = egui::Mesh::default();
    // A slanted band: clear edges, bright middle.
    let pts = [
        (pos2(x - w / 2.0 + tilt, rect.top()), Color32::TRANSPARENT),
        (pos2(x + tilt, rect.top()), c),
        (pos2(x + w / 2.0 + tilt, rect.top()), Color32::TRANSPARENT),
        (pos2(x - w / 2.0 - tilt, rect.bottom()), Color32::TRANSPARENT),
        (pos2(x - tilt, rect.bottom()), c),
        (pos2(x + w / 2.0 - tilt, rect.bottom()), Color32::TRANSPARENT),
    ];
    for (pos, col) in pts {
        mesh.colored_vertex(pos, col);
    }
    mesh.add_triangle(0, 1, 4);
    mesh.add_triangle(0, 4, 3);
    mesh.add_triangle(1, 2, 5);
    mesh.add_triangle(1, 5, 4);
    p.add(mesh);
}

/// A round glow: `color` in the middle fading to nothing at `r`.
pub fn glow(p: &Painter, center: Pos2, r: f32, color: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    let n = 40;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(center + vec2(a.cos(), a.sin()) * r, Color32::TRANSPARENT);
    }
    for i in 1..=n as u32 {
        mesh.add_triangle(0, i, i + 1);
    }
    p.add(mesh);
}

/// A game's art in a tile that grows and glows with focus.
/// `urls` in order of preference. Returns the drawn (grown) rect.
pub fn art_tile(ui: &mut Ui, t: &Theme, rect: Rect, urls: &[&str], title: &str, hit: &Hit, s: f32, sheen_e: f32) -> Rect {
    let r = 10.0 * s;
    let shown = grow(rect, 1.0 + 0.07 * hit.glow);
    shadow(ui.painter(), shown, r, s, 0.35 + 0.65 * hit.glow, Color32::from_black_alpha(200));
    if !w::paint_cover_first(ui, shown, urls, radius(r)) {
        // Loading, or no art at all: the name on a gradient (like the library's tiles).
        title_card(ui.painter(), t, shown, title, s);
    }
    if hit.focused {
        sheen(ui.painter(), shown, sheen_e);
    }
    shown
}

/// Art-less card: the title on the theme's gradient.
pub fn title_card(p: &Painter, t: &Theme, rect: Rect, title: &str, s: f32) {
    let r = 10.0 * s;
    p.rect_filled(rect, radius(r), t.surface);
    let inner = rect.shrink(r);
    w::gradient(&p.with_clip_rect(inner), inner, t.accent.gamma_multiply(0.25), t.accent_2.gamma_multiply(0.25), true);
    wrapped(p, rect.min + vec2(16.0 * s, 16.0 * s), title, w::font_bold(22.0 * s), t.text, rect.width() - 32.0 * s, 4);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Play,
    Primary,
    Secondary,
    Danger,
    /// Shown but does nothing now (Queued…).
    Muted,
}

/// A big pill button. `progress` fills it (downloads).
pub fn pill(ui: &Ui, t: &Theme, rect: Rect, icon: Option<&str>, label: &str, kind: Kind, hit: &Hit, s: f32, progress: Option<f32>) {
    let p = ui.painter();
    let shown = grow(rect, 1.0 + 0.04 * hit.glow);
    let r = shown.height() / 2.0;
    let (fill, text) = match kind {
        Kind::Play => (t.play, Color32::WHITE),
        Kind::Primary => (t.accent, t.on_accent),
        Kind::Danger => (t.danger, Color32::WHITE),
        Kind::Secondary => (w::mix(t.raised, Color32::WHITE, 0.06), t.text),
        Kind::Muted => (t.surface, t.text_dim),
    };
    let fill = w::mix(fill, Color32::WHITE, 0.12 * hit.glow);
    shadow(p, shown, r, s, 0.3 + 0.5 * hit.glow, if matches!(kind, Kind::Play | Kind::Primary) { fill } else { Color32::from_black_alpha(160) });
    p.rect_filled(shown, radius(r), fill);
    if let Some(f) = progress {
        let done = Rect::from_min_size(shown.min, vec2(shown.width() * f.clamp(0.0, 1.0), shown.height()));
        p.with_clip_rect(done).rect_filled(shown, radius(r), w::mix(t.accent, Color32::WHITE, 0.1));
    }
    // A sheen on the main buttons now and then, so they read as "press me".
    if matches!(kind, Kind::Play | Kind::Primary) {
        let time = ui.input(|i| i.time) as f32;
        sheen(p, shown, (time * 0.25).fract() * 2.5);
    }
    let font = w::font_bold(24.0 * s);
    let label_w = text_width(p, label, font.clone());
    let icon_w = if icon.is_some() { 30.0 * s } else { 0.0 };
    let start = shown.center().x - (label_w + icon_w) / 2.0;
    if let Some(i) = icon {
        p.text(pos2(start, shown.center().y), Align2::LEFT_CENTER, i, w::font(24.0 * s), text);
    }
    p.text(pos2(start + icon_w, shown.center().y), Align2::LEFT_CENTER, label, font, text);
}

/// Width a pill needs for this label.
pub fn pill_width(ui: &Ui, icon: bool, label: &str, s: f32) -> f32 {
    text_width(ui.painter(), label, w::font_bold(24.0 * s)) + if icon { 30.0 * s } else { 0.0 } + 72.0 * s
}

/// Thin progress bar with a moving shine.
pub fn bar(ui: &Ui, t: &Theme, rect: Rect, value: f32, animate: bool) {
    let p = ui.painter();
    let r = rect.height() / 2.0;
    p.rect_filled(rect, radius(r), Color32::from_white_alpha(28));
    let done = Rect::from_min_size(rect.min, vec2(rect.width() * value.clamp(0.0, 1.0), rect.height()));
    if done.width() > 0.5 {
        let c = p.with_clip_rect(done);
        w::gradient(&c, rect, t.accent, t.accent_2, true);
        if animate {
            let time = ui.input(|i| i.time) as f32;
            sheen(&c, Rect::from_min_size(done.min, vec2(done.width().max(rect.height() * 4.0), rect.height())), (time * 0.6).fract());
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
    }
}

/// A button glyph for the hints: a round "A", a pill "LB", or a key cap.
pub fn glyph(p: &Painter, t: &Theme, left_center: Pos2, label: &str, round: bool, s: f32, flash: f32) -> Rect {
    let font = w::font_bold(17.0 * s);
    let tw = text_width(p, label, font.clone());
    let h = 34.0 * s;
    let wdt = if round { h } else { (tw + 22.0 * s).max(h) };
    let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - h / 2.0), vec2(wdt, h));
    let fill = w::mix(Color32::from_white_alpha(30), t.accent, flash);
    p.rect_filled(rect, radius(if round { h / 2.0 } else { 8.0 * s }), fill);
    p.rect_stroke(rect, radius(if round { h / 2.0 } else { 8.0 * s }), egui::Stroke::new(1.5 * s, Color32::from_white_alpha(70)), egui::StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, label, font, t.text);
    rect
}

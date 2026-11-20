//! Drawing a message's pieces: text with emoticons and links, mentions,
//! spoilers, code, quotes, `/me`, `/random`, `/flip`, stickers, pictures and
//! chat effects, then preview cards for its links.

use super::{ChatUi, Env, Msg, cards};
use client::ChatTarget;
use egui::{Margin, RichText, Sense, Stroke, Ui, vec2};
use extensions::Action;
use i18n::t;
use model::chat::Part;
use ui::{Theme, icons, widgets as w};

/// Everything a message shows.
#[allow(clippy::too_many_arguments)]
pub(super) fn body(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, m: &Msg, name: &str, mem: &mut ChatUi, out: &mut Vec<Action>) {
    let parts = model::chat::rich(&m.raw);
    let key = (m.time, m.ordinal);
    // Just one to three emoticons: drawn large, like Steam.
    let emotes = parts.iter().filter(|p| matches!(p, Part::Emoticon(_))).count();
    let only_emotes = emotes > 0 && emotes <= 3 && parts.iter().all(|p| matches!(p, Part::Emoticon(_)) || matches!(p, Part::Text(s) if s.trim().is_empty()));
    if only_emotes {
        ui.horizontal(|ui| {
            for p in &parts {
                if let Part::Emoticon(n) = p {
                    let s = t.dense(48.0);
                    ui.add(egui::Image::new(model::chat::emoticon_url(n, true)).fit_to_exact_size(vec2(s, s))).on_hover_text(format!(":{n}:"));
                }
            }
        });
    } else {
        let mut spoiler = 0;
        let mut r = Ctx { t, mem, out, key, spoiler: &mut spoiler, name };
        blocks(ui, &mut r, &parts);
    }
    if let Some(inv) = &m.invite {
        super::invite_card(ui, t, env.state, m.from, inv, out);
    }
    // Previews for its links (not pictures: they show in the message).
    let links = model::chat::links(&parts);
    for url in links.iter().filter(|u| !cards::is_image(u)).take(2) {
        cards::card(ui, t, env, url, out);
    }
    let _ = to;
}

struct Ctx<'a> {
    t: &'a Theme,
    mem: &'a mut ChatUi,
    out: &'a mut Vec<Action>,
    key: (u64, u32),
    /// Counts spoilers, so each remembers whether it's shown.
    spoiler: &'a mut usize,
    /// The sender's name (for `/me`).
    name: &'a str,
}

fn is_block(p: &Part) -> bool {
    matches!(p, Part::Pre(_) | Part::Quote { .. } | Part::Image { .. } | Part::Sticker(_) | Part::Random { .. } | Part::Flip(_) | Part::Effect(_))
        || matches!(p, Part::Link { url, .. } if cards::is_image(url))
}

/// Inline runs wrap like text; pictures, quotes and the like get their own line.
fn blocks(ui: &mut Ui, r: &mut Ctx, parts: &[Part]) {
    let mut run: Vec<&Part> = Vec::new();
    for p in parts {
        if is_block(p) {
            inline(ui, r, &run);
            run.clear();
            block(ui, r, p);
        } else {
            run.push(p);
        }
    }
    inline(ui, r, &run);
}

fn text_font(t: &Theme) -> egui::FontId {
    egui::FontId::new(t.typography.body_m.size, t.typography.body_m.weight.family())
}

fn inline(ui: &mut Ui, r: &mut Ctx, parts: &[&Part]) {
    // Nothing but spaces and line breaks between blocks: skip.
    if parts.iter().all(|p| matches!(p, Part::Text(s) if s.trim().is_empty())) {
        return;
    }
    let t = r.t;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        for p in parts {
            piece(ui, r, p);
        }
    });
    let _ = t;
}

/// Plain text, word by word (so it wraps around emoticons), keeping its
/// line breaks.
fn words(ui: &mut Ui, t: &Theme, s: &str, rich: impl Fn(&str) -> RichText) {
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            ui.end_row();
        }
        for word in line.split_inclusive(' ') {
            ui.add(egui::Label::new(rich(word)).wrap_mode(egui::TextWrapMode::Extend));
        }
    }
    let _ = t;
}

fn piece(ui: &mut Ui, r: &mut Ctx, p: &Part) {
    let t = r.t;
    match p {
        Part::Text(s) => words(ui, t, s, |w| RichText::new(w).font(text_font(t)).color(t.text)),
        Part::Emoticon(n) => {
            let s = t.typography.body_m.size * 1.45;
            ui.add(egui::Image::new(model::chat::emoticon_url(n, false)).fit_to_exact_size(vec2(s, s))).on_hover_text(format!(":{n}:"));
        }
        Part::Link { url, text } => {
            let shown = if text.chars().count() > 70 { format!("{}…", text.chars().take(68).collect::<String>()) } else { text.clone() };
            let resp = ui.add(egui::Label::new(RichText::new(shown).font(text_font(t)).color(t.accent)).sense(Sense::click()));
            if resp.hovered() {
                ui.painter().hline(resp.rect.x_range(), resp.rect.bottom(), Stroke::new(1.0, t.accent));
            }
            if resp.on_hover_text(url).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if let Some(a) = super::open_link(url) {
                    r.out.push(a);
                }
            }
        }
        Part::Mention { text, .. } => {
            ui.label(RichText::new(text).font(text_font(t)).color(t.accent).strong());
        }
        Part::Code(s) => {
            w::component_frame(t, &t.components.chip).inner_margin(Margin::symmetric(t.spacing.xs as i8, 1)).show(ui, |ui| {
                ui.label(RichText::new(s).monospace().size(t.typography.body_s.size + 1.0).color(t.text));
            });
        }
        Part::Spoiler(inner) => {
            let idx = *r.spoiler;
            *r.spoiler += 1;
            let key = (r.key.0, r.key.1, idx);
            if r.mem.revealed.contains(&key) {
                for p in inner {
                    piece(ui, r, p);
                }
            } else {
                // A bar the size of the hidden text; a click shows it.
                let plain: String = inner.iter().map(plain_of).collect();
                let g = ui.painter().layout_no_wrap(plain, text_font(t), egui::Color32::PLACEHOLDER);
                let size = vec2(g.size().x.min(ui.available_width().max(40.0)).max(t.dense(48.0)), g.size().y);
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                let fill = if resp.hovered() { t.surface_hover } else { t.chrome };
                ui.painter().rect_filled(rect, t.corner_small(), fill);
                if resp.on_hover_text(t!("Spoiler: click to show")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    r.mem.revealed.insert(key);
                }
            }
        }
        Part::Me(inner) => {
            ui.label(RichText::new(format!("{} ", r.name)).font(text_font(t)).color(t.accent).italics());
            for p in inner {
                match p {
                    Part::Text(s) => words(ui, t, s, |w| RichText::new(w).font(text_font(t)).color(t.text).italics()),
                    other => piece(ui, r, other),
                }
            }
        }
        // Blocks inside a spoiler or `/me` (rare): as their own line.
        other => {
            ui.end_row();
            block(ui, r, other);
            ui.end_row();
        }
    }
}

fn plain_of(p: &Part) -> String {
    match p {
        Part::Text(s) | Part::Code(s) | Part::Pre(s) => s.clone(),
        Part::Emoticon(n) => format!(":{n}:"),
        Part::Link { text, .. } | Part::Mention { text, .. } => text.clone(),
        Part::Spoiler(i) | Part::Me(i) | Part::Quote { parts: i, .. } => i.iter().map(plain_of).collect(),
        _ => "…".into(),
    }
}

/// A chip for a result: an icon and a line of text.
fn chip(ui: &mut Ui, t: &Theme, icon: &str, text: String) {
    w::component_frame(t, &t.components.card).inner_margin(Margin::symmetric(t.spacing.md as i8, t.spacing.xs as i8 + 2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon).font(w::font(t.typography.body_l.size)).color(t.accent));
            ui.label(w::body_m(t, text, t.text));
        });
    });
}

fn block(ui: &mut Ui, r: &mut Ctx, p: &Part) {
    let t = r.t;
    match p {
        Part::Pre(s) => {
            w::component_frame(t, &t.components.card).inner_margin(Margin::same(t.spacing.sm as i8)).show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(s).monospace().size(t.typography.body_s.size + 1.0).color(t.text)).wrap());
            });
        }
        Part::Quote { author, parts } => {
            let resp = egui::Frame::new().inner_margin(Margin { left: t.spacing.md as i8, right: t.spacing.sm as i8, top: t.spacing.xs as i8, bottom: t.spacing.xs as i8 }).show(ui, |ui| {
                if !author.is_empty() {
                    ui.label(w::body_s(t, t!("{name} said:", name = author), t.text_dim));
                }
                blocks(ui, r, parts);
            });
            let rr = resp.response.rect;
            ui.painter().rect_filled(egui::Rect::from_min_size(rr.min, vec2(3.0, rr.height())), egui::CornerRadius::same(2), t.accent_2);
        }
        Part::Random { min, max, result } => {
            let text = match result {
                Some(n) => t!("Random number from {min} to {max}: {n}", min = min, max = max, n = n).to_string(),
                None => t!("Random number from {min} to {max}", min = min, max = max).to_string(),
            };
            chip(ui, t, icons::DICE, text);
        }
        Part::Flip(result) => {
            let text = match result {
                Some(true) => t!("Flipped a coin: heads"),
                Some(false) => t!("Flipped a coin: tails"),
                None => t!("Flipped a coin"),
            };
            chip(ui, t, icons::COINS, text.to_string());
        }
        Part::Effect(name) => chip(ui, t, icons::SPARKLE, t!("Chat effect: {name}", name = effect_name(name)).to_string()),
        Part::Sticker(name) => {
            let s = t.dense(120.0);
            ui.add(egui::Image::new(model::chat::sticker_url(name)).max_size(vec2(s, s)).show_loading_spinner(true)).on_hover_text(name);
        }
        Part::Image { src, width, height } => image(ui, r, src, *width, *height),
        Part::Link { url, .. } => image(ui, r, url, 0, 0),
        other => {
            ui.horizontal_wrapped(|ui| piece(ui, r, other));
        }
    }
}

/// "confetti" → "Confetti".
fn effect_name(s: &str) -> String {
    let s = s.replace(['_', '-'], " ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// A picture in a message: fits the chat; a click opens it large.
fn image(ui: &mut Ui, r: &mut Ctx, src: &str, width: u32, height: u32) {
    let t = r.t;
    let max = vec2(ui.available_width().min(t.dense(360.0)), t.dense(260.0));
    // Big pictures load downscaled, like the rest of the app's.
    let sized = crate::sized_images::inline_url(src, max.x, max.y, ui.ctx().pixels_per_point());
    let img = egui::Image::new(sized).max_width(max.x).max_height(max.y).fit_to_original_size(1.0).corner_radius(t.corner_small()).sense(Sense::click());
    let mut resp = ui.add(img);
    if resp.rect.height() < 4.0 {
        // Still loading: a placeholder its size.
        let size = if width > 0 && height > 0 {
            let k = (max.x / width as f32).min(max.y / height as f32).min(1.0);
            vec2(width as f32 * k, height as f32 * k)
        } else {
            vec2(max.x, max.y * 0.6)
        };
        let (rect, rr) = ui.allocate_exact_size(size, Sense::click());
        w::skeleton(ui, t, rect, t.corner_small());
        resp = rr;
    }
    if resp.hovered() {
        ui.painter().rect_stroke(resp.rect, t.corner_small(), Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
    }
    if resp.on_hover_text(t!("Click to open it large")).on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
        r.mem.viewing = Some(src.to_string());
    }
}

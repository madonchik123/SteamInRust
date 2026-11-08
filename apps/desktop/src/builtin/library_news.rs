//! Game updates feed and the in-app post reader (no browser).

use super::Library;
use egui::{Align, Align2, Color32, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::AppId;
use std::sync::{Arc, Mutex};
use store::events::{EventKind, GameEvent};
use ui::{Theme, icons, widgets as w};

pub type EventsSlot = Arc<Mutex<Option<Vec<GameEvent>>>>;

/// Which post is open in the reader.
pub struct Reader {
    pub app: AppId,
    pub index: usize,
}

pub fn kind_color(t: &Theme, k: EventKind) -> Color32 {
    match k {
        EventKind::MajorUpdate => t.accent_2,
        EventKind::SmallUpdate | EventKind::Update => t.accent,
        EventKind::InGameEvent => t.in_game,
        _ => t.text_dim,
    }
}

pub fn date(unix: u64) -> String {
    ui::date(unix)
}

impl Library {
    pub(super) fn events(&mut self, id: AppId) -> Option<Vec<GameEvent>> {
        if !self.events.contains_key(&id) {
            let slot: EventsSlot = Arc::new(Mutex::new(None));
            if let (Some(rt), Some(ctx)) = (&self.rt, &self.egui) {
                let (out, ctx) = (slot.clone(), ctx.clone());
                let api = store::StoreClient::new("us", i18n::language().steam_name());
                rt.spawn(async move {
                    let items = api.events(id, 12).await.unwrap_or_else(|e| {
                        tracing::warn!("events for {id}: {e}");
                        Vec::new()
                    });
                    *out.lock().unwrap() = Some(items);
                    ctx.request_repaint();
                });
            }
            self.events.insert(id, slot);
        }
        self.events[&id].lock().unwrap().clone()
    }

    /// The updates feed on a game's page.
    pub(super) fn updates_feed(&mut self, ui: &mut Ui, t: &Theme, id: AppId) {
        ui.label(w::text_bold(t!("UPDATES AND NEWS"), 12.0, t.text_faint));
        ui.add_space(6.0);
        let Some(events) = self.events(id) else {
            for _ in 0..3 {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
                w::skeleton(ui, t, r, t.corner());
                ui.add_space(10.0);
            }
            return;
        };
        if events.is_empty() {
            ui.label(w::text(t!("No news for this game yet."), 13.5, t.text_faint));
            return;
        }
        for (i, e) in events.iter().enumerate() {
            ui.label(w::text_semibold(date(e.time).to_uppercase(), 12.0, t.text_dim));
            ui.add_space(4.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 160.0), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
            ui.painter().rect_filled(rect, t.corner(), w::mix(t.surface, t.surface_hover, hover));
            ui.painter().rect_stroke(rect, t.corner(), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let img = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(244.0, 136.0));
            match e.capsule.as_deref().or(e.header.as_deref()) {
                Some(u) => w::image_or_skeleton(ui, t, img, u, t.corner_small()),
                None => w::placeholder(ui, t, img, "", t.corner_small()),
            }
            let text = Rect::from_min_max(egui::pos2(img.right() + 16.0, rect.top() + 14.0), rect.max - vec2(14.0, 12.0));
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
            c.label(w::text_bold(i18n::tr_dyn(e.kind.label()).to_uppercase(), 11.0, kind_color(t, e.kind)));
            c.add(egui::Label::new(w::text_bold(&e.title, 17.0, t.text)).truncate());
            c.add_space(2.0);
            let summary: String = e.summary.chars().take(260).collect();
            c.add(egui::Label::new(w::text(summary, 13.0, t.text_dim)).wrap());
            let foot = Rect::from_min_max(egui::pos2(text.left(), rect.bottom() - 26.0), egui::pos2(text.right(), rect.bottom() - 8.0));
            let mut f = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(Layout::right_to_left(Align::Center)));
            f.label(w::text(format!("{}  {}", icons::THUMBS_UP, e.votes_up), 12.5, t.text_faint));
            f.label(w::text(format!("{}  {}", icons::CHAT, e.comments), 12.5, t.text_faint));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.reader = Some(Reader { app: id, index: i });
            }
            ui.add_space(14.0);
        }
    }

    /// Full-window post reader with up/down navigation, like Steam's.
    pub(super) fn reader(&mut self, ui: &mut Ui, t: &Theme, _ctx: &mut Ctx) {
        let Some((app, wanted)) = self.reader.as_ref().map(|r| (r.app, r.index)) else { return };
        let Some(events) = self.events(app) else { return };
        if events.is_empty() {
            self.reader = None;
            return;
        }
        let index = wanted.min(events.len() - 1);
        let e = &events[index];
        let mut close = false;
        let mut step: i32 = 0;
        let screen = ui.ctx().content_rect();
        egui::Area::new(egui::Id::unique("news-reader"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ui.ctx(), |ui| {
                let bg = ui.allocate_rect(screen, Sense::click());
                ui.painter().rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(190));
                if bg.clicked() {
                    close = true;
                }
                let width = (screen.width() - 200.0).min(900.0);
                let panel = Rect::from_center_size(screen.center(), vec2(width, screen.height() - 60.0));
                ui.painter().rect_filled(panel, t.corner(), t.bg);
                ui.painter().rect_stroke(panel, t.corner(), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(panel.shrink(14.0)).layout(Layout::top_down(Align::Min)));
                egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(Margin::same(16)).show(&mut c, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(w::text_bold(i18n::tr_dyn(e.kind.label()).to_uppercase(), 11.5, kind_color(t, e.kind)));
                        ui.label(w::text_semibold(t!("PUBLISHED"), 11.5, t.text_dim));
                        ui.label(w::text(date(e.time), 12.0, t.text_faint));
                    });
                    ui.add(egui::Label::new(w::text_bold(&e.title, 24.0, t.accent)).wrap());
                });
                c.add_space(10.0);
                egui::ScrollArea::vertical().id_salt(("reader", app, index)).auto_shrink([false, false]).show(&mut c, |ui| {
                    let wdt = ui.available_width() - 20.0;
                    if let Some(h) = &e.header {
                        let hgt = wdt * 0.3;
                        let (r, _) = ui.allocate_exact_size(vec2(wdt, hgt), Sense::hover());
                        if !w::paint_contain(ui, r, h, Align2::CENTER_CENTER) {
                            w::skeleton(ui, t, r, t.corner_small());
                        }
                        ui.add_space(10.0);
                    }
                    crate::builtin::store::blocks(ui, t, &e.body, wdt);
                    ui.add_space(30.0);
                });
                // Side buttons: close, previous, next.
                let x = panel.right() + 16.0;
                let buttons = [(icons::X, 0, 50.0), (icons::CARET_DOWN, 1, 30.0), (icons::CARET_DOWN, 2, 30.0)];
                let mut y = panel.top();
                for (icon, kind, h) in buttons {
                    let r = Rect::from_min_size(egui::pos2(x, y), vec2(50.0, h));
                    let resp = ui.interact(r, egui::Id::unique(("reader-btn", kind)), Sense::click());
                    let enabled = match kind {
                        1 => index > 0,
                        2 => index + 1 < events.len(),
                        _ => true,
                    };
                    let fill = if resp.hovered() && enabled { t.surface_hover } else { t.raised };
                    ui.painter().rect_filled(r, t.corner_small(), fill);
                    ui.painter().rect_stroke(r, t.corner_small(), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    let glyph = if kind == 1 { icons::CARET_UP } else { icon };
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, w::font(if kind == 0 { 24.0 } else { 16.0 }), if enabled { t.text } else { t.text_faint });
                    if resp.clicked() && enabled {
                        match kind {
                            0 => close = true,
                            1 => step = -1,
                            _ => step = 1,
                        }
                    }
                    y += h + 8.0;
                }
            });
        let keys = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
        if keys.0 {
            close = true;
        }
        if keys.1 && index > 0 {
            step = -1;
        }
        if keys.2 && index + 1 < events.len() {
            step = 1;
        }
        if close {
            self.reader = None;
        } else if let Some(r) = &mut self.reader {
            r.index = (index as i32 + step).max(0) as usize;
        }
    }
}

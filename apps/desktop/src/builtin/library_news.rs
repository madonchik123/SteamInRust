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
/// People playing a game right now, once Steam has answered.
pub type PlayersSlot = Arc<Mutex<Option<u32>>>;

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

/// The icon beside a post of this kind.
pub fn kind_icon(k: EventKind) -> &'static str {
    match k {
        EventKind::MajorUpdate => icons::SPARKLE,
        EventKind::SmallUpdate | EventKind::Update => icons::GEAR,
        EventKind::InGameEvent => icons::CALENDAR,
        _ => icons::NOTE,
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

    /// People in the game right now (Steam's count), asked again every
    /// minute while the page is open. The last number stays meanwhile.
    pub(super) fn current_players(&mut self, id: AppId) -> Option<u32> {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(60);
        let (Some(rt), Some(ctx)) = (self.rt.clone(), self.egui.clone()) else { return None };
        if self.players.get(&id).is_none_or(|(asked, _)| asked.elapsed() >= EVERY) {
            let slot = self.players.get(&id).map(|(_, s)| s.clone()).unwrap_or_default();
            let (out, repaint) = (slot.clone(), ctx.clone());
            let api = store::StoreClient::new("us", i18n::language().steam_name());
            rt.spawn(async move {
                match api.current_players(id).await {
                    Ok(n) => *out.lock().unwrap() = n,
                    Err(e) => tracing::debug!("players in {id}: {e}"),
                }
                repaint.request_repaint();
            });
            self.players.insert(id, (std::time::Instant::now(), slot));
        }
        ctx.request_repaint_after(EVERY);
        *self.players[&id].1.lock().unwrap()
    }

    /// A game's news (Figma "Activity" `523:6699`): posts by day, each with
    /// its kind and title; a click opens the reader.
    pub(super) fn activity(&mut self, ui: &mut Ui, t: &Theme, id: AppId) {
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::body_l(t, t!("Activity"), t.text_dim));
            ui.add_space(t.spacing.sm);
            let Some(events) = self.events(id) else {
                for _ in 0..3 {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 63.0), Sense::hover());
                    w::skeleton(ui, t, r, t.corner());
                    ui.add_space(t.spacing.md);
                }
                return;
            };
            if events.is_empty() {
                ui.label(w::body_m(t, t!("No news for this game yet."), t.text_faint));
                return;
            }
            let mut day = String::new();
            for (i, e) in events.iter().enumerate() {
                let d = date(e.time);
                if d != day {
                    if !day.is_empty() {
                        ui.add_space(t.spacing.md);
                    }
                    day = d;
                    let r = ui.label(w::body_m(t, &day, t.accent)).rect;
                    let y = r.bottom() + t.spacing.xs;
                    let x = ui.max_rect().x_range();
                    ui.painter().hline(x, y, Stroke::new(1.0, t.accent_2));
                    ui.add_space(t.spacing.sm + 2.0);
                }
                let pad = t.spacing.sm + 2.0;
                let title = egui::text::LayoutJob::simple(e.title.clone(), t.typography.body_m.font(), t.text, (ui.available_width() - pad * 3.0 - 40.0).max(40.0));
                let galley = ui.painter().layout_job(title);
                let kind_h = t.typography.body_l.size + 4.0;
                let h = (pad * 2.0 + kind_h + t.spacing.xs + galley.size().y).max(63.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
                let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                let p = ui.painter();
                p.rect_filled(rect, t.corner(), w::mix(t.raised, t.surface_hover, hover));
                // The stripe at the left in the post's color.
                let stripe = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
                p.rect_filled(stripe, egui::CornerRadius { nw: t.corner().nw, sw: t.corner().sw, ne: 0, se: 0 }, kind_color(t, e.kind));
                let icon = Rect::from_min_size(rect.min + vec2(pad + 3.0, pad), vec2(40.0, 40.0));
                w::icon_centered(p, icon.center(), kind_icon(e.kind), 30.0, t.text_dim);
                let x = icon.right() + t.spacing.sm - 1.0;
                p.text(egui::pos2(x, rect.top() + pad), Align2::LEFT_TOP, i18n::tr_dyn(e.kind.label()), t.typography.body_l.font(), t.text_dim);
                p.galley(egui::pos2(x, rect.top() + pad + kind_h + t.spacing.xs), galley, t.text);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.reader = Some(Reader { app: id, index: i });
                }
                ui.add_space(t.spacing.xs);
            }
        });
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
                ui.painter().rect_filled(screen, CornerRadius::ZERO, t.overlay_dark(3));
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
                        ui.label(w::styled(i18n::tr_dyn(e.kind.label()).to_uppercase(), t.typography.caption, kind_color(t, e.kind)));
                        ui.label(w::styled(t!("PUBLISHED"), t.typography.caption, t.text_dim));
                        ui.label(w::body_s(t, date(e.time), t.text_faint));
                    });
                    ui.add(egui::Label::new(w::heading_m(t, &e.title, t.accent)).wrap());
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

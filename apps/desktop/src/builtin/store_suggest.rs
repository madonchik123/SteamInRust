//! Store: games matching what's typed in a search box, shown under it as
//! you type (after a short pause): each with its picture, name and price.
//! Used by the store's own search in the page bar and by the app-wide
//! search (the only search box on the store in themes without a page bar).

use super::{Load, Shared, Store, View};
use egui::{Rect, Sense, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use store::Item;
use ui::{Theme, icons, widgets as w};

/// How long typing has to pause before asking Steam.
const PAUSE: f64 = 0.3;
/// How many games to show.
const SHOWN: usize = 8;

#[derive(Default)]
pub(crate) struct Suggest {
    /// The text and when it last changed.
    typed: String,
    since: f64,
    /// The text asked about, and Steam's answer.
    asked: Option<(String, Shared<Vec<Item>>)>,
    /// The last answer that arrived (shown while a newer one loads).
    last: Option<(String, Vec<Item>)>,
    /// The store's own search box: the dropdown is open, this row is picked.
    pub open: bool,
    pub sel: Option<usize>,
}

impl Store {
    /// Games matching `query` (what's ready so far: `None` before the
    /// first answer). Asks Steam once typing has paused; content the user
    /// chose to hide is left out.
    pub(crate) fn suggestions_for(&mut self, ctx: &Ctx, query: &str, now: f64) -> Option<Vec<Item>> {
        let q = query.trim();
        if q.chars().count() < 2 || !self.sync_locale(ctx) {
            return None;
        }
        if self.suggest.typed != q {
            self.suggest.typed = q.to_string();
            self.suggest.since = now;
        }
        if now - self.suggest.since >= PAUSE && self.suggest.asked.as_ref().is_none_or(|(a, _)| a != q) {
            let (api, qq) = (self.api.clone(), q.to_string());
            let slot = self.fetch(async move { api.search(&qq).await });
            self.suggest.asked = Some((q.to_string(), slot));
        } else if now - self.suggest.since < PAUSE {
            self.egui.request_repaint_after(std::time::Duration::from_secs_f64(PAUSE - (now - self.suggest.since) + 0.02));
        }
        if let Some((a, slot)) = &self.suggest.asked {
            if let Load::Ready(v) = &*slot.lock().unwrap() {
                self.suggest.last = Some((a.clone(), v.clone()));
            }
        }
        let (_, items) = self.suggest.last.clone()?;
        let shown: Vec<Item> = items.into_iter().filter(|i| self.safe(i.app_id) != Some(false)).take(SHOWN).collect();
        self.flush_ratings();
        Some(shown)
    }

    /// Still waiting for Steam about `query`.
    pub(crate) fn suggestions_loading(&self, query: &str) -> bool {
        let q = query.trim();
        self.suggest.last.as_ref().is_none_or(|(a, _)| a != q) && q.chars().count() >= 2
    }

    /// The store's search box in the page bar, with its dropdown.
    pub(super) fn search_box(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, width: f32) {
        let now = ui.input(|i| i.time);
        let q = self.bar_query.trim().to_string();
        let items = if self.suggest.open { self.suggestions_for(ctx, &q, now).unwrap_or_default() } else { Vec::new() };
        // The rows: the games, then "See all results".
        let rows = items.len() + 1;
        // Up and down move through the rows while the field has the focus
        // (taken before the field sees them).
        if self.suggest.open && !q.is_empty() {
            let (down, up) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)));
            if down {
                self.suggest.sel = Some(self.suggest.sel.map_or(0, |i| (i + 1).min(rows - 1)));
            }
            if up {
                self.suggest.sel = self.suggest.sel.and_then(|i| i.checked_sub(1));
            }
        }
        let before = self.bar_query.clone();
        let r = w::bar_search(ui, t, &mut self.bar_query, t!("Search the store"), width);
        if self.bar_query != before {
            self.suggest.sel = None;
            self.suggest.open = true;
        }
        if r.gained_focus() {
            self.suggest.open = true;
        }
        let mut pick: Option<usize> = None;
        if r.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                // Enter: the picked game, else all results.
                pick = Some(self.suggest.sel.unwrap_or(rows - 1));
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.suggest.open = false;
            }
        }
        if self.suggest.open && q.chars().count() >= 2 {
            let loading = self.suggestions_loading(&q);
            let (clicked, outside) = suggestion_popup(ui.ctx(), t, r.rect, &q, &items, loading, &mut self.suggest.sel);
            if clicked.is_some() {
                pick = clicked;
            } else if outside && !r.has_focus() {
                self.suggest.open = false;
            }
        }
        // What's in the box now (typing and Enter can come in one frame).
        let q = self.bar_query.trim().to_string();
        if let Some(i) = pick.filter(|_| !q.is_empty()) {
            self.suggest.open = false;
            self.suggest.sel = None;
            match items.get(i) {
                Some(item) => {
                    self.bar_query.clear();
                    self.open(View::App(item.app_id));
                }
                None => self.start_search(q),
            }
        }
    }
}

/// The dropdown under a search box (`anchor`): one row per game, then "See
/// all results for …". Kept inside the window. Returns a clicked row and
/// whether the pointer clicked somewhere else.
pub(crate) fn suggestion_popup(ctx: &egui::Context, t: &Theme, anchor: Rect, query: &str, items: &[Item], loading: bool, sel: &mut Option<usize>) -> (Option<usize>, bool) {
    let screen = ctx.content_rect();
    let width = anchor.width().max(t.dense(420.0)).min(screen.width() - 16.0);
    let x = anchor.left().min(screen.right() - width - 8.0).max(screen.left() + 8.0);
    let mut clicked = None;
    let area = egui::Area::new(egui::Id::unique("store-suggestions")).order(egui::Order::Foreground).fixed_pos(pos2(x, anchor.bottom() + t.spacing.xs + 2.0)).show(ctx, |ui| {
        w::component_frame(t, &t.components.menu).show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            if items.is_empty() {
                let text = if loading { t!("Searching…") } else { t!("No games match.") };
                ui.horizontal(|ui| {
                    ui.add_space(t.padding_of(&t.components.menu_item).x);
                    ui.label(w::body_s(t, text, t.text_faint));
                });
                ui.add_space(t.spacing.xs);
            }
            for (i, item) in items.iter().enumerate() {
                let r = suggestion_row(ui, t, item, *sel == Some(i));
                if r.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                    *sel = Some(i);
                }
                if r.clicked() {
                    clicked = Some(i);
                }
            }
            let all = items.len();
            let r = w::menu_row(ui, t, icons::SEARCH, &t!("See all results for “{q}”", q = query), *sel == Some(all));
            if r.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                *sel = Some(all);
            }
            if r.clicked() {
                clicked = Some(all);
            }
        });
    });
    let outside = ctx.input(|i| i.pointer.any_click()) && !area.response.hovered() && !ctx.pointer_hover_pos().is_some_and(|p| anchor.contains(p));
    (clicked, outside)
}

/// One suggested game (the `menu_item` look, with hover): its small
/// picture, its name, and its price on the right.
pub(crate) fn suggestion_row(ui: &mut Ui, t: &Theme, item: &Item, selected: bool) -> egui::Response {
    let c = &t.components.menu_item;
    let pad = t.padding_of(c).x;
    let h = t.dense(52.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, w::interact(ui, &resp, selected));
        w::paint_box(ui, t, rect, c, &look, if selected { 1.0 } else { 0.0 });
        let pic_h = h - t.dense(14.0);
        let pic = Rect::from_min_size(pos2(rect.left() + pad * 0.6, rect.center().y - pic_h / 2.0), vec2((pic_h * 460.0 / 215.0).round(), pic_h));
        w::image_or_skeleton(ui, t, pic, &item.header, t.corner_small());
        // The price on the right, the name in what's left.
        let mut right = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(rect.right() - t.dense(170.0), rect.top()), pos2(rect.right() - pad * 0.6, rect.bottom()))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        let price_left = if item.price.final_ == Some(0) {
            right.label(w::body_m(t, t!("Free"), look.text)).rect.left()
        } else if item.price.final_.is_some() {
            w::price_tag(&mut right, t, &item.price.label(), item.price.original_label().as_deref(), item.price.discount);
            right.min_rect().left()
        } else {
            rect.right() - pad
        };
        let x = pic.right() + t.spacing.md;
        let fid = t.font_of(c);
        let mut job = egui::text::LayoutJob::simple_singleline(item.name.clone(), fid, look.text);
        job.wrap.max_width = (price_left - t.spacing.md - x).max(10.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, look.text);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

//! Inventory: the Community Market search, in the app (it used to open
//! Steam's website). Search by name, pick a game, and see an item's price,
//! history and order book.

use super::{Inventory, View, wallet_currency};
use client::Command;
use egui::{Align2, Color32, Sense, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, MarketResult};
use ui::{Theme, icons, widgets as w};

/// Games with the busiest markets, as quick filters (Steam's own picks).
const GAMES: [(AppId, &str); 5] = [(730, "Counter-Strike 2"), (570, "Dota 2"), (440, "Team Fortress 2"), (252490, "Rust"), (753, "Steam")];

#[derive(Default)]
pub(super) struct MarketSearch {
    query: String,
    app: Option<AppId>,
    start: u32,
    /// What was last asked for: (query, game, start).
    asked: Option<(String, Option<AppId>, u32)>,
    selected: Option<MarketResult>,
}

impl Inventory {
    pub(super) fn market_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let want = (self.market.query.trim().to_string(), self.market.app, self.market.start);
        if self.market.asked.as_ref() != Some(&want) {
            self.market.asked = Some(want.clone());
            ctx.send(Command::MarketSearch { query: want.0.clone(), app: want.1, start: want.2 });
        }
        egui::ScrollArea::vertical().id_salt("market-search").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(26.0);
            w::centered(ui, 1180.0, 32.0, |ui| {
                ui.horizontal(|ui| {
                    if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Inventory"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        self.view = View::Items;
                    }
                });
                w::page_title(ui, t, t!("Community Market"), Some(&t!("Buy and sell items with other players")));
                ui.horizontal(|ui| {
                    let r = w::search(ui, t, &mut self.market.query, &t!("Search the Market"), 360.0);
                    if r.changed() {
                        self.market.start = 0;
                    }
                    ui.add_space(8.0);
                    if w::chip(ui, t, &t!("All games"), self.market.app.is_none()).clicked() {
                        self.market.app = None;
                        self.market.start = 0;
                    }
                    for (id, name) in GAMES {
                        if w::chip(ui, t, name, self.market.app == Some(id)).clicked() {
                            self.market.app = Some(id);
                            self.market.start = 0;
                        }
                    }
                });
                ui.add_space(14.0);
                let answer = ctx.state.market_search.as_ref().filter(|(k, _)| *k == want).map(|(_, r)| r.clone());
                let (results, total) = match answer {
                    None => {
                        for _ in 0..8 {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
                            w::skeleton(ui, t, r.shrink2(vec2(0.0, 4.0)), t.corner_small());
                        }
                        return;
                    }
                    Some(Err(e)) => {
                        w::empty(ui, t, icons::WARNING, t!("Couldn't search the Market"), &i18n::tr_dyn(&e));
                        if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            self.market.asked = None;
                        }
                        return;
                    }
                    Some(Ok(r)) => r,
                };
                ui.label(w::text(t!("{n} results", n = total), 13.0, t.text_dim));
                ui.add_space(8.0);
                let right_w = 380.0_f32.min(ui.available_width() * 0.4);
                let left_w = ui.available_width() - right_w - 20.0;
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(left_w);
                        if results.is_empty() {
                            ui.label(w::text(t!("Nothing matches your search."), 13.5, t.text_faint));
                        }
                        for item in &results {
                            let chosen = self.market.selected.as_ref().is_some_and(|s| s.hash_name == item.hash_name && s.app_id == item.app_id);
                            if result_row(ui, t, item, chosen).clicked() {
                                self.market.selected = Some(item.clone());
                            }
                        }
                        // Pages of 30.
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let page = self.market.start / 30 + 1;
                            let pages = total.div_ceil(30).max(1);
                            if self.market.start > 0 && w::button(ui, t, Some(icons::CARET_LEFT), t!("Previous"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                self.market.start = self.market.start.saturating_sub(30);
                            }
                            ui.label(w::text(t!("Page {n} of {total}", n = page, total = pages), 12.5, t.text_dim));
                            if page < pages && w::button(ui, t, Some(icons::CARET_RIGHT), t!("Next"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                self.market.start += 30;
                            }
                        });
                    });
                    ui.vertical(|ui| {
                        ui.set_width(right_w);
                        match self.market.selected.clone() {
                            Some(item) => self.market_item(ui, t, ctx, &item),
                            None => {
                                w::card(t).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(w::text(t!("Pick an item to see its price history and the buy and sell orders."), 13.0, t.text_dim));
                                });
                            }
                        }
                    });
                });
            });
        });
    }

    /// The chosen result: price, history chart and order book.
    fn market_item(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, item: &MarketResult) {
        let currency = wallet_currency(ctx);
        let key = (item.app_id, item.hash_name.clone());
        if self.full_asked.insert(key.clone()) {
            ctx.send(Command::MarketPrice { app_id: item.app_id, hash_name: item.hash_name.clone(), currency, country: ctx.state.country.clone(), full: true });
        }
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                ui.painter().rect_filled(r, t.corner_small(), t.raised);
                w::image_or_skeleton(ui, t, r.shrink(4.0), &item.icon, t.corner_small());
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(w::text_bold(&item.name, 16.0, name_color(t, &item.color))).wrap());
                    ui.label(w::text(&item.app_name, 12.5, t.text_dim));
                    ui.label(w::text(t!("{n} for sale", n = item.listings), 12.5, t.text_faint));
                });
            });
            ui.add_space(8.0);
            match ctx.state.market_prices.get(&key).cloned() {
                None => {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 160.0), Sense::hover());
                    w::skeleton(ui, t, r, t.corner_small());
                }
                Some(Err(e)) => {
                    ui.label(w::text(i18n::tr_dyn(&e), 12.5, t.text_faint));
                    if ui.link(w::text(t!("Try again"), 12.5, t.accent)).clicked() {
                        self.full_asked.remove(&key);
                    }
                }
                Some(Ok(p)) => {
                    let lowest = p.lowest_sell.map(|c| super::market::money(c, currency)).or(p.lowest.clone()).unwrap_or_else(|| item.price.clone());
                    ui.horizontal(|ui| {
                        ui.label(w::text(t!("Starting at"), 13.0, t.text_dim));
                        ui.label(w::text_bold(lowest, 20.0, t.text));
                    });
                    if let Some(v) = p.volume {
                        ui.label(w::text(t!("{n} sold in the last 24 hours", n = v), 12.5, t.text_dim));
                    }
                    ui.add_space(6.0);
                    super::market::chart(ui, t, &p, currency);
                    ui.add_space(6.0);
                    super::market::ladders(ui, t, &p, currency);
                }
            }
        });
    }
}

fn name_color(t: &Theme, hex: &str) -> Color32 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok().filter(|_| hex.len() >= 6);
    v.map(|v| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)).unwrap_or(t.text)
}

fn result_row(ui: &mut Ui, t: &Theme, item: &MarketResult, chosen: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() || chosen, 0.1);
    let p = ui.painter();
    p.rect_filled(r.shrink2(vec2(0.0, 3.0)), t.corner_small(), w::mix(t.surface, t.surface_hover, hover));
    if chosen {
        p.rect_stroke(r.shrink2(vec2(0.0, 3.0)), t.corner_small(), egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    }
    let img = egui::Rect::from_min_size(r.min + vec2(10.0, 9.0), vec2(46.0, 46.0));
    w::image_or_skeleton(ui, t, img, &item.icon, t.corner_small());
    let p = ui.painter();
    let name_w = r.width() - 260.0;
    let name = ui.painter().layout(item.name.clone(), w::font_semibold(14.0), name_color(t, &item.color), name_w.max(80.0));
    p.galley(img.right_top() + vec2(12.0, 4.0), name, t.text);
    p.text(img.right_bottom() + vec2(12.0, -6.0), Align2::LEFT_BOTTOM, &item.app_name, w::font(12.0), t.text_faint);
    p.text(r.right_center() - vec2(14.0, 8.0), Align2::RIGHT_CENTER, &item.price, w::font_bold(15.0), t.text);
    p.text(r.right_center() - vec2(14.0, -10.0), Align2::RIGHT_CENTER, t!("{n} listed", n = item.listings), w::font(12.0), t.text_faint);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

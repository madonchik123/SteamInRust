//! Inventory: the Community Market search, in the app (it used to open
//! Steam's website). Search by name, pick a game, and see an item's price,
//! history and order book.

use super::{Inventory, View, wallet_currency};
use client::Command;
use egui::{Align, Align2, Color32, Layout, Sense, Ui, vec2};
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
    /// Only this game's trading cards.
    cards: Option<AppId>,
    /// Which of your own Market lists shows.
    mine: MyMarket,
}

/// Your own Market lists on the Market page.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum MyMarket {
    #[default]
    Listings,
    Orders,
    History,
}

impl MarketSearch {
    /// Start a search (from a link).
    pub(super) fn open(&mut self, app: Option<AppId>, query: &str) {
        // "cards:<appid>": that game's trading cards, shown as a chip.
        self.cards = query.strip_prefix("cards:").and_then(|g| g.parse().ok());
        self.query = if self.cards.is_some() { String::new() } else { query.to_string() };
        self.app = if self.cards.is_some() { Some(753) } else { app };
        self.start = 0;
        self.selected = None;
    }
}

impl Inventory {
    pub(super) fn market_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        // "<game> trading card(s)": Steam's text search matches the wrong
        // items, so use its card filter for that game (one in your library).
        if self.market.cards.is_none() {
            let q = self.market.query.trim().to_lowercase();
            let game = ["trading cards", "trading card"].iter().find_map(|end| q.strip_suffix(end)).map(str::trim).filter(|g| !g.is_empty());
            if let Some(g) = game {
                if let Some(id) = ctx.state.apps.as_ref().and_then(|apps| apps.keys().copied().find(|id| ctx.state.app_name(*id).is_some_and(|n| n.to_lowercase() == g))) {
                    self.market.cards = Some(id);
                    self.market.query.clear();
                    self.market.app = Some(753);
                    self.market.start = 0;
                }
            }
        }
        let query = match self.market.cards {
            Some(game) => format!("cards:{game}"),
            None => self.market.query.trim().to_string(),
        };
        let want = (query, self.market.app, self.market.start);
        if self.market.asked.as_ref() != Some(&want) {
            self.market.asked = Some(want.clone());
            ctx.send(Command::MarketSearch { query: want.0.clone(), app: want.1, start: want.2 });
        }
        let signed_in = ctx.state.steam_id != 0;
        if signed_in && !self.listings_requested {
            self.listings_requested = true;
            ctx.send(Command::LoadMyListings);
        }
        egui::ScrollArea::vertical().id_salt("market-search").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(t.spacing.xl * 2.0);
            w::centered(ui, 1564.0, t.spacing.xl, |ui| {
                // Title, and the ways to your own items (Figma `532:6677`).
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(w::heading_l(t, t!("Community Market"), t.text));
                        ui.add_space(t.spacing.md);
                        ui.label(w::styled(t!("Buy and sell items with other players"), t.typography.button_large, t.text_dim));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.md;
                        if signed_in && w::button(ui, t, None, t!("Sell an item"), w::Kind::Primary, w::Size::Lg).on_hover_text(t!("Pick an item in your inventory to sell it")).clicked() {
                            self.view = View::Items;
                        }
                        if signed_in && w::button(ui, t, None, t!("View inventory"), w::Kind::Secondary, w::Size::Lg).clicked() {
                            self.view = View::Items;
                        }
                    });
                });
                ui.add_space(t.spacing.xl * 2.0);
                if signed_in {
                    self.my_market(ui, t, ctx);
                    ui.add_space(t.spacing.xl * 2.5);
                }
                // The Market's items: what sells most, a game filter on the left.
                w::filter_bar(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if let Some(game) = self.market.cards {
                            let name = ctx.state.app_name(game).map(String::from).unwrap_or_else(|| game.to_string());
                            if w::sort_chip(ui, t, &format!("{name} · {}  {}", t!("Trading cards"), icons::X), true).on_hover_text(t!("Show everything")).clicked() {
                                self.market.cards = None;
                                self.market.start = 0;
                            }
                        } else {
                            w::sort_chip(ui, t, &t!("Popular items"), true);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let width = 374.0f32.min(ui.available_width() - 8.0).max(120.0);
                            if self.market.cards.is_none() && w::search(ui, t, &mut self.market.query, &t!("Search the Market"), width).changed() {
                                self.market.start = 0;
                            }
                        });
                    });
                });
                ui.add_space(t.spacing.xl - t.spacing.xs);
                let side = 286.0f32.min(ui.available_width() * 0.25);
                let gap = t.spacing.lg + 2.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(vec2(side, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(side);
                        self.game_filter(ui, t, ctx);
                    });
                    let main = ui.available_width();
                    ui.allocate_ui_with_layout(vec2(main, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(main);
                        self.market_results(ui, t, ctx, &want);
                    });
                });
            });
            ui.add_space(t.spacing.xl * 2.0);
        });
    }

    /// "Filter by game": all games, or one of the busiest markets.
    fn game_filter(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx) {
        w::filter_bar(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.spacing.xs;
            ui.label(w::caption_text(t, &t!("Filter by game")));
            ui.add_space(t.spacing.sm);
            let mut games: Vec<(Option<AppId>, String)> = vec![(None, t!("All games").to_string())];
            games.extend(GAMES.iter().map(|(id, name)| (Some(*id), if *id == 753 { t!("Cards and profile items").to_string() } else { ctx.state.app_name(*id).unwrap_or(name).to_string() })));
            for (id, name) in games {
                let on = self.market.app == id && self.market.cards.is_none();
                if filter_row(ui, t, &name, on).clicked() {
                    self.market.app = id;
                    self.market.cards = None;
                    self.market.start = 0;
                }
            }
        });
    }

    /// The items found: a header row, then one row per item; clicking one
    /// opens its price history and orders under it.
    fn market_results(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, want: &(String, Option<AppId>, u32)) {
        let answer = ctx.state.market_search.as_ref().filter(|(k, _)| k == want).map(|(_, r)| r.clone());
        let (results, total) = match answer {
            None => {
                for _ in 0..8 {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 77.0), Sense::hover());
                    w::skeleton(ui, t, r.shrink2(vec2(0.0, t.spacing.xs)), t.corner());
                }
                return;
            }
            Some(Err(e)) => {
                w::empty(ui, t, icons::WARNING, t!("Couldn't search the Market"), &i18n::tr_dyn(&e));
                ui.vertical_centered(|ui| {
                    if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        self.market.asked = None;
                    }
                });
                return;
            }
            Some(Ok(r)) => r,
        };
        let cols = [t!("Quantity").to_string(), t!("Price").to_string()];
        header_row(ui, t, &t!("{n} results", n = total), &cols);
        if results.is_empty() {
            ui.label(w::body_m(t, t!("Nothing matches your search."), t.text_faint));
        }
        for item in &results {
            let chosen = self.market.selected.as_ref().is_some_and(|s| s.hash_name == item.hash_name && s.app_id == item.app_id);
            if result_row(ui, t, item, chosen).clicked() {
                self.market.selected = if chosen { None } else { Some(item.clone()) };
            }
            if chosen {
                self.market_item(ui, t, ctx, item);
                ui.add_space(t.spacing.md);
            }
        }
        // Pages of 30.
        ui.add_space(t.spacing.sm);
        let page = self.market.start / 30 + 1;
        let pages = total.div_ceil(30).max(1);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if let Some(p) = w::page_numbers(ui, t, page, pages) {
                    self.market.start = (p - 1) * 30;
                }
        });
    }

    /// Your listings, buy orders and history, as tabs (Figma `532:6677`).
    fn my_market(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let mut tab = self.market.mine;
        w::filter_bar(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.xs + 1.0;
                for (v, label) in [(MyMarket::Listings, t!("My sell listings")), (MyMarket::Orders, t!("My buy orders")), (MyMarket::History, t!("My market history"))] {
                    if w::sort_chip(ui, t, &label, tab == v).clicked() {
                        tab = v;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::REFRESH, &t!("Refresh"), false, 0).clicked() {
                        ctx.send(Command::LoadMyListings);
                    }
                });
            });
        });
        self.market.mine = tab;
        ui.add_space(t.spacing.xl - t.spacing.xs);
        if tab == MyMarket::History {
            self.listings_tab = super::market::ListingsTab::History;
            self.history_page(ui, t, ctx);
            return;
        }
        let data = match &ctx.state.my_listings {
            None => {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 77.0), Sense::hover());
                w::skeleton(ui, t, r, t.corner());
                return;
            }
            Some(Err(e)) => {
                ui.label(w::body_m(t, t!("Couldn't load your listings: {error}", error = i18n::tr_dyn(e)), t.text_dim));
                return;
            }
            Some(Ok(d)) => d.clone(),
        };
        if tab == MyMarket::Listings {
            let cols = [t!("Listed on").to_string(), t!("Price").to_string(), t!("You receive").to_string(), String::new()];
            header_row(ui, t, &t!("Name"), &cols);
            if data.listings.is_empty() {
                ui.label(w::body_m(t, t!("Items you put up for sale show up here."), t.text_faint));
            }
            for l in &data.listings {
                let listed = if l.created > 0 { ui::date(l.created) } else { String::new() };
                let status = match l.status {
                    model::ListingStatus::NeedsConfirmation => t!("Waiting for confirmation").to_string(),
                    model::ListingStatus::OnHold => t!("On hold").to_string(),
                    _ => l.game.clone(),
                };
                let cells = [listed, super::market::money(l.buyer_pays, l.currency), super::market::money(l.you_receive, l.currency)];
                if table_row(ui, t, &econ_icon(&l.icon), &l.name, l.name_color.as_deref(), &status, &cells, Some(t!("Remove").to_string())) {
                    ctx.send(Command::CancelListing { listing_id: l.listing_id });
                }
            }
        } else {
            let cols = [t!("Quantity").to_string(), t!("Price").to_string(), String::new()];
            header_row(ui, t, &t!("Name"), &cols);
            if data.buy_orders.is_empty() {
                ui.label(w::body_m(t, t!("You have no buy orders."), t.text_faint));
            }
            for o in &data.buy_orders {
                let cells = [t!("{left} of {n} left", left = o.remaining, n = o.quantity), super::market::money(o.price, o.currency)];
                if table_row(ui, t, &econ_icon(&o.icon), &o.name, None, "", &cells, Some(t!("Cancel").to_string())) {
                    ctx.send(Command::CancelBuyOrder { order_id: o.order_id });
                }
            }
        }
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
                    ui.add(egui::Label::new(w::styled(&item.name, ui::theme::TextStyle::new(t.typography.body_l.size, ui::theme::Weight::Bold), name_color(t, &item.color))).wrap());
                    ui.label(w::body_s(t, &item.app_name, t.text_dim));
                    ui.label(w::body_s(t, t!("{n} for sale", n = item.listings), t.text_faint));
                });
            });
            ui.add_space(8.0);
            match ctx.state.market_prices.get(&key).cloned() {
                None => {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 160.0), Sense::hover());
                    w::skeleton(ui, t, r, t.corner_small());
                }
                Some(Err(e)) => {
                    ui.label(w::body_s(t, i18n::tr_dyn(&e), t.text_faint));
                    if ui.link(w::body_s(t, t!("Try again"), t.accent)).clicked() {
                        self.full_asked.remove(&key);
                    }
                }
                Some(Ok(p)) => {
                    let lowest = p.lowest_sell.map(|c| super::market::money(c, currency)).or(p.lowest.clone()).unwrap_or_else(|| item.price.clone());
                    ui.horizontal(|ui| {
                        ui.label(w::body_m(t, t!("Starting at"), t.text_dim));
                        ui.label(w::styled(lowest, t.typography.price, t.text));
                    });
                    if let Some(v) = p.volume {
                        ui.label(w::body_s(t, t!("{n} sold in the last 24 hours", n = v), t.text_dim));
                    }
                    ui.add_space(6.0);
                    super::market::chart(ui, t, &p, currency);
                    ui.add_space(6.0);
                    super::market::ladders(ui, t, &p, currency);
                    ui.add_space(t.spacing.sm);
                    self.buy_buttons(ui, t, ctx, item.app_id, &item.hash_name, &item.name, &item.icon, Some(&p));
                }
            }
        });
    }
}

fn name_color(t: &Theme, hex: &str) -> Color32 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok().filter(|_| hex.len() >= 6);
    v.map(|v| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)).unwrap_or(t.text)
}

/// Width of each column after the name in a Market table.
const COLUMN: f32 = 170.0;

fn econ_icon(icon: &str) -> String {
    if icon.is_empty() { String::new() } else { model::art::econ(icon, 192) }
}

/// A table's column names over its rows (Figma `532:6677`): the first on
/// the left, the rest in fixed columns from the right.
fn header_row(ui: &mut Ui, t: &Theme, first: &str, cols: &[String]) {
    w::filter_bar(t).inner_margin(egui::Margin::symmetric(t.spacing.md as i8 + 2, t.spacing.sm as i8 + 2)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
        let style = t.typography.body_m.font();
        ui.painter().text(r.left_center(), Align2::LEFT_CENTER, first, style.clone(), t.text_dim);
        for (i, c) in cols.iter().rev().enumerate() {
            let x = r.right() - COLUMN * i as f32 - COLUMN / 2.0;
            ui.painter().text(egui::pos2(x, r.center().y), Align2::CENTER_CENTER, c, style.clone(), t.text_dim);
        }
    });
    ui.add_space(t.spacing.sm + 2.0);
}

/// A row of a Market table: the item's picture, name and a line under it,
/// then `cells` in columns, and an optional button in the last one.
/// True when the button was clicked.
#[allow(clippy::too_many_arguments)]
fn table_row(ui: &mut Ui, t: &Theme, image: &str, name: &str, color: Option<&str>, sub: &str, cells: &[String], action: Option<String>) -> bool {
    let mut clicked = false;
    let columns = cells.len() + usize::from(action.is_some());
    w::post_card(t).inner_margin(egui::Margin::symmetric(t.spacing.md as i8 + 2, t.spacing.md as i8 + 2)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
        let img = egui::Rect::from_min_size(r.min, vec2(50.0, 50.0));
        ui.painter().rect_filled(img, t.corner_small(), t.raised);
        if !image.is_empty() {
            w::paint_contain(ui, img.shrink(4.0), image, Align2::CENTER_CENTER);
        }
        let text_w = (r.width() - 64.0 - COLUMN * columns as f32).max(60.0);
        let name_g = ui.painter().layout(name.to_string(), w::font_bold(t.typography.body_m.size), color.map(|c| name_color(t, c)).unwrap_or(t.text), text_w);
        let sub_g = ui.painter().layout_no_wrap(sub.to_string(), t.typography.body_s.font(), t.text_dim);
        let x = img.right() + t.spacing.lg - 2.0;
        let clip = ui.painter().with_clip_rect(egui::Rect::from_min_size(egui::pos2(x, r.top()), vec2(text_w, r.height())));
        clip.galley(egui::pos2(x, r.top() + 4.0), name_g, t.text);
        clip.galley(egui::pos2(x, r.bottom() - 18.0), sub_g, t.text_dim);
        let first = r.right() - COLUMN * columns as f32;
        for (i, c) in cells.iter().enumerate() {
            let cx = first + COLUMN * i as f32 + COLUMN / 2.0;
            ui.painter().text(egui::pos2(cx, r.center().y), Align2::CENTER_CENTER, c, t.typography.body_m.font(), t.text);
        }
        if let Some(label) = action {
            let cell = egui::Rect::from_min_size(egui::pos2(r.right() - COLUMN, r.top()), vec2(COLUMN, r.height()));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell).layout(Layout::right_to_left(Align::Center)));
            clicked = w::button(&mut child, t, None, &label, w::Kind::Tertiary, w::Size::Sm).clicked();
        }
    });
    ui.add_space(t.spacing.md);
    clicked
}

/// A choice in the "Filter by game" column.
fn filter_row(ui: &mut Ui, t: &Theme, label: &str, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let c = &t.components.sort;
    let look = t.look(c, w::interact(ui, &resp, on));
    w::paint_box(ui, t, r, c, &look, if on { 1.0 } else { 0.0 });
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), t.typography.body_s.font(), look.text);
    job.wrap.max_width = r.width() - t.spacing.md * 2.0;
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(r.left() + t.spacing.md, r.center().y - g.size().y / 2.0), g, look.text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An item found on the Market (Figma `532:6677`).
fn result_row(ui: &mut Ui, t: &Theme, item: &MarketResult, chosen: bool) -> egui::Response {
    let top = ui.cursor().top();
    let cells = [crate::builtin::hub::post::number(item.listings), item.price.clone()];
    table_row(ui, t, &item.icon, &item.name, Some(item.color.as_str()).filter(|c| !c.is_empty()), &item.app_name, &cells, None);
    let rect = egui::Rect::from_min_max(egui::pos2(ui.min_rect().left(), top), egui::pos2(ui.max_rect().right(), ui.cursor().top() - t.spacing.md));
    let resp = ui.interact(rect, ui.scope_id().with(("market-row", &item.app_id, &item.hash_name)), Sense::click());
    if chosen || resp.hovered() {
        ui.painter().rect_stroke(rect, t.corner(), egui::Stroke::new(1.5, if chosen { t.accent } else { t.surface_hover }), egui::StrokeKind::Inside);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

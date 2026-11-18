//! Store: filtered browsing, wishlist, cart (with gifting), price history.

use super::{Load, Shared, Store, View};
use client::Command;
use egui::{Align, Align2, Color32, Layout, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, SteamId};
use store::query::{QueryItem, Sort};
use ui::{Theme, icons, widgets as w};

/// Popular tags offered as quick filters (Steam tag ids).
const TAGS: &[u32] = &[19, 21, 122, 9, 597, 599, 492, 1664, 1667, 1774, 1695, 3810, 1662, 4182, 3859, 1685, 701, 699, 1742, 3964, 128, 3871];

pub struct Gift {
    pub app: AppId,
    pub package: u32,
    pub name: String,
    pub to: Option<SteamId>,
    /// The cart line this picks a friend for (None: add a new gift copy).
    pub line: Option<u64>,
    /// Filter for the friend list.
    pub search: String,
    /// Region checks per friend ("can Steam deliver this gift to them?").
    pub checks: std::collections::HashMap<SteamId, Shared<store::gift::GiftCheck>>,
}

fn snapshot<T: Clone>(slot: &Shared<T>) -> Option<Result<T, String>> {
    match &*slot.lock().unwrap() {
        Load::Loading => None,
        Load::Ready(v) => Some(Ok(v.clone())),
        Load::Failed(e) => Some(Err(e.clone())),
    }
}

/// How the wishlist is sorted.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum WishSort {
    /// Your own order (Steam's wishlist ranking).
    #[default]
    Rank,
    Added,
    Name,
    Price,
    Discount,
    Released,
    Reviews,
}

/// The wishlist's view, sorting and filters.
#[derive(Default)]
pub struct WishView {
    pub sort: WishSort,
    pub query: String,
    pub grid: bool,
    /// 0: any system, 1: macOS, 2: SteamOS + Linux.
    pub platform: u8,
    /// In cents.
    pub max_price: Option<u64>,
    pub on_sale: bool,
}

/// The browse page's tabs (Figma `660:10019`): a sort, or a filter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    All,
    Trending,
    TopSellers,
    TopRated,
    Discounted,
    MostPlayed,
    ComingSoon,
}

impl Tab {
    const ALL: [Tab; 7] = [Tab::All, Tab::Trending, Tab::TopSellers, Tab::TopRated, Tab::Discounted, Tab::MostPlayed, Tab::ComingSoon];

    fn label(self) -> &'static str {
        match self {
            Tab::All => t!("All items"),
            Tab::Trending => t!("New and trending"),
            Tab::TopSellers => t!("Top sellers"),
            Tab::TopRated => t!("Top rated"),
            Tab::Discounted => t!("Discounted"),
            Tab::MostPlayed => t!("Most played"),
            Tab::ComingSoon => t!("Coming soon"),
        }
    }

    /// Which tab the filters amount to.
    fn of(f: &store::query::Filters) -> Option<Tab> {
        Some(match (f.sort, f.only_discounted, f.coming_soon) {
            (_, true, false) => Tab::Discounted,
            (_, false, true) => Tab::ComingSoon,
            (Sort::Popular, false, false) => Tab::All,
            (Sort::NewAndTrending, false, false) => Tab::Trending,
            (Sort::TopSellers, false, false) => Tab::TopSellers,
            (Sort::TopRated, false, false) => Tab::TopRated,
            (Sort::MostPlayed, false, false) => Tab::MostPlayed,
            _ => return None,
        })
    }

    fn apply(self, f: &mut store::query::Filters) {
        f.only_discounted = self == Tab::Discounted;
        f.coming_soon = self == Tab::ComingSoon;
        f.sort = match self {
            Tab::Trending => Sort::NewAndTrending,
            Tab::TopSellers => Sort::TopSellers,
            Tab::TopRated => Sort::TopRated,
            Tab::MostPlayed => Sort::MostPlayed,
            _ => Sort::Popular,
        };
    }
}

/// A browse result as a store card's game.
fn as_item(it: &QueryItem) -> store::Item {
    store::Item { app_id: it.app_id, name: it.name.clone(), header: it.header.clone(), large: None, price: it.price.clone(), windows: it.platforms.0 }
}

/// A bar across a list page: "View" with list and grid, then sort choices
/// (Figma `300:321`'s "Sort by" bar). True when the view changed.
fn list_bar(ui: &mut Ui, t: &Theme, grid: &mut bool, add: impl FnOnce(&mut Ui)) {
    w::component_frame(t, &t.components.store_card).inner_margin(egui::Margin::symmetric(t.dense(20.0) as i8, t.dense(6.0) as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // Wraps onto a second line rather than widening the page.
        ui.horizontal_wrapped(|ui| {
            ui.set_min_height(t.dense(41.0));
            ui.label(w::body_m(t, t!("View"), t.text_dim));
            ui.spacing_mut().item_spacing.x = t.spacing.xs;
            if w::icon_box(ui, t, icons::LIST, t!("List"), !*grid).clicked() {
                *grid = false;
            }
            if w::icon_box(ui, t, icons::GRID, t!("Grid"), *grid).clicked() {
                *grid = true;
            }
            ui.add_space(t.dense(60.0));
            ui.spacing_mut().item_spacing.x = t.dense(6.0);
            add(ui);
        });
    });
}

/// A collapsible group in a filter column, with its name and a caret.
fn filter_group(ui: &mut Ui, t: &Theme, id: &str, title: &str, add: impl FnOnce(&mut Ui)) {
    let key = ui.scope_id().with(("filter-group", id));
    let mut open = ui.data_mut(|d| *d.get_temp_mut_or(key, true));
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), t.dense(36.0)), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let color = w::mix(t.text, t.accent, hover);
    ui.painter().text(r.left_center(), Align2::LEFT_CENTER, title, t.typography.body_l.font(), color);
    ui.painter().text(r.right_center() - vec2(4.0, 0.0), Align2::RIGHT_CENTER, if open { icons::CARET_DOWN } else { icons::CARET_RIGHT }, t.typography.body_m.font(), color);
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(key, open));
    }
    if open {
        add(ui);
    }
    ui.add_space(t.dense(6.0));
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, t.border));
    ui.add_space(t.dense(6.0));
}

/// A choice in a filter group: a full-width row with a count after it,
/// filled while it's on (the `tag` style's rows).
fn filter_row(ui: &mut Ui, t: &Theme, label: &str, count: Option<usize>, on: bool) -> egui::Response {
    let c = &t.components.tag;
    let h = t.dense(25.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let look = t.look(c, w::interact(ui, &resp, on));
    if on || resp.hovered() {
        w::paint_box(ui, t, rect, c, &look, 0.0);
    }
    let x = t.dense(9.0);
    ui.painter().text(pos2(rect.left() + x, rect.center().y), Align2::LEFT_CENTER, label, t.typography.body_s.font(), if on { look.text } else { t.text });
    if let Some(n) = count {
        ui.painter().text(pos2(rect.right() - x, rect.center().y), Align2::RIGHT_CENTER, super::thousands(n as u64), t.typography.body_s.font(), t.text_dim);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A filter column's frame: "FILTERS", a reset button, then `add`.
fn filter_column(ui: &mut Ui, t: &Theme, can_reset: bool, add: impl FnOnce(&mut Ui)) -> bool {
    let mut reset = false;
    w::component_frame(t, &t.components.store_card).inner_margin(egui::Margin::symmetric(t.dense(14.0) as i8, t.dense(14.0) as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(w::caption_text(t, &t!("Filters").to_uppercase()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(can_reset, |ui| {
                    reset = w::icon_box(ui, t, icons::FUNNEL, t!("Clear the filters"), false).clicked();
                });
            });
        });
        ui.add_space(t.spacing.sm);
        add(ui);
    });
    reset
}

impl Store {
    // ---------- browse ----------

    /// Browse (Figma "browse" `660:10019`): the categories, then every game
    /// with a sort or filter tab, the filters on the left.
    pub(super) fn browse_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let mut f = self.filters.clone();
        f.hidden_descriptors = self.hidden.iter().copied().collect();
        if self.query.as_ref().is_none_or(|(key, page, _)| *key != f || *page != self.browse_page) {
            let (api, ff, start) = (self.api.clone(), f.clone(), self.browse_page * 48);
            let slot = self.fetch(async move { api.query(&ff, start, 48).await });
            self.query = Some((f.clone(), self.browse_page, slot));
        }
        ui.label(w::heading_l(t, t!("Top categories"), t.text));
        ui.add_space(t.dense(20.0));
        self.category_strip(ui, t, "browse-categories", t.dense(240.0));
        ui.add_space(t.dense(50.0));
        ui.label(w::heading_l(t, t!("Browse games"), t.text));
        ui.add_space(t.dense(20.0));
        let mut grid = self.wish.grid;
        let current = Tab::of(&self.filters);
        list_bar(ui, t, &mut grid, |ui| {
            for tab in Tab::ALL {
                if w::sort_chip(ui, t, tab.label(), current == Some(tab)).clicked() {
                    tab.apply(&mut self.filters);
                    self.browse_page = 0;
                }
            }
        });
        self.wish.grid = grid;
        ui.add_space(t.dense(12.0));
        let gap = t.dense(12.0);
        let side_w = t.dense(290.0).min(ui.available_width() * 0.3);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.vertical(|ui| {
                ui.set_width(side_w);
                self.filter_panel(ui, t);
            });
            ui.vertical(|ui| {
                let width = ui.available_width();
                ui.set_width(width);
                let Some((_, _, slot)) = &self.query else { return };
                match snapshot(slot) {
                    None => w::loading(ui, t, t!("Searching…")),
                    Some(Err(e)) => w::empty(ui, t, icons::WARNING, t!("Search failed"), &e),
                    Some(Ok((items, total))) => {
                        let owned = |id: &AppId| ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(id));
                        let items: Vec<QueryItem> = items.into_iter().filter(|i| !self.hide_owned || !owned(&i.app_id)).collect();
                        ui.label(w::body_m(t, t!("{n} results", n = total), t.text_dim));
                        ui.add_space(t.spacing.xs);
                        self.list_or_grid(ui, t, ctx, &items, None);
                        let pages = total.div_ceil(48).max(1) as u32;
                        ui.add_space(t.spacing.sm);
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(self.browse_page > 0, |ui| {
                                if w::button(ui, t, Some(icons::BACK), t!("Previous"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                    self.browse_page -= 1;
                                }
                            });
                            ui.label(w::body_m(t, t!("Page {page} of {pages}", page = self.browse_page + 1, pages = pages), t.text_dim));
                            ui.add_enabled_ui(self.browse_page + 1 < pages, |ui| {
                                if w::button(ui, t, Some(icons::CARET_RIGHT), t!("Next"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                    self.browse_page += 1;
                                }
                            });
                        });
                    }
                }
            });
        });
    }

    /// Games as rows, or as store cards in the grid view. `ranks` gives the
    /// wishlist's numbers and dates added.
    fn list_or_grid(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, items: &[QueryItem], ranks: Option<&[(usize, u64)]>) {
        if self.wish.grid {
            let gap = t.dense(16.0);
            let per = (((ui.available_width() + gap) / (t.dense(300.0) + gap)).floor() as usize).max(1);
            let cw = ((ui.available_width() - gap * (per as f32 - 1.0)) / per as f32).floor();
            let wishlist = ctx.state.wishlist.clone().unwrap_or_default();
            for row in items.chunks(per) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for it in row {
                        let wished = wishlist.iter().any(|(a, _)| *a == it.app_id);
                        let out = super::home::store_card(ui, t, &as_item(it), cw, self.owned.contains(&it.app_id), wished, ctx.state.steam_id != 0);
                        self.micro(ui, it.app_id, out.img, t.corner_small(), out.hovered);
                        if out.wish {
                            ctx.send(if wished { Command::WishlistRemove { app_id: it.app_id } } else { Command::WishlistAdd { app_id: it.app_id } });
                        } else if out.open {
                            self.open(View::App(it.app_id));
                        }
                    }
                });
                ui.add_space(gap);
            }
            return;
        }
        for (i, it) in items.iter().enumerate() {
            if self.result_row(ui, t, ctx, it, ranks.and_then(|r| r.get(i).copied())) {
                self.open(View::App(it.app_id));
            }
            ui.add_space(t.dense(8.0));
        }
    }

    fn filter_panel(&mut self, ui: &mut Ui, t: &Theme) {
        let before = self.filters.clone();
        let defaults = store::query::Filters { sort: self.filters.sort, only_discounted: self.filters.only_discounted, coming_soon: self.filters.coming_soon, ..Default::default() };
        let can_reset = self.filters != defaults || self.hide_owned;
        let mut hide_owned = self.hide_owned;
        let reset = filter_column(ui, t, can_reset, |ui| {
            // Any of Steam's tags, by name; the ones in use as removable tags.
            let width = ui.available_width();
            w::search(ui, t, &mut self.tag_query, t!("Find a tag…"), width);
            let query = self.tag_query.trim().to_string();
            if query.chars().count() >= 2 {
                let found: Vec<(u32, String)> = self.find_tags(&query).into_iter().filter(|(id, _)| !self.filters.tags.contains(id)).take(12).collect();
                ui.add_space(t.spacing.sm);
                if found.is_empty() {
                    ui.label(w::body_s(t, t!("No tags match."), t.text_faint));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
                    for (id, name) in found {
                        if w::tag(ui, t, &format!("{}  {name}", icons::PLUS), None, w::TagState::Unchecked).clicked() {
                            self.filters.tags.push(id);
                            self.tag_query.clear();
                        }
                    }
                });
            }
            if !self.filters.tags.is_empty() {
                ui.add_space(t.spacing.sm);
                let tags = self.filters.tags.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
                    for tag in tags {
                        let name = self.tag_name(tag).unwrap_or_else(|| format!("#{tag}"));
                        if w::tag(ui, t, &name, None, w::TagState::Filtered).on_hover_text(t!("Remove this tag")).clicked() {
                            self.filters.tags.retain(|x| *x != tag);
                        }
                    }
                });
            }
            ui.add_space(t.spacing.md);
            let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, t.border));
            ui.add_space(t.dense(6.0));
            filter_group(ui, t, "genres", t!("Popular tags"), |ui| {
                for tag in TAGS {
                    let name = self.tag_name(*tag).unwrap_or_else(|| format!("#{tag}"));
                    let on = self.filters.tags.contains(tag);
                    if filter_row(ui, t, &name, None, on).clicked() {
                        if on {
                            self.filters.tags.retain(|x| x != tag);
                        } else {
                            self.filters.tags.push(*tag);
                        }
                    }
                }
            });
            let f = &mut self.filters;
            filter_group(ui, t, "price", t!("Price"), |ui| {
                let options: [(Option<u32>, bool, &str); 6] = [
                    (None, false, "Any price"),
                    (None, true, "Free"),
                    (Some(500), false, "Under $5"),
                    (Some(1000), false, "Under $10"),
                    (Some(2000), false, "Under $20"),
                    (Some(4000), false, "Under $40"),
                ];
                for (max, free, label) in options {
                    let on = f.max_price == max && f.only_free == free;
                    if filter_row(ui, t, &i18n::tr_dyn(label), None, on).clicked() {
                        f.max_price = max;
                        f.only_free = free;
                    }
                }
            });
            filter_group(ui, t, "reviews", t!("Reviews"), |ui| {
                for (min, label) in [(None, "Any reviews"), (Some(7), "Mostly positive and up"), (Some(8), "Very positive and up")] {
                    if filter_row(ui, t, &i18n::tr_dyn(label), None, f.min_review_score == min).clicked() {
                        f.min_review_score = min;
                    }
                }
            });
            filter_group(ui, t, "type", t!("Type"), |ui| {
                for (on, label) in [(&mut f.games, t!("Games")), (&mut f.dlc, t!("DLC")), (&mut f.software, t!("Software")), (&mut f.demos, t!("Demos")), (&mut f.soundtracks, t!("Soundtracks"))] {
                    if filter_row(ui, t, label, None, *on).clicked() {
                        *on = !*on;
                    }
                }
            });
            ui.add_space(t.spacing.xs);
            w::check(ui, t, &mut hide_owned, w::body_s(t, t!("Hide games I own"), t.text));
        });
        self.hide_owned = hide_owned;
        if reset {
            self.filters = store::query::Filters { sort: self.filters.sort, only_discounted: self.filters.only_discounted, coming_soon: self.filters.coming_soon, ..Default::default() };
            self.hide_owned = false;
            self.tag_query.clear();
        }
        if self.filters != before {
            self.browse_page = 0;
        }
    }

    /// One game in a list (browse, wishlist; Figma `300:321`'s rows): art,
    /// name, top tags, systems and release date, the review bar; on the
    /// right the heart, then the price and Add to Cart. With `rank`, the
    /// wishlist's number and the date it was added. Returns whether it was
    /// opened.
    fn result_row(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, it: &QueryItem, rank: Option<(usize, u64)>) -> bool {
        let c = &t.components.store_card;
        let pad = t.dense(10.0);
        let h = t.dense(152.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let look = t.look(c, w::interact(ui, &resp, false));
        w::paint_box(ui, t, rect, c, &look, 0.0);
        // Narrow (a small window, or the friends list open): a smaller
        // picture and price column, so the middle keeps room for tags.
        let compact = rect.width() < t.dense(980.0);
        let mut left = rect.left() + pad;
        if let Some((n, _)) = rank {
            let col = Rect::from_min_size(pos2(left, rect.top()), vec2(t.dense(26.0), h));
            ui.painter().text(col.center() - vec2(0.0, t.dense(16.0)), Align2::CENTER_CENTER, "=", t.typography.body_l.font(), t.text_dim);
            let num = Rect::from_center_size(col.center() + vec2(0.0, t.dense(22.0)), vec2(t.dense(24.0), t.dense(24.0)));
            ui.painter().rect_filled(num, t.corner_small(), t.chrome);
            ui.painter().text(num.center(), Align2::CENTER_CENTER, (n + 1).to_string(), t.typography.body_s.font(), t.text_dim);
            left = col.right() + t.dense(8.0);
        }
        let img_h = h - pad * 2.0 - if compact { t.dense(20.0) } else { 0.0 };
        let img = Rect::from_min_size(pos2(left, rect.center().y - img_h / 2.0), vec2((img_h * 460.0 / 215.0).round(), img_h));
        w::image_or_skeleton(ui, t, img, &it.header, t.corner_small());
        hover_wash(ui, t, img, &resp);
        let owned = ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&it.app_id));
        let wished = ctx.state.wishlist.as_ref().is_some_and(|l| l.iter().any(|(a, _)| *a == it.app_id));
        let right_w = if compact { t.dense(230.0) } else { t.dense(330.0) };
        let mid = Rect::from_min_max(pos2(img.right() + t.dense(16.0), rect.top() + pad), pos2(rect.right() - right_w, rect.bottom() - pad));
        let opened = resp.clicked();
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(mid).layout(Layout::top_down(Align::Min)));
        // Nothing in the middle may run under the price on the right.
        c.set_clip_rect(mid.intersect(ui.clip_rect()));
        c.spacing_mut().item_spacing.y = t.dense(5.0);
        c.add(egui::Label::new(w::heading_m(t, &it.name, look.text)).truncate().selectable(false));
        // Top tags, like Steam's: "+" opens the rest in a popup (the row
        // keeps its size), and a tag opens the store filtered by it.
        let mut open_tag = None;
        let names: Vec<(u32, String)> = it.tags.iter().filter_map(|tag| Some((*tag, self.tag_name(*tag)?))).collect();
        let tag_c = &t.components.tag;
        let measure = |s: &str| c.painter().layout_no_wrap(s.to_string(), t.font_of(tag_c), t.text_dim).size().x + t.padding_of(tag_c).x * 2.0 + t.spacing.xs;
        let room = c.available_width() - measure("+") - if it.demo.is_some() { measure(t!("Demo available")) + t.spacing.sm } else { 0.0 };
        let mut shown = 0;
        let mut used = 0.0;
        for (i, (_, name)) in names.iter().enumerate().take(4) {
            if used + measure(name) > room {
                break;
            }
            used += measure(name);
            shown = i + 1;
        }
        c.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
            for (tag, name) in names.iter().take(shown) {
                if w::tag(ui, t, name, None, w::TagState::Plain).on_hover_text(t!("Browse games with this tag")).clicked() {
                    open_tag = Some(*tag);
                }
            }
            if names.len() > shown {
                let more = w::tag(ui, t, "+", None, w::TagState::Plain).on_hover_text(t!("More tags"));
                egui::Popup::from_toggle_button_response(&more).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(320.0).show(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
                        for (tag, name) in names.iter().skip(shown) {
                            if w::tag(ui, t, name, None, w::TagState::Plain).on_hover_text(t!("Browse games with this tag")).clicked() {
                                open_tag = Some(*tag);
                            }
                        }
                    });
                });
            }
            // Like Steam: a demo to try, on the game's page.
            if it.demo.is_some() {
                w::tag(ui, t, t!("Demo available"), None, w::TagState::Current);
            }
        });
        if let Some(tag) = open_tag {
            self.open_tag(tag);
        }
        c.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.dense(10.0);
            let (win, mac, lin) = it.platforms;
            let systems: String = [(win, icons::WINDOWS), (mac, icons::APPLE), (lin, icons::LINUX)].iter().filter(|x| x.0).map(|x| x.1).collect::<Vec<_>>().join("  ");
            ui.label(w::body_l(t, systems, t.text_dim));
            if it.coming_soon {
                ui.label(w::body_m(t, format!("{}  {}", icons::CALENDAR, i18n::tr_dyn(&it.release_text)), t.accent));
            } else if it.released > 0 {
                ui.label(w::body_m(t, ui::date(it.released), t.text_dim));
            }
            if owned {
                ui.label(w::body_m(t, format!("{}  {}", icons::CHECK, t!("In library")), t.play));
            }
        });
        if it.review_count > 0 {
            c.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.dense(10.0);
                w::review_bar(ui, t, it.review_percent.min(100) as u8, t.dense(138.0));
                if !compact {
                    ui.label(w::body_s(t, t!("{count} user reviews", count = super::thousands(it.review_count)), t.accent));
                }
            });
        }
        // Right: the heart on top (and when it was wishlisted), price and
        // Add to Cart at the bottom.
        let side = Rect::from_min_max(pos2(rect.right() - right_w, rect.top() + pad), rect.max - vec2(pad, pad));
        let row_h = t.dense(44.0);
        let mut r = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(side.min, vec2(side.width(), row_h))).layout(Layout::right_to_left(Align::Center)));
        r.spacing_mut().item_spacing.x = t.dense(14.0);
        if ctx.state.steam_id != 0 {
            let tip = if wished { t!("Remove from wishlist") } else { t!("Add to wishlist") };
            if w::wishlist_button_sized(&mut r, t, wished, None, Some(row_h)).on_hover_text(tip).clicked() {
                ctx.send(if wished { Command::WishlistRemove { app_id: it.app_id } } else { Command::WishlistAdd { app_id: it.app_id } });
            }
        }
        if let Some((_, added)) = rank.filter(|(_, a)| *a > 0) {
            r.label(w::body_s(t, t!("Added on {date}", date = ui::date(added)), t.text_dim));
        }
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(side.left(), side.bottom() - row_h), side.max)).layout(Layout::right_to_left(Align::Center)));
        b.spacing_mut().item_spacing.x = t.dense(13.0);
        if !owned && !it.coming_soon {
            if let Some(pkg) = it.package_id {
                let add = if compact {
                    w::button(&mut b, t, Some(icons::CART), "", w::Kind::Primary, w::Size::Md).on_hover_text(t!("Add to cart"))
                } else {
                    w::button(&mut b, t, None, t!("Add to cart"), w::Kind::Primary, w::Size::Md)
                };
                if add.clicked() {
                    self.add_to_cart(ctx, it.app_id, pkg, &it.name, None, owned);
                }
            }
        }
        match &it.formatted_price {
            _ if it.coming_soon => {
                b.label(w::body_l(t, t!("Coming soon"), t.text_dim));
            }
            Some(p) => w::price(&mut b, t, p, it.formatted_original.as_deref(), it.price.discount),
            None if it.is_free => {
                b.label(w::styled(t!("Free"), t.typography.price, t.text_dim));
            }
            None => {}
        }
        // How long the discount lasts, like the redesign's cards: before the price.
        if !compact {
            if let Some(ends) = discount_ends_text(it.discount_ends) {
                b.label(w::body_s(t, ends, t.text_dim));
            }
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        opened
    }

    // ---------- wishlist ----------

    /// The wishlist (Figma "wishlist" `300:321`).
    pub(super) fn wishlist_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.persona(ctx.state.steam_id).name;
        ui.horizontal(|ui| {
            let title = if me.is_empty() { t!("Your wishlist").to_string() } else { t!("{name}'s Wishlist", name = me) };
            ui.label(w::heading_l(t, title, t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(ui, t, None, t!("My Profile"), w::Kind::Secondary, w::Size::Md).clicked() {
                    ctx.navigate("profile", None);
                }
            });
        });
        ui.add_space(t.dense(20.0));
        let Some(list) = ctx.state.wishlist.clone() else {
            w::loading(ui, t, t!("Loading…"));
            return;
        };
        if list.is_empty() {
            w::empty(ui, t, icons::HEART, t!("Your wishlist is empty"), t!("Add games from their store pages and you'll be told when they go on sale."));
            return;
        }
        let ids: Vec<AppId> = list.iter().map(|x| x.0).collect();
        if self.wish_items.as_ref().is_none_or(|(k, _)| *k != ids) {
            let (api, ids2) = (self.api.clone(), ids.clone());
            let slot = self.fetch(async move { api.items(&ids2).await });
            self.wish_items = Some((ids.clone(), slot));
        }
        let mut grid = self.wish.grid;
        let mut sort = self.wish.sort;
        list_bar(ui, t, &mut grid, |ui| {
            ui.label(w::body_m(t, t!("Sort by"), t.text_dim));
            for (s, label) in [
                (WishSort::Rank, t!("Your rank")),
                (WishSort::Added, t!("Date added")),
                (WishSort::Name, t!("Name")),
                (WishSort::Price, t!("Price")),
                (WishSort::Discount, t!("Discount")),
                (WishSort::Released, t!("Release date")),
                (WishSort::Reviews, t!("Review score")),
            ] {
                if w::sort_chip(ui, t, label, sort == s).clicked() {
                    sort = s;
                }
            }
        });
        self.wish.grid = grid;
        self.wish.sort = sort;
        ui.add_space(t.dense(12.0));
        let Some(Ok(items)) = self.wish_items.as_ref().and_then(|(_, s)| snapshot(s)) else {
            w::loading(ui, t, t!("Loading…"));
            return;
        };
        let order: Vec<(AppId, usize, u64)> = list.iter().enumerate().map(|(i, (a, added))| (*a, i, *added)).collect();
        let rank_of = |id: AppId| order.iter().find(|o| o.0 == id).map(|o| (o.1, o.2)).unwrap_or((usize::MAX, 0));
        let gap = t.dense(12.0);
        let side_w = t.dense(290.0).min(ui.available_width() * 0.3);
        let v = &mut self.wish;
        let q = v.query.trim().to_lowercase();
        let price = |i: &QueryItem| i.price.final_.unwrap_or(0);
        let passes_others = |i: &QueryItem, v: &WishView| -> bool {
            (q.is_empty() || i.name.to_lowercase().contains(&q))
                && v.max_price.is_none_or(|m| price(i) < m)
                && (!v.on_sale || i.price.discount > 0)
                && match v.platform {
                    1 => i.platforms.1,
                    2 => i.platforms.2,
                    _ => true,
                }
        };
        // Counts beside each choice: how many pass with it and the other filters.
        let count = |f: &dyn Fn(&mut WishView)| -> usize {
            let mut probe = WishView { sort: v.sort, query: v.query.clone(), grid: v.grid, platform: v.platform, max_price: v.max_price, on_sale: v.on_sale };
            f(&mut probe);
            items.iter().filter(|i| passes_others(i, &probe)).count()
        };
        let mut shown: Vec<QueryItem> = items.iter().filter(|i| passes_others(i, v)).cloned().collect();
        match v.sort {
            WishSort::Rank => shown.sort_by_key(|i| rank_of(i.app_id).0),
            WishSort::Added => shown.sort_by_key(|i| std::cmp::Reverse(rank_of(i.app_id).1)),
            WishSort::Name => shown.sort_by_key(|i| i.name.to_lowercase()),
            WishSort::Price => shown.sort_by_key(price),
            WishSort::Discount => shown.sort_by_key(|i| std::cmp::Reverse(i.price.discount)),
            WishSort::Released => shown.sort_by_key(|i| std::cmp::Reverse(i.released)),
            WishSort::Reviews => shown.sort_by_key(|i| std::cmp::Reverse((i.review_percent, i.review_count))),
        }
        let platform_counts = [count(&|p| p.platform = 0), count(&|p| p.platform = 1), count(&|p| p.platform = 2)];
        let price_counts = [count(&|p| p.max_price = None), count(&|p| p.max_price = Some(500)), count(&|p| p.max_price = Some(1000)), count(&|p| p.max_price = Some(2000))];
        let sale_count = count(&|p| p.on_sale = true);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.vertical(|ui| {
                ui.set_width(side_w);
                let v = &mut self.wish;
                let can_reset = v.platform != 0 || v.max_price.is_some() || v.on_sale || !v.query.is_empty();
                let reset = filter_column(ui, t, can_reset, |ui| {
                    let width = ui.available_width();
                    w::search(ui, t, &mut v.query, t!("Search by name"), width);
                    ui.add_space(t.spacing.md);
                    filter_group(ui, t, "wish-platform", t!("Platform"), |ui| {
                        for (i, label) in [t!("Any system"), "macOS", t!("SteamOS + Linux")].into_iter().enumerate() {
                            if filter_row(ui, t, label, Some(platform_counts[i]), v.platform == i as u8).clicked() {
                                v.platform = i as u8;
                            }
                        }
                    });
                    filter_group(ui, t, "wish-price", t!("Price"), |ui| {
                        for (i, (max, label)) in [(None, "Any price"), (Some(500), "Under $5"), (Some(1000), "Under $10"), (Some(2000), "Under $20")].into_iter().enumerate() {
                            if filter_row(ui, t, &i18n::tr_dyn(label), Some(price_counts[i]), v.max_price == max).clicked() {
                                v.max_price = max;
                            }
                        }
                    });
                    filter_group(ui, t, "wish-discount", t!("Discount"), |ui| {
                        if filter_row(ui, t, t!("On sale now"), Some(sale_count), v.on_sale).clicked() {
                            v.on_sale = !v.on_sale;
                        }
                    });
                });
                if reset {
                    *v = WishView { sort: v.sort, grid: v.grid, ..Default::default() };
                }
            });
            ui.vertical(|ui| {
                let width = ui.available_width();
                ui.set_width(width);
                let on_sale = items.iter().filter(|i| i.price.discount > 0).count();
                ui.label(w::body_m(t, t!("{n} games · {sale} on sale now", n = items.len(), sale = on_sale), t.text_dim));
                ui.add_space(t.spacing.xs);
                if shown.is_empty() {
                    w::empty(ui, t, icons::FUNNEL, t!("Nothing matches"), t!("Try fewer filters."));
                }
                let ranks: Vec<(usize, u64)> = shown.iter().map(|i| rank_of(i.app_id)).collect();
                self.list_or_grid(ui, t, ctx, &shown, Some(&ranks));
            });
        });
    }

    // ---------- cart ----------

    /// The cart (Figma "cart" `397:11087`): a row per copy, the estimated
    /// total with Continue Shopping and the purchase buttons, and a few
    /// games on sale beside it.
    pub(super) fn cart_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.persona(ctx.state.steam_id).name;
        let title = if me.is_empty() { t!("Your cart").to_string() } else { t!("{name}'s Shopping Cart", name = me) };
        ui.label(w::heading_l(t, title, t.text));
        ui.add_space(t.dense(30.0));
        let Some(lines) = ctx.state.cart.clone() else {
            w::loading(ui, t, t!("Loading…"));
            return;
        };
        let pkgs: Vec<u32> = lines.iter().map(|l| l.package_id).collect();
        if !lines.is_empty() && self.cart_names.as_ref().is_none_or(|(k, _)| *k != pkgs) {
            let (api, p2) = (self.api.clone(), pkgs.clone());
            let slot = self.fetch(async move { api.packages(&p2).await });
            self.cart_names = Some((pkgs.clone(), slot));
        }
        let names = self.cart_names.as_ref().and_then(|(_, s)| snapshot(s)).and_then(|r| r.ok()).unwrap_or_default();
        let gap = t.dense(17.0);
        let total_w = ui.available_width();
        let right_w = if total_w > t.dense(900.0) { (total_w * 449.0 / 1560.0).clamp(300.0, 449.0) } else { 0.0 };
        let left_w = if right_w > 0.0 { total_w - right_w - gap } else { total_w };
        let in_cart: Vec<AppId> = names.iter().map(|n| n.3).collect();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.vertical(|ui| {
                ui.set_width(left_w);
                ui.spacing_mut().item_spacing.y = t.dense(8.0);
                if lines.is_empty() {
                    w::empty(ui, t, icons::CART, t!("Your cart is empty"), t!("Add games from the store; it's the same cart as on Steam's website."));
                    return;
                }
                let mut acts = Vec::new();
                for l in &lines {
                    let (name, art, app) = names.iter().find(|n| n.0 == l.package_id).map(|n| (n.1.clone(), n.2.clone(), n.3)).unwrap_or_else(|| (format!("#{}", l.package_id), String::new(), 0));
                    let owned = app != 0 && self.owned.contains(&app);
                    let same: Vec<model::CartLine> = lines.iter().filter(|x| x.package_id == l.package_id).cloned().collect();
                    let (act, open) = cart_row(ui, t, ctx, l, &same, &name, &art, None, owned);
                    if let Some(a) = act {
                        acts.push((a, app, l.package_id, name.clone()));
                    }
                    if open && app != 0 {
                        self.open(View::App(app));
                    }
                }
                for (a, app, pkg, name) in acts {
                    self.cart_act(ctx, a, app, pkg, &name);
                }
                self.cart_total(ui, t, ctx, &lines);
            });
            if right_w > 0.0 {
                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    self.cart_suggestions(ui, t, ctx, &in_cart);
                });
            }
        });
    }

    /// "Estimated total", Remove all, Continue Shopping and the purchase buttons.
    fn cart_total(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, lines: &[model::CartLine]) {
        super::store_page::block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(w::heading_s(t, t!("Estimated total"), t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = t.dense(40.0);
                    let all = w::icon_box(ui, t, icons::TRASH, t!("Remove everything from the cart"), false);
                    egui::Popup::from_toggle_button_response(&all).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(260.0).show(|ui| {
                        ui.label(w::body_m(t, t!("Remove all {n} items from your cart?", n = lines.len()), t.text));
                        ui.add_space(t.spacing.sm);
                        if w::button(ui, t, Some(icons::TRASH), t!("Remove all"), w::Kind::Danger, w::Size::Sm).clicked() {
                            for l in lines {
                                ctx.send(Command::CartRemove { line_item_id: l.line_item_id });
                            }
                            ui.close();
                        }
                    });
                    if let Some(total) = sum_prices(lines.iter().map(|l| l.price.as_str())) {
                        ui.label(w::styled(total, t.typography.price, t.price));
                    }
                });
            });
            ui.add_space(t.dense(70.0));
            ui.horizontal(|ui| {
                if w::button(ui, t, None, t!("Continue shopping"), w::Kind::Secondary, w::Size::Lg).clicked() {
                    self.open(View::Home);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = t.dense(9.0);
                    let tip = t!("Steam only takes payment on its website, so checkout opens in your browser.");
                    // Gifts are picked per copy above; Steam takes the payment on its own page.
                    if w::button(ui, t, None, t!("Checkout"), w::Kind::Primary, w::Size::Lg).on_hover_text(tip).clicked() {
                        crate::links::open_http("https://store.steampowered.com/cart");
                    }
                });
            });
            ui.add_space(t.spacing.sm);
            ui.label(w::body_s(t, t!("Payment happens on Steam's checkout page. Your cart is already there."), t.text_faint));
        });
    }

    /// "Items you might be interested in": games on sale you don't have.
    fn cart_suggestions(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, in_cart: &[AppId]) {
        let Some(Ok((_, sections))) = self.front_data() else { return };
        let picks: Vec<store::Item> = sections
            .iter()
            .filter(|s| s.id == "specials" || s.id == "top_sellers")
            .flat_map(|s| s.items.iter())
            .filter(|i| !self.owned.contains(&i.app_id) && !in_cart.contains(&i.app_id) && self.safe(i.app_id) == Some(true))
            .take(2)
            .cloned()
            .collect();
        if picks.is_empty() {
            return;
        }
        ui.label(w::heading_s(t, t!("Items you might be interested in"), t.text));
        ui.add_space(t.dense(6.0));
        let wishlist = ctx.state.wishlist.clone().unwrap_or_default();
        let width = ui.available_width();
        for item in &picks {
            let wished = wishlist.iter().any(|(a, _)| *a == item.app_id);
            let out = super::home::store_card(ui, t, item, width, false, wished, ctx.state.steam_id != 0);
            self.micro(ui, item.app_id, out.img, t.corner_small(), out.hovered);
            if out.wish {
                ctx.send(if wished { Command::WishlistRemove { app_id: item.app_id } } else { Command::WishlistAdd { app_id: item.app_id } });
            } else if out.open {
                self.open(View::App(item.app_id));
            }
            ui.add_space(t.dense(10.0));
        }
    }

    // ---------- purchase box helpers ----------

    /// The buy box's button (Figma "main-cta" `426:4355`): the price and
    /// "Add to Cart" ("In Cart" once there; it then offers another copy or a
    /// gift). Gifts are chosen in the cart.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn purchase_actions(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, package: Option<u32>, name: &str, price: Option<(String, Option<String>, u8)>, owned: bool) {
        let Some(pkg) = package else { return };
        let copies = ctx.state.cart.as_ref().map(|c| c.iter().filter(|l| l.package_id == pkg).count()).unwrap_or(0);
        let (label, original, discount) = price.clone().unwrap_or_default();
        let button = if copies > 1 {
            t!("In cart ({n})", n = copies).to_string()
        } else if copies == 1 {
            t!("In cart").to_string()
        } else if owned {
            t!("Buy as a gift").to_string()
        } else {
            t!("Add to cart").to_string()
        };
        let r = w::main_cta_rtl(ui, t, &label, original.as_deref(), discount, &button);
        if copies > 0 {
            if r.on_hover_text(t!("Add another copy, or make one a gift")).clicked() {
                self.cart_popup = Some(CartPopup { app: id, package: pkg, name: name.to_string(), price, owned, waiting: false });
            }
        } else if r.clicked() {
            self.add_to_cart(ctx, id, pkg, name, price, owned);
        }
    }

    /// Add a copy like Steam does: for this account when it can be, else as
    /// a gift (already owned, or a copy for the account is in the cart).
    pub(super) fn add_to_cart(&mut self, ctx: &mut Ctx, app: AppId, package: u32, name: &str, price: Option<(String, Option<String>, u8)>, owned: bool) {
        let mine_in_cart = ctx.state.cart.as_ref().is_some_and(|c| c.iter().any(|l| l.package_id == package && !l.is_gift));
        ctx.send(Command::CartAdd { package_id: package, gift: owned || mine_in_cart, gift_to: None });
        self.cart_popup = Some(CartPopup { app, package, name: name.to_string(), price, owned, waiting: true });
    }

    /// Steam's "Added to your cart!" window: every copy of this game in the
    /// cart, each for the account or a gift, with Add and Remove.
    pub(super) fn cart_popup_ui(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(pop) = self.cart_popup.clone() else { return };
        let all = ctx.state.cart.clone().unwrap_or_default();
        let lines: Vec<model::CartLine> = all.iter().filter(|l| l.package_id == pop.package).cloned().collect();
        // Every copy removed: nothing left to show.
        if lines.is_empty() && !pop.waiting {
            self.cart_popup = None;
            return;
        }
        if !lines.is_empty() && pop.waiting {
            if let Some(p) = &mut self.cart_popup {
                p.waiting = false;
            }
        }
        let mut close = false;
        let mut acts = Vec::new();
        // Centered, and never wider or taller than the window allows.
        let screen = ui.ctx().content_rect();
        let width = (screen.width() - 2.0 * t.dense(48.0)).clamp(320.0, t.dense(720.0));
        let list_h = (screen.height() - t.dense(300.0)).clamp(120.0, t.dense(420.0));
        egui::Modal::new(egui::Id::unique("cart-popup")).show(ui.ctx(), |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(w::heading_m(t, t!("Added to your cart!"), t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            if lines.is_empty() {
                w::loading(ui, t, t!("Adding…"));
            }
            egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = t.dense(8.0);
                for l in &lines {
                    let art = model::art::header(pop.app);
                    let (act, open) = cart_row(ui, t, ctx, l, &lines, &pop.name, &art, pop.price.as_ref(), pop.owned);
                    if let Some(a) = act {
                        acts.push(a);
                    }
                    if open {
                        close = true;
                        self.open(View::App(pop.app));
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if w::button(ui, t, None, t!("Continue shopping"), w::Kind::Secondary, w::Size::Md).clicked() {
                    close = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::button(ui, t, Some(icons::CART), &t!("View my cart ({n})", n = all.len()), w::Kind::Primary, w::Size::Md).clicked() {
                        close = true;
                        self.open(View::Cart);
                    }
                });
            });
        });
        for a in acts {
            self.cart_act(ctx, a, pop.app, pop.package, &pop.name);
        }
        if close {
            self.cart_popup = None;
        }
    }

    /// Carry out what a cart row asked for.
    pub(super) fn cart_act(&mut self, ctx: &mut Ctx, act: CartAct, app: AppId, package: u32, name: &str) {
        match act {
            CartAct::SetGift(line, gift) => ctx.send(Command::CartModify { line_item_id: line, gift, gift_to: None }),
            // Another copy: always a gift (one copy per account).
            CartAct::AddCopy => ctx.send(Command::CartAdd { package_id: package, gift: true, gift_to: None }),
            CartAct::Remove(line) => ctx.send(Command::CartRemove { line_item_id: line }),
            CartAct::ChooseFriend(line, to) => {
                self.gift = Some(Gift { app, package, name: name.to_string(), to, line: Some(line), search: String::new(), checks: Default::default() });
            }
        }
    }

    pub(super) fn gift_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        // Start the region check for the picked friend (once per friend).
        let pending = self.gift.as_ref().and_then(|g| g.to.filter(|to| !g.checks.contains_key(to)).map(|to| (g.app, to)));
        if let Some((app, to)) = pending {
            let (api, cc) = (self.api.clone(), self.locale.0.clone());
            let slot = self.fetch(async move { api.gift_check(app, &cc, to).await });
            if let Some(g) = &mut self.gift {
                g.checks.insert(to, slot);
            }
        }
        let Some(g) = &mut self.gift else { return };
        let mut close = false;
        let mut send = None;
        egui::Modal::new(egui::Id::unique(("gift", g.package))).show(ui.ctx(), |ui| {
            ui.set_width(440.0);
            ui.label(w::text_bold(t!("Buy {name} as a gift", name = g.name), 19.0, t.text));
            ui.label(w::text(t!("Pick a friend. The gift goes into your cart and is delivered after checkout."), 13.0, t.text_dim));
            ui.add_space(8.0);
            w::search(ui, t, &mut g.search, &t!("Search friends"), ui.available_width());
            ui.add_space(6.0);
            // Online friends first, then by name; avatar and status on each row.
            let q = g.search.trim().to_lowercase();
            let mut friends: Vec<_> = ctx.state.friend_list().into_iter().filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q)).collect();
            friends.sort_by_key(|p| (p.state == model::PersonaState::Offline, !p.in_game(), p.name.to_lowercase()));
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for p in &friends {
                    let sel = g.to == Some(p.steam_id);
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                    let hover = ui.ctx().animate_bool_with_time(r.id, r.hovered(), 0.1);
                    let fill = if sel { t.accent.gamma_multiply(0.22) } else { t.surface_hover.gamma_multiply(hover) };
                    ui.painter().rect_filled(rect, t.corner_small(), fill);
                    if sel {
                        ui.painter().rect_stroke(rect, t.corner_small(), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                    }
                    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 5.0))).layout(Layout::left_to_right(Align::Center)));
                    w::avatar(&mut row, t, &model::art::avatar(p.avatar_hash.as_deref()), 34.0, Some(crate::shell::status_color(t, p)));
                    row.add_space(4.0);
                    row.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        // A friend Steam hasn't sent a name for yet shows their SteamID.
                        let unnamed = p.name.is_empty() || p.name.chars().all(|c| c.is_ascii_digit());
                        let name = if unnamed { t!("Friend {id}", id = p.steam_id) } else { p.name.clone() };
                        ui.add(egui::Label::new(w::text_medium(name, 14.0, if unnamed { t.text_dim } else { t.text })).truncate());
                        ui.add(egui::Label::new(w::text(crate::shell::status_text(ctx.state, p), 12.0, crate::shell::status_color(t, p))).truncate());
                    });
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        g.to = Some(p.steam_id);
                    }
                }
                if friends.is_empty() {
                    ui.label(w::text(t!("No friends match."), 13.0, t.text_faint));
                }
            });
            ui.add_space(8.0);
            // Can Steam deliver it to them? (Regional price rule.)
            if let Some(to) = g.to {
                let name = ctx.state.persona(to).name;
                let line = match g.checks.get(&to).and_then(snapshot) {
                    None => (icons::CLOCK, t!("Checking whether Steam allows this gift in {name}'s region…", name = name), t.text_dim),
                    Some(Err(_)) => (icons::INFO, t!("Couldn't check {name}'s region. Steam decides at checkout.", name = name), t.text_dim),
                    Some(Ok(c)) => gift_line(t, &c, &name),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(w::text(line.0, 15.0, line.2));
                    ui.add(egui::Label::new(w::text(line.1, 13.0, line.2)).wrap());
                });
                ui.label(w::text(t!("Steam blocks gifts when the game is more than about 10% cheaper in the friend's region (base prices, converted)."), 11.5, t.text_faint));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = if g.line.is_some() { t!("Give it to them") } else { t!("Add gift to cart") };
                if ui.add_enabled_ui(g.to.is_some(), |ui| w::button(ui, t, Some(icons::CART), ok, w::Kind::Primary, w::Size::Md)).inner.clicked() {
                    send = g.to.map(|to| (g.package, to));
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked() {
                    close = true;
                }
            });
        });
        if let Some((package, to)) = send {
            match self.gift.as_ref().and_then(|g| g.line) {
                Some(line) => ctx.send(Command::CartModify { line_item_id: line, gift: true, gift_to: Some(to) }),
                None => ctx.send(Command::CartAdd { package_id: package, gift: true, gift_to: Some(to) }),
            }
            close = true;
        }
        if close {
            self.gift = None;
        }
    }

    // ---------- price history ----------

    /// Remember the prices we see, so there's a history even without IsThereAnyDeal.
    pub(super) fn log_price(&mut self, id: AppId, cents: u64) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let entry = self.price_log.entry(id).or_default();
        if entry.last().is_none_or(|(_, c)| *c != cents) {
            entry.push((now, cents));
            self.price_log_dirty = true;
        }
    }

    /// The price history block: only with an IsThereAnyDeal key (Settings
    /// → Store preferences); without one the page doesn't show it.
    pub(super) fn price_history(&mut self, ui: &mut Ui, t: &Theme, id: AppId) {
        if self.itad_key.trim().is_empty() {
            return;
        }
        super::store_page::block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::heading_s(t, t!("Price history"), t.text_dim));
            ui.add_space(t.spacing.sm);
            if !self.history.contains_key(&id) {
                let (api, key) = (self.api.clone(), self.itad_key.clone());
                let slot = self.fetch(async move { api.price_history(&key, id).await });
                self.history.insert(id, slot);
            }
            match snapshot(&self.history[&id]) {
                None => {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 160.0), Sense::hover());
                    w::skeleton(ui, t, r, t.corner_small());
                }
                Some(Err(e)) => {
                    ui.label(w::text(t!("Price history isn't available: {error}", error = e), 13.0, t.text_faint));
                }
                Some(Ok(h)) => {
                    if let Some((low, when)) = h.low {
                        ui.label(w::text_semibold(t!("All-time low on Steam: {price} ({date})", price = format!("{low:.2} {}", h.currency), date = ui::date(when)), 14.0, t.play));
                    }
                    let pts: Vec<(u64, f64)> = h.points.iter().map(|p| (p.time, p.price)).collect();
                    if pts.len() >= 2 {
                        chart(ui, t, &pts, &h.currency);
                    }
                    ui.horizontal(|ui| {
                        ui.label(w::text(t!("Price data:"), 12.0, t.text_faint));
                        if ui.link(w::text("IsThereAnyDeal", 12.0, t.accent)).clicked() {
                            crate::links::open_http(h.url.as_deref().unwrap_or("https://isthereanydeal.com"));
                        }
                    });
                }
            }
        });
    }
}

/// A step chart of price over time.
fn chart(ui: &mut Ui, t: &Theme, pts: &[(u64, f64)], currency: &str) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 170.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, t.corner_small(), t.raised);
    let area = rect.shrink2(vec2(44.0, 16.0));
    let (t0, t1) = (pts.first().unwrap().0, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).max(pts.last().unwrap().0 + 1));
    let max = pts.iter().map(|x| x.1).fold(0.0, f64::max).max(0.01) * 1.1;
    let x = |time: u64| area.left() + area.width() * ((time - t0) as f32 / (t1 - t0) as f32);
    let y = |v: f64| area.bottom() - area.height() * (v / max) as f32;
    for i in 0..=3 {
        let v = max * i as f64 / 3.0;
        p.hline(area.x_range(), y(v), Stroke::new(1.0, t.border));
        p.text(pos2(rect.left() + 6.0, y(v)), Align2::LEFT_CENTER, format!("{v:.0}"), w::font(10.5), t.text_faint);
    }
    let mut line: Vec<Pos2> = Vec::new();
    for (i, (time, v)) in pts.iter().enumerate() {
        let next_t = pts.get(i + 1).map(|n| n.0).unwrap_or(t1);
        line.push(pos2(x(*time), y(*v)));
        line.push(pos2(x(next_t), y(*v)));
    }
    p.add(egui::Shape::line(line, Stroke::new(2.0, t.accent)));
    if let Some(hover) = resp.hover_pos() {
        let time = t0 + ((hover.x - area.left()) / area.width() * (t1 - t0) as f32).max(0.0) as u64;
        if let Some((_, v)) = pts.iter().rev().find(|(pt, _)| *pt <= time) {
            p.vline(hover.x, area.y_range(), Stroke::new(1.0, t.text_faint));
            p.text(pos2(hover.x + 6.0, area.top()), Align2::LEFT_TOP, format!("{v:.2} {currency}  ·  {}", ui::date(time)), w::font_semibold(12.0), t.text);
        }
    }
    let _ = Color32::TRANSPARENT;
}

/// The region check, in words: (icon, text, color).
fn gift_line(t: &Theme, c: &store::gift::GiftCheck, name: &str) -> (&'static str, String, egui::Color32) {
    use store::gift::Verdict;
    let price = |p: &Option<(u64, String)>| p.as_ref().map(|(cents, cur)| store::Price::format(*cents, cur)).unwrap_or_default();
    match &c.verdict {
        Verdict::Ok => {
            let diff = c.difference.map(|d| format!("{d:+.0}%")).unwrap_or_default();
            (icons::CHECK, t!("{name} can receive it: {theirs} there vs {yours} for you ({diff}).", name = name, theirs = price(&c.theirs), yours = price(&c.yours), diff = diff), t.play)
        }
        Verdict::Blocked => {
            let diff = c.difference.map(|d| format!("{:.0}%", -d)).unwrap_or_default();
            (icons::WARNING, t!("Steam will most likely refuse: it's {diff} cheaper in {name}'s region ({theirs} vs {yours}). A Steam gift card works instead.", diff = diff, name = name, theirs = price(&c.theirs), yours = price(&c.yours)), t.danger)
        }
        Verdict::Unknown(why) if why == "no country" => (icons::INFO, t!("{name} hasn't set a country on their profile, so the region can't be checked.", name = name), t.text_dim),
        Verdict::Unknown(_) => (icons::INFO, t!("Can't compare prices for {name}'s region. Steam decides at checkout.", name = name), t.text_dim),
    }
}

/// The "Added to your cart!" popup for one game.
#[derive(Clone)]
pub struct CartPopup {
    pub app: AppId,
    pub package: u32,
    pub name: String,
    /// (price, original price, discount %) as shown on the game page.
    pub price: Option<(String, Option<String>, u8)>,
    pub owned: bool,
    /// Just added: Steam hasn't sent the cart back yet.
    pub waiting: bool,
}

/// What a cart row's controls asked for.
pub enum CartAct {
    /// "For my account" (false) or "This is a gift" (true).
    SetGift(u64, bool),
    /// Another copy of the same game.
    AddCopy,
    Remove(u64),
    /// Pick (or change) the friend a gift line is for.
    ChooseFriend(u64, Option<SteamId>),
}

/// One cart line (Figma "cart" `397:11087`'s rows), laid out in fixed
/// areas so nothing runs into anything at any width: the picture; the name
/// on its own line with "For my account" / "This is a gift" under it (and,
/// for a gift, who it's for on the line after); then a column on the right
/// with room kept for the price, Add another copy and Remove. `same` = every
/// line of this package. Returns what was asked for, and whether the game
/// was clicked.
#[allow(clippy::too_many_arguments)]
fn cart_row(ui: &mut Ui, t: &Theme, ctx: &Ctx, l: &model::CartLine, same: &[model::CartLine], name: &str, art: &str, price: Option<&(String, Option<String>, u8)>, owned: bool) -> (Option<CartAct>, bool) {
    let mut act = None;
    let mut open = false;
    // One copy per account: "For my account" only while no other copy is
    // for the account, and not for a game already owned.
    let mine_elsewhere = same.iter().any(|x| x.line_item_id != l.line_item_id && !x.is_gift);
    let can_be_mine = !owned && !mine_elsewhere;
    let c = &t.components.store_card;
    let pad = t.dense(13.0);
    let gap = t.dense(16.0);
    let width = ui.available_width();
    let narrow = width < t.dense(720.0);
    let art_size = if narrow { vec2(t.dense(120.0), t.dense(56.0)) } else { vec2(t.dense(160.0), t.dense(75.0)) };
    // The middle column: name, the dropdown, and a gift's recipient.
    let name_h = (t.typography.heading_s.size * 1.35).round();
    let drop_h = t.height_of(&t.components.sort, 39.0);
    let line_h = (t.typography.body_s.size * 1.5).round();
    let spacing = t.dense(6.0);
    let mid_h = name_h + spacing + drop_h + if l.is_gift { spacing + line_h } else { 0.0 };
    let h = pad * 2.0 + art_size.y.max(mid_h);
    let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return (None, false);
    }
    w::paint_box(ui, t, rect, c, &t.look(c, Default::default()), 0.0);
    let inner = rect.shrink(pad);
    // The picture, opening the game's page.
    let art_r = Rect::from_min_size(pos2(inner.left(), inner.center().y - art_size.y / 2.0), art_size);
    let art_resp = ui.interact(art_r, ui.scope_id().with(("cart-art", l.line_item_id)), Sense::click());
    w::image_or_skeleton(ui, t, art_r, art, t.corner_small());
    hover_wash(ui, t, art_r, &art_resp);
    open |= art_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    // The right column: price, then Add and Remove, with room kept for them.
    let icon_w = t.height_of(&t.components.icon_box, 34.0);
    let right_w = (t.dense(150.0) + 2.0 * icon_w + t.dense(24.0)).min(inner.width() * 0.45);
    let right = Rect::from_min_max(pos2(inner.right() - right_w, inner.top()), inner.max);
    let mut r = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)));
    r.spacing_mut().item_spacing.x = t.dense(8.0);
    if w::icon_box(&mut r, t, icons::TRASH, t!("Remove"), false).clicked() {
        act = Some(CartAct::Remove(l.line_item_id));
    }
    if w::icon_box(&mut r, t, icons::PLUS, t!("Another copy, as a gift"), false).clicked() {
        act = Some(CartAct::AddCopy);
    }
    r.add_space(t.dense(8.0));
    match price {
        Some((label, original, discount)) => w::price_tag(&mut r, t, label, original.as_deref(), *discount),
        None => {
            r.label(w::styled(&l.price, t.typography.price, t.price));
        }
    }
    // The middle: everything stays inside its own area.
    let mid = Rect::from_min_max(pos2(art_r.right() + gap, inner.top()), pos2(right.left() - gap, inner.bottom()));
    let mid = Rect::from_center_size(pos2(mid.center().x, inner.center().y), vec2(mid.width().max(10.0), mid_h.min(inner.height())));
    let mut m = ui.new_child(egui::UiBuilder::new().max_rect(mid).layout(Layout::top_down(Align::Min)));
    m.set_clip_rect(mid.expand(2.0).intersect(ui.clip_rect()));
    m.spacing_mut().item_spacing.y = spacing;
    let name_r = m.add(egui::Label::new(w::heading_s(t, name, t.text)).truncate().sense(Sense::click()).selectable(false));
    if name_r.hovered() {
        m.painter().hline(name_r.rect.x_range(), name_r.rect.bottom(), Stroke::new(1.0, t.text));
    }
    open |= name_r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    let current = if l.is_gift { t!("This is a gift") } else { t!("For my account") };
    let dd = w::dropdown(&mut m, t, current, t.dense(150.0).min(mid.width()));
    egui::Popup::menu(&dd).show(|ui| {
        let mine = ui.add_enabled_ui(can_be_mine || !l.is_gift, |ui| w::menu_item(ui, t, if l.is_gift { "" } else { icons::CHECK }, t!("For my account"))).inner;
        let mine = if owned { mine.on_disabled_hover_text(t!("You already own this game")) } else { mine.on_disabled_hover_text(t!("Another copy is already for your account")) };
        if mine.clicked() && l.is_gift {
            act = Some(CartAct::SetGift(l.line_item_id, false));
        }
        if w::menu_item(ui, t, if l.is_gift { icons::CHECK } else { "" }, t!("This is a gift")).clicked() && !l.is_gift {
            act = Some(CartAct::SetGift(l.line_item_id, true));
        }
    });
    if l.is_gift {
        let base = ctx.state.steam_id & 0xFFFF_FFFF_0000_0000;
        let to = l.gift_to.map(|acc| base | acc as u64);
        let (text, color, tip) = match to {
            Some(id) => (format!("{}  {} · {}", icons::USER, t!("For {name}", name = ctx.state.persona(id).name), t!("Change")), t.accent, None),
            None => (format!("{}  {}", icons::USER, t!("Choose a friend…")), t.accent, Some(t!("Or pick who gets it on Steam's checkout page"))),
        };
        let r = m.add(egui::Label::new(w::body_s(t, text, color)).truncate().sense(Sense::click()).selectable(false));
        if r.hovered() {
            m.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, color));
        }
        let r = match tip {
            Some(tip) => r.on_hover_text(tip),
            None => r,
        };
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            act = Some(CartAct::ChooseFriend(l.line_item_id, to));
        }
    }
    (act, open)
}

/// A light wash over a picture while the pointer is on it (clickable art).
pub(super) fn hover_wash(ui: &Ui, t: &Theme, rect: Rect, resp: &egui::Response) {
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("wash"), resp.hovered(), 0.12);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, t.corner_small(), t.overlay_light(0).gamma_multiply(hover));
    }
}

/// Add up prices as Steam wrote them ("$14.99", "1 299,00₽"…), in the
/// first one's format. `None` if any can't be read.
pub(super) fn sum_prices<'a>(prices: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut total: u64 = 0;
    let mut first: Option<(String, String, Option<char>, Option<char>)> = None;
    for p in prices {
        // The number: from the first digit to the last.
        let start = p.find(|c: char| c.is_ascii_digit())?;
        let end = p.rfind(|c: char| c.is_ascii_digit())? + 1;
        let num = &p[start..end];
        // A decimal mark is a '.' or ',' with exactly two digits after it.
        let dec = num.char_indices().rev().find(|(_, c)| *c == '.' || *c == ',').filter(|(i, _)| num.len() - i - 1 == 2).map(|(_, c)| c);
        let digits: String = num.chars().filter(|c| c.is_ascii_digit()).collect();
        let v: u64 = digits.parse().ok()?;
        total += if dec.is_some() { v } else { v * 100 };
        if first.is_none() {
            let group = num.chars().find(|c| !c.is_ascii_digit() && Some(*c) != dec);
            first = Some((p[..start].to_string(), p[end..].to_string(), dec, group));
        }
    }
    let (pre, post, dec, group) = first?;
    let whole = total / 100;
    let mut w = whole.to_string();
    if let Some(g) = group {
        let s = w.clone();
        w.clear();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                w.push(g);
            }
            w.push(c);
        }
    }
    let n = match dec {
        Some(d) => format!("{w}{d}{:02}", total % 100),
        None => w,
    };
    Some(format!("{pre}{n}{post}"))
}

#[cfg(test)]
mod tests {
    use super::sum_prices;

    #[test]
    fn adds_up_prices() {
        assert_eq!(sum_prices(["$14.99", "$59.99"].into_iter()).as_deref(), Some("$74.98"));
        assert_eq!(sum_prices(["1 299,00₽", "1,00₽"].into_iter()).as_deref(), Some("1 300,00₽"));
        assert_eq!(sum_prices(["$1,014.99", "$5.00"].into_iter()).as_deref(), Some("$1,019.99"));
        assert_eq!(sum_prices(["4 480₸", "20₸"].into_iter()).as_deref(), Some("4 500₸"));
        assert_eq!(sum_prices(["Free"].into_iter()), None);
    }
}

/// "Ends in 5 hours" when close, else "Offer ends 21 Nov"; `None` without
/// an end date (or once it's over).
pub(super) fn discount_ends_text(ends: u64) -> Option<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let left = ends.checked_sub(now).filter(|l| *l > 0 && ends > 0)?;
    Some(match left {
        0..3600 => t!("Ends in {n} min", n = (left / 60).max(1)),
        3600..172_800 => t!("Ends in {n} hours", n = left / 3600),
        _ => t!("Offer ends {date}", date = ui::date(ends)),
    })
}

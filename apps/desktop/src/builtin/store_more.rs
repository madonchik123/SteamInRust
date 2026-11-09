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

impl Store {
    // ---------- browse ----------

    pub(super) fn browse_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let mut f = self.filters.clone();
        f.hidden_descriptors = self.hidden.iter().copied().collect();
        if self.query.as_ref().is_none_or(|(key, page, _)| *key != f || *page != self.browse_page) {
            let (api, ff, start) = (self.api.clone(), f.clone(), self.browse_page * 48);
            let slot = self.fetch(async move { api.query(&ff, start, 48).await });
            self.query = Some((f.clone(), self.browse_page, slot));
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            ui.vertical(|ui| {
                ui.set_width(250.0);
                self.filter_panel(ui, t);
            });
            ui.vertical(|ui| {
                let Some((_, _, slot)) = &self.query else { return };
                match snapshot(slot) {
                    None => w::loading(ui, t, t!("Searching…")),
                    Some(Err(e)) => w::empty(ui, t, icons::WARNING, t!("Search failed"), &e),
                    Some(Ok((items, total))) => {
                        let owned = |id: &AppId| ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(id));
                        let items: Vec<QueryItem> = items.into_iter().filter(|i| !self.hide_owned || !owned(&i.app_id)).collect();
                        ui.label(w::text(t!("{n} results", n = total), 13.5, t.text_dim));
                        ui.add_space(8.0);
                        for it in &items {
                            if self.result_row(ui, t, ctx, it) {
                                self.open(View::App(it.app_id));
                            }
                        }
                        let pages = total.div_ceil(48).max(1) as u32;
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(self.browse_page > 0, |ui| {
                                if w::button(ui, t, Some(icons::BACK), t!("Previous"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                    self.browse_page -= 1;
                                }
                            });
                            ui.label(w::text(t!("Page {page} of {pages}", page = self.browse_page + 1, pages = pages), 13.0, t.text_dim));
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

    fn filter_panel(&mut self, ui: &mut Ui, t: &Theme) {
        let before = self.filters.clone();
        let f = &mut self.filters;
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::text_bold(t!("SORT BY"), 11.0, t.text_faint));
            egui::ComboBox::from_id_salt("browse-sort").selected_text(i18n::tr_dyn(f.sort.label())).width(200.0).show_ui(ui, |ui| {
                for s in Sort::ALL {
                    ui.selectable_value(&mut f.sort, s, i18n::tr_dyn(s.label()));
                }
            });
            ui.add_space(10.0);
            ui.label(w::text_bold(t!("PRICE"), 11.0, t.text_faint));
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
                if w::radio(ui, t, on, i18n::tr_dyn(label)).clicked() {
                    f.max_price = max;
                    f.only_free = free;
                }
            }
            w::check(ui, t, &mut f.only_discounted, t!("Special offers only"));
            ui.add_space(10.0);
            ui.label(w::text_bold(t!("REVIEWS"), 11.0, t.text_faint));
            for (min, label) in [(None, "Any reviews"), (Some(7), "Mostly positive and up"), (Some(8), "Very positive and up")] {
                if w::radio(ui, t, f.min_review_score == min, i18n::tr_dyn(label)).clicked() {
                    f.min_review_score = min;
                }
            }
            ui.add_space(10.0);
            ui.label(w::text_bold(t!("TYPE"), 11.0, t.text_faint));
            w::check(ui, t, &mut f.games, t!("Games"));
            w::check(ui, t, &mut f.dlc, t!("DLC"));
            w::check(ui, t, &mut f.software, t!("Software"));
            w::check(ui, t, &mut f.demos, t!("Demos"));
            w::check(ui, t, &mut f.soundtracks, t!("Soundtracks"));
            w::check(ui, t, &mut f.coming_soon, t!("Coming soon only"));
        });
        ui.add_space(10.0);
        let mut hide_owned = self.hide_owned;
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            w::check(ui, t, &mut hide_owned, t!("Hide games I own"));
        });
        self.hide_owned = hide_owned;
        ui.add_space(10.0);
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::text_bold(t!("TAGS"), 11.0, t.text_faint));
            // Tags picked elsewhere (a game page, the search below) first,
            // so every active tag is visible and can be turned off.
            let extra: Vec<u32> = self.filters.tags.iter().copied().filter(|t| !TAGS.contains(t)).collect();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for tag in extra.iter().chain(TAGS.iter()) {
                    let name = self.tag_name(*tag).unwrap_or_else(|| format!("#{tag}"));
                    let on = self.filters.tags.contains(tag);
                    if w::chip(ui, t, &name, on).clicked() {
                        if on {
                            self.filters.tags.retain(|x| x != tag);
                        } else {
                            self.filters.tags.push(*tag);
                        }
                    }
                }
            });
            // Any of Steam's tags, by name.
            ui.add_space(8.0);
            let width = ui.available_width();
            w::search(ui, t, &mut self.tag_query, t!("Find a tag…"), width);
            let query = self.tag_query.trim().to_string();
            if query.chars().count() >= 2 {
                let found: Vec<(u32, String)> = self.find_tags(&query).into_iter().filter(|(id, _)| !TAGS.contains(id) && !self.filters.tags.contains(id)).take(12).collect();
                ui.add_space(6.0);
                if found.is_empty() {
                    ui.label(w::text(t!("No tags match."), 12.5, t.text_faint));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for (id, name) in found {
                        if w::chip(ui, t, &format!("{}  {name}", icons::PLUS), false).clicked() {
                            self.filters.tags.push(id);
                            self.tag_query.clear();
                        }
                    }
                });
            }
            if !self.filters.tags.is_empty() && ui.link(w::text(t!("Clear tags"), 12.5, t.text_dim)).clicked() {
                self.filters.tags.clear();
            }
        });
        if self.filters != before {
            self.browse_page = 0;
        }
    }

    /// A wide result row; returns true when clicked.
    fn result_row(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, it: &QueryItem) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 74.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
        ui.painter().rect_filled(rect, t.corner_small(), w::mix(t.surface, t.surface_hover, hover));
        let img = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(124.0, 58.0));
        w::image_or_skeleton(ui, t, img, &it.header, t.corner_small());
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(img.right() + 14.0, rect.top()), rect.max - vec2(14.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        c.vertical(|ui| {
            ui.add_space(10.0);
            ui.add(egui::Label::new(w::text_semibold(&it.name, 15.0, t.text)).truncate());
            ui.horizontal(|ui| {
                let (win, mac, lin) = it.platforms;
                let p: String = [(win, icons::WINDOWS), (mac, icons::APPLE), (lin, icons::LINUX)].iter().filter(|x| x.0).map(|x| x.1).collect::<Vec<_>>().join(" ");
                ui.label(w::text(p, 12.5, t.text_faint));
                if let Some(l) = &it.review_label {
                    ui.label(w::text(format!("{}  {}%", i18n::tr_dyn(l), it.review_percent), 12.5, super::review_color(t, it.review_score)));
                }
                if ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&it.app_id)) {
                    w::pill(ui, t!("In library"), t.play, t.play.gamma_multiply(0.15));
                }
            });
        });
        c.with_layout(Layout::right_to_left(Align::Center), |ui| {
            match &it.formatted_price {
                Some(p) => w::price_tag(ui, t, p, None, it.price.discount),
                None => {
                    ui.label(w::text_semibold(t!("Free"), 14.0, t.text));
                }
            }
        });
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }

    // ---------- wishlist ----------

    pub(super) fn wishlist_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        w::page_title(ui, t, t!("Your wishlist"), None);
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
        let Some(Ok(items)) = self.wish_items.as_ref().and_then(|(_, s)| snapshot(s)) else {
            w::loading(ui, t, t!("Loading…"));
            return;
        };
        let on_sale = items.iter().filter(|i| i.price.discount > 0).count();
        ui.label(w::text(t!("{n} games · {sale} on sale now", n = items.len(), sale = on_sale), 13.5, t.text_dim));
        ui.add_space(8.0);
        let mut sorted = items.clone();
        sorted.sort_by_key(|i| std::cmp::Reverse(i.price.discount));
        for it in &sorted {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - 150.0);
                    if self.result_row(ui, t, ctx, it) {
                        self.open(View::App(it.app_id));
                    }
                });
                ui.vertical(|ui| {
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        if let Some(pkg) = it.package_id {
                            if w::icon_button(ui, t, icons::CART, t!("Add to cart"), false, 0).clicked() {
                                let owned = self.owned.contains(&it.app_id);
                                self.add_to_cart(ctx, it.app_id, pkg, &it.name, None, owned);
                            }
                        }
                        if w::icon_button(ui, t, icons::TRASH, t!("Remove from wishlist"), false, 0).clicked() {
                            ctx.send(Command::WishlistRemove { app_id: it.app_id });
                        }
                    });
                });
            });
            ui.add_space(4.0);
        }
    }

    // ---------- cart ----------

    pub(super) fn cart_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        w::page_title(ui, t, t!("Your cart"), None);
        let Some(lines) = ctx.state.cart.clone() else {
            w::loading(ui, t, t!("Loading…"));
            return;
        };
        if lines.is_empty() {
            w::empty(ui, t, icons::CART, t!("Your cart is empty"), t!("Add games from the store; it's the same cart as on Steam's website."));
            return;
        }
        let pkgs: Vec<u32> = lines.iter().map(|l| l.package_id).collect();
        if self.cart_names.as_ref().is_none_or(|(k, _)| *k != pkgs) {
            let (api, p2) = (self.api.clone(), pkgs.clone());
            let slot = self.fetch(async move { api.packages(&p2).await });
            self.cart_names = Some((pkgs.clone(), slot));
        }
        let names = self.cart_names.as_ref().and_then(|(_, s)| snapshot(s)).and_then(|r| r.ok()).unwrap_or_default();
        let mut acts = Vec::new();
        for l in &lines {
            let (name, art, app) = names.iter().find(|n| n.0 == l.package_id).map(|n| (n.1.clone(), n.2.clone(), n.3)).unwrap_or_else(|| (format!("#{}", l.package_id), String::new(), 0));
            let owned = app != 0 && self.owned.contains(&app);
            let same: Vec<model::CartLine> = lines.iter().filter(|x| x.package_id == l.package_id).cloned().collect();
            if let Some(a) = cart_row(ui, t, ctx, l, &same, &name, &art, None, owned) {
                acts.push((a, app, l.package_id, name.clone()));
            }
            ui.add_space(6.0);
        }
        for (a, app, pkg, name) in acts {
            self.cart_act(ctx, a, app, pkg, &name);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let r = w::button(ui, t, Some(icons::CART), t!("Continue to checkout"), w::Kind::Primary, w::Size::Lg)
                .on_hover_text(t!("Steam only takes payment on its website, so checkout opens in your browser."));
            if r.clicked() {
                let _ = open::that_detached("https://store.steampowered.com/cart");
            }
            ui.label(w::text(t!("Payment happens on Steam's checkout page. Your cart is already there."), 12.5, t.text_faint));
        });
    }

    // ---------- purchase box helpers ----------

    /// The buy box's buttons, like Steam's: a wishlist heart and one
    /// "Add to Cart" (or "In Cart") button. Gifts are chosen in the cart.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn purchase_actions(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, package: Option<u32>, name: &str, price: Option<(String, Option<String>, u8)>, owned: bool) {
        let wished = ctx.state.wishlist.as_ref().is_some_and(|w| w.iter().any(|x| x.0 == id));
        if !owned && w::icon_button(ui, t, icons::HEART, if wished { t!("On wishlist") } else { t!("Add to wishlist") }, wished, 0).clicked() {
            ctx.send(if wished { Command::WishlistRemove { app_id: id } } else { Command::WishlistAdd { app_id: id } });
        }
        let Some(pkg) = package else { return };
        let copies = ctx.state.cart.as_ref().map(|c| c.iter().filter(|l| l.package_id == pkg).count()).unwrap_or(0);
        ui.add_space(6.0);
        if copies > 0 && !owned {
            let label = if copies > 1 { t!("In cart ({n})", n = copies).to_string() } else { t!("In cart").to_string() };
            if w::button(ui, t, Some(icons::CART), &label, w::Kind::Play, w::Size::Lg).on_hover_text(t!("Add another copy, or make one a gift")).clicked() {
                self.cart_popup = Some(CartPopup { app: id, package: pkg, name: name.to_string(), price, owned });
            }
        } else if w::button(ui, t, Some(icons::CART), t!("Add to cart"), w::Kind::Primary, w::Size::Lg).clicked() {
            self.add_to_cart(ctx, id, pkg, name, price, owned);
        }
    }

    /// Add a copy like Steam does: for this account when it can be, else as
    /// a gift (already owned, or a copy for the account is in the cart).
    pub(super) fn add_to_cart(&mut self, ctx: &mut Ctx, app: AppId, package: u32, name: &str, price: Option<(String, Option<String>, u8)>, owned: bool) {
        let mine_in_cart = ctx.state.cart.as_ref().is_some_and(|c| c.iter().any(|l| l.package_id == package && !l.is_gift));
        ctx.send(Command::CartAdd { package_id: package, gift: owned || mine_in_cart, gift_to: None });
        self.cart_popup = Some(CartPopup { app, package, name: name.to_string(), price, owned });
    }

    /// Steam's "Added to your cart!" window: every copy of this game in the
    /// cart, each for the account or a gift, with Add and Remove.
    pub(super) fn cart_popup_ui(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(pop) = self.cart_popup.clone() else { return };
        let all = ctx.state.cart.clone().unwrap_or_default();
        let lines: Vec<model::CartLine> = all.iter().filter(|l| l.package_id == pop.package).cloned().collect();
        let mut close = false;
        let mut acts = Vec::new();
        egui::Modal::new(egui::Id::unique("cart-popup")).show(ui.ctx(), |ui| {
            ui.set_width(620.0);
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("Added to your cart!"), 20.0, t.text));
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
            egui::ScrollArea::vertical().max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
                for l in &lines {
                    let art = model::art::header(pop.app);
                    if let Some(a) = cart_row(ui, t, ctx, l, &lines, &pop.name, &art, pop.price.as_ref(), pop.owned) {
                        acts.push(a);
                    }
                    ui.add_space(6.0);
                }
            });
            ui.add_space(10.0);
            ui.columns(2, |c| {
                if w::button(&mut c[0], t, None, t!("Continue shopping"), w::Kind::Secondary, w::Size::Md).clicked() {
                    close = true;
                }
                c[1].with_layout(Layout::right_to_left(Align::Center), |ui| {
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

    pub(super) fn price_history(&mut self, ui: &mut Ui, t: &Theme, id: AppId) {
        w::section(ui, t, t!("Price history"), None);
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.itad_key.trim().is_empty() {
                let local = self.price_log.get(&id).cloned().unwrap_or_default();
                if local.len() >= 2 {
                    let pts: Vec<(u64, f64)> = local.iter().map(|(t, c)| (*t, *c as f64 / 100.0)).collect();
                    chart(ui, t, &pts, "");
                    ui.label(w::text(t!("Prices you've seen in SteamInRust."), 12.0, t.text_faint));
                } else {
                    ui.label(w::text(t!("Add a free IsThereAnyDeal API key in Settings → Store preferences to see this game's full price history and all-time low."), 13.0, t.text_dim));
                }
                return;
            }
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
                            let _ = open::that_detached(h.url.clone().unwrap_or_else(|| "https://isthereanydeal.com".into()));
                        }
                    });
                }
            }
        });
        ui.add_space(16.0);
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

/// One cart line like Steam's: art, name, "For my account" / "This is a
/// gift", price, and Add | Remove. `same` = every line of this package.
#[allow(clippy::too_many_arguments)]
fn cart_row(ui: &mut Ui, t: &Theme, ctx: &Ctx, l: &model::CartLine, same: &[model::CartLine], name: &str, art: &str, price: Option<&(String, Option<String>, u8)>, owned: bool) -> Option<CartAct> {
    let mut act = None;
    // One copy per account: "For my account" only while no other copy is
    // for the account, and not for a game already owned.
    let mine_elsewhere = same.iter().any(|x| x.line_item_id != l.line_item_id && !x.is_gift);
    let can_be_mine = !owned && !mine_elsewhere;
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(170.0, 80.0), Sense::hover());
            w::image_or_skeleton(ui, t, r, art, t.corner_small());
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.add(egui::Label::new(w::text_semibold(name, 15.0, t.text)).truncate());
                ui.add_space(4.0);
                let current = if l.is_gift { t!("This is a gift") } else { t!("For my account") };
                egui::ComboBox::from_id_salt(("cart-line", l.line_item_id)).selected_text(current).width(170.0).show_ui(ui, |ui| {
                    let mine = ui.add_enabled(can_be_mine || !l.is_gift, egui::Button::selectable(!l.is_gift, t!("For my account")));
                    let mine = if owned { mine.on_disabled_hover_text(t!("You already own this game")) } else { mine.on_disabled_hover_text(t!("Another copy is already for your account")) };
                    if mine.clicked() && l.is_gift {
                        act = Some(CartAct::SetGift(l.line_item_id, false));
                    }
                    if ui.add(egui::Button::selectable(l.is_gift, t!("This is a gift"))).clicked() && !l.is_gift {
                        act = Some(CartAct::SetGift(l.line_item_id, true));
                    }
                });
                if l.is_gift {
                    let base = ctx.state.steam_id & 0xFFFF_FFFF_0000_0000;
                    let to = l.gift_to.map(|acc| base | acc as u64);
                    ui.horizontal(|ui| {
                        match to {
                            Some(id) => {
                                ui.label(w::text(format!("{}  {}", icons::USER, t!("For {name}", name = ctx.state.persona(id).name)), 12.5, t.accent));
                                if ui.link(w::text(t!("Change"), 12.5, t.text_dim)).clicked() {
                                    act = Some(CartAct::ChooseFriend(l.line_item_id, to));
                                }
                            }
                            None => {
                                if ui.link(w::text(format!("{}  {}", icons::USER, t!("Choose a friend…")), 12.5, t.accent)).on_hover_text(t!("Or pick who gets it on Steam's checkout page")).clicked() {
                                    act = Some(CartAct::ChooseFriend(l.line_item_id, None));
                                }
                            }
                        }
                    });
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.vertical(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| match price {
                        Some((label, original, discount)) => w::price_tag(ui, t, label, original.as_deref(), *discount),
                        None => {
                            ui.label(w::text_semibold(&l.price, 15.0, t.text));
                        }
                    });
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if ui.link(w::text(t!("Remove"), 12.5, t.text_dim)).clicked() {
                            act = Some(CartAct::Remove(l.line_item_id));
                        }
                        ui.label(w::text("|", 12.5, t.text_faint));
                        if ui.link(w::text(t!("Add"), 12.5, t.text_dim)).on_hover_text(t!("Another copy, as a gift")).clicked() {
                            act = Some(CartAct::AddCopy);
                        }
                    });
                });
            });
        });
    });
    act
}

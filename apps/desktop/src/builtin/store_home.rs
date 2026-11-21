//! Store: the page bar, the home page (the featured carousel, shelves of
//! games, categories) and the store's game card. Figma "client store"
//! `426:3487`, the carousel's seven states `634:15231`.

use super::{Store, View};
use client::Command;
use egui::{Align, Align2, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use store::{Item, Section};
use ui::{Theme, icons, widgets as w};

/// How long the carousel stays on a game before moving on (seconds).
const DWELL: f64 = 8.0;
/// How long a slide takes (seconds).
const SLIDE: f32 = 0.45;

/// "Browse by category": the Steam tag, its picture's name on Steam's
/// store, and its English name (until Steam's own tag names load).
pub(super) const CATEGORIES: &[(u32, &str, &str)] = &[
    (19, "action", "Action"),
    (122, "rpg", "RPG"),
    (113, "freetoplay", "Free to Play"),
    (4085, "anime", "Anime"),
    (1685, "multiplayer_coop", "Co-op"),
    (699, "racing", "Racing"),
    (21, "adventure", "Adventure"),
    (9, "strategy", "Strategy"),
    (599, "simulation", "Simulation"),
    (701, "sports", "Sports"),
    (1667, "horror", "Horror"),
    (1695, "exploration_open_world", "Open World"),
    (1662, "survival", "Survival"),
    (3942, "science_fiction", "Sci-fi"),
    (597, "casual", "Casual"),
    (1664, "puzzle_matching", "Puzzle"),
];

/// What a store card asked for.
pub(super) struct CardOut {
    pub open: bool,
    pub wish: bool,
    pub hovered: bool,
    /// Where the picture is (for the clip played over it).
    pub img: Rect,
}

impl Store {
    // ---------- the page bar ----------

    /// The store's tabs, search and lists in the page bar (Figma "header"
    /// `302:1092`): Home, Browse, Discover, Curators, Points Shop, the search, then
    /// Wishlist, Cart and the store's preferences on the right.
    pub(super) fn store_bar(&mut self, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        let t = ctx.theme.clone();
        ui.spacing_mut().item_spacing.x = t.spacing.sm - 1.0;
        let v = self.view.clone();
        let signed_in = ctx.state.steam_id != 0;
        let tabs = [
            (t!("Home").to_string(), matches!(v, View::Home | View::App(_) | View::Offer(..)), View::Home, true),
            (t!("Browse|store").to_string(), matches!(v, View::Browse | View::Section(_) | View::Search(_) | View::Creator(..)), View::Browse, true),
            (t!("Discover").to_string(), matches!(v, View::Discover | View::Discovery(_)), View::Discover, signed_in),
            (t!("Curators").to_string(), matches!(v, View::Curators | View::Curator(_)), View::Curators, true),
            (t!("Points Shop").to_string(), v == View::Points, View::Points, true),
        ];
        for (label, on, view, show) in tabs {
            if show && w::sub_tab(ui, &t, &label, on).clicked() {
                self.open(view);
            }
        }
        ui.add_space(t.dense(30.0));
        // What Wishlist, Cart and the gear took last frame (wider in some
        // languages): the search never runs under them.
        let right_id = ui.scope_id().with("store-bar-right");
        let right = ui.data(|d| d.get_temp::<f32>(right_id)).unwrap_or(0.0).max(t.dense(250.0)) + t.spacing.md;
        let width = (ui.available_width() - right).clamp(120.0, 374.0);
        if ui.available_width() - width >= right {
            self.search_box(ui, &t, ctx, width);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.sm - 1.0;
            let gear = w::icon_box(ui, &t, icons::GEAR, t!("Store preferences"), false);
            egui::Popup::from_toggle_button_response(&gear).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(t.dense(440.0)).show(|ui| {
                use extensions::Extension;
                ui.spacing_mut().item_spacing.y = t.spacing.xs;
                self.settings(ui, ctx);
            });
            let cart = ctx.state.cart.as_ref().map(|c| c.len()).unwrap_or(0);
            let r = w::sub_tab(ui, &t, t!("Cart"), v == View::Cart);
            if cart > 0 {
                w::badge(ui.painter(), &t, r.rect.right_top() + vec2(-3.0, 3.0), cart);
            }
            if r.clicked() {
                self.open(View::Cart);
            }
            if signed_in && w::sub_tab(ui, &t, t!("Wishlist"), v == View::Wishlist).clicked() {
                self.open(View::Wishlist);
            }
            let used = ui.min_rect().width();
            ui.data_mut(|d| d.insert_temp(right_id, used));
        });
        true
    }

    // ---------- home ----------

    pub(super) fn home(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(data) = self.front_data() else {
            let w_ = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(w_, t.dense(540.0)), Sense::hover());
            w::skeleton(ui, t, r, t.corner());
            return;
        };
        let (spotlight, sections) = match data {
            Ok(d) => d,
            Err(e) => {
                w::empty(ui, t, icons::WARNING, t!("The store didn't load"), &e);
                if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Md).clicked() {
                    self.front = None;
                }
                return;
            }
        };
        // Hide items the user filtered out; wait for ratings before showing art.
        let spotlight: Vec<Item> = spotlight.into_iter().filter(|i| self.safe(i.app_id) == Some(true)).collect();
        let sections: Vec<Section> = sections
            .into_iter()
            .map(|mut s| {
                s.items.retain(|i| self.safe(i.app_id) != Some(false));
                s
            })
            .collect();
        let gap = t.dense(30.0);
        let between = t.dense(70.0);
        ui.label(w::heading_s(t, t!("Featured").to_uppercase(), t.text));
        ui.add_space(gap);
        if spotlight.is_empty() {
            let w_ = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(w_, t.dense(537.0)), Sense::hover());
            w::skeleton(ui, t, r, t.corner());
        } else {
            self.carousel(ui, t, ctx, &spotlight);
        }
        ui.add_space(between);
        for (i, s) in sections.iter().enumerate() {
            self.shelf(ui, t, ctx, s);
            ui.add_space(between);
            if i == 0 {
                self.categories(ui, t);
                ui.add_space(between);
            }
        }
        if sections.is_empty() {
            self.categories(ui, t);
        }
    }

    /// The featured carousel (Figma `634:15231`): one game at a time, the
    /// current dot filling up until it slides on to the next; the pointer
    /// over it holds it still.
    fn carousel(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, items: &[Item]) {
        let now = ui.input(|i| i.time);
        let n = items.len();
        self.hero %= n;
        let width = ui.available_width();
        let c = &t.components.shelf;
        let pad = t.padding_of(c);
        let side = pad.x + t.dense(35.0);
        let slide_w = width - side * 2.0;
        let slide_h = (slide_w * 488.0 / 1281.0).max(t.dense(330.0));
        let (rect, resp) = ui.allocate_exact_size(vec2(width, slide_h + pad.y * 2.0), Sense::hover());
        w::paint_box(ui, t, rect, c, &t.look(c, Default::default()), 0.0);
        // The pointer anywhere over it, or the window in the background,
        // holds the timer (and a window in the background isn't redrawn for it).
        let focused = ui.input(|i| i.focused);
        if ui.rect_contains_pointer(rect) || !focused {
            self.hero_changed += ui.input(|i| i.stable_dt).min(0.5) as f64;
        }
        let mut go = None;
        let elapsed = now - self.hero_changed;
        if elapsed > DWELL {
            go = Some((self.hero + 1) % n);
        }
        if focused {
            // The page dot fills up over 8 s: 8 frames a second is plenty.
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(125));
        }
        let _ = resp;

        // The slides, sliding: the strip moves by one slide (plus a gap) per game.
        let strip = Rect::from_min_size(pos2(rect.left() + side, rect.top() + pad.y), vec2(slide_w, slide_h));
        let pos = ui.ctx().animate_value_with_time(ui.scope_id().with("hero-slide"), self.hero as f32, SLIDE);
        let step = slide_w + t.dense(30.0);
        for (i, item) in items.iter().enumerate() {
            let x = strip.left() + (i as f32 - pos) * step;
            let r = Rect::from_min_size(pos2(x, strip.top()), strip.size());
            if !r.intersects(strip) {
                continue;
            }
            let mut child = ui.new_child(UiBuilder::new().max_rect(r).layout(Layout::top_down(Align::Min)));
            child.set_clip_rect(strip.intersect(ui.clip_rect()));
            if let Some(open) = self.slide(&mut child, t, ctx, item, r) {
                self.open(View::App(open));
            }
        }
        for (dir, x) in [(-1i32, rect.left() + side / 2.0 + pad.x / 2.0), (1, rect.right() - side / 2.0 - pad.x / 2.0)] {
            if arrow(ui, t, pos2(x, rect.center().y), dir, "hero") {
                go = Some((self.hero as i32 + dir).rem_euclid(n as i32) as usize);
            }
        }
        ui.add_space(t.dense(22.0));
        let fill = (elapsed / DWELL) as f32;
        if let Some(i) = w::pager(ui, t, n, self.hero, fill, t.dense(35.0)) {
            go = Some(i);
        }
        if let Some(i) = go {
            self.hero = i;
            self.hero_changed = now;
        }
    }

    /// One game in the carousel: the big picture, then its name, blurb,
    /// four screenshots, tags, Wishlist and the price with Buy. Returns a
    /// game to open.
    fn slide(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, item: &Item, rect: Rect) -> Option<model::AppId> {
        let c = &t.components.store_card;
        let resp = ui.interact(rect, ui.scope_id().with(("slide", item.app_id)), Sense::click());
        let look = t.look(c, w::interact(ui, &resp, false));
        w::paint_box(ui, t, rect, c, &look, 0.0);
        let pad = t.padding_of(c).x;
        let inner = rect.shrink(pad);
        let gap = t.dense(17.0);
        let img = Rect::from_min_size(inner.min, vec2((inner.width() * 0.612).round(), inner.height()));
        w::image_or_skeleton(ui, t, img, item.large.as_deref().unwrap_or(&item.header), t.corner_small());
        super::more::hover_wash(ui, t, img, &resp);
        let owned = self.owned.contains(&item.app_id);
        if owned {
            super::in_library_badge(ui, t, img);
        }
        let col = Rect::from_min_max(pos2(img.right() + gap, inner.top()), inner.max);
        let mut open = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();

        // Bottom: Wishlist on the left, the price and Buy on the right.
        let row_h = t.dense(44.0);
        let bottom = Rect::from_min_max(pos2(col.left(), col.bottom() - row_h), col.max);
        // The price and Buy first (right to left), then Wishlist in what's left.
        let mut rb = ui.new_child(UiBuilder::new().max_rect(bottom).layout(Layout::right_to_left(Align::Center)));
        if owned {
            rb.label(w::body_l(t, t!("In library"), t.play));
        } else {
            let label = if item.price.final_.unwrap_or(0) == 0 { t!("Free").to_string() } else { item.price.label() };
            let r = w::main_cta_rtl(&mut rb, t, &label, item.price.original_label().as_deref(), item.price.discount, t!("Buy Now"));
            if r.clicked() {
                open = true;
            }
        }
        let left_w = (bottom.width() - rb.min_rect().width() - gap).max(0.0);
        let mut b = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(bottom.min, vec2(left_w, row_h))).layout(Layout::left_to_right(Align::Center)));
        let wished = ctx.state.wishlist.as_ref().is_some_and(|l| l.iter().any(|(a, _)| *a == item.app_id));
        if !owned && ctx.state.steam_id != 0 && left_w >= row_h {
            // Just the heart when the word doesn't fit, so it never runs into the price.
            let word = t!("Wishlist");
            let c = &t.components.wishlist;
            let word_w = ui.painter().layout_no_wrap(word.to_string(), t.font_of(c), egui::Color32::PLACEHOLDER).size().x;
            // The button's own width with the word (see `wishlist_button_sized`).
            let needed = t.padding_of(c).x * 2.0 + (row_h * 0.42).round() + 13.0 + word_w;
            let label = (needed <= left_w).then_some(word);
            let r = w::wishlist_button_sized(&mut b, t, wished, label, Some(row_h));
            if r.clicked() {
                ctx.send(if wished { Command::WishlistRemove { app_id: item.app_id } } else { Command::WishlistAdd { app_id: item.app_id } });
            }
        }

        // Above it: name, blurb, screenshots, tags (each part only if it fits).
        let upper = Rect::from_min_max(col.min, pos2(col.right(), bottom.top() - gap));
        let mut u = ui.new_child(UiBuilder::new().max_rect(upper).layout(Layout::top_down(Align::Min)));
        u.set_clip_rect(upper.intersect(ui.clip_rect()));
        u.spacing_mut().item_spacing.y = t.dense(11.0);
        u.add(egui::Label::new(w::heading_l(t, &item.name, t.text)).truncate().selectable(false));
        let d = self.details(item.app_id);
        let tag_h = t.height_of(&t.components.tag, 30.0);
        match &d {
            Some(d) => {
                // Three lines at most.
                let mut job = egui::text::LayoutJob::single_section(d.short_description.clone(), egui::TextFormat::simple(t.typography.body_m.font(), t.text));
                job.wrap = egui::text::TextWrapping { max_width: col.width(), max_rows: 3, break_anywhere: false, overflow_character: Some('…') };
                u.add(egui::Label::new(job).selectable(false));
                u.add_space(t.dense(6.0) - u.spacing().item_spacing.y);
                let room = (upper.bottom() - u.cursor().top() - tag_h - gap).max(0.0);
                let tw = ((col.width() - t.dense(7.0)) / 2.0).floor();
                let th = (tw * 111.0 / 231.0).floor();
                let rows = if 2.0 * th + t.dense(7.0) <= room { 2 } else if th <= room { 1 } else { 0 };
                for pair in d.screenshots.chunks(2).take(rows) {
                    u.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.dense(7.0);
                        for s in pair {
                            let (r, _) = ui.allocate_exact_size(vec2(tw, th), Sense::hover());
                            w::image_or_skeleton(ui, t, r, &s.thumb, egui::CornerRadius::ZERO);
                        }
                    });
                }
            }
            None => {
                let (r, _) = u.allocate_exact_size(vec2(col.width(), (col.width() * 0.5).min(upper.height() * 0.6)), Sense::hover());
                w::skeleton(&u, t, r, t.corner_small());
            }
        }
        // Tags: the game's top tags (its genres until they load), then the platforms.
        let tags_row = Rect::from_min_max(pos2(col.left(), upper.bottom() - tag_h), upper.max);
        let mut tr = ui.new_child(UiBuilder::new().max_rect(tags_row).layout(Layout::left_to_right(Align::Center)));
        tr.spacing_mut().item_spacing.x = t.spacing.xs;
        tr.set_clip_rect(tags_row.intersect(ui.clip_rect()));
        let names = self.top_tags(item.app_id, d.as_ref().map(|d| d.genres.as_slice()).unwrap_or(&[]), 4);
        let mut picked = None;
        tr.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if item.windows {
                ui.label(w::styled(icons::WINDOWS, ui::theme::TextStyle::new(t.typography.heading_s.size, Default::default()), t.text_dim));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let mut used = 0.0;
                for (id, name) in &names {
                    let need = ui.painter().layout_no_wrap(name.clone(), t.font_of(&t.components.tag), t.text).size().x + t.padding_of(&t.components.tag).x * 2.0 + t.spacing.xs;
                    if used + need > ui.available_width() - tag_h {
                        break;
                    }
                    used += need;
                    if w::tag(ui, t, name, None, w::TagState::Plain).clicked() {
                        picked = *id;
                    }
                }
                if !names.is_empty() && w::tag(ui, t, "+", None, w::TagState::Plain).on_hover_text(t!("Open the game's page")).clicked() {
                    open = true;
                }
            });
        });
        if let Some(tag) = picked {
            self.open_tag(tag);
        }
        open.then_some(item.app_id)
    }

    /// A game's top tags as (tag id, name), or its genres until the tags load.
    pub(super) fn top_tags(&mut self, id: model::AppId, genres: &[String], n: usize) -> Vec<(Option<u32>, String)> {
        let info = self.browse_info(id);
        let tags: Vec<(Option<u32>, String)> = info.map(|i| i.tags.iter().take(n).filter_map(|id| Some((Some(*id), self.tag_name(*id)?))).collect()).unwrap_or_default();
        if tags.is_empty() { genres.iter().take(n).map(|g| (None, g.clone())).collect() } else { tags }
    }

    /// A shelf of games (Figma "Special Offers" `426:3536`): its title and
    /// "See more", the cards a page at a time with arrows, dots under it.
    /// Top sellers are a two-column list instead (`426:3801`).
    fn shelf(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, s: &Section) {
        let title = i18n::tr_dyn(&s.title);
        if shelf_head(ui, t, &title) {
            self.open(View::Section(s.id.clone()));
        }
        ui.add_space(t.dense(30.0));
        let rows = s.id == "top_sellers";
        let gap = t.dense(21.0);
        let (side, _) = shelf_insets(t);
        let inner_w = ui.available_width() - side * 2.0;
        let (per_row, lines) = if rows { (if inner_w > t.dense(720.0) { 2 } else { 1 }, 3) } else { ((((inner_w + gap) / (t.dense(330.0) + gap)).floor() as usize).clamp(1, 4), 1) };
        let per = per_row * lines;
        let pages = s.items.len().div_ceil(per).max(1);
        let page = self.shelf_page.get(&s.id).copied().unwrap_or(0).min(pages - 1);
        let card_w = ((inner_w - gap * (per_row as f32 - 1.0)) / per_row as f32).floor();
        let item_h = if rows { row_card_height(t) } else { card_height(t, card_w) };
        let height = item_h * lines as f32 + gap * (lines as f32 - 1.0);
        let (inner, turn) = shelf_frame(ui, t, &s.id, height, pages > 1);
        let mut c = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        c.spacing_mut().item_spacing = vec2(gap, gap);
        let wishlist = ctx.state.wishlist.clone().unwrap_or_default();
        let items: Vec<Item> = s.items.iter().skip(page * per).take(per).cloned().collect();
        for line in items.chunks(per_row) {
            c.horizontal(|ui| {
                for item in line {
                    if self.safe(item.app_id).is_none() {
                        let (r, _) = ui.allocate_exact_size(vec2(card_w, item_h), Sense::hover());
                        w::skeleton(ui, t, r, t.corner());
                        continue;
                    }
                    let owned = self.owned.contains(&item.app_id);
                    let wished = wishlist.iter().any(|(a, _)| *a == item.app_id);
                    let out = if rows { row_card(ui, t, item, card_w, owned, wished, ctx.state.steam_id != 0) } else { store_card(ui, t, item, card_w, owned, wished, ctx.state.steam_id != 0) };
                    let corner = if rows { t.corner_small() } else { t.corner_small() };
                    self.micro(ui, item.app_id, out.img, corner, out.hovered);
                    if out.wish {
                        ctx.send(if wished { Command::WishlistRemove { app_id: item.app_id } } else { Command::WishlistAdd { app_id: item.app_id } });
                    } else if out.open {
                        self.open(View::App(item.app_id));
                    }
                }
            });
        }
        if turn != 0 {
            self.shelf_page.insert(s.id.clone(), (page as i32 + turn).rem_euclid(pages as i32) as usize);
        }
        ui.add_space(t.dense(22.0));
        if pages > 1 {
            if let Some(p) = w::pager(ui, t, pages, page, 1.0, t.dense(35.0)) {
                self.shelf_page.insert(s.id.clone(), p);
            }
        }
    }

    /// "Browse by category" (Figma "Browse Steam" `426:3952`): Steam's
    /// category pictures, each opening Browse with that tag.
    pub(super) fn categories(&mut self, ui: &mut Ui, t: &Theme) {
        if shelf_head(ui, t, t!("Browse by category")) {
            self.open(View::Browse);
        }
        ui.add_space(t.dense(30.0));
        self.category_strip(ui, t, "home-categories", t.dense(240.0));
    }

    /// Category tiles a page at a time, `tile` wide at most.
    pub(super) fn category_strip(&mut self, ui: &mut Ui, t: &Theme, key: &str, tile: f32) {
        let gap = t.dense(18.0);
        let (side, _) = shelf_insets(t);
        let inner_w = ui.available_width() - side * 2.0;
        let per = (((inner_w + gap) / (tile + gap)).floor() as usize).clamp(1, CATEGORIES.len());
        let pages = CATEGORIES.len().div_ceil(per);
        let page = self.shelf_page.get(key).copied().unwrap_or(0).min(pages - 1);
        let tw = ((inner_w - gap * (per as f32 - 1.0)) / per as f32).floor();
        let th = (tw * 233.0 / 240.0).round();
        let (inner, turn) = shelf_frame(ui, t, key, th, pages > 1);
        let mut c = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Min)));
        c.spacing_mut().item_spacing.x = gap;
        let (cc, lang) = self.locale.clone();
        let mut picked = None;
        for (tag, slug, english) in CATEGORIES.iter().skip(page * per).take(per) {
            let (r, resp) = c.allocate_exact_size(vec2(tw, th), Sense::click());
            let path = if slug.contains('/') || *slug == "freetoplay" { slug.to_string() } else { format!("category/{slug}") };
            let url = format!("https://store.fastly.steamstatic.com/categories/homepageimage/{path}?cc={cc}&l={lang}");
            w::image_or_skeleton(&c, t, r, &url, t.corner());
            let hover = c.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
            c.painter().rect_filled(r, t.corner(), w::mix(t.overlay_dark(2), t.overlay_dark(0), hover));
            let name = self.tag_name(*tag).unwrap_or_else(|| english.to_string());
            c.painter().text(r.center(), Align2::CENTER_CENTER, name, t.typography.heading_m.font(), t.text);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                picked = Some(*tag);
            }
        }
        if turn != 0 {
            self.shelf_page.insert(key.to_string(), (page as i32 + turn).rem_euclid(pages as i32) as usize);
        }
        if let Some(tag) = picked {
            self.open_tag(tag);
        }
    }
}

/// A shelf's title and its "See more" button; true when that was clicked.
pub(super) fn shelf_head(ui: &mut Ui, t: &Theme, title: &str) -> bool {
    ui.horizontal(|ui| {
        ui.label(w::heading_s(t, title, t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| w::button(ui, t, None, t!("See more"), w::Kind::Tertiary, w::Size::Sm).clicked()).inner
    })
    .inner
}

/// Room at a shelf's sides (for the arrows) and above and below its content.
pub(super) fn shelf_insets(t: &Theme) -> (f32, f32) {
    let pad = t.padding_of(&t.components.shelf);
    (pad.x + t.dense(45.0), pad.y)
}

/// The block behind a shelf (`shelf`), `height` of content, with arrows at
/// the sides when there's more than a page. Returns the content's rectangle
/// and -1 or 1 when an arrow was clicked.
pub(super) fn shelf_frame(ui: &mut Ui, t: &Theme, key: &str, height: f32, arrows: bool) -> (Rect, i32) {
    let c = &t.components.shelf;
    let (side, pad_y) = shelf_insets(t);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height + pad_y * 2.0), Sense::hover());
    w::paint_box(ui, t, rect, c, &t.look(c, Default::default()), 0.0);
    let mut turn = 0;
    if arrows {
        for (dir, x) in [(-1i32, rect.left() + side - t.dense(9.0) - t.dense(12.0)), (1, rect.right() - side + t.dense(21.0))] {
            if arrow(ui, t, pos2(x, rect.center().y), dir, key) {
                turn = dir;
            }
        }
    }
    (Rect::from_min_size(pos2(rect.left() + side, rect.top() + pad_y), vec2(rect.width() - side * 2.0, height)), turn)
}

/// A carousel arrow (Figma "Arrow forward ios", 24 px); true when clicked.
fn arrow(ui: &mut Ui, t: &Theme, center: egui::Pos2, dir: i32, key: &str) -> bool {
    let size = t.dense(24.0);
    let r = Rect::from_center_size(center, vec2(size + 12.0, size * 2.5));
    let resp = ui.interact(r, ui.scope_id().with(("arrow", key, dir)), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let icon = if dir < 0 { icons::CARET_LEFT } else { icons::CARET_RIGHT };
    ui.painter().text(center, Align2::CENTER_CENTER, icon, w::font(size), w::mix(t.text, t.accent, hover));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A store card's height at `width` (Figma `426:4456`: 409 × 338).
pub(super) fn card_height(t: &Theme, width: f32) -> f32 {
    let pad = t.padding_of(&t.components.store_card).x;
    let img_h = ((width - pad * 2.0) * 197.0 / 379.0).round();
    pad * 2.0 + img_h + t.spacing.sm * 2.0 + t.dense(51.0) + t.dense(44.0)
}

/// A game on a store shelf (`store_card`; Figma `426:4456`): its picture,
/// name, systems, price and a wishlist heart.
pub(super) fn store_card(ui: &mut Ui, t: &Theme, item: &Item, width: f32, owned: bool, wished: bool, signed_in: bool) -> CardOut {
    let c = &t.components.store_card;
    let pad = t.padding_of(c).x;
    let gap = t.spacing.sm;
    let img_w = width - pad * 2.0;
    let img_h = (img_w * 197.0 / 379.0).round();
    let (title_h, row_h) = (t.dense(51.0), t.dense(44.0));
    let (rect, resp) = ui.allocate_exact_size(vec2(width, card_height(t, width)), Sense::click());
    let img = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(img_w, img_h));
    let mut out = CardOut { open: false, wish: false, hovered: false, img };
    if !ui.is_rect_visible(rect) {
        return out;
    }
    let look = t.look(c, w::interact(ui, &resp, false));
    w::paint_box(ui, t, rect, c, &look, 0.0);
    w::image_or_skeleton(ui, t, img, &item.header, t.corner_small());
    super::more::hover_wash(ui, t, img, &resp);
    if owned {
        super::in_library_badge(ui, t, img);
    }
    let title = Rect::from_min_size(pos2(img.left() + 2.0, img.bottom() + gap), vec2(img_w - 2.0, title_h));
    let mut tc = ui.new_child(UiBuilder::new().max_rect(title).layout(Layout::left_to_right(Align::Center)));
    tc.add(egui::Label::new(w::heading_m(t, &item.name, look.text)).truncate().selectable(false));
    let row = Rect::from_min_size(pos2(img.left(), title.bottom() + gap), vec2(img_w, row_h));
    let mut rc = ui.new_child(UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Center)));
    rc.spacing_mut().item_spacing.x = t.dense(25.0);
    if !owned && signed_in {
        let tip = if wished { t!("Remove from wishlist") } else { t!("Add to wishlist") };
        out.wish = w::wishlist_button_sized(&mut rc, t, wished, None, Some(row_h)).on_hover_text(tip).clicked();
    }
    price_or_owned(&mut rc, t, item, owned);
    if item.windows {
        rc.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.label(w::styled(icons::WINDOWS, ui::theme::TextStyle::new(t.typography.heading_s.size, Default::default()), t.text_dim));
        });
    }
    out.hovered = resp.hovered() || ui.rect_contains_pointer(rect);
    out.open = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    out
}

/// "In library", or the price block.
fn price_or_owned(ui: &mut Ui, t: &Theme, item: &Item, owned: bool) {
    if owned {
        ui.label(w::body_l(t, t!("In library"), t.play));
    } else if item.price.final_ == Some(0) {
        ui.label(w::styled(t!("Free"), t.typography.price, t.text_dim));
    } else if item.price.final_.is_some() {
        w::price(ui, t, &item.price.label(), item.price.original_label().as_deref(), item.price.discount);
    }
}

/// A row card's height (Figma "Top Sellers" `426:3801`).
pub(super) fn row_card_height(t: &Theme) -> f32 {
    let pad = t.padding_of(&t.components.store_card).y.min(t.dense(10.0));
    t.dense(67.0) + pad * 2.0
}

/// A game in a two-column list (top sellers): small picture, name, systems,
/// price and the heart.
pub(super) fn row_card(ui: &mut Ui, t: &Theme, item: &Item, width: f32, owned: bool, wished: bool, signed_in: bool) -> CardOut {
    let c = &t.components.store_card;
    let pad = t.padding_of(c).y.min(t.dense(10.0));
    let h = row_card_height(t);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    let img_h = h - pad * 2.0;
    let img = Rect::from_min_size(rect.min + vec2(pad, pad), vec2((img_h * 460.0 / 215.0).round(), img_h));
    let mut out = CardOut { open: false, wish: false, hovered: false, img };
    if !ui.is_rect_visible(rect) {
        return out;
    }
    let look = t.look(c, w::interact(ui, &resp, false));
    w::paint_box(ui, t, rect, c, &look, 0.0);
    w::image_or_skeleton(ui, t, img, &item.header, t.corner_small());
    super::more::hover_wash(ui, t, img, &resp);
    let body = Rect::from_min_max(pos2(img.right() + t.dense(14.0), rect.top() + pad), rect.max - vec2(pad, pad));
    let mut b = ui.new_child(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)));
    b.add(egui::Label::new(w::heading_s(t, &item.name, look.text)).truncate().selectable(false));
    let row_h = t.dense(36.0);
    let row = Rect::from_min_max(pos2(body.left(), body.bottom() - row_h), body.max);
    let mut rc = ui.new_child(UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Center)));
    rc.spacing_mut().item_spacing.x = t.dense(14.0);
    if !owned && signed_in {
        let tip = if wished { t!("Remove from wishlist") } else { t!("Add to wishlist") };
        out.wish = w::wishlist_button_sized(&mut rc, t, wished, None, Some(row_h)).on_hover_text(tip).clicked();
    }
    if owned {
        rc.label(w::body_m(t, t!("In library"), t.play));
    } else if item.price.final_ == Some(0) {
        rc.label(w::body_l(t, t!("Free"), t.text_dim));
    } else if item.price.final_.is_some() {
        w::price_tag(&mut rc, t, &item.price.label(), item.price.original_label().as_deref(), item.price.discount);
    }
    if item.windows {
        rc.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.label(w::body_l(t, icons::WINDOWS, t.text_dim));
        });
    }
    out.hovered = resp.hovered() || ui.rect_contains_pointer(rect);
    out.open = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    out
}

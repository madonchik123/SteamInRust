//! The library's screens in the redesign (Figma "library home" `324:1652`
//! and its filter, grid, detail-view and collections frames): the page bar,
//! the game list beside the page, the grid and list views, collections and
//! a game's page.

use super::{Entry, Filter, Library, MainAction, MenuAct, Setup, Sort, Tab, View, entries, facts, props, setup_state};
use egui::{Align, Align2, Layout, Margin, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, InstalledGame, art};
use ui::{Theme, icons, widgets as w};

/// A group of games in the Collections tab: (key, title, games, index in
/// `collections` when it's one of yours).
type Group<'a> = (String, String, Vec<Entry<'a>>, Option<usize>);

impl Library {
    // ---------- the page bar ----------

    /// The library's page bar (Figma "library header" `375:2459`): Home,
    /// Collections, News and Updates; the search and the filter, recent and
    /// ready-to-play buttons; the view buttons; Activate a Product and Add a
    /// non-Steam Game on the right.
    pub(super) fn bar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        // The bar is drawn before the page: a click here shows on the next frame.
        let before = self.bar_state();
        self.bar_inner(ui, t, ctx);
        if self.bar_state() != before {
            ui.ctx().request_repaint();
        }
    }

    /// What the bar's buttons change.
    fn bar_state(&self) -> (Tab, View, bool, Filter, Option<AppId>, Option<usize>, usize) {
        (self.tab, self.view, self.recent, self.filter, self.selected, self.custom_selected, self.adv.count())
    }

    fn bar_inner(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let full = ui.max_rect();
        let gap = t.spacing.sm - 1.0;
        // The right end first, when there's room, so the rest gets what's left.
        let mut right = full.right();
        if full.width() >= 1500.0 {
            let mut r = ui.new_child(UiBuilder::new().max_rect(full).layout(Layout::right_to_left(Align::Center)));
            r.spacing_mut().item_spacing.x = gap;
            if w::sub_tab(&mut r, t, t!("Add a non-Steam Game"), false).clicked() {
                self.adding = Some(super::CustomGame::default());
            }
            if w::sub_tab(&mut r, t, t!("Activate a Product"), false).clicked() {
                ctx.navigate("activate", None);
            }
            right = r.min_rect().left() - t.spacing.xl;
        }
        let mut ui = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(full.min, pos2(right.max(full.left() + 100.0), full.bottom()))).layout(Layout::left_to_right(Align::Center)));
        let ui = &mut ui;
        ui.spacing_mut().item_spacing.x = gap;
        for (tab, label) in [(Tab::Home, t!("Home")), (Tab::Collections, t!("Collections")), (Tab::News, t!("News and Updates"))] {
            if w::sub_tab(ui, t, label, self.tab == tab).clicked() {
                self.open_tab(tab);
            }
        }
        ui.add_space(t.spacing.xl);
        // Search, then the filter buttons; the view buttons when there's room.
        let boxes = t.height_of(&t.components.icon_box, 40.0) + t.spacing.sm - 2.0;
        let filters_w = boxes * if model::THIS_OS != "windows" { 4.0 } else { 3.0 };
        let view_w = boxes * 3.0 + 60.0;
        let room = ui.available_width() - filters_w - t.spacing.md;
        let show_view = room - view_w - t.spacing.xl * 2.0 >= 160.0;
        let search_w = (room - if show_view { view_w + t.spacing.xl * 2.0 } else { 0.0 }).clamp(120.0, 374.0);
        w::bar_search(ui, t, &mut self.search, t!("Search games"), search_w);
        ui.spacing_mut().item_spacing.x = t.spacing.sm - 2.0;
        self.filter_buttons(ui, t, ctx);
        if show_view {
            ui.add_space(t.spacing.xl * 2.0);
            ui.label(w::body_m(t, t!("View|menu"), t.text));
            self.view_buttons(ui, t);
        }
    }

    /// Open a tab of the page bar: its overview, not a game's page.
    fn open_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.selected = None;
        self.custom_selected = None;
    }

    /// Filters (a popup), recent games and ready to play; off Windows also
    /// "only games that run here".
    fn filter_buttons(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let n = self.adv.count();
        let tip = if n > 0 { format!("{} ({n})", t!("Filters")) } else { t!("Filters").to_string() };
        let r = w::icon_box(ui, t, icons::FUNNEL, &tip, n > 0);
        egui::Popup::from_toggle_button_response(&r)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(860.0)
            .show(|ui| self.adv_panel(ui, t, ctx));
        if w::icon_box(ui, t, icons::CLOCK, t!("Recent games"), self.recent).clicked() {
            self.recent = !self.recent;
        }
        let ready = self.filter == Filter::Installed;
        if w::icon_box(ui, t, icons::PLAY, t!("Ready to play"), ready).clicked() {
            self.filter = if ready { Filter::All } else { Filter::Installed };
        }
        // Off Windows many games have no version for this system: one
        // click shows only the ones that run here.
        if model::THIS_OS != "windows" {
            let os = model::THIS_OS;
            let on = self.adv.systems.len() == 1 && self.adv.systems.contains(os);
            let icon = if os == "macos" { icons::APPLE } else { icons::LINUX };
            if w::icon_box(ui, t, icon, &t!("Only games that run on {os}", os = client::download::os_name(os)), on).clicked() {
                self.adv.systems.clear();
                if !on {
                    self.adv.systems.insert(os);
                }
            }
        }
    }

    /// Library home, grid or list (Figma "icon select" `346:2628`).
    fn view_buttons(&mut self, ui: &mut Ui, t: &Theme) {
        for (view, icon, tip) in [(View::Home, icons::SQUARE, t!("Library home")), (View::Grid, icons::GRID, t!("Grid view")), (View::List, icons::LIST, t!("List view"))] {
            let on = self.view == view && self.selected.is_none() && self.custom_selected.is_none() && self.tab == Tab::Home;
            if w::icon_box(ui, t, icon, tip, on).clicked() {
                self.view = view;
                self.open_tab(Tab::Home);
            }
        }
    }

    /// The page bar's controls at the top of the game list, for themes
    /// without a page bar.
    fn side_controls(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.sm - 2.0, t.spacing.sm - 2.0);
            for (tab, label) in [(Tab::Home, t!("Home")), (Tab::Collections, t!("Collections")), (Tab::News, t!("News and Updates"))] {
                if w::chip(ui, t, label, self.tab == tab).clicked() {
                    self.open_tab(tab);
                }
            }
        });
        ui.add_space(t.spacing.sm);
        let width = ui.available_width() - t.spacing.sm;
        w::search(ui, t, &mut self.search, t!("Search games"), width);
        ui.add_space(t.spacing.sm);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.sm - 2.0, t.spacing.sm - 2.0);
            self.filter_buttons(ui, t, ctx);
            ui.add_space(t.spacing.sm);
            self.view_buttons(ui, t);
            if w::icon_box(ui, t, icons::PLUS, t!("Add a non-Steam game"), false).clicked() {
                self.adding = Some(super::CustomGame::default());
            }
        });
        ui.add_space(t.spacing.md);
    }

    // ---------- the game list beside the page ----------

    pub(super) fn sidebar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if !t.layout.page_bar {
            self.side_controls(ui, t, ctx);
        }
        if ctx.state.apps.is_none() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(t.typography.body_s.size).color(t.accent));
                ui.label(w::body_s(t, t!("Loading owned games…"), t.text_faint));
            });
        }
        let (state, local) = (ctx.state, ctx.local);
        egui::ScrollArea::vertical().id_salt("lib-side").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = t.spacing.xs + 1.0;
            let width = ui.available_width() - t.spacing.md;
            if self.tab == Tab::Collections {
                self.side_collections(ui, t, ctx, width);
            } else {
                let mut items = self.filtered_in(state, local, self.filter);
                if state.apps.is_none() && items.is_empty() {
                    for i in 0..10 {
                        let (r, _) = ui.allocate_exact_size(vec2(width, t.height_of(&t.components.game_row, 33.0)), Sense::hover());
                        w::skeleton(ui, t, Rect::from_min_size(r.min + vec2(13.0, 6.0), Vec2::splat(21.0)), t.corner_small());
                        w::skeleton(ui, t, Rect::from_min_size(r.min + vec2(44.0, 10.0), vec2(80.0 + (i * 37 % 90) as f32, 12.0)), t.corner_small());
                    }
                }
                if self.recent {
                    // Grouped by the month last played, newest first (Figma `727:40268`).
                    items.sort_by_key(|e| std::cmp::Reverse(last_played(state, e)));
                    let mut month = None;
                    for e in &items {
                        let m = last_played(state, e).map(month_of);
                        if m != month || (month.is_none() && e.app_id == items[0].app_id) {
                            month = m;
                            ui.add_space(t.spacing.xs);
                            ui.label(w::body_l(t, m.map(month_label).unwrap_or_else(|| t!("Not played yet").to_string()), t.text_dim));
                        }
                        self.side_row(ui, t, ctx, e, width);
                    }
                } else {
                    items.sort_by_key(|e| e.name.to_lowercase());
                    for e in &items {
                        self.side_row(ui, t, ctx, e, width);
                    }
                }
            }
            ui.add_space(t.spacing.md);
            if !self.custom.is_empty() {
                w::caption(ui, t, &t!("Non-Steam — {n}", n = self.custom.len()));
                for i in 0..self.custom.len() {
                    let name = self.custom[i].name.clone();
                    let sel = self.custom_selected == Some(i);
                    let (r, icon_rect) = self.row_box(ui, t, width, sel);
                    w::icon_centered(ui.painter(), icon_rect.center(), icons::CONTROLLER, icon_rect.height(), t.text_dim);
                    row_text(ui, t, r.rect, icon_rect, &name, if sel { t.text } else { t.text_dim });
                    if r.clicked() {
                        self.custom_selected = Some(i);
                        self.selected = None;
                    }
                }
                ui.add_space(t.spacing.md);
            }
            if !self.hidden.is_empty() {
                w::check(ui, t, &mut self.show_hidden, w::body_s(t, t!("Show hidden games"), t.text_faint));
            }
        });
    }

    /// A row's box in the `game_row` style; returns it and where its icon goes.
    fn row_box(&self, ui: &mut Ui, t: &Theme, width: f32, selected: bool) -> (egui::Response, Rect) {
        let c = &t.components.game_row;
        let h = t.height_of(c, 33.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(width.max(40.0), h), Sense::click());
        if ui.is_rect_visible(rect) {
            let look = t.look(c, w::interact(ui, &resp, selected));
            w::paint_box(ui, t, rect, c, &look, 0.0);
        }
        let s = (h * 0.64).round();
        let icon = Rect::from_min_size(pos2(rect.left() + t.padding_of(c).x, rect.center().y - s / 2.0), Vec2::splat(s));
        (resp.on_hover_cursor(egui::CursorIcon::PointingHand), icon)
    }

    /// One game in the list (Figma: the "tag" rows of `324:1652`): its icon
    /// and name, dim when not installed, a bar along the bottom while it
    /// installs, a dot while you play it.
    fn side_row(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, e: &Entry, width: f32) {
        let sel = self.selected == Some(e.app_id) && self.custom_selected.is_none();
        let (resp, icon) = self.row_box(ui, t, width, sel);
        let rect = resp.rect;
        if ui.is_rect_visible(rect) {
            let url = e.owned.and_then(|o| o.icon.as_deref()).map(|h| art::icon(e.app_id, h));
            // The big header only when the game has no icon at all.
            let header = art::header(e.app_id);
            let urls: Vec<&str> = url.as_deref().into_iter().chain([header.as_str()]).collect();
            if !w::paint_cover_first(ui, icon, &urls, t.corner_small()) {
                ui.painter().rect_filled(icon, t.corner_small(), t.surface_hover);
            }
            let setup = setup_state(ctx, e.app_id, e.installed.is_some());
            let busy = matches!(setup, Setup::Downloading(_) | Setup::Queued);
            let look = t.look(&t.components.game_row, w::interact(ui, &resp, sel));
            let color = if busy { t.accent } else if e.installed.is_some() || sel { look.text } else { t.text_dim };
            row_text(ui, t, rect, icon, e.name, color);
            if ctx.state.playing == Some(e.app_id) {
                ui.painter().circle_filled(rect.right_center() - vec2(t.spacing.md, 0.0), t.spacing.xs, t.in_game);
            }
            // Installing: a thin bar along the bottom of the row, like Steam.
            if let Setup::Downloading(d) = &setup {
                let track = Rect::from_min_max(pos2(icon.right() + t.spacing.sm, rect.bottom() - 4.0), pos2(rect.right() - t.spacing.sm, rect.bottom() - 2.0));
                ui.painter().rect_filled(track, t.corner_small(), t.raised);
                let fill = Rect::from_min_size(track.min, vec2(track.width() * d.fraction(), track.height()));
                ui.painter().rect_filled(fill, t.corner_small(), if d.paused { t.text_faint } else { t.accent });
            }
        }
        let id = e.app_id;
        self.game_menu(t, &resp, id, e.installed.is_some(), ctx.state.playing == Some(id));
        if resp.clicked() {
            self.selected = Some(id);
            self.custom_selected = None;
        }
    }

    /// The Collections tab's list: New Collection, then each collection as
    /// a group that opens and closes (Figma "Component 3" `368:14735`).
    fn side_collections(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, width: f32) {
        let c = &t.components.button_secondary;
        let (r, resp) = ui.allocate_exact_size(vec2(width, (t.height_of(c, 40.0) * 0.78).round()), Sense::click());
        let look = t.look(c, w::interact(ui, &resp, false));
        w::paint_box(ui, t, r, c, &look, 0.0);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("+ {}", t!("New Collection")), t.typography.body_m.font(), look.text);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.naming = Some((None, String::new(), Vec::new()));
        }
        ui.add_space(t.spacing.sm);
        let (state, local) = (ctx.state, ctx.local);
        let groups = self.groups(state, local);
        let mut remove = None;
        for (key, title, games, index) in &groups {
            let open = !self.collapsed.contains(key);
            let shelf = &t.components.library_shelf;
            let frame = egui::Frame::new()
                .fill(if open { w::solid_fill(t, shelf) } else { egui::Color32::TRANSPARENT })
                .corner_radius(t.corner_small())
                .inner_margin(Margin::symmetric(0, if open { t.spacing.xs as i8 } else { 0 }));
            frame.show(ui, |ui| {
                ui.set_width(width);
                let (hr, hresp) = ui.allocate_exact_size(vec2(width, t.dense(38.0)), Sense::click());
                let hover = ui.ctx().animate_bool_with_time(hresp.id, hresp.hovered(), 0.1);
                let pad = t.spacing.md;
                ui.painter().text(pos2(hr.left() + pad, hr.center().y), Align2::LEFT_CENTER, title.to_uppercase(), t.typography.body_l.font(), t.text);
                w::icon_centered(ui.painter(), pos2(hr.right() - pad - 8.0, hr.center().y), if open { icons::CARET_UP } else { icons::CARET_DOWN }, t.typography.body_l.size, w::mix(t.text_dim, t.text, hover));
                let hresp = hresp.on_hover_cursor(egui::CursorIcon::PointingHand);
                if hresp.clicked() && !self.collapsed.remove(key) {
                    self.collapsed.insert(key.clone());
                }
                if let Some(i) = *index {
                    hresp.context_menu(|ui| {
                        if w::menu_item(ui, t, icons::NOTE, t!("Rename")).clicked() {
                            self.naming = Some((Some(i), title.clone(), Vec::new()));
                            ui.close();
                        }
                        if w::menu_item_tinted(ui, t, icons::TRASH, t!("Delete collection"), Some(t.danger)).clicked() {
                            remove = Some(i);
                            ui.close();
                        }
                    });
                } else if key.starts_with("dyn:") {
                    hresp.on_hover_text(t!("A dynamic collection: Steam fills it from its filters. Change them in Steam."));
                }
                if open {
                    ui.add_space(t.spacing.xs);
                    ui.horizontal(|ui| {
                        ui.add_space(t.spacing.md);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = t.spacing.xs + 1.0;
                            if games.is_empty() {
                                ui.label(w::body_s(t, t!("No games here yet. Right-click a game to add it."), t.text_faint));
                            }
                            for e in games {
                                self.side_row(ui, t, ctx, e, width - t.spacing.md * 2.0);
                            }
                        });
                    });
                    ui.add_space(t.spacing.xs);
                }
            });
            ui.add_space(t.spacing.sm);
        }
        drop(groups);
        if let Some(i) = remove {
            self.collections.remove(i);
        }
    }

    /// Favorites, your collections and Steam's dynamic ones, each with its
    /// games (after the search and filters).
    fn groups<'a>(&self, state: &'a client::State, local: &'a local::LocalState) -> Vec<Group<'a>> {
        let all = self.filtered_in(state, local, self.filter);
        let pick = |f: &dyn Fn(&Entry) -> bool| -> Vec<Entry<'a>> {
            let mut v: Vec<Entry<'a>> = all.iter().filter(|e| f(e)).map(|e| Entry { app_id: e.app_id, name: e.name, installed: e.installed, owned: e.owned }).collect();
            v.sort_by_key(|e| e.name.to_lowercase());
            v
        };
        let mut groups: Vec<Group<'a>> = Vec::new();
        if !self.favorites.is_empty() {
            groups.push(("favorite".into(), t!("Favorites").to_string(), pick(&|e| self.favorites.contains(&e.app_id)), None));
        }
        for (i, c) in self.collections.iter().enumerate() {
            groups.push((format!("c:{}", c.steam_id.clone().unwrap_or_else(|| c.name.clone())), c.name.clone(), pick(&|e| c.games.contains(&e.app_id)), Some(i)));
        }
        for (name, f) in &self.dynamic {
            groups.push((format!("dyn:{name}"), name.clone(), pick(&|e| f.matches(&facts(e))), None));
        }
        groups
    }

    // ---------- the page ----------

    /// What the page shows: a game, a tab's overview or the library home in
    /// the chosen view.
    pub(super) fn main(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        match (self.custom_selected, self.selected, self.tab) {
            (Some(i), _, _) => self.custom_page(i, ui, t, ctx),
            (None, Some(id), _) => self.game_page(id, ui, t, ctx),
            (None, None, Tab::Collections) => self.collections_page(ui, t, ctx),
            (None, None, Tab::News) => self.news_page(ui, t, ctx),
            (None, None, Tab::Home) => match self.view {
                View::Home => self.home(ui, t, ctx),
                View::Grid | View::List => self.all_games(ui, t, ctx),
            },
        }
    }

    /// The page's margins (Figma: 34 px in from the list, 22 from the bar).
    pub(super) fn page_scroll(ui: &mut Ui, t: &Theme, salt: &str, add: impl FnOnce(&mut Ui)) {
        egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: (t.spacing.xl + t.spacing.sm + 2.0) as i8, right: (t.spacing.xl + t.spacing.sm + 2.0) as i8, top: t.spacing.xl as i8, bottom: (t.spacing.xl * 2.0) as i8 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui)
            });
        });
    }

    /// "ALL GAMES (308)" with the sort button (Figma `375:2897`).
    pub(super) fn games_header(&mut self, ui: &mut Ui, t: &Theme, title: &str, n: usize, collapse: Option<&str>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.sm + 1.0;
            ui.label(w::heading_s(t, format!("{} ({n})", title.to_uppercase()), t.text));
            self.sort_button(ui, t);
            if let Some(key) = collapse {
                let open = !self.collapsed.contains(key);
                if small_box(ui, t, if open { icons::CARET_UP } else { icons::CARET_DOWN }, if open { t!("Hide") } else { t!("Show") }, false).clicked() && !self.collapsed.remove(key) {
                    self.collapsed.insert(key.to_string());
                }
            }
        });
    }

    /// Sort order and card size, in a popup.
    fn sort_button(&mut self, ui: &mut Ui, t: &Theme) {
        let r = small_box(ui, t, icons::SORT, t!("Sort"), false);
        egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.label(w::body_s(t, t!("Sort by"), t.text_dim));
            w::segmented(ui, t, &mut self.sort, &[(Sort::Name, t!("Name")), (Sort::Recent, t!("Recent")), (Sort::Playtime, t!("Hours")), (Sort::Size, t!("Size"))]);
            if self.view != View::List {
                ui.add_space(t.spacing.sm);
                ui.label(w::body_s(t, t!("Tile size"), t.text_dim));
                ui.add(egui::Slider::new(&mut self.tile, 110.0..=260.0).show_value(false));
            }
        });
    }

    /// Every game, as a grid of cards or a list (Figma `363:5243`, `375:2803`).
    fn all_games(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let (state, local) = (ctx.state, ctx.local);
        Self::page_scroll(ui, t, "lib-all", |ui| {
            let items = self.filtered_in(state, local, self.filter);
            self.games_header(ui, t, t!("All games"), items.len(), None);
            ui.add_space(t.spacing.xl);
            if items.is_empty() {
                if state.apps.is_none() {
                    w::loading(ui, t, t!("Loading your library…"));
                } else {
                    w::empty(ui, t, icons::SEARCH, t!("No games match"), t!("Try a different search or filter."));
                }
                return;
            }
            self.games(ui, t, ctx, &items);
        });
    }

    /// Games in the chosen view: cards, or rows for the list view.
    fn games(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, items: &[Entry]) {
        if self.view == View::List {
            self.games_list(ui, t, ctx, items);
        } else {
            self.games_grid(ui, t, ctx, items);
        }
    }

    /// Cards in rows, spread to fill the width (Figma: 191 px cards, 60 px apart).
    pub(super) fn games_grid(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, items: &[Entry]) {
        let size = w::game_card_size(t, self.tile);
        let min_gap = t.spacing.xl;
        let avail = ui.available_width();
        let cols = (((avail + min_gap) / (size.x + min_gap)).floor() as usize).max(1);
        let gap = if cols > 1 { ((avail - cols as f32 * size.x) / (cols - 1) as f32).clamp(min_gap, size.x * 0.4) } else { min_gap };
        for row in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for e in row {
                    self.card(ui, t, ctx, e, size.x);
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
    }

    /// A game's card with its hover strip: Play (or Install…), last played, time played.
    fn card(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, e: &Entry, width: f32) {
        let id = e.app_id;
        let action = self.main_action(ctx, id);
        let (icon, tip, act) = match action {
            MainAction::Play => (Some(icons::PLAY), t!("Play"), Some(MenuAct::Play)),
            MainAction::Running => (Some(icons::STOP), t!("Stop"), Some(MenuAct::Stop)),
            MainAction::Update => (Some(icons::DOWNLOAD), t!("Update"), Some(MenuAct::Update)),
            MainAction::Install => (Some(icons::DOWNLOAD), t!("Install"), Some(MenuAct::Install)),
            MainAction::Busy { .. } | MainAction::Queued => (Some(icons::DOWNLOAD), t!("Open Downloads"), Some(MenuAct::Downloads)),
            MainAction::StorePage => (Some(icons::STORE), t!("Store page"), Some(MenuAct::Store)),
            _ => (None, "", None),
        };
        let last = last_played(ctx.state, e).map(ui::date);
        let played = minutes_played(ctx.state, e).filter(|m| *m > 0).map(ui::playtime);
        let info = w::TileInfo { action: icon, action_tip: tip, last_played: last.as_deref(), playtime: played.as_deref() };
        let (r, clicked) = w::game_card_info(ui, t, &art::capsule(id), &art::header(id), e.name, e.installed.is_some(), width, &info);
        self.game_menu(t, &r, id, e.installed.is_some(), ctx.state.playing == Some(id));
        if clicked {
            self.menu_request = act.map(|a| (id, a));
        } else if r.clicked() {
            self.selected = Some(id);
            self.custom_selected = None;
        }
    }

    /// One row per game (Figma "library home detail view" `375:2803`): the
    /// wide picture, the name, the main button, the facts, favorite and
    /// properties.
    fn games_list(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, items: &[Entry]) {
        let c = &t.components.library_shelf;
        let h = t.dense(126.0);
        let pad = t.spacing.sm + 2.0;
        for e in items {
            let id = e.app_id;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
            if !ui.is_rect_visible(rect) {
                ui.add_space(t.spacing.lg - ui.spacing().item_spacing.y);
                continue;
            }
            let look = t.look(c, w::interact(ui, &resp, false));
            w::paint_box(ui, t, rect, c, &look, 0.0);
            let img = Rect::from_min_size(rect.min + Vec2::splat(pad), vec2(((h - pad * 2.0) * 363.0 / 106.0).round(), h - pad * 2.0));
            w::image_or_title(ui, t, img, &art::header(id), e.name, t.corner_small());
            if e.installed.is_none() {
                ui.painter().rect_filled(img, t.corner_small(), t.overlay_dark(1));
            }
            let body = Rect::from_min_max(pos2(img.right() + t.spacing.lg + 2.0, rect.top() + t.spacing.lg), rect.max - vec2(t.spacing.lg, t.spacing.lg));
            let mut c2 = ui.new_child(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)));
            c2.add(egui::Label::new(w::heading_s(t, e.name, t.text)).truncate());
            c2.add_space((body.height() - 22.0 - 44.0 - 10.0).max(t.spacing.sm));
            c2.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.xl;
                // Favorite and properties at the right end, laid out first.
                let buttons = ui.max_rect();
                let mut right = ui.new_child(UiBuilder::new().max_rect(buttons).layout(Layout::right_to_left(Align::Center)));
                right.spacing_mut().item_spacing.x = t.spacing.sm + 2.0;
                if w::icon_box(&mut right, t, icons::GEAR, t!("Properties…"), false).clicked() {
                    self.props = Some(props::Props { app: id, tab: props::PropTab::General, ..Default::default() });
                }
                let fav = self.favorites.contains(&id);
                if w::icon_box(&mut right, t, icons::STAR, if fav { t!("Remove from favorites") } else { t!("Add to favorites") }, fav).clicked() && !self.favorites.remove(&id) {
                    self.favorites.insert(id);
                }
                let room = right.min_rect().left() - buttons.left() - t.spacing.xl;
                ui.set_max_width(room.max(0.0));
                if let Some(a) = self.main_button(ui, t, ctx, id, w::Size::Md) {
                    self.menu_request = Some((id, a));
                }
                ui.add_space(t.spacing.sm);
                ui.set_clip_rect(ui.clip_rect().intersect(Rect::from_min_max(pos2(buttons.left(), rect.top()), pos2(buttons.left() + room, rect.bottom()))));
                let last = last_played(ctx.state, e).map(ui::date).unwrap_or_else(|| t!("Never").to_string());
                icon_stat(ui, t, icons::CALENDAR, t!("Last played"), &last, None);
                let played = minutes_played(ctx.state, e).map(ui::playtime).unwrap_or_else(|| "—".into());
                icon_stat(ui, t, icons::CLOCK, t!("Play time"), &played, None);
                if let Some((got, total)) = achievements(ctx.state, id) {
                    icon_stat(ui, t, icons::TROPHY, t!("Achievements"), &format!("{got}/{total}"), Some(got as f32 / total as f32));
                }
                if let Some(g) = e.installed {
                    icon_stat(ui, t, icons::DRIVES, t!("Disk size"), &ui::bytes(g.size_on_disk), None);
                }
            });
            self.game_menu(t, &resp, id, e.installed.is_some(), ctx.state.playing == Some(id));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && self.menu_request.is_none() {
                self.selected = Some(id);
                self.custom_selected = None;
            }
            ui.add_space(t.spacing.lg - ui.spacing().item_spacing.y);
        }
    }

    /// A game's main button for a row: Play, Install, Update…; returns what
    /// to do once the frame is drawn.
    fn main_button(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, id: AppId, size: w::Size) -> Option<MenuAct> {
        let b = |ui: &mut Ui, icon: &str, label: &str, kind: w::Kind| w::button(ui, t, Some(icon), label, kind, size).clicked();
        match self.main_action(ctx, id) {
            MainAction::Play => b(ui, icons::PLAY, t!("Play"), w::Kind::Play).then_some(MenuAct::Play),
            MainAction::Running => b(ui, icons::STOP, t!("Stop"), w::Kind::Danger).then_some(MenuAct::Stop),
            MainAction::Update => b(ui, icons::DOWNLOAD, t!("Update"), w::Kind::Install).then_some(MenuAct::Update),
            MainAction::Install => b(ui, icons::DOWNLOAD, t!("Install"), w::Kind::Install).then_some(MenuAct::Install),
            MainAction::Busy { fraction, verifying, paused } => {
                let what = if verifying { t!("Verifying…") } else if paused { t!("Paused") } else { t!("Updating…") };
                b(ui, icons::DOWNLOAD, &format!("{what} {}%", (fraction * 100.0) as u32), w::Kind::Pause).then_some(MenuAct::Downloads)
            }
            MainAction::Queued => b(ui, icons::CLOCK, t!("Queued"), w::Kind::Pause).then_some(MenuAct::Downloads),
            MainAction::StorePage => b(ui, icons::STORE, t!("Store page"), w::Kind::Primary).then_some(MenuAct::Store),
            MainAction::Syncing => {
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::CLOUD_CHECK), t!("Syncing Steam Cloud…"), w::Kind::Play, size));
                None
            }
            MainAction::FirstTimeSetup => {
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::DOWNLOAD), t!("Performing first-time setup…"), w::Kind::Play, size));
                None
            }
            MainAction::Uninstalling => {
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::TRASH), t!("Uninstall"), w::Kind::Danger, size));
                None
            }
        }
    }

    /// Every collection as a block of cards or rows (Figma "library
    /// collections" `368:13794`, `375:2966`).
    fn collections_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let (state, local) = (ctx.state, ctx.local);
        Self::page_scroll(ui, t, "lib-collections", |ui| {
            let groups = self.groups(state, local);
            if groups.is_empty() {
                w::empty(ui, t, icons::STACK, t!("No collections yet"), t!("Make one with New Collection, or right-click a game."));
                ui.vertical_centered(|ui| {
                    if w::button(ui, t, Some(icons::PLUS), t!("New Collection"), w::Kind::Primary, w::Size::Md).clicked() {
                        self.naming = Some((None, String::new(), Vec::new()));
                    }
                });
                return;
            }
            for (key, title, games, _) in &groups {
                w::component_frame(t, &t.components.library_shelf).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.games_header(ui, t, title, games.len(), Some(key));
                    if !self.collapsed.contains(key) {
                        ui.add_space(t.spacing.xl);
                        if games.is_empty() {
                            ui.label(w::body_m(t, t!("No games here yet. Right-click a game to add it."), t.text_dim));
                        } else {
                            self.games(ui, t, ctx, games);
                        }
                    }
                });
                ui.add_space(t.spacing.lg + t.spacing.xs);
            }
        });
    }

    /// News from the games you play, as a wall of posts.
    fn news_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let ids = self.recent_ids(ctx);
        let (posts, loading) = self.whats_new_posts(&ids);
        let now = super::now();
        Self::page_scroll(ui, t, "lib-news", |ui| {
            ui.label(w::heading_s(t, t!("News and Updates").to_uppercase(), t.text));
            ui.add_space(t.spacing.xl);
            if posts.is_empty() {
                if loading {
                    w::loading(ui, t, t!("Loading news…"));
                } else {
                    w::empty(ui, t, icons::NOTE, t!("No news yet"), t!("Updates and events from the games you play show up here."));
                }
                return;
            }
            let card = vec2(300.0, 230.0);
            let gap = t.spacing.lg;
            let cols = (((ui.available_width() + gap) / (card.x + gap)).floor() as usize).max(1);
            let width = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
            for row in posts.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (id, index, e) in row {
                        self.news_card(ui, t, ctx, *id, *index, e, vec2(width, card.y), now);
                    }
                });
                ui.add_space(gap);
            }
        });
    }

    // ---------- a game's page ----------

    /// A game's page (Figma "library home" `324:1652`): its hero picture,
    /// the play bar, the links under it, then the cards and the news.
    pub(super) fn game_page(&mut self, id: AppId, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let all = entries(ctx.state, ctx.local);
        let Some(e) = all.iter().find(|e| e.app_id == id) else {
            self.selected = None;
            return;
        };
        let (name, installed, owned) = (e.name.to_string(), e.installed.cloned(), e.owned.cloned());
        drop(all);
        let download = ctx.state.downloads.get(&id).cloned().filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed));
        let queued = ctx.state.download_queue.contains(&id);
        if owned.is_some() && !ctx.state.achievement_list.contains_key(&id) && ctx.state.online() && self.asked_achievements.insert(id) {
            ctx.send(client::Command::LoadAchievements { app_id: id, language: i18n::language().steam_name().into() });
        }
        let side = (t.spacing.xl + t.spacing.sm + 2.0).round();
        egui::ScrollArea::vertical().id_salt(("game", id)).auto_shrink([false, false]).show(ui, |ui| {
            // The hero, fading into the page.
            let full = ui.available_width();
            let (hero, _) = ui.allocate_exact_size(vec2(full, t.dense(330.0)), Sense::hover());
            let art = Rect::from_min_size(hero.min, vec2(full, hero.height() + t.spacing.xl * 4.0));
            if !w::paint_cover_fade(ui, art, &[&props::hero(id), &art::header(id)], 0.6) {
                w::skeleton(ui, t, hero, egui::CornerRadius::ZERO);
            }
            let logo = Rect::from_min_max(hero.left_top() + vec2(side + t.spacing.xl, t.spacing.xl * 2.0), hero.left_bottom() + vec2(side + 440.0, -t.spacing.xl * 2.5));
            if !w::paint_contain(ui, logo, &props::logo(id), Align2::LEFT_BOTTOM) {
                let size = t.typography.heading_l.size * 1.5;
                ui.painter().text(logo.left_bottom(), Align2::LEFT_BOTTOM, &name, egui::FontId::new(size, t.typography.heading_l.weight.family()), t.text);
            }

            // The play bar (Figma "Group 60" `370:15515`).
            egui::Frame::new().inner_margin(Margin { left: (side - 10.0) as i8 + 18, right: side as i8, top: 0, bottom: 0 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.play_bar(ui, t, ctx, id, &name, installed.as_ref(), owned.as_ref(), download.as_ref(), queued);
            });
            ui.add_space(t.spacing.xl);

            egui::Frame::new().inner_margin(Margin::symmetric(side as i8 - 9, 0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.links(ui, t, ctx, id);
                ui.add_space(t.spacing.xl + 3.0);
                // Two columns when there's room, stacked in a narrow window
                // (the window can also be tiny for a moment, e.g. while restoring).
                let total = ui.available_width();
                let side_w = (total * 0.37).clamp(300.0, 560.0);
                if total >= 820.0 {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.xl - 3.0;
                        ui.vertical(|ui| {
                            ui.set_width(total - side_w - t.spacing.xl + 3.0);
                            self.cards(ui, t, ctx, id, installed.as_ref(), owned.as_ref());
                        });
                        ui.vertical(|ui| {
                            ui.set_width(side_w);
                            self.activity(ui, t, id);
                        });
                    });
                } else {
                    self.cards(ui, t, ctx, id, installed.as_ref(), owned.as_ref());
                    self.activity(ui, t, id);
                }
                ui.add_space(t.spacing.xl * 2.0);
            });
        });
        self.dialogs(ui, t, ctx, id, &name, installed.as_ref());
    }

    /// The name, the main button, the facts and the buttons at the right.
    #[allow(clippy::too_many_arguments)]
    fn play_bar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, name: &str, installed: Option<&InstalledGame>, owned: Option<&model::OwnedApp>, download: Option<&model::DownloadStatus>, queued: bool) {
        let full = ui.max_rect();
        let row_h = (t.height_of(&t.components.button_play, 40.0) * 1.325).round();
        // The buttons at the right end first (Figma "Frame 91": 40 px boxes, 11 apart).
        let (rr, _) = ui.allocate_exact_size(vec2(full.width(), row_h), Sense::hover());
        let mut right = ui.new_child(UiBuilder::new().max_rect(rr).layout(Layout::right_to_left(Align::Center)));
        right.spacing_mut().item_spacing.x = t.spacing.md - 1.0;
        let more = w::icon_box(&mut right, t, icons::INFO, t!("Manage"), false);
        egui::Popup::menu(&more).width(240.0).show(|ui| self.manage_menu(ui, t, ctx, id, installed));
        if w::icon_box(&mut right, t, icons::GEAR, t!("Properties…"), false).clicked() {
            self.props = Some(props::Props { app: id, tab: props::PropTab::General, ..Default::default() });
        }
        let fav = self.favorites.contains(&id);
        if w::icon_box(&mut right, t, icons::STAR, if fav { t!("Remove from favorites") } else { t!("Add to favorites") }, fav).clicked() && !self.favorites.remove(&id) {
            self.favorites.insert(id);
        }
        if w::icon_box(&mut right, t, icons::USERS, t!("Community hub"), false).clicked() {
            ctx.navigate("hub", Some(id.to_string()));
        }
        if installed.is_some() && w::icon_box(&mut right, t, icons::CONTROLLER, t!("Controller settings"), false).clicked() {
            self.props = Some(props::Props { app: id, tab: props::PropTab::Controller, ..Default::default() });
        }
        ctx.game_actions(id, &mut right);
        crate::offline_ui::cloud_pending(&mut right, t, ctx.state, id);
        let left = Rect::from_min_max(rr.min, pos2(right.min_rect().left() - t.spacing.xl, rr.bottom()));
        if left.width() < 120.0 {
            return;
        }
        let mut c = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(left.min, pos2(left.right(), left.bottom() + 400.0))).layout(Layout::left_to_right(Align::Min).with_main_wrap(true)));
        c.spacing_mut().item_spacing = vec2(t.spacing.xl + 3.0, t.spacing.md);
        c.set_clip_rect(c.clip_rect().intersect(Rect::from_min_max(left.min, pos2(left.right(), left.bottom() + 400.0))));
        let title_h = row_h;
        // Each piece asks for its own width, so the row wraps between pieces.
        let title_w = ui.painter().layout_no_wrap(name.to_string(), t.typography.heading_m.font(), t.text).size().x.min(360.0).min(left.width());
        c.allocate_ui_with_layout(vec2(title_w, title_h), Layout::left_to_right(Align::Center), |ui| {
            ui.add(egui::Label::new(w::heading_m(t, name, t.text)).truncate());
        });
        let rest = c.available_size_before_wrap().x;
        c.allocate_ui_with_layout(vec2(rest, title_h), Layout::left_to_right(Align::Center), |ui| {
            self.main_big_button(ui, t, ctx, id, installed, owned, download, queued);
        });
        let stat = |c: &mut Ui, icon: &str, label: &str, value: &str, bar: Option<f32>| {
            let width = icon_stat_width(c, t, label, value, bar.is_some());
            c.allocate_ui_with_layout(vec2(width, title_h), Layout::left_to_right(Align::Center), |ui| icon_stat(ui, t, icon, label, value, bar)).inner
        };
        if let Some(g) = installed {
            stat(&mut c, icons::CALENDAR, t!("Last played"), &g.last_played.map(ui::date).unwrap_or_else(|| t!("Never").to_string()), None);
            let (value, tip) = crate::offline_ui::playtime_value(ctx.state, id, g.playtime_minutes);
            let r = stat(&mut c, icons::CLOCK, t!("Play time"), &value, None);
            if let Some(tip) = tip {
                r.on_hover_text(tip);
            }
            if let Some(cloud) = self.cloud_status(ctx, id) {
                stat(&mut c, icons::CLOUD_CHECK, t!("Cloud status"), &cloud, None);
            }
        }
        if let Some((got, total)) = achievements(ctx.state, id) {
            stat(&mut c, icons::TROPHY, t!("Achievements"), &format!("{got}/{total}"), Some(got as f32 / total as f32));
        }
        if let Some(g) = installed {
            // Like Steam: the game and its Workshop items together.
            let workshop = local::manage::workshop_size(g);
            let r = stat(&mut c, icons::DRIVES, t!("Size"), &ui::bytes(g.size_on_disk + workshop), None);
            if workshop > 0 {
                r.on_hover_text(t!("{game} game + {workshop} Workshop items", game = ui::bytes(g.size_on_disk), workshop = ui::bytes(workshop)));
            }
            // Where the game's Steam features come from (see docs/steam-compatibility.md).
            // Worked out in the background: reading the install folder
            // would hold up the frame (the stat appears a moment later).
            let mode = owned.and_then(|o| client::launch_mode_peek(o, &g.install_dir));
            if owned.is_some() && mode.is_none() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            }
            if let Some(mode) = mode {
                let (mode, why) = match mode {
                    client::LaunchMode::SteamInRust => (t!("SteamInRust"), t!("Achievements, cloud saves and friends work through SteamInRust.").to_string()),
                    client::LaunchMode::OfficialSteam { anti_cheat } => (t!("Runs through Steam (anti-cheat)"), t!("Uses {name}: starts through the official Steam client, which opens in the background.", name = anti_cheat)),
                    client::LaunchMode::NoSteam { reason } => (t!("Starts without Steam"), i18n::tr_dyn(&reason)),
                };
                stat(&mut c, icons::LIGHTNING, t!("Steam features"), mode, None).on_hover_text(why);
            }
        } else if let Some(o) = owned {
            let size: u64 = o.depots.iter().filter(|d| d.dlc_app.is_none() && model::os_matches(&d.os, model::THIS_OS)).filter_map(|d| d.manifest("public").map(|m| m.1)).sum();
            if size > 0 {
                stat(&mut c, icons::DRIVES, t!("Download size"), &format!("≈ {}", ui::bytes(size)), None);
            }
        }
        // Which systems it has versions for, and whether this one is among them.
        if let Some(o) = owned {
            let systems = o.systems();
            if !systems.is_empty() {
                let here = client::download::os_name(model::THIS_OS);
                let names: Vec<&str> = systems.iter().map(|s| client::download::os_name(s)).collect();
                let why = if o.runs_on_this_os() {
                    t!("Has a version made for {os}: it runs here as it is.", os = here).to_string()
                } else {
                    t!("Made for {list} only: it doesn't run on {os}.", list = names.join(", "), os = here).to_string()
                };
                stat(&mut c, icons::DESKTOP, t!("Runs on"), &names.join(" · "), None).on_hover_text(why);
            }
        }
        // The wrapped facts can take more than one line.
        let used = c.min_rect().bottom() - rr.bottom();
        if used > 1.0 {
            ui.add_space(used);
        }
    }

    /// The play bar's big button, with what's going on beside it (a
    /// download, Workshop items, first-time setup…).
    #[allow(clippy::too_many_arguments)]
    fn main_big_button(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, installed: Option<&InstalledGame>, owned: Option<&model::OwnedApp>, download: Option<&model::DownloadStatus>, queued: bool) {
        let lg = w::Size::Lg;
        let playing = ctx.state.playing == Some(id);
        if let Some(d) = download {
            self.download_bar(ui, t, ctx, id, d);
            return;
        }
        if queued {
            if w::button(ui, t, Some(icons::CLOCK), t!("Queued"), w::Kind::Pause, lg).on_hover_text(t!("Open Downloads")).clicked() {
                ctx.navigate("downloads", None);
            }
            return;
        }
        match installed {
            Some(_) if self.uninstalling.contains_key(&id) => {
                let p = self.uninstalling[&id].progress.fraction();
                super::uninstall_bar(ui, t, p);
            }
            Some(_) if playing => {
                if w::button(ui, t, Some(icons::STOP), t!("Stop"), w::Kind::Danger, lg).on_hover_text(t!("Close the game")).clicked() {
                    ctx.send(client::Command::StopGame { app_id: id });
                }
            }
            Some(g) if self.needs_update(ctx, g) => {
                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Update"), w::Kind::Install, lg).clicked() {
                    self.queue_install(ctx, id, Some(g.library.clone()));
                }
            }
            // Like Steam: Workshop items the game needs download before it starts.
            Some(_) if ctx.state.workshop_preparing.contains_key(&id) => {
                let (done, total, title) = ctx.state.workshop_preparing[&id].clone();
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::DOWNLOAD), t!("Preparing to play"), w::Kind::Play, lg));
                ui.vertical(|ui| {
                    ui.label(w::body_m(t, t!("Updating Workshop items: {done} of {total}", done = done + 1, total = total), t.text));
                    w::progress(ui, t, done as f32 / total.max(1) as f32, 260.0);
                    ui.label(w::body_s(t, &title, t.text_faint));
                });
            }
            Some(_) if ctx.state.first_time_setup.contains_key(&id) => {
                // Like Steam: DirectX, Visual C++ and the game's own setup first.
                let (done, total, step) = ctx.state.first_time_setup[&id].clone();
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::DOWNLOAD), t!("Performing first-time setup…"), w::Kind::Play, lg));
                ui.vertical(|ui| {
                    ui.label(w::body_m(t, t!("Installing: {step}", step = step), t.text));
                    w::progress(ui, t, done as f32 / total.max(1) as f32, 260.0);
                    ui.label(w::body_s(t, t!("Step {n} of {total}", n = (done + 1).min(total), total = total), t.text_faint));
                });
            }
            Some(_) if ctx.state.cloud_syncing.contains(&id) => {
                // Like Steam: the game starts once its saves are in sync.
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::CLOUD_CHECK), t!("Syncing Steam Cloud…"), w::Kind::Play, lg));
                ui.add(egui::Spinner::new().color(t.accent));
            }
            Some(g) => {
                if w::play_button(ui, t, t!("Play")).clicked() {
                    self.launch_game(ctx, id, g.install_dir.clone());
                }
            }
            // Like Steam on a Mac: Windows-only games stay listed but can't be installed.
            None if owned.is_some_and(|o| !o.runs_on_this_os()) => {
                let os = client::download::os_name(model::THIS_OS);
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::DOWNLOAD), &t!("Not available on {os}", os = os), w::Kind::Install, lg))
                    .response
                    .on_disabled_hover_text(t!("This game doesn't run on {os}.", os = os));
            }
            None if owned.is_some() => {
                if w::install_button(ui, t, t!("Install")).clicked() {
                    self.install_dialog = Some(super::InstallDialog::new(id, ctx));
                }
            }
            None => {
                if w::button(ui, t, Some(icons::STORE), t!("Store page"), w::Kind::Primary, lg).clicked() {
                    ctx.navigate("store", Some(format!("app:{id}")));
                }
            }
        }
    }

    /// Steam Cloud for an installed game, when there's something to say.
    fn cloud_status(&self, ctx: &Ctx, id: AppId) -> Option<String> {
        let all_off = self.flags.as_ref().is_some_and(|f| !f.lock().unwrap().cloud);
        if all_off || self.games.get(&id).is_some_and(|s| s.cloud_off) {
            return Some(t!("Off").to_string());
        }
        if ctx.state.cloud_syncing.contains(&id) {
            return Some(t!("Syncing…").to_string());
        }
        if ctx.state.cloud_pending.contains(&id) {
            return Some(t!("Waiting to upload").to_string());
        }
        match ctx.state.cloud.get(&id) {
            Some(r) if !r.errors.is_empty() => Some(t!("Couldn't sync").to_string()),
            Some(_) => Some(t!("Up to date").to_string()),
            None => None,
        }
    }

    /// The row of links under the play bar (Figma "sorting" `523:6604`).
    fn links(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId) {
        let has_dlc = ctx.state.apps.as_ref().is_some_and(|a| a.values().any(|d| d.is_dlc() && d.parent == Some(id) && d.store_assets));
        let mut frame = w::card(t);
        frame.inner_margin = Margin::symmetric(t.spacing.xs as i8 + 1, t.spacing.xs as i8);
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
                let link = |ui: &mut Ui, label: &str| w::button(ui, t, None, label, w::Kind::Ghost, w::Size::Sm).clicked();
                if link(ui, t!("Store page")) {
                    ctx.navigate("store", Some(format!("app:{id}")));
                }
                if has_dlc && link(ui, t!("DLC")) {
                    self.scroll_to_dlc = true;
                }
                for (label, page) in [(t!("Community hub"), "hub"), (t!("Discussions"), "discussions"), (t!("Workshop"), "workshop"), (t!("Guides"), "guides"), (t!("Achievements"), "achievements")] {
                    if link(ui, label) {
                        ctx.navigate(page, Some(id.to_string()));
                    }
                }
                if link(ui, t!("Support")) {
                    ctx.navigate("support", Some(format!("game:{id}")));
                }
                // The Market has no native page yet.
                if w::button(ui, t, Some(icons::EXTERNAL), t!("Market"), w::Kind::Ghost, w::Size::Sm).on_hover_text(t!("Opens in your browser")).clicked() {
                    crate::links::open_http(&format!("https://steamcommunity.com/market/search?appid={id}"));
                }
            });
        });
    }

    /// The left column: friends, achievements, DLC, Workshop, notes, details.
    fn cards(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, installed: Option<&InstalledGame>, owned: Option<&model::OwnedApp>) {
        ui.spacing_mut().item_spacing.y = t.spacing.lg;
        // Friends playing it now.
        let friends: Vec<_> = ctx.state.friend_list().into_iter().filter(|p| p.game_app_id == Some(id)).collect();
        if !friends.is_empty() {
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(w::body_l(t, t!("Friends playing now"), t.text));
                ui.add_space(t.spacing.xs);
                ui.horizontal_wrapped(|ui| {
                    for p in friends {
                        w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 53.0, Some(t.in_game)).on_hover_text(&p.name);
                    }
                });
            });
        }
        self.achievements_card(ui, t, ctx, id);
        self.dlc_strip(ui, t, ctx, id);
        // Workshop.
        if let Some(g) = installed {
            let items = local::manage::workshop_items(g);
            if !items.is_empty() {
                w::card(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(w::body_l(t, t!("Steam Workshop"), t.text));
                    let total: u64 = items.iter().map(|i| i.size).sum();
                    ui.label(w::body_m(t, t!("{n} items · {size}", n = items.len(), size = ui::bytes(total)), t.text_dim));
                    ui.horizontal(|ui| {
                        if w::button(ui, t, None, t!("Manage"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            self.props = Some(props::Props { app: id, tab: props::PropTab::Workshop, ..Default::default() });
                        }
                        if w::button(ui, t, None, t!("Browse the Workshop"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            ctx.navigate("workshop", Some(id.to_string()));
                        }
                    });
                });
            }
        }
        // Notes.
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::body_l(t, t!("Notes"), t.text));
            let s = self.games.entry(id).or_default();
            ui.add(egui::TextEdit::multiline(&mut s.notes).hint_text(t!("Write a note for yourself…")).desired_rows(3).desired_width(f32::INFINITY));
        });
        // Details.
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::body_l(t, t!("Details"), t.text));
            ui.add_space(t.spacing.xs);
            let label = |ui: &mut Ui, s: &str| ui.label(w::caption_text(t, s));
            egui::Grid::new("game-details").num_columns(2).spacing(vec2(t.spacing.lg, t.spacing.sm - 2.0)).show(ui, |ui| {
                label(ui, t!("App ID"));
                ui.label(w::body_m(t, id.to_string(), t.text));
                ui.end_row();
                if let Some(n) = self.current_players(id) {
                    label(ui, t!("Playing now"));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.sm - 2.0;
                        let (dot, _) = ui.allocate_exact_size(Vec2::splat(t.spacing.sm), Sense::hover());
                        ui.painter().circle_filled(dot.center(), t.spacing.xs, t.play);
                        ui.label(w::body_m(t, super::super::store::thousands(n as u64), t.text));
                    })
                    .response
                    .on_hover_text(t!("People playing on Steam right now. Updates every minute."));
                    ui.end_row();
                }
                if let Some(g) = installed {
                    let (build, branch) = local::manage::installed_build(g);
                    label(ui, t!("Build"));
                    ui.label(w::body_m(t, format!("{build}{}", if branch != "public" { format!(" ({branch})") } else { String::new() }), t.text));
                    ui.end_row();
                    label(ui, t!("Last updated"));
                    ui.label(w::body_m(t, g.last_updated.map(ui::date).unwrap_or_else(|| "—".into()), t.text));
                    ui.end_row();
                }
                if let Some(o) = owned.filter(|o| !o.branches.is_empty()) {
                    label(ui, t!("Versions"));
                    ui.label(w::body_m(t, o.branches.len().to_string(), t.text));
                    ui.end_row();
                }
            });
        });
    }

    /// Your achievements and the ones still locked (Figma "Frame 136" `713:7675`).
    fn achievements_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId) {
        let Some(Ok(list)) = ctx.state.achievement_list.get(&id) else { return };
        if list.is_empty() {
            return;
        }
        let (got, locked): (Vec<&model::Achievement>, Vec<&model::Achievement>) = list.iter().partition(|a| a.unlocked);
        let resp = w::card(t)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let half = (ui.available_width() - t.spacing.xl) / 2.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = t.spacing.xl;
                    for (title, items, color, done) in [(t!("Achievements"), &got, t.text, true), (t!("Locked achievements"), &locked, t.text_dim, false)] {
                        if items.is_empty() {
                            continue;
                        }
                        ui.vertical(|ui| {
                            ui.set_width(half);
                            ui.label(w::body_l(t, title, color));
                            ui.add_space(t.spacing.xs);
                            let size = 75.0f32.min((half - t.spacing.sm * 4.0) / 5.0).max(32.0);
                            let shown = (((half + t.spacing.sm) / (size + t.spacing.sm)).floor() as usize).clamp(1, 5);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = t.spacing.sm;
                                let more = items.len().saturating_sub(shown - 1);
                                let icons_n = if more > 1 { shown - 1 } else { items.len().min(shown) };
                                for a in items.iter().take(icons_n) {
                                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
                                    let url = if done || a.icon_gray.is_empty() { &a.icon } else { &a.icon_gray };
                                    w::image_or_skeleton(ui, t, r, url, t.corner_small());
                                    if !done {
                                        ui.painter().rect_filled(r, t.corner_small(), t.overlay_dark(1));
                                    }
                                    resp.on_hover_text(format!("{}\n{}", a.name, a.description));
                                }
                                if more > 1 {
                                    let (r, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
                                    ui.painter().rect_filled(r, t.corner_small(), t.raised);
                                    ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("+{more}"), t.typography.body_l.font(), t.text);
                                }
                            });
                        });
                    }
                });
            })
            .response
            .interact(Sense::click());
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            ctx.navigate("achievements", Some(id.to_string()));
        }
    }

    /// The ids of the games played lately (the whole account's when Steam
    /// has sent it), newest first.
    pub(super) fn recent_ids(&self, ctx: &Ctx) -> Vec<AppId> {
        let mut recent: Vec<(AppId, u64)> = match ctx.state.player_games.get(&ctx.state.steam_id) {
            Some(Ok(list)) if !list.is_empty() => list.iter().filter_map(|g| Some((g.app_id, g.last_played?))).collect(),
            _ => ctx.local.games.iter().filter_map(|g| Some((g.app_id, g.last_played?))).collect(),
        };
        recent.sort_by_key(|g| std::cmp::Reverse(g.1));
        recent.into_iter().map(|g| g.0).collect()
    }
}

/// The name in a list row, cut short with "…" when it doesn't fit.
fn row_text(ui: &Ui, t: &Theme, rect: Rect, icon: Rect, name: &str, color: egui::Color32) {
    let x = icon.right() + t.spacing.sm + 2.0;
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), t.font_of(&t.components.game_row), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - x - t.spacing.lg).max(10.0));
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
}

/// A small square button (Figma's 26 px "icon box" beside a heading).
fn small_box(ui: &mut Ui, t: &Theme, icon: &str, tip: &str, selected: bool) -> egui::Response {
    let c = &t.components.icon_box;
    let size = (t.height_of(c, 40.0) * 0.65).round();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let look = t.look(c, w::interact(ui, &resp, selected));
    w::paint_box(ui, t, rect, c, &look, 0.0);
    w::icon_centered(ui.painter(), rect.center(), icon, (size * 0.7).round(), look.icon);
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A fact with its icon (Figma `340:1113`): a faint icon, the label in
/// capitals over the value, and a bar after the value when `bar` is set.
pub(super) fn icon_stat(ui: &mut Ui, t: &Theme, icon: &str, label: &str, value: &str, bar: Option<f32>) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.md;
        let s = (t.typography.body_l.size * 1.9).round();
        let (r, _) = ui.allocate_exact_size(Vec2::splat(s), Sense::hover());
        w::icon_centered(ui.painter(), r.center(), icon, (s * 0.8).round(), t.text_dim.gamma_multiply(0.4));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = t.spacing.xs;
            ui.label(w::body_m(t, label.to_uppercase(), t.text_dim));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.md;
                ui.label(w::body_l(t, value, t.text));
                if let Some(f) = bar {
                    ui.vertical(|ui| {
                        ui.add_space(t.spacing.sm);
                        w::progress(ui, t, f, 65.0);
                    });
                }
            });
        });
    })
    .response
}

/// How wide [`icon_stat`] draws.
fn icon_stat_width(ui: &Ui, t: &Theme, label: &str, value: &str, bar: bool) -> f32 {
    let p = ui.painter();
    let l = p.layout_no_wrap(label.to_uppercase(), t.typography.body_m.font(), t.text).size().x;
    let v = p.layout_no_wrap(value.to_string(), t.typography.body_l.font(), t.text).size().x + if bar { 65.0 + t.spacing.md } else { 0.0 };
    (t.typography.body_l.size * 1.9).round() + t.spacing.md + l.max(v) + 1.0
}

/// When a game was last played: this PC's record, else the account's.
fn last_played(state: &client::State, e: &Entry) -> Option<u64> {
    e.installed.and_then(|g| g.last_played).filter(|t| *t > 0).or_else(|| match state.player_games.get(&state.steam_id) {
        Some(Ok(list)) => list.iter().find(|g| g.app_id == e.app_id).and_then(|g| g.last_played),
        _ => None,
    })
}

/// Minutes played: this PC's record, else the account's.
fn minutes_played(state: &client::State, e: &Entry) -> Option<u64> {
    e.installed.and_then(|g| g.playtime_minutes).or_else(|| match state.player_games.get(&state.steam_id) {
        Some(Ok(list)) => list.iter().find(|g| g.app_id == e.app_id).map(|g| g.playtime_minutes),
        _ => None,
    })
}

/// (unlocked, total) achievements, when Steam has sent them.
fn achievements(state: &client::State, id: AppId) -> Option<(u32, u32)> {
    state
        .achievements
        .get(&id)
        .copied()
        .or_else(|| match state.achievement_list.get(&id) {
            Some(Ok(list)) if !list.is_empty() => Some((list.iter().filter(|a| a.unlocked).count() as u32, list.len() as u32)),
            _ => None,
        })
        .filter(|a| a.1 > 0)
}

/// (year, month 1–12) of a time.
fn month_of(unix: u64) -> (i64, u32) {
    let days = (unix / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m)
}

/// "October", or "October 2024" for an earlier year.
fn month_label((year, month): (i64, u32)) -> String {
    let names = [t!("January"), t!("February"), t!("March"), t!("April"), t!("May|month"), t!("June"), t!("July"), t!("August"), t!("September"), t!("October"), t!("November"), t!("December")];
    let name = names[(month.clamp(1, 12) - 1) as usize];
    if year == month_of(super::now()).0 { name.to_string() } else { format!("{name} {year}") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn months() {
        assert_eq!(super::month_of(0), (1970, 1));
        // 2024-02-29 12:00 UTC
        assert_eq!(super::month_of(1_709_208_000), (2024, 2));
        // 2023-12-31 23:59 UTC
        assert_eq!(super::month_of(1_704_067_140), (2023, 12));
    }
}

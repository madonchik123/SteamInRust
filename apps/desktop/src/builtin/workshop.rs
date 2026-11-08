//! Steam Workshop browser, per game (native, via PublishedFile.QueryFiles).

use client::{Command, Event};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Ui, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, WorkshopEntry, art};
use std::collections::HashSet;
use ui::{Theme, icons, widgets as w};

/// Sort orders, as Steam's Workshop offers them (EPublishedFileQueryType).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Trending,
    TopRated,
    MostSubscribed,
    Newest,
    Updated,
    MostPlayed,
}

impl Tab {
    fn query(self) -> (u32, u32) {
        match self {
            Tab::Trending => (3, 7),
            Tab::TopRated => (0, 0),
            Tab::MostSubscribed => (9, 0),
            Tab::Newest => (1, 0),
            Tab::Updated => (21, 0),
            Tab::MostPlayed => (14, 0),
        }
    }
}

#[derive(Default)]
pub struct Workshop {
    app: Option<AppId>,
    tab: Tab,
    page: u32,
    search: String,
    submitted: String,
    request: u64,
    loading: bool,
    items: Vec<WorkshopEntry>,
    total: u64,
    subscribed: HashSet<u64>,
    /// Trending period in days.
    period: u32,
    /// The game's own tag groups (from its Workshop page), and picked tags.
    categories: Option<Result<Vec<model::WorkshopTagCategory>, String>>,
    tags_requested: Option<AppId>,
    tags: std::collections::BTreeSet<String>,
    hide_subscribed: bool,
    /// Search text inside the open tag dropdown (dropdown index, text).
    tag_search: (usize, String),
}

impl Workshop {
    fn query(&mut self, ctx: &Ctx) {
        let Some(app) = self.app else { return };
        self.request += 1;
        self.loading = true;
        let (q, days) = self.tab.query();
        let days = if self.tab == Tab::Trending && self.period > 0 { self.period } else { days };
        let tags = self.tags.iter().cloned().collect();
        ctx.send(Command::WorkshopQuery { request_id: self.request, app_id: app, query_type: q, page: self.page.max(1), search: self.submitted.clone(), days, tags });
    }
}

impl Extension for Workshop {
    fn manifest(&self) -> Manifest {
        Manifest { id: "workshop", name: "Workshop", author: "SteamInRust", description: "Steam Workshop browser", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("workshop", "Workshop", icons::CUBE).hidden().order(950)]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        let app = arg.and_then(|a| a.parse().ok());
        if app != self.app {
            self.app = app;
            self.page = 1;
            self.items.clear();
            self.request += 1;
            self.loading = false;
            self.search.clear();
            self.submitted.clear();
            self.categories = None;
            self.tags.clear();
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut Ctx) {
        match event {
            Event::WorkshopResults { request_id, total, items } if *request_id == self.request => {
                self.items = items.clone();
                self.total = *total;

                self.loading = false;
            }
            Event::WorkshopTags { app_id, result } if Some(*app_id) == self.app => {
                self.categories = Some(result.clone());
            }
            Event::WorkshopSubscribed { file_id, result } => match result {
                Ok(()) => {
                    self.subscribed.insert(*file_id);
                    ctx.toast(t!("Subscribed. Steam will download it with the game."));
                }
                Err(e) => ctx.toast(e.clone()),
            },
            _ => {}
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        let Some(app) = self.app else {
            w::empty(ui, &t, icons::CUBE, t!("Workshop"), t!("Open a game's Workshop from its library page."));
            return;
        };
        if self.items.is_empty() && !self.loading {
            self.query(ctx);
        }
        if self.tags_requested != Some(app) {
            self.tags_requested = Some(app);
            ctx.send(Command::LoadWorkshopTags { app_id: app, language: i18n::language().steam_name().into() });
        }
        // Items already installed count as subscribed.
        if let Some(g) = ctx.local.games.iter().find(|g| g.app_id == app) {
            for i in local::manage::workshop_items(g) {
                self.subscribed.insert(i.id);
            }
        }
        let name = ctx.state.app_name(app).map(String::from).unwrap_or_default();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let full = ui.available_width();
            let (banner, _) = ui.allocate_exact_size(vec2(full, 180.0), Sense::hover());
            if !w::paint_cover(ui, banner, &art::hero(app), CornerRadius::ZERO) {
                w::gradient(ui.painter(), banner, t.accent.gamma_multiply(0.3), t.accent_2.gamma_multiply(0.2), true);
            }
            w::fade_down(ui, banner, t.bg, 0.0);
            ui.painter().text(banner.left_bottom() + vec2(32.0, -24.0), Align2::LEFT_BOTTOM, format!("{name} · {}", t!("Workshop")), w::font_bold(30.0), Color32::WHITE);

            ui.add_space(12.0);
            w::centered(ui, 1300.0, 28.0, |ui| {
                // One compact toolbar: sort, the game's own tag groups as
                // dropdowns, "hide what I have", search. Picked tags show
                // underneath as removable chips.
                let mut changed = false;
                ui.horizontal_top(|ui| {
                    let search_w = 280.0;
                    let left_w = (ui.available_width() - search_w - 16.0).max(240.0);
                    ui.allocate_ui_with_layout(vec2(left_w, 30.0), Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                        changed |= self.sort_menu(ui, &t);
                        if let Some(Ok(cats)) = &self.categories {
                            for (i, cat) in cats.clone().iter().enumerate() {
                                changed |= self.tag_menu(ui, &t, i, cat);
                            }
                        }
                        w::toggle_chip(ui, &t, t!("Hide items I have"), &mut self.hide_subscribed);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        let r = w::search(ui, &t, &mut self.search, t!("Search the Workshop"), search_w);
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.submitted = self.search.trim().to_string();
                            self.page = 1;
                            changed = true;
                        }
                    });
                });
                if !self.tags.is_empty() || !self.submitted.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        if !self.submitted.is_empty() && small_chip(ui, &t, &format!("{}  \u{201C}{}\u{201D}", icons::SEARCH, self.submitted)).clicked() {
                            self.submitted.clear();
                            self.search.clear();
                            changed = true;
                        }
                        let shown = |tag: &str| -> String {
                            match &self.categories {
                                Some(Ok(cats)) => cats.iter().flat_map(|c| c.tags.iter()).find(|(t, _)| t == tag).map(|(_, s)| s.clone()).unwrap_or_else(|| tag.to_string()),
                                _ => tag.to_string(),
                            }
                        };
                        let mut remove = None;
                        for tag in &self.tags {
                            if small_chip(ui, &t, &shown(tag)).clicked() {
                                remove = Some(tag.clone());
                            }
                        }
                        if let Some(tag) = remove {
                            self.tags.remove(&tag);
                            changed = true;
                        }
                        if self.tags.len() + usize::from(!self.submitted.is_empty()) > 1 && ui.link(w::text(t!("Clear all"), 12.5, t.accent)).clicked() {
                            self.tags.clear();
                            self.submitted.clear();
                            self.search.clear();
                            changed = true;
                        }
                    });
                }
                if changed {
                    self.page = 1;
                    self.query(ctx);
                }
                ui.add_space(14.0);
                if self.loading && self.items.is_empty() {
                    w::loading(ui, &t, t!("Loading…"));
                    return;
                }
                if self.items.is_empty() {
                    w::empty(ui, &t, icons::CUBE, t!("Nothing here yet"), t!("This game has no Workshop items that match."));
                    return;
                }
                let gap = 16.0;
                let card_w = 220.0;
                let cols = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
                let cw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
                let items: Vec<WorkshopEntry> = self.items.iter().filter(|i| !self.hide_subscribed || !self.subscribed.contains(&i.id)).cloned().collect();
                for row in items.chunks(cols) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for it in row {
                            self.card(ui, &t, ctx, app, it, cw);
                        }
                    });
                    ui.add_space(gap);
                }
                // Paging.
                let pages = self.total.div_ceil(30).max(1) as u32;
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(self.page > 1, |ui| {
                        if w::button(ui, &t, Some(icons::BACK), t!("Previous"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            self.page -= 1;
                            self.query(ctx);
                        }
                    });
                    ui.label(w::text(t!("Page {page} of {pages}", page = self.page, pages = pages), 13.0, t.text_dim));
                    ui.add_enabled_ui(self.page < pages, |ui| {
                        if w::button(ui, &t, Some(icons::CARET_RIGHT), t!("Next"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            self.page += 1;
                            self.query(ctx);
                        }
                    });
                });
                ui.add_space(30.0);
            });
        });
    }
}

impl Workshop {
    fn card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, it: &WorkshopEntry, width: f32) {
        ui.vertical(|ui| {
            ui.set_width(width);
            let (img, resp) = ui.allocate_exact_size(vec2(width, width), Sense::click());
            match &it.preview {
                Some(u) => w::image_or_skeleton(ui, t, img, u, t.corner_small()),
                None => w::placeholder(ui, t, img, &it.title, t.corner_small()),
            }
            if resp.hovered() {
                ui.painter().rect_stroke(img, t.corner_small(), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            }
            ui.add(egui::Label::new(w::text_semibold(&it.title, 14.0, t.text)).truncate());
            // Rating as five stars.
            let votes = it.votes_up + it.votes_down;
            let score = if votes == 0 { 0.0 } else { it.votes_up as f32 / votes as f32 };
            let stars = (score * 5.0).round() as usize;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                for i in 0..5 {
                    ui.label(w::text(icons::STAR, 12.0, if i < stars { t.away } else { t.border }));
                }
                ui.add_space(6.0);
                ui.label(w::text(t!("{n} subscribers", n = it.subscriptions), 11.5, t.text_faint));
            });
            ui.horizontal(|ui| {
                ui.label(w::text(ui::bytes(it.size), 11.5, t.text_faint));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.subscribed.contains(&it.id) {
                        w::pill(ui, t!("Subscribed"), t.play, t.play.gamma_multiply(0.15));
                    } else if w::button(ui, t, Some(icons::PLUS), t!("Subscribe"), w::Kind::Primary, w::Size::Sm).clicked() {
                        ctx.send(Command::WorkshopSubscribe { app_id: app, file_id: it.id });
                    }
                });
            });
            let r = resp.on_hover_text(&it.description);
            if r.clicked() {
                ctx.navigate("workshop_item", Some(it.id.to_string()));
            }
            let _ = Rect::NOTHING;
        });
    }
}

impl Workshop {
    fn sort_label(&self) -> String {
        let name = match self.tab {
            Tab::Trending => t!("Most popular"),
            Tab::TopRated => t!("Most liked"),
            Tab::MostSubscribed => t!("Most downloaded"),
            Tab::MostPlayed => t!("Most played"),
            Tab::Newest => t!("Newest"),
            Tab::Updated => t!("Recently updated"),
        };
        if self.tab != Tab::Trending {
            return name.to_string();
        }
        let period = match self.period {
            1 => t!("today"),
            30 => t!("this month"),
            90 => t!("last 3 months"),
            365 => t!("this year"),
            _ => t!("this week"),
        };
        format!("{name} · {period}")
    }

    /// Sort order (and, for "Most popular", the period) in one dropdown.
    fn sort_menu(&mut self, ui: &mut Ui, t: &Theme) -> bool {
        let r = w::chip(ui, t, &format!("{}  {}  {}", icons::SORT, self.sort_label(), icons::CARET_DOWN), false);
        let mut changed = false;
        egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(230.0).show(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let sorts = [
                (Tab::Trending, t!("Most popular")),
                (Tab::TopRated, t!("Most liked")),
                (Tab::MostSubscribed, t!("Most downloaded")),
                (Tab::MostPlayed, t!("Most played")),
                (Tab::Newest, t!("Newest")),
                (Tab::Updated, t!("Recently updated")),
            ];
            for (tab, label) in sorts {
                if w::radio(ui, t, self.tab == tab, label).clicked() && self.tab != tab {
                    self.tab = tab;
                    changed = true;
                }
            }
            if self.tab == Tab::Trending {
                ui.add_space(6.0);
                ui.label(w::text_bold(t!("PERIOD"), 10.5, t.text_faint));
                let current = if self.period == 0 { 7 } else { self.period };
                for (days, label) in [(1, t!("Today")), (7, t!("Week")), (30, t!("Month")), (90, t!("3 months")), (365, t!("Year"))] {
                    if w::radio(ui, t, current == days, label).clicked() && current != days {
                        self.period = days;
                        changed = true;
                    }
                }
            }
        });
        if changed {
            self.page = 1;
        }
        changed
    }

    /// One of the game's own tag groups as a dropdown: checkboxes (or a
    /// single choice), with a search box for long lists.
    fn tag_menu(&mut self, ui: &mut Ui, t: &Theme, index: usize, cat: &model::WorkshopTagCategory) -> bool {
        let title = if cat.name.is_empty() { t!("Tags").to_string() } else { cat.name.clone() };
        let picked: Vec<(String, String)> = cat.tags.iter().filter(|(tag, _)| self.tags.contains(tag)).cloned().collect();
        let label = match picked.as_slice() {
            [] => format!("{title}  {}", icons::CARET_DOWN),
            [one] if cat.single => format!("{title}: {}  {}", one.1, icons::CARET_DOWN),
            many => format!("{title} · {}  {}", many.len(), icons::CARET_DOWN),
        };
        let r = w::chip(ui, t, &label, !picked.is_empty());
        let mut changed = false;
        let popup = egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(260.0);
        if popup.is_open() && self.tag_search.0 != index {
            self.tag_search = (index, String::new());
        }
        popup.show(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if cat.tags.len() > 10 {
                let width = ui.available_width();
                w::search(ui, t, &mut self.tag_search.1, t!("Find"), width);
                ui.add_space(4.0);
            }
            let q = self.tag_search.1.trim().to_lowercase();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                if cat.single && q.is_empty() && w::radio(ui, t, picked.is_empty(), t!("Any")).clicked() && !picked.is_empty() {
                    for (tag, _) in &cat.tags {
                        self.tags.remove(tag);
                    }
                    changed = true;
                }
                for (tag, shown) in cat.tags.iter().filter(|(_, s)| q.is_empty() || s.to_lowercase().contains(&q)) {
                    let mut on = self.tags.contains(tag);
                    let clicked = if cat.single { w::radio(ui, t, on, shown.as_str()).clicked() } else { w::check(ui, t, &mut on, shown.as_str()).changed() };
                    if clicked {
                        if cat.single {
                            for (other, _) in &cat.tags {
                                self.tags.remove(other);
                            }
                            self.tags.insert(tag.clone());
                        } else if on {
                            self.tags.insert(tag.clone());
                        } else {
                            self.tags.remove(tag);
                        }
                        changed = true;
                    }
                }
            });
        });
        changed
    }
}

/// A small picked-filter chip with an x; clicking removes it.
fn small_chip(ui: &mut Ui, t: &Theme, label: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(format!("{label}   {}", icons::X), w::font_medium(12.0), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, 24.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(12), t.accent.gamma_multiply(0.14 + 0.08 * hover));
    p.galley(egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, w::mix(t.accent, t.text, 0.3 * hover));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Remove"))
}

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
                ui.horizontal(|ui| {
                    let sorts = [
                        (Tab::Trending, t!("Most popular")),
                        (Tab::TopRated, t!("Most liked")),
                        (Tab::MostSubscribed, t!("Most downloaded")),
                        (Tab::MostPlayed, t!("Most played")),
                        (Tab::Newest, t!("Newest")),
                        (Tab::Updated, t!("Recently updated")),
                    ];
                    for (tab, label) in sorts {
                        if w::chip(ui, &t, label, self.tab == tab && self.submitted.is_empty()).clicked() {
                            self.tab = tab;
                            self.page = 1;
                            self.submitted.clear();
                            self.search.clear();
                            self.query(ctx);
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let r = w::search(ui, &t, &mut self.search, t!("Search the Workshop"), 300.0);
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.submitted = self.search.trim().to_string();
                            self.page = 1;
                            self.query(ctx);
                        }
                    });
                });
                // Filters: trending period, tags, hide what you have.
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    if self.tab == Tab::Trending && self.submitted.is_empty() {
                        let mut period = if self.period == 0 { 7 } else { self.period };
                        let before = period;
                        w::segmented(ui, &t, &mut period, &[(1, &t!("Today")), (7, &t!("Week")), (30, &t!("Month")), (90, &t!("3 months")), (365, &t!("Year"))]);
                        if period != before {
                            self.period = period;
                            self.page = 1;
                            self.query(ctx);
                        }
                    }
                    w::toggle_chip(ui, &t, t!("Hide items I have"), &mut self.hide_subscribed);
                });
                // The game's own filter groups, like the sidebar on Steam's Workshop.
                let mut changed = false;
                if let Some(Ok(cats)) = &self.categories {
                    let cats = cats.clone();
                    ui.add_space(8.0);
                    egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(t.corner()).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for cat in &cats {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                                let title = if cat.name.is_empty() { t!("Tags").to_string() } else { cat.name.clone() };
                                ui.add_sized(vec2(130.0, 28.0), egui::Label::new(w::text_bold(title.to_uppercase(), 11.0, t.text_faint)).truncate());
                                if cat.single {
                                    let any = !cat.tags.iter().any(|(tag, _)| self.tags.contains(tag));
                                    if w::chip(ui, &t, t!("Any"), any).clicked() && !any {
                                        for (tag, _) in &cat.tags {
                                            self.tags.remove(tag);
                                        }
                                        changed = true;
                                    }
                                }
                                for (tag, shown) in cat.tags.iter().take(40) {
                                    let on = self.tags.contains(tag);
                                    if w::chip(ui, &t, shown, on).clicked() {
                                        if cat.single {
                                            for (other, _) in &cat.tags {
                                                self.tags.remove(other);
                                            }
                                        }
                                        if on {
                                            self.tags.remove(tag);
                                        } else {
                                            self.tags.insert(tag.clone());
                                        }
                                        changed = true;
                                    }
                                }
                                if cat.tags.len() > 40 {
                                    ui.label(w::text(t!("+{n} more", n = cat.tags.len() - 40), 12.0, t.text_faint));
                                }
                            });
                            ui.add_space(2.0);
                        }
                        if !self.tags.is_empty() && ui.link(w::text(t!("Clear filters"), 12.5, t.accent)).clicked() {
                            self.tags.clear();
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

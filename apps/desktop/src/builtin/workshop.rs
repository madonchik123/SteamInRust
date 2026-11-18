//! The Workshop browser, per game (native, via PublishedFile.QueryFiles),
//! inside the game's hub (Figma `512:11944`, `509:10969`).

use client::{Command, Event};
use egui::{Align, Align2, Layout, Sense, Ui, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, WorkshopEntry};
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
    /// Tag groups showing all their tags.
    expanded: HashSet<usize>,
    bar_search: String,
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

    fn page_bar(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        use crate::builtin::community_home::{BarTab, page_bar};
        page_bar(ui, ctx, BarTab::Workshop, &mut self.bar_search);
        true
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
            // Request 0 is never sent: it's `--demo`'s sample page, taken
            // while nothing else has come.
            Event::WorkshopResults { request_id, total, items } if *request_id == self.request || (*request_id == 0 && self.items.is_empty()) => {
                self.items = items.clone();
                self.total = *total;

                self.loading = false;
            }
            // Downloads of subscribed items (after subscribing, or before a launch).
            Event::WorkshopSync { result, .. } => match result {
                Ok(r) => {
                    match r.downloaded.as_slice() {
                        [] => {}
                        [(_, title)] => ctx.toast(t!("Workshop item downloaded: {title}", title = title)),
                        many => ctx.toast(t!("{n} Workshop items downloaded", n = many.len())),
                    }
                    for (_, title, why) in &r.failed {
                        ctx.toast(t!("Couldn't download {title}: {why}", title = title, why = why));
                    }
                }
                Err(e) => ctx.toast(t!("Couldn't check Workshop items: {e}", e = e)),
            },
            Event::WorkshopTags { app_id, result } if Some(*app_id) == self.app => {
                self.categories = Some(result.clone());
            }
            Event::WorkshopSubscribed { file_id, result } => match result {
                Ok(()) => {
                    self.subscribed.insert(*file_id);
                    ctx.toast(t!("Subscribed. Downloading it if the game is installed…"));
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
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            crate::builtin::hub::backdrop(ui, app);
            w::centered(ui, 1564.0, t.spacing.xl, |ui| {
                // The game's hub header, with Workshop open (Figma `512:11944`).
                if let Some(v) = crate::builtin::hub::header(ui, &t, ctx.state, app, crate::builtin::hub::HubTab::Workshop, false) {
                    let (page, arg) = v.to_page();
                    ctx.navigate(&page, arg);
                }
                let mut changed = self.sort_bar(ui, &t);
                ui.add_space(t.spacing.xl - t.spacing.xs);
                let total = ui.available_width();
                let gap = t.spacing.lg + 2.0;
                let side = if total < 820.0 { total } else { 285.0 };
                let list = |this: &mut Self, ui: &mut Ui, ctx: &mut Ctx| {
                    if this.loading && this.items.is_empty() {
                        w::loading(ui, &t, t!("Loading…"));
                        return;
                    }
                    if this.items.is_empty() {
                        w::empty(ui, &t, icons::CUBE, t!("Nothing here yet"), t!("This game has no Workshop items that match."));
                        return;
                    }
                    let items: Vec<WorkshopEntry> = this.items.iter().filter(|i| !this.hide_subscribed || !this.subscribed.contains(&i.id)).cloned().collect();
                    for it in &items {
                        this.row(ui, &t, ctx, app, it);
                    }
                    let pages = this.total.div_ceil(30).max(1) as u32;
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if let Some(p) = w::page_numbers(ui, &t, this.page, pages) {
                            this.page = p;
                            this.query(ctx);
                        }
                    });
                };
                if total < 820.0 {
                    changed |= self.filters(ui, &t);
                    ui.add_space(gap);
                    list(self, ui, ctx);
                } else {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.allocate_ui_with_layout(vec2(side, 0.0), Layout::top_down(Align::Min), |ui| {
                            ui.set_width(side);
                            changed |= self.filters(ui, &t);
                        });
                        let rest = ui.available_width();
                        ui.allocate_ui_with_layout(vec2(rest, 0.0), Layout::top_down(Align::Min), |ui| {
                            ui.set_width(rest);
                            list(self, ui, ctx);
                        });
                    });
                }
                if changed {
                    self.page = 1;
                    self.query(ctx);
                }
                ui.add_space(t.spacing.xl * 2.0);
            });
        });
    }
}

impl Workshop {
    /// "Sort by" chips and the search (Figma `512:11944`). True if either changed.
    fn sort_bar(&mut self, ui: &mut Ui, t: &Theme) -> bool {
        let mut changed = false;
        w::filter_bar(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let labels = [
                    (Tab::Trending, t!("Most Popular").to_string()),
                    (Tab::Newest, t!("Most Recent").to_string()),
                    (Tab::MostSubscribed, t!("Most Subscribed").to_string()),
                    (Tab::TopRated, t!("Top Rated").to_string()),
                    (Tab::Updated, t!("Last Updated").to_string()),
                    (Tab::MostPlayed, t!("Most played").to_string()),
                ];
                let opts: Vec<(Tab, &str)> = labels.iter().map(|(v, l)| (*v, l.as_str())).collect();
                let search_w = 374.0f32.min(ui.available_width() * 0.3).max(140.0);
                ui.allocate_ui_with_layout(vec2(ui.available_width() - search_w - t.spacing.md, 39.0), Layout::left_to_right(Align::Center), |ui| {
                    changed |= w::sort_by(ui, t, &mut self.tab, &opts);
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = w::search(ui, t, &mut self.search, t!("Search the Workshop"), search_w);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && self.search.trim() != self.submitted {
                        self.submitted = self.search.trim().to_string();
                        changed = true;
                    }
                    if self.search.is_empty() && !self.submitted.is_empty() {
                        self.submitted.clear();
                        changed = true;
                    }
                });
            });
        });
        changed
    }

    /// The filters column (Figma `512:11944`): the period for "Most
    /// Popular", the game's own tag groups, and hiding what you have.
    fn filters(&mut self, ui: &mut Ui, t: &Theme) -> bool {
        let mut changed = false;
        w::filter_bar(t).inner_margin(egui::Margin::same(t.spacing.lg as i8 - 2)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.spacing.xs;
            ui.horizontal(|ui| {
                ui.label(w::caption_text(t, &t!("Filters")));
                if !self.tags.is_empty() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::icon_button(ui, t, icons::X, &t!("Clear all"), false, 0).clicked() {
                            self.tags.clear();
                            changed = true;
                        }
                    });
                }
            });
            ui.add_space(t.spacing.sm);
            w::toggle_row(ui, t, &mut self.hide_subscribed, t!("Hide items I have"));
            if self.tab == Tab::Trending {
                ui.add_space(t.spacing.md);
                ui.label(w::body_l(t, t!("Period"), t.text));
                let current = if self.period == 0 { 7 } else { self.period };
                for (days, label) in [(1, t!("Today")), (7, t!("Week")), (30, t!("Month")), (90, t!("3 months")), (365, t!("Year"))] {
                    if filter_row(ui, t, label, None, current == days).clicked() && current != days {
                        self.period = days;
                        changed = true;
                    }
                }
            }
            let cats = match &self.categories {
                Some(Ok(c)) => c.clone(),
                _ => Vec::new(),
            };
            for (i, cat) in cats.iter().enumerate() {
                ui.add_space(t.spacing.md);
                let title = if cat.name.is_empty() { t!("Tags").to_string() } else { cat.name.clone() };
                ui.label(w::body_l(t, title, t.text));
                let open = self.expanded.contains(&i);
                let shown = if open { cat.tags.len() } else { cat.tags.len().min(6) };
                if cat.single && filter_row(ui, t, &t!("All"), None, !cat.tags.iter().any(|(tag, _)| self.tags.contains(tag))).clicked() {
                    for (tag, _) in &cat.tags {
                        self.tags.remove(tag);
                    }
                    changed = true;
                }
                for (tag, label) in cat.tags.iter().take(shown) {
                    let on = self.tags.contains(tag);
                    if filter_row(ui, t, label, None, on).clicked() {
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
                if cat.tags.len() > 6 {
                    let label = if open { t!("See less") } else { t!("See more") };
                    if ui.add(egui::Label::new(w::body_s(t, label, t.text_dim)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        if open {
                            self.expanded.remove(&i);
                        } else {
                            self.expanded.insert(i);
                        }
                    }
                }
            }
        });
        changed
    }

    /// One item (Figma `512:11944`): its picture, name, counts, rating and
    /// tags; Subscribe on the right.
    fn row(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, it: &WorkshopEntry) {
        let mut open = false;
        w::post_card(t).inner_margin(egui::Margin::same(t.spacing.lg as i8 + 3)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.lg + 3.0;
                let (img, resp) = ui.allocate_exact_size(vec2(263.0, 124.0), Sense::click());
                match &it.preview {
                    Some(u) => w::image_or_skeleton(ui, t, img, u, t.corner_small()),
                    None => w::placeholder(ui, t, img, &it.title, t.corner_small()),
                }
                open |= resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&it.description).clicked();
                let right = 200.0;
                ui.allocate_ui_with_layout(vec2((ui.available_width() - right).max(120.0), 124.0), Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = t.spacing.md;
                    let r = ui.add(egui::Label::new(w::heading_s(t, &it.title, t.accent)).truncate().sense(Sense::click()));
                    open |= r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.md - 2.0;
                        w::stat_chip(ui, t, icons::DOWNLOAD, None, Some(&group(it.subscriptions))).on_hover_text(t!("Subscribers"));
                        w::stat_chip(ui, t, icons::HEART, None, Some(&group(it.favorited))).on_hover_text(t!("Favorites"));
                        ui.add_space(t.spacing.xs);
                        let votes = it.votes_up + it.votes_down;
                        let stars = if votes == 0 { 0 } else { (it.votes_up as f32 / votes as f32 * 5.0).round() as usize };
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for i in 0..5 {
                            ui.label(w::body_l(t, icons::STAR, if i < stars { t.accent_2 } else { t.text_faint }));
                        }
                        ui.add_space(t.spacing.sm);
                        ui.label(w::body_m(t, t!("{n} ratings", n = group(votes as u64)), t.accent_2));
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.xs;
                        for tag in it.tags.iter().take(4) {
                            w::tag(ui, t, tag, None, w::TagState::Plain);
                        }
                    });
                });
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 124.0), Layout::right_to_left(Align::Max), |ui| {
                    if self.subscribed.contains(&it.id) {
                        w::button(ui, t, Some(icons::CHECK), t!("Subscribed"), w::Kind::Secondary, w::Size::Md);
                    } else if w::button(ui, t, None, t!("Subscribe"), w::Kind::Primary, w::Size::Md).clicked() {
                        ctx.send(Command::WorkshopSubscribe { app_id: app, file_id: it.id });
                    }
                });
            });
        });
        ui.add_space(t.spacing.lg - ui.spacing().item_spacing.y);
        if open {
            ctx.navigate("workshop_item", Some(it.id.to_string()));
        }
    }
}

/// 12345 -> "12,345".
fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A choice in the filters column, in the `sort` style.
fn filter_row(ui: &mut Ui, t: &Theme, label: &str, count: Option<&str>, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let c = &t.components.sort;
    let look = t.look(c, w::interact(ui, &resp, on));
    w::paint_box(ui, t, r, c, &look, if on { 1.0 } else { 0.0 });
    let pad = t.spacing.md;
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), t.typography.body_s.font(), look.text);
    job.wrap.max_width = r.width() - pad * 2.0 - 50.0;
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(r.left() + pad, r.center().y - g.size().y / 2.0), g, look.text);
    if let Some(n) = count {
        ui.painter().text(egui::pos2(r.right() - pad, r.center().y), Align2::RIGHT_CENTER, n, t.typography.body_s.font(), t.text_dim);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

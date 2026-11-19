//! The Community home (Figma `487:5076` "community home"): built around
//! what people do, in three sections:
//!
//! - **Friends activity**: your friends' screenshots, artwork, reviews,
//!   achievements, new games and posts, as a feed. Steam still keeps this
//!   feed (it's no longer linked from its client); `client::feed` reads it.
//! - **Explore**: the community-wide feed of popular and recent screenshots,
//!   artwork, broadcasts, videos, Workshop items, guides and reviews.
//! - **Developers**: news from the developers of the games you play, and
//!   announcements of games you follow.
//!
//! Also the Discussions landing (Figma `495:5839`) and the page bar shared
//! by every Community page (Home, Discussions, Workshop, Market,
//! Broadcasts). Everything here only reads.

mod demo_data;

pub use demo_data::events as demo_events;

use crate::builtin::hub::{self, post};
use client::feed::{ExploreQuery, Post, PostKind, Section};
use client::hub::{News, Request};
use client::{Command, Event, State};
use egui::{Align, Align2, Layout, Rect, Sense, Ui, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, art};
use std::collections::HashSet;
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

/// The page's id.
pub const PAGE: &str = "community_home";

/// How many days of friends' activity show at first (one page is a day).
const FIRST_DAYS: usize = 7;
/// Games whose news the Developers section reads.
const NEWS_GAMES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Part {
    #[default]
    Friends,
    Explore,
    Developers,
}

/// The tabs of the Community page bar (Figma `489:5265`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarTab {
    Home,
    Discussions,
    Workshop,
    Market,
    Broadcasts,
    /// A page of the section that isn't one of the tabs (a game's hub).
    Other,
}

/// Filters of the friends feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FriendFilter {
    #[default]
    All,
    Screenshots,
    Artwork,
    Workshop,
    Reviews,
    Achievements,
    Games,
    Posts,
}

impl FriendFilter {
    const ALL: [FriendFilter; 8] = [FriendFilter::All, FriendFilter::Screenshots, FriendFilter::Artwork, FriendFilter::Workshop, FriendFilter::Reviews, FriendFilter::Achievements, FriendFilter::Games, FriendFilter::Posts];

    fn label(self) -> String {
        match self {
            FriendFilter::All => t!("All"),
            FriendFilter::Screenshots => t!("Screenshots"),
            FriendFilter::Artwork => t!("Artwork"),
            FriendFilter::Workshop => t!("Workshop"),
            FriendFilter::Reviews => t!("Reviews"),
            FriendFilter::Achievements => t!("Achievements"),
            FriendFilter::Games => t!("New games"),
            FriendFilter::Posts => t!("Posts"),
        }
        .to_string()
    }

    fn keeps(self, p: &Post) -> bool {
        match self {
            // Game announcements belong to Developers.
            FriendFilter::All => p.kind != PostKind::Announcement,
            FriendFilter::Screenshots => matches!(p.kind, PostKind::Screenshot | PostKind::Video),
            FriendFilter::Artwork => p.kind == PostKind::Artwork,
            FriendFilter::Workshop => matches!(p.kind, PostKind::Workshop | PostKind::Guide),
            FriendFilter::Reviews => p.kind == PostKind::Review,
            FriendFilter::Achievements => p.kind == PostKind::Achievement,
            FriendFilter::Games => p.kind == PostKind::Purchase,
            FriendFilter::Posts => matches!(p.kind, PostKind::Status | PostKind::Other),
        }
    }
}

fn section_label(s: Section) -> String {
    match s {
        Section::All => t!("All"),
        Section::Screenshots => t!("Screenshots"),
        Section::Artwork => t!("Artwork"),
        Section::Broadcasts => t!("Broadcasts"),
        Section::Videos => t!("Videos"),
        Section::Workshop => t!("Workshop"),
        Section::Guides => t!("Guides"),
        Section::Reviews => t!("Reviews"),
    }
    .to_string()
}

/// Which news the Developers section shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DevFilter {
    #[default]
    All,
    Played,
    Followed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Home,
    Discussions,
}

#[derive(Default)]
pub struct CommunityHome {
    view: View,
    part: Part,
    friend_filter: FriendFilter,
    explore_section: Section,
    explore_recent: bool,
    /// Pages of the community-wide feed shown.
    explore_pages: u32,
    /// Days of friends' activity shown.
    days: usize,
    dev_filter: DevFilter,
    /// Requests already sent (until "Try again").
    asked: HashSet<String>,
    search: String,
    bar_search: String,
    /// Scroll back to the top on the next frame.
    to_top: bool,
    /// The status you're writing for your friends.
    status: StatusBox,
}

/// The "post a status" box at the top of your friends' activity, like
/// Steam's activity page has.
#[derive(Default)]
struct StatusBox {
    text: String,
    /// The game it's tagged with.
    game: Option<(AppId, String)>,
    /// Typed in the game menu's search.
    game_search: String,
    /// Waiting for Steam; the post to show once it's in.
    sending: Option<Post>,
    error: Option<String>,
    /// What you posted, at the top of the feed until Steam's feed has it.
    posted: Vec<Post>,
    /// Reading the feed again after a post: `posted` goes once it's back.
    refreshing: bool,
}

impl CommunityHome {
    fn send(&mut self, ctx: &mut Ctx, key: String, cmd: impl FnOnce() -> Command) {
        if self.asked.insert(key) {
            ctx.send(cmd());
        }
    }

    fn open(&mut self, ctx: &mut Ctx, click: post::Click) {
        match click {
            post::Click::Open(url) => match hub::route(&url) {
                Some(v) => {
                    let (page, arg) = v.to_page();
                    ctx.navigate(&page, arg);
                }
                None => {
                    crate::links::open_http(&url);
                }
            },
            post::Click::Hub(app) => ctx.navigate("hub", Some(app.to_string())),
            post::Click::Store(app) => ctx.navigate("store", Some(format!("app:{app}"))),
            post::Click::Profile(id) => ctx.navigate("profile", Some(id.to_string())),
            post::Click::News(app) => ctx.navigate("hub", Some(format!("{app}/news"))),
        }
    }
}

impl Extension for CommunityHome {
    fn manifest(&self) -> Manifest {
        Manifest { id: "community_home", name: "Community", author: "SteamInRust", description: "Friends activity, community content and developer news", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new(PAGE, "Community", icons::USERS_THREE).main().order(30)]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        self.to_top = true;
        match arg.unwrap_or_default() {
            "discussions" => self.view = View::Discussions,
            "broadcasts" => {
                self.view = View::Home;
                self.part = Part::Explore;
                self.explore_section = Section::Broadcasts;
                self.explore_pages = 1;
            }
            "explore" => (self.view, self.part) = (View::Home, Part::Explore),
            "developers" => (self.view, self.part) = (View::Home, Part::Developers),
            "friends" => (self.view, self.part) = (View::Home, Part::Friends),
            _ => self.view = View::Home,
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut Ctx) {
        match event {
            // Signed in again (or as someone else): load the feeds afresh.
            Event::LoggedIn { .. } => self.asked.clear(),
            Event::Hub(client::hub::Response::StatusPosted { result }) => {
                if self.status_answered(result) {
                    ctx.toast(t!("Status posted"));
                }
            }
            Event::Hub(client::hub::Response::Activity { start: 0, result: Ok(_) }) if self.status.refreshing => {
                self.status.refreshing = false;
                self.status.posted.clear();
            }
            _ => {}
        }
    }

    fn page_bar(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        let tab = match (self.view, self.part, self.explore_section) {
            (View::Discussions, ..) => BarTab::Discussions,
            (View::Home, Part::Explore, Section::Broadcasts) => BarTab::Broadcasts,
            _ => BarTab::Home,
        };
        if let Some(q) = page_bar(ui, ctx, tab, &mut self.bar_search) {
            self.search = q;
        }
        true
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        if self.days == 0 {
            self.days = FIRST_DAYS;
        }
        if self.explore_pages == 0 {
            self.explore_pages = 1;
        }
        let mut scroll = egui::ScrollArea::vertical().id_salt(("community-home", self.view as u8)).auto_shrink([false, false]);
        if std::mem::take(&mut self.to_top) {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| match self.view {
            View::Home => self.home(ui, &t, ctx),
            View::Discussions => self.discussions(ui, &t, ctx),
        });
    }
}

// ---------- the page bar ----------

/// The page bar of every Community page: Home, Discussions, Workshop,
/// Market, Broadcasts, and a search for a game's community. Returns a
/// search to run (Enter in the box).
pub fn page_bar(ui: &mut Ui, ctx: &mut Ctx, current: BarTab, search: &mut String) -> Option<String> {
    let t = ctx.theme.clone();
    ui.spacing_mut().item_spacing.x = 7.0;
    let tabs = [
        (BarTab::Home, t!("Home"), PAGE, None),
        (BarTab::Discussions, t!("Discussions"), PAGE, Some("discussions")),
        (BarTab::Workshop, t!("Workshop"), "workshop", None),
        (BarTab::Market, t!("Market"), "inventory", Some("market")),
        (BarTab::Broadcasts, t!("Broadcasts"), PAGE, Some("broadcasts")),
    ];
    for (tab, label, page, arg) in tabs {
        if w::sub_tab(ui, &t, &label, current == tab).clicked() {
            ctx.navigate(page, arg.map(String::from));
        }
    }
    ui.add_space(30.0);
    let width = (ui.available_width() - 40.0).clamp(160.0, 581.0);
    let r = w::bar_search(ui, &t, search, t!("Search for a game's community…"), width);
    let mut out = None;
    if r.has_focus() || !search.trim().is_empty() {
        if let Some(app) = game_matches(ctx.state, search, 8).first().map(|g| g.0).filter(|_| r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            ctx.navigate("hub", Some(app.to_string()));
            search.clear();
        } else if r.has_focus() {
            out = Some(search.clone());
        }
    }
    if r.has_focus() && !search.trim().is_empty() {
        let matches = game_matches(ctx.state, search, 8);
        let id = egui::Id::unique("community-bar-search");
        egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(r.rect.left_bottom() + vec2(0.0, 4.0)).show(ui.ctx(), |ui| {
            w::component_frame(&t, &t.components.menu).show(ui, |ui| {
                ui.set_width(r.rect.width() - 12.0);
                if matches.is_empty() {
                    ui.label(w::body_m(&t, t!("No games found"), t.text_dim));
                }
                for (app, name) in matches {
                    if w::menu_item(ui, &t, icons::USERS_THREE, &name).clicked() {
                        ctx.navigate("hub", Some(app.to_string()));
                        search.clear();
                    }
                }
            });
        });
    }
    out
}

/// Games whose name has `query`: yours first (most played), then the rest
/// of your library.
fn game_matches(state: &State, query: &str, max: usize) -> Vec<(AppId, String)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(AppId, String)> = Vec::new();
    for (app, name) in my_games(state).into_iter() {
        if name.to_lowercase().contains(&q) && !out.iter().any(|(a, _)| *a == app) {
            out.push((app, name));
        }
    }
    if let Some(apps) = &state.apps {
        let mut rest: Vec<_> = apps.values().filter(|a| !a.is_dlc() && a.name.to_lowercase().contains(&q)).map(|a| (a.app_id, a.name.clone())).collect();
        rest.sort_by(|a, b| a.1.cmp(&b.1));
        for g in rest {
            if !out.iter().any(|(a, _)| *a == g.0) {
                out.push(g);
            }
        }
    }
    out.truncate(max);
    out
}

/// Your games, most played recently first: Steam's list when it came,
/// else your library.
fn my_games(state: &State) -> Vec<(AppId, String)> {
    if let Some(Ok(list)) = state.player_games.get(&state.steam_id) {
        if !list.is_empty() {
            let mut list = list.clone();
            list.sort_by_key(|g| (std::cmp::Reverse(g.playtime_2weeks), std::cmp::Reverse(g.last_played.unwrap_or(0)), std::cmp::Reverse(g.playtime_minutes)));
            return list.into_iter().map(|g| (g.app_id, g.name)).collect();
        }
    }
    let mut apps: Vec<(AppId, String)> = state.apps.as_ref().map(|a| a.values().filter(|a| !a.is_dlc() && a.kind == "game").map(|a| (a.app_id, a.name.clone())).collect()).unwrap_or_default();
    apps.sort_by(|a, b| a.1.cmp(&b.1));
    apps
}

// ---------- home ----------

impl CommunityHome {
    fn home(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        self.hero(ui, t, ctx);
        let pad = t.spacing.xl;
        w::centered(ui, 1564.0, pad, |ui| {
            ui.spacing_mut().item_spacing.y = t.spacing.xl - t.spacing.xs;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                for (part, label) in [(Part::Friends, t!("Friends activity")), (Part::Explore, t!("Explore")), (Part::Developers, t!("Developers"))] {
                    if w::sub_tab(ui, t, &label, self.part == part).clicked() {
                        self.part = part;
                    }
                }
            });
            match self.part {
                Part::Friends => self.friends(ui, t, ctx),
                Part::Explore => self.explore(ui, t, ctx),
                Part::Developers => self.developers(ui, t, ctx),
            }
            ui.add_space(t.spacing.xl * 2.0);
        });
    }

    /// The welcome, over a soft collage of your games' art (Figma
    /// `494:5450`), and a search for a game's community.
    fn hero(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let full = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(full, 300.0), Sense::hover());
        let games: Vec<AppId> = my_games(ctx.state).into_iter().map(|g| g.0).take(4).collect();
        let pieces = [(0.0, 0.0, 0.37, 0.9), (0.34, 0.0, 0.36, 0.8), (0.57, 0.0, 0.28, 0.96), (0.83, 0.0, 0.2, 0.87)];
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        // Soft, like the design's art at a third behind the welcome.
        for (i, app) in games.iter().enumerate() {
            let (x, y, wf, hf) = pieces[i];
            let r = Rect::from_min_size(rect.min + vec2(rect.width() * x, rect.height() * y), vec2(rect.width() * wf, rect.height() * hf)).intersect(rect);
            w::paint_cover_fading(ui, r, &art::hero(*app), 0.22);
        }
        let title = t!("Welcome to the Community. Search for a game to get started.");
        painter.text(egui::pos2(rect.center().x, rect.top() + 92.0), Align2::CENTER_CENTER, title, t.typography.heading_m.font(), t.text);
        let sw = 514.0f32.min(full - 40.0).max(120.0);
        let sr = Rect::from_min_size(egui::pos2(rect.center().x - sw / 2.0, rect.top() + 140.0), vec2(sw, 55.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(sr).layout(Layout::left_to_right(Align::Center)));
        let r = w::search_large(&mut child, t, &mut self.search, t!("Search…"), sw);
        if !self.search.trim().is_empty() {
            let matches = game_matches(ctx.state, &self.search, 6);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter {
                if let Some((app, _)) = matches.first() {
                    ctx.navigate("hub", Some(app.to_string()));
                    self.search.clear();
                }
            }
            let id = egui::Id::unique("community-hero-search");
            egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(r.rect.left_bottom() + vec2(0.0, 4.0)).show(ui.ctx(), |ui| {
                w::component_frame(t, &t.components.menu).show(ui, |ui| {
                    ui.set_width(sw - 12.0);
                    if matches.is_empty() {
                        ui.label(w::body_m(t, t!("No games found"), t.text_dim));
                    }
                    for (app, name) in &matches {
                        if w::menu_item(ui, t, icons::USERS_THREE, name).clicked() {
                            ctx.navigate("hub", Some(app.to_string()));
                            self.search.clear();
                        }
                    }
                });
            });
        }
    }

    /// A row of filter chips in the filter bar, and optionally a "Viewing"
    /// sort on the right.
    fn filter_row<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, String)], right: impl FnOnce(&mut Ui)) -> bool {
        let mut changed = false;
        w::filter_bar(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.xs + 1.0, t.spacing.xs + 1.0);
                for (opt, label) in options {
                    if w::sort_chip(ui, t, label, *value == *opt).clicked() && *value != *opt {
                        *value = *opt;
                        changed = true;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), right);
            });
        });
        changed
    }

    // ----- friends -----

    fn friends(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if ctx.state.steam_id != 0 {
            self.status_box(ui, t, ctx);
        }
        let options: Vec<(FriendFilter, String)> = FriendFilter::ALL.iter().map(|f| (*f, f.label())).collect();
        Self::filter_row(ui, t, &mut self.friend_filter, &options, |_| {});
        if ctx.state.steam_id == 0 {
            w::empty(ui, t, icons::USERS, t!("Sign in to see your friends"), t!("Your friends' screenshots, reviews and achievements show here."));
            return;
        }
        // What you just posted, on top until Steam's feed has it.
        let mine: Vec<Post> = self.status.posted.iter().rev().filter(|p| self.friend_filter.keeps(p)).cloned().collect();
        // Day pages, newest first, until we have the days asked for.
        let lang = i18n::language().steam_name().to_string();
        let mut posts: Vec<&Post> = mine.iter().collect();
        let mut seen = HashSet::new();
        let mut start = 0u64;
        let mut loading = false;
        let mut failed: Option<String> = None;
        let mut more = true;
        let mut shown_days = 0;
        let state = ctx.state;
        while shown_days < self.days {
            match state.hub.activity.get(&start) {
                None => {
                    let l = lang.clone();
                    self.send(ctx, format!("activity:{start}"), || Command::Hub(Request::Activity { start, language: l }));
                    loading = true;
                    break;
                }
                Some(Err(e)) => {
                    failed = Some(e.clone());
                    break;
                }
                Some(Ok(feed)) => {
                    // Steam can list the same event on two days' pages: once here.
                    for p in feed.posts.iter().filter(|p| self.friend_filter.keeps(p)) {
                        if seen.insert(post::key(p)) {
                            posts.push(p);
                        }
                    }
                    shown_days += 1;
                    if feed.next == 0 {
                        more = false;
                        break;
                    }
                    start = feed.next;
                }
            }
        }
        // A quiet week: keep reading back a little further on its own.
        if !loading && failed.is_none() && more && posts.len() < 6 && self.days < 30 && shown_days >= self.days {
            self.days += FIRST_DAYS;
            ui.ctx().request_repaint();
        }
        if posts.is_empty() {
            if let Some(e) = failed {
                if error(ui, t, &e) {
                    self.asked.retain(|k| !k.starts_with("activity:"));
                }
            } else if loading {
                skeletons(ui, t);
            } else {
                w::empty(ui, t, icons::USERS, t!("Nothing new from your friends"), t!("When your friends share screenshots, write reviews or unlock achievements, it shows here."));
            }
            return;
        }
        if let Some(c) = post::masonry(ui, t, state, &posts, "friends", true) {
            self.open(ctx, c);
        }
        if loading {
            w::loading(ui, t, &t!("Loading…"));
        } else if let Some(e) = failed {
            if error(ui, t, &e) {
                self.asked.retain(|k| !k.starts_with("activity:"));
            }
        } else if more {
            ui.vertical_centered(|ui| {
                if w::button(ui, t, Some(icons::CARET_DOWN), t!("Show older activity"), w::Kind::Secondary, w::Size::Md).clicked() {
                    self.days += FIRST_DAYS;
                }
            });
        }
    }

    /// "Post a status for your friends…": your avatar, the text, a game to
    /// tag it with, and Post (Steam's activity page has the same).
    fn status_box(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.persona(ctx.state.steam_id);
        let sending = self.status.sending.is_some();
        let mut submit = false;
        w::post_card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.md;
                w::avatar(ui, t, &art::avatar(me.avatar_hash.as_deref()), 40.0, None);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = t.spacing.sm;
                    let input = &t.components.input;
                    w::component_frame(t, input).inner_margin(egui::Margin::symmetric(t.padding_of(input).x as i8, t.spacing.sm as i8 + 2)).show(ui, |ui| {
                        // Ctrl+Enter posts, like sending a message (taken
                        // before the field would make it a new line).
                        let id = egui::Id::unique("community-status-text");
                        if ui.memory(|m| m.has_focus(id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
                            submit = true;
                        }
                        ui.add_enabled(
                            !sending,
                            egui::TextEdit::multiline(&mut self.status.text)
                                .id(id)
                                .hint_text(w::body_m(t, t!("Post a status for your friends…"), t.text_dim))
                                .font(t.typography.body_m.font())
                                .text_color(t.text)
                                .frame(egui::Frame::NONE)
                                .desired_rows(2)
                                .char_limit(1000)
                                .desired_width(f32::INFINITY),
                        )
                        .with_edit_menu();
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.sm;
                        self.game_menu(ui, t, ctx.state);
                        ui.label(w::body_s(t, t!("Your friends see what you post here."), t.text_faint));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let ready = !self.status.text.trim().is_empty() && !sending;
                            let label = if sending { t!("Posting…") } else { t!("Post|status") };
                            let posted = ui.add_enabled_ui(ready, |ui| w::button(ui, t, Some(icons::SEND), label, w::Kind::Primary, w::Size::Md).clicked()).inner;
                            submit |= posted;
                        });
                    });
                    if let Some(e) = &self.status.error {
                        ui.label(w::body_s(t, e, t.danger));
                    }
                });
            });
        });
        if submit {
            self.post_status(ctx);
        }
    }

    /// "Tag with game": your games, with a search.
    fn game_menu(&mut self, ui: &mut Ui, t: &Theme, state: &State) {
        let label = match &self.status.game {
            Some((_, name)) => name.clone(),
            None => t!("Tag with game").to_string(),
        };
        let r = w::dropdown(ui, t, &label, 180.0);
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(280.0).show(|ui| {
            let width = ui.available_width();
            w::search(ui, t, &mut self.status.game_search, t!("Find a game"), width);
            ui.add_space(t.spacing.xs);
            if w::menu_item(ui, t, icons::X, t!("No game")).clicked() {
                self.status.game = None;
                ui.close();
            }
            let q = self.status.game_search.trim().to_lowercase();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (app, name) in my_games(state).into_iter().filter(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q)).take(40) {
                    if w::menu_item(ui, t, icons::CONTROLLER, &name).clicked() {
                        self.status.game = Some((app, name));
                        ui.close();
                    }
                }
            });
        });
    }

    /// Send the status (in `--demo`, it's only added here).
    fn post_status(&mut self, ctx: &mut Ctx) {
        let demo = std::env::args().any(|a| a == "--demo");
        if let Some(req) = self.submit(ctx.state, demo) {
            ctx.send(Command::Hub(req));
        }
    }

    /// Take the status as written: in the demo it's posted here at once;
    /// else it waits for Steam and the request to send comes back.
    fn submit(&mut self, state: &State, demo: bool) -> Option<Request> {
        let text = self.status.text.trim().to_string();
        if text.is_empty() || self.status.sending.is_some() {
            return None;
        }
        let me = state.persona(state.steam_id);
        let (app_id, game) = self.status.game.clone().unwrap_or_default();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let post = Post {
            kind: PostKind::Status,
            author: state.steam_id,
            author_name: me.name.clone(),
            avatar: Some(art::avatar(me.avatar_hash.as_deref())),
            app_id,
            game: game.clone(),
            action: if app_id != 0 { t!("posted a status about {game}", game = game) } else { String::new() },
            text: text.clone(),
            when: t!("Today").to_string(),
            time: now,
            ..Default::default()
        };
        self.status.error = None;
        if demo {
            self.status.posted.push(post);
            self.status.text.clear();
            self.status.game = None;
            return None;
        }
        self.status.sending = Some(post);
        Some(Request::PostStatus { text, app_id })
    }

    /// Steam answered a posted status. True when it's in.
    fn status_answered(&mut self, result: &Result<(), String>) -> bool {
        let post = self.status.sending.take();
        match result {
            Ok(()) => {
                self.status.posted.extend(post);
                self.status.text.clear();
                self.status.game = None;
                self.status.error = None;
                // Steam's feed has it now: read it again.
                self.status.refreshing = true;
                self.asked.retain(|k| !k.starts_with("activity:"));
                true
            }
            Err(e) => {
                self.status.error = Some(t!("Couldn't post your status: {error}", error = i18n::tr_dyn(e)));
                false
            }
        }
    }

    // ----- explore -----

    fn explore(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let options: Vec<(Section, String)> = Section::ALL.iter().map(|s| (*s, section_label(*s))).collect();
        let mut recent = self.explore_recent;
        let changed = Self::filter_row(ui, t, &mut self.explore_section, &options, |ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.xs + 1.0;
            // Right to left: the last one first.
            if w::sort_chip(ui, t, &t!("Most Recent"), recent).clicked() {
                recent = true;
            }
            if w::sort_chip(ui, t, &t!("Most Popular"), !recent).clicked() {
                recent = false;
            }
            ui.add_space(t.spacing.sm);
            ui.label(w::body_l(t, t!("Viewing"), t.text_dim));
        });
        if changed || recent != self.explore_recent {
            self.explore_recent = recent;
            self.explore_pages = 1;
        }
        let lang = i18n::language().steam_name().to_string();
        let state = ctx.state;
        let mut posts: Vec<&Post> = Vec::new();
        let mut loading = false;
        let mut failed = None;
        let mut last_full = false;
        for page in 1..=self.explore_pages {
            let q = ExploreQuery { section: self.explore_section, recent: self.explore_recent, page, language: lang.clone(), ..Default::default() };
            match state.hub.explore.get(&q.key()) {
                None => {
                    self.send(ctx, format!("explore:{}", q.key()), || Command::Hub(Request::Explore(q)));
                    loading = true;
                    break;
                }
                Some(Err(e)) => {
                    failed = Some(e.clone());
                    break;
                }
                Some(Ok(feed)) => {
                    last_full = !feed.posts.is_empty();
                    // The same post can come back on the next page.
                    for p in &feed.posts {
                        if !posts.iter().any(|x| x.url == p.url && x.file_id == p.file_id) {
                            posts.push(p);
                        }
                    }
                }
            }
        }
        if posts.is_empty() {
            if let Some(e) = failed {
                if error(ui, t, &e) {
                    self.asked.retain(|k| !k.starts_with("explore:"));
                }
            } else if loading {
                skeletons(ui, t);
            } else if self.explore_section == Section::Broadcasts {
                w::empty(ui, t, icons::MONITOR, t!("No one is broadcasting right now"), t!("Check back later."));
            } else {
                w::empty(ui, t, icons::USERS_THREE, t!("Nothing here yet"), t!("Check back later."));
            }
            return;
        }
        if let Some(c) = post::masonry(ui, t, state, &posts, "explore", true) {
            self.open(ctx, c);
        }
        if loading {
            w::loading(ui, t, &t!("Loading…"));
        } else if failed.is_none() && last_full && self.explore_pages < 10 {
            ui.vertical_centered(|ui| {
                if w::button(ui, t, Some(icons::CARET_DOWN), t!("See more"), w::Kind::Secondary, w::Size::Md).clicked() {
                    self.explore_pages += 1;
                }
            });
        }
    }

    // ----- developers -----

    fn developers(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let options = [(DevFilter::All, t!("All").to_string()), (DevFilter::Played, t!("Games you play").to_string()), (DevFilter::Followed, t!("Games you follow").to_string())];
        Self::filter_row(ui, t, &mut self.dev_filter, &options, |_| {});
        let state = ctx.state;
        if state.steam_id == 0 {
            w::empty(ui, t, hub::NEWSPAPER, t!("Sign in to see news"), t!("News from the developers of your games shows here."));
            return;
        }
        if !state.player_games.contains_key(&state.steam_id) {
            let (me, lang) = (state.steam_id, i18n::language().steam_name().to_string());
            self.send(ctx, "player-games".into(), || Command::LoadPlayerGames { steam_id: me, language: lang });
        }
        let mut news: Vec<Post> = Vec::new();
        let mut loading = false;
        if self.dev_filter != DevFilter::Followed {
            for (app, name) in my_games(state).into_iter().take(NEWS_GAMES) {
                match state.hub.news.get(&app) {
                    None => {
                        self.send(ctx, format!("news:{app}"), || Command::Hub(Request::News { app_id: app, count: 3 }));
                        loading = true;
                    }
                    Some(Ok(list)) => news.extend(list.iter().map(|n| news_post(app, &name, n))),
                    Some(Err(_)) => {}
                }
            }
        }
        if self.dev_filter != DevFilter::Played {
            // Announcements of games you follow come in your activity feed.
            let lang = i18n::language().steam_name().to_string();
            match state.hub.activity.get(&0) {
                None => {
                    self.send(ctx, "activity:0".into(), || Command::Hub(Request::Activity { start: 0, language: lang }));
                    loading = true;
                }
                Some(Ok(_)) => {
                    for feed in state.hub.activity.values().flatten() {
                        for p in feed.posts.iter().filter(|p| p.kind == PostKind::Announcement) {
                            if !news.iter().any(|n| n.url == p.url) {
                                news.push(p.clone());
                            }
                        }
                    }
                }
                Some(Err(_)) => {}
            }
        }
        news.sort_by_key(|p| std::cmp::Reverse(p.time));
        if news.is_empty() {
            if loading {
                skeletons(ui, t);
            } else {
                w::empty(ui, t, hub::NEWSPAPER, t!("No news yet"), t!("News from the developers of your games shows here."));
            }
            return;
        }
        let refs: Vec<&Post> = news.iter().take(40).collect();
        if let Some(c) = post::masonry(ui, t, state, &refs, "developers", true) {
            self.open(ctx, c);
        }
        if loading {
            w::loading(ui, t, &t!("Loading…"));
        }
    }

    // ----- discussions -----

    /// The Discussions landing (Figma `495:5839`): your games' forums.
    fn discussions(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.add_space(t.spacing.xl * 2.5);
        ui.vertical_centered(|ui| {
            ui.label(w::heading_m(t, t!("Welcome to the discussions"), t.text));
            ui.add_space(t.spacing.md);
            ui.label(w::styled(t!("Pick a forum to start the conversation."), t.typography.button_large, t.text_dim));
        });
        ui.add_space(t.spacing.xl * 2.0);
        let state = ctx.state;
        if state.steam_id != 0 && !state.player_games.contains_key(&state.steam_id) {
            let (me, lang) = (state.steam_id, i18n::language().steam_name().to_string());
            self.send(ctx, "player-games".into(), || Command::LoadPlayerGames { steam_id: me, language: lang });
        }
        let games = my_games(state);
        w::centered(ui, 1564.0, t.spacing.xl, |ui| {
            ui.label(w::heading_s(t, t!("Your game forums"), t.text));
            ui.add_space(t.spacing.sm);
            if games.is_empty() {
                w::empty(ui, t, icons::CHATS, t!("No games yet"), t!("The forums of the games you own show here."));
                return;
            }
            let gap = t.spacing.xl - t.spacing.xs;
            let cols = ((ui.available_width() + gap) / (360.0 + gap)).floor().clamp(1.0, 4.0) as usize;
            let cw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
            for row in games.iter().take(24).collect::<Vec<_>>().chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (app, name) in row {
                        if forum_tile(ui, t, state, *app, name, cw).clicked() {
                            ctx.navigate("discussions", Some(app.to_string()));
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            ui.add_space(t.spacing.xl * 2.0);
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = t.spacing.md;
                ui.label(w::heading_s(t, t!("About discussions"), t.text_dim));
                for (bold, rest) in [
                    (t!("Everyone is invited!"), t!("The discussions are for everyone, new and advanced players alike.")),
                    (t!("Searching is key!"), t!("Before you post a question, search the forum to see whether it has been answered.")),
                    (t!("Be kind."), t!("If someone is spamming or harassing, report the post and let the moderators take a look.")),
                ] {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.xs;
                        ui.label(w::styled(bold, ui::theme::TextStyle::new(t.typography.body_l.size, ui::theme::Weight::Bold), t.text));
                        ui.label(w::body_l(t, rest, t.text));
                    });
                }
            });
        });
        ui.add_space(t.spacing.xl * 2.0);
    }
}

/// A forum in the Discussions landing: the game's icon, name and a caret.
fn forum_tile(ui: &mut Ui, t: &Theme, state: &State, app: AppId, name: &str, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 72.0), Sense::click());
    let c = &t.components.card;
    let look = t.look(c, w::interact(ui, &resp, false));
    let hover_fill = if resp.hovered() { t.surface_hover } else { w::solid_fill(t, c) };
    ui.painter().rect_filled(rect, t.corner(), hover_fill);
    let _ = look;
    let icon = Rect::from_center_size(egui::pos2(rect.left() + 20.0 + 16.0, rect.center().y), vec2(32.0, 32.0));
    let hash = state.apps.as_ref().and_then(|a| a.get(&app)).and_then(|o| o.icon.clone());
    let drawn = hash.is_some_and(|h| w::paint_cover(ui, icon, &art::icon(app, &h), t.corner_small()));
    if !drawn && !w::paint_cover(ui, icon, &art::capsule(app), t.corner_small()) {
        ui.painter().rect_filled(icon, t.corner_small(), t.raised);
    }
    let text_rect = Rect::from_min_max(egui::pos2(icon.right() + 18.0, rect.top()), egui::pos2(rect.right() - 40.0, rect.bottom()));
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), t.typography.body_l.font(), t.text);
    job.wrap.max_width = text_rect.width();
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(text_rect.left(), rect.center().y - g.size().y / 2.0), g, t.text);
    w::icon_centered(ui.painter(), egui::pos2(rect.right() - 22.0, rect.center().y), icons::CARET_RIGHT, 16.0, t.text_dim);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A game's news post as a feed post. Opens in the game's hub.
fn news_post(app: AppId, game: &str, n: &News) -> Post {
    let doc = hub::rich::parse(&n.contents);
    let image = doc.blocks.iter().find_map(|b| if let hub::rich::Block::Image(u) = b { Some(u.clone()) } else { None });
    // The words of the post's paragraphs (not its pictures' links).
    let plain: String = doc
        .blocks
        .iter()
        .filter_map(|b| if let hub::rich::Block::Para(spans) = b { Some(spans.iter().map(|s| s.text.as_str()).collect::<String>()) } else { None })
        .filter(|p| !p.trim().is_empty())
        .collect::<Vec<_>>()
        .join("
");
    Post {
        kind: PostKind::Announcement,
        app_id: app,
        game: game.to_string(),
        author_name: if n.author.is_empty() { n.feed.clone() } else { n.author.clone() },
        title: n.title.clone(),
        text: plain.chars().take(400).collect(),
        images: vec![image.unwrap_or_else(|| art::header(app))],
        url: n.url.clone(),
        time: n.time,
        ..Default::default()
    }
}

/// Grey boxes while the first posts load.
fn skeletons(ui: &mut Ui, t: &Theme) {
    let gap = t.spacing.xl - t.spacing.xs;
    let w2 = (ui.available_width() - gap) / 2.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for h in [420.0, 300.0] {
            let (r, _) = ui.allocate_exact_size(vec2(w2, h), Sense::hover());
            w::skeleton(ui, t, r, t.corner());
        }
    });
}

/// An error and "Try again"; true when clicked.
fn error(ui: &mut Ui, t: &Theme, msg: &str) -> bool {
    let mut again = false;
    ui.vertical_centered(|ui| {
        ui.add_space(t.spacing.xl * 2.0);
        ui.label(w::text(icons::WARNING, 34.0, t.text_faint));
        ui.label(w::body_m(t, i18n::tr_dyn(msg), t.text_dim));
        again = w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Sm).clicked();
    });
    again
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in() -> State {
        let mut state = State::default();
        state.steam_id = 76561197960265729;
        state
    }

    #[test]
    fn a_demo_status_shows_at_once() {
        let mut home = CommunityHome::default();
        home.status.text = "  one more round?  ".into();
        home.status.game = Some((570, "Dota 2".into()));
        assert!(home.submit(&signed_in(), true).is_none());
        assert_eq!(home.status.posted.len(), 1);
        let p = &home.status.posted[0];
        assert_eq!((p.kind, p.app_id, p.text.as_str(), p.author), (PostKind::Status, 570, "one more round?", 76561197960265729));
        assert!(home.status.text.is_empty() && home.status.game.is_none());
        // Nothing to post: nothing happens.
        assert!(home.submit(&signed_in(), true).is_none());
        assert_eq!(home.status.posted.len(), 1);
    }

    #[test]
    fn a_status_waits_for_steam() {
        let mut home = CommunityHome::default();
        home.status.text = "gg".into();
        match home.submit(&signed_in(), false) {
            Some(Request::PostStatus { text, app_id }) => assert_eq!((text.as_str(), app_id), ("gg", 0)),
            other => panic!("expected a post, got {other:?}"),
        }
        // Sent once, kept until Steam answers.
        assert!(home.submit(&signed_in(), false).is_none());
        assert!(home.status.posted.is_empty() && home.status.text == "gg");
        // A refusal keeps the text and says why.
        assert!(!home.status_answered(&Err("You've been posting too frequently".into())));
        assert_eq!(home.status.text, "gg");
        assert!(home.status.error.is_some());
        // Then it goes through: on top of the feed, the feed read again.
        home.asked.insert("activity:0".into());
        assert!(home.submit(&signed_in(), false).is_some());
        assert!(home.status_answered(&Ok(())));
        assert_eq!(home.status.posted.len(), 1);
        assert!(home.status.text.is_empty() && home.status.refreshing);
        assert!(!home.asked.contains("activity:0"));
    }

    #[test]
    fn friend_filters_sort_posts() {
        let p = |kind| Post { kind, ..Default::default() };
        assert!(FriendFilter::All.keeps(&p(PostKind::Review)));
        assert!(!FriendFilter::All.keeps(&p(PostKind::Announcement)));
        assert!(FriendFilter::Screenshots.keeps(&p(PostKind::Screenshot)));
        assert!(!FriendFilter::Screenshots.keeps(&p(PostKind::Artwork)));
        assert!(FriendFilter::Workshop.keeps(&p(PostKind::Guide)));
        assert!(FriendFilter::Games.keeps(&p(PostKind::Purchase)));
        assert!(FriendFilter::Posts.keeps(&p(PostKind::Status)));
    }

    #[test]
    fn news_becomes_a_post() {
        let n = News { id: "1".into(), title: "Patch".into(), contents: "[img]https://x/a.png[/img]Fixed [b]bugs[/b].".into(), feed: "Announcements".into(), time: 5, ..Default::default() };
        let p = news_post(620, "Portal 2", &n);
        assert_eq!((p.kind, p.app_id, p.title.as_str(), p.time), (PostKind::Announcement, 620, "Patch", 5));
        assert_eq!(p.images, ["https://x/a.png"]);
        assert!(p.text.contains("Fixed") && p.text.contains("bugs") && !p.text.contains("[b]") && !p.text.contains("https://"), "{}", p.text);
        assert_eq!(p.author_name, "Announcements");
        assert_eq!(p.url, "");
    }
}

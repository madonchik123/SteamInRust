//! Native community pages, laid out like Steam's: a game's community hub
//! (screenshots, artwork, videos, news), Workshop item pages, guides,
//! discussions and achievements (yours, compared with a friend's).
//!
//! Data comes from `client::hub` (Steam's services, plus community pages
//! where there's no service). The same [`HubView`] draws them as pages in
//! the main window and inside windows of the Shift+Tab overlay, so nothing
//! here opens a browser except links to other websites.

mod achievements;
pub mod demo;
mod files;
mod forum;
pub mod rich;

use client::hub::{self, FileKind, FileQuery, Request};
use client::{Command, Event, State};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Ui, vec2};
use extensions::{Action, Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, SteamId, art};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use ui::{Theme, icons, widgets as w};

/// Phosphor glyphs the shared icon list doesn't have yet.
pub const THUMBS_DOWN: &str = "\u{E48C}";
pub const FILM: &str = "\u{E792}";
pub const NEWSPAPER: &str = "\u{E344}";
pub const IMAGES: &str = "\u{E836}";

/// Tabs of a game's community hub, as on Steam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HubTab {
    #[default]
    All,
    Discussions,
    Screenshots,
    Artwork,
    Videos,
    Workshop,
    News,
    Guides,
}

impl HubTab {
    const ALL: [HubTab; 8] = [HubTab::All, HubTab::Discussions, HubTab::Screenshots, HubTab::Artwork, HubTab::Videos, HubTab::Workshop, HubTab::News, HubTab::Guides];

    fn label(self) -> String {
        match self {
            HubTab::All => t!("All"),
            HubTab::Discussions => t!("Discussions"),
            HubTab::Screenshots => t!("Screenshots"),
            HubTab::Artwork => t!("Artwork"),
            HubTab::Videos => t!("Videos"),
            HubTab::Workshop => t!("Workshop"),
            HubTab::News => t!("News"),
            HubTab::Guides => t!("Guides"),
        }
        .to_string()
    }

    fn slug(self) -> &'static str {
        match self {
            HubTab::All => "all",
            HubTab::Discussions => "discussions",
            HubTab::Screenshots => "screenshots",
            HubTab::Artwork => "artwork",
            HubTab::Videos => "videos",
            HubTab::Workshop => "workshop",
            HubTab::News => "news",
            HubTab::Guides => "guides",
        }
    }

    fn from_slug(s: &str) -> HubTab {
        HubTab::ALL.into_iter().find(|t| t.slug() == s).unwrap_or_default()
    }
}

/// One page of community content.
#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Hub { app: AppId, tab: HubTab },
    /// A Workshop item, screenshot, artwork or guide (guides show as guides).
    Item(u64),
    Guides(AppId),
    Forum { app: AppId, forum: String },
    /// A discussion thread by its Steam link.
    Thread(String),
    Achievements { app: AppId, friend: Option<SteamId> },
    /// Screenshots, artwork, videos, Workshop items or guides someone shared.
    UserFiles { user: SteamId, kind: FileKind },
    /// Another page of the app (store, Workshop browser, profile).
    Page(String, Option<String>),
}

/// The pages this extension draws.
pub const PAGES: [&str; 7] = ["hub", "workshop_item", "guides", "discussions", "thread", "achievements", "user_files"];

/// URL-ish names for kinds of shared files (page arguments).
pub fn kind_slug(k: FileKind) -> &'static str {
    match k {
        FileKind::Screenshots => "screenshots",
        FileKind::Art => "artwork",
        FileKind::Videos => "videos",
        FileKind::Guides => "guides",
        FileKind::Collections => "collections",
        FileKind::Items => "workshop",
    }
}

fn kind_from_slug(s: &str) -> FileKind {
    [FileKind::Screenshots, FileKind::Art, FileKind::Videos, FileKind::Guides, FileKind::Collections, FileKind::Items].into_iter().find(|k| kind_slug(*k) == s).unwrap_or(FileKind::Screenshots)
}

impl View {
    pub fn to_page(&self) -> (String, Option<String>) {
        let p = |s: &str| s.to_string();
        match self {
            View::Hub { app, tab: HubTab::All } => (p("hub"), Some(app.to_string())),
            View::Hub { app, tab } => (p("hub"), Some(format!("{app}/{}", tab.slug()))),
            View::Item(id) => (p("workshop_item"), Some(id.to_string())),
            View::Guides(app) => (p("guides"), Some(app.to_string())),
            View::Forum { app, forum } if forum.is_empty() => (p("discussions"), Some(app.to_string())),
            View::Forum { app, forum } => (p("discussions"), Some(format!("{app}/{forum}"))),
            View::Thread(url) => (p("thread"), Some(url.clone())),
            View::Achievements { app, friend: None } => (p("achievements"), Some(app.to_string())),
            View::Achievements { app, friend: Some(f) } => (p("achievements"), Some(format!("{app}/{f}"))),
            View::UserFiles { user, kind } => (p("user_files"), Some(format!("{user}/{}", kind_slug(*kind)))),
            View::Page(page, arg) => (page.clone(), arg.clone()),
        }
    }

    pub fn from_page(page: &str, arg: Option<&str>) -> Option<View> {
        let arg = arg.unwrap_or_default();
        let (first, rest) = arg.split_once('/').unwrap_or((arg, ""));
        let app = first.parse::<AppId>().ok();
        Some(match page {
            "hub" => View::Hub { app: app?, tab: HubTab::from_slug(rest) },
            "workshop_item" | "guide" => View::Item(arg.parse().ok()?),
            "guides" => View::Guides(app?),
            "discussions" => View::Forum { app: app?, forum: rest.to_string() },
            "thread" => View::Thread(arg.to_string()).filter_thread()?,
            "achievements" => View::Achievements { app: app?, friend: rest.parse().ok() },
            "user_files" => View::UserFiles { user: first.parse().ok()?, kind: kind_from_slug(rest) },
            _ => return None,
        })
    }

    fn filter_thread(self) -> Option<View> {
        match &self {
            View::Thread(u) if u.starts_with("https://steamcommunity.com/") => Some(self),
            _ => None,
        }
    }

    /// The game this page belongs to, if known without loading.
    fn app(&self) -> Option<AppId> {
        match self {
            View::Hub { app, .. } | View::Guides(app) | View::Forum { app, .. } | View::Achievements { app, .. } => Some(*app),
            View::Thread(url) => url.split("/app/").nth(1)?.split('/').next()?.parse().ok(),
            _ => None,
        }
    }

    /// Short name for a back button or window title.
    pub fn title(&self, state: &State) -> String {
        let game = |a: &AppId| state.app_name(*a).unwrap_or_default().to_string();
        match self {
            View::Hub { app, tab: HubTab::All } => format!("{} · {}", game(app), t!("Community hub")),
            View::Hub { app, tab } => format!("{} · {}", game(app), tab.label()),
            View::Item(id) => match state.hub.items.get(id) {
                Some(Ok(i)) => i.entry.title.clone(),
                _ => t!("Workshop").to_string(),
            },
            View::Guides(app) => format!("{} · {}", game(app), t!("Guides")),
            View::Forum { app, .. } => format!("{} · {}", game(app), t!("Discussions")),
            View::Thread(_) => t!("Discussions").to_string(),
            View::Achievements { app, .. } => format!("{} · {}", game(app), t!("Achievements")),
            View::UserFiles { user, kind } => format!("{} · {}", state.persona(*user).name, files::kind_title(*kind)),
            View::Page(p, _) => p.clone(),
        }
    }
}

/// Where a Steam link goes in the app. `None` = not a page we draw (it then
/// opens in the browser).
pub fn route(url: &str) -> Option<View> {
    let url = rich::unwrap_link(url);
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let param = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(|v| v.split('#').next().unwrap_or(v).to_string());
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let num = |s: Option<&&str>| s.and_then(|s| s.parse::<u64>().ok());
    match host {
        "steamcommunity.com" => match parts.as_slice() {
            ["sharedfiles" | "workshop", "filedetails", ..] => Some(View::Item(param("id")?.parse().ok()?)),
            ["workshop", "browse", ..] => Some(View::Page("workshop".into(), Some(param("appid")?))),
            ["app", id, more @ ..] => {
                let app = id.parse().ok()?;
                Some(match more {
                    [] => View::Hub { app, tab: HubTab::All },
                    ["guides", ..] => View::Guides(app),
                    ["screenshots", ..] => View::Hub { app, tab: HubTab::Screenshots },
                    ["images", ..] => View::Hub { app, tab: HubTab::Artwork },
                    ["videos", ..] => View::Hub { app, tab: HubTab::Videos },
                    ["allnews" | "news", ..] => View::Hub { app, tab: HubTab::News },
                    ["workshop", ..] => View::Page("workshop".into(), Some(app.to_string())),
                    ["achievements", ..] => View::Achievements { app, friend: None },
                    [forum @ ("discussions" | "tradingforum" | "eventcomments"), tail @ ..] => {
                        // /discussions/<n>/<topic>/ is a thread; /discussions/<n>/ a forum.
                        let (sub, topic) = match (forum, tail) {
                            (&"discussions", [n, topic, ..]) => (format!("discussions/{n}/"), Some(*topic)),
                            (&"discussions", [n]) => (format!("discussions/{n}/"), None),
                            (&"discussions", []) => (String::new(), None),
                            (f, [topic, ..]) => (format!("{f}/"), Some(*topic)),
                            (f, []) => (format!("{f}/"), None),
                        };
                        match topic.filter(|t| t.chars().all(|c| c.is_ascii_digit())) {
                            Some(topic) => {
                                View::Thread(format!("https://steamcommunity.com/app/{app}/{sub}{topic}/"))
                            }
                            None => View::Forum { app, forum: sub },
                        }
                    }
                    _ => View::Hub { app, tab: HubTab::All },
                })
            }
            ["stats", id, ..] => Some(View::Achievements { app: id.parse().ok()?, friend: None }),
            ["profiles", id, "stats", app, ..] => Some(View::Achievements { app: app.parse().ok()?, friend: id.parse().ok() }),
            ["profiles", id, what @ ("screenshots" | "images" | "videos" | "myworkshopfiles"), ..] => {
                let kind = match *what {
                    "images" => FileKind::Art,
                    "videos" => FileKind::Videos,
                    "myworkshopfiles" if param("section").as_deref() == Some("guides") => FileKind::Guides,
                    "myworkshopfiles" => FileKind::Items,
                    _ => FileKind::Screenshots,
                };
                Some(View::UserFiles { user: num(Some(id))?, kind })
            }
            ["profiles", id, "inventory", ..] => Some(View::Page("inventory".into(), Some(num(Some(id))?.to_string()))),
            ["profiles", id, ..] => Some(View::Page("profile".into(), Some(num(Some(id))?.to_string()))),
            _ => None,
        },
        "store.steampowered.com" => match parts.as_slice() {
            ["app", id, ..] => Some(View::Page("store".into(), Some(format!("app:{}", id.parse::<AppId>().ok()?)))),
            _ => None,
        },
        _ => None,
    }
}

/// Workshop subscriptions changed this session (the Workshop browser and the
/// item page share them).
static SUBSCRIBED: Mutex<Option<HashMap<u64, bool>>> = Mutex::new(None);

pub fn set_subscribed(id: u64, on: bool) {
    SUBSCRIBED.lock().unwrap().get_or_insert_with(Default::default).insert(id, on);
}

pub fn subscribed(id: u64) -> Option<bool> {
    SUBSCRIBED.lock().unwrap().as_ref().and_then(|m| m.get(&id).copied())
}

/// What a view needs from the app while it draws; commands, actions and
/// pages to open are collected and handed over afterwards.
pub struct Io<'a> {
    pub state: &'a State,
    pub local: &'a local::LocalState,
    pub cmds: Vec<Command>,
    pub actions: Vec<Action>,
    pub open: Vec<View>,
    /// Drawn inside an overlay window (narrower, no app navigation).
    pub overlay: bool,
}

impl<'a> Io<'a> {
    pub fn new(state: &'a State, local: &'a local::LocalState, overlay: bool) -> Self {
        Self { state, local, cmds: Vec::new(), actions: Vec::new(), open: Vec::new(), overlay }
    }
    fn send(&mut self, c: Command) {
        self.cmds.push(c);
    }
    fn go(&mut self, v: View) {
        self.open.push(v);
    }
    fn lang(&self) -> String {
        i18n::language().steam_name().to_string()
    }
}

/// Filters and paging of a list (guides, screenshots…).
#[derive(Debug, Clone)]
struct ListState {
    sort: u32,
    days: u32,
    search: String,
    submitted: String,
    page: u32,
    tags: BTreeSet<String>,
    my_language: bool,
}

impl Default for ListState {
    fn default() -> Self {
        Self { sort: 3, days: 7, search: String::new(), submitted: String::new(), page: 1, tags: BTreeSet::new(), my_language: false }
    }
}

/// Draws community views and remembers their per-page state. The main
/// window has one; the overlay has its own with a back button.
#[derive(Default)]
pub struct HubView {
    /// Open views; the last one shows.
    pub stack: Vec<View>,
    asked: HashSet<String>,
    lists: HashMap<(AppId, HubTab), ListState>,
    /// Page shown per forum / thread.
    pages: HashMap<String, u32>,
    /// Which picture of an item is large.
    media: HashMap<u64, usize>,
    /// Mature items the user chose to see.
    mature_ok: HashSet<u64>,
    lightbox: Option<String>,
    /// Open news post.
    news_open: Option<String>,
    /// Section to scroll to in a guide.
    scroll_to: Option<usize>,
    ach_filter: u8,
    ach_sort: u8,
    show_all_notes: bool,
}

impl HubView {
    pub fn current(&self) -> Option<&View> {
        self.stack.last()
    }

    /// Show a view (replacing the history when `reset`).
    pub fn open(&mut self, v: View, reset: bool) {
        if reset {
            self.stack.clear();
        }
        if self.stack.last() != Some(&v) {
            self.stack.push(v);
        }
        self.scroll_to = None;
    }

    /// Send a request once (until `retry`).
    fn ask(&mut self, io: &mut Io, key: String, have: bool, req: impl FnOnce() -> Request) {
        if !have && self.asked.insert(key) {
            io.send(Command::Hub(req()));
        }
    }

    fn retry(&mut self, io: &mut Io, req: Request) {
        io.send(Command::Hub(req));
    }

    /// Open a link from content: our own page when we have one, the
    /// picture viewer for pictures, the browser for other websites.
    fn follow(&mut self, io: &mut Io, url: &str) {
        if let Some(img) = url.strip_prefix("image:") {
            self.lightbox = Some(img.to_string());
        } else if let Some(v) = route(url) {
            io.go(v);
        } else if url.starts_with("https://") || url.starts_with("http://") {
            let _ = open::that_detached(url);
        }
    }

    /// Draw the current view, filling `ui`.
    pub fn show(&mut self, ui: &mut Ui, t: &Theme, io: &mut Io) {
        let Some(view) = self.stack.last().cloned() else {
            w::empty(ui, t, icons::USERS, t!("Community"), t!("Open a game's community hub from its library page."));
            return;
        };
        if io.overlay && self.stack.len() > 1 {
            let back = self.stack[self.stack.len() - 2].title(io.state);
            ui.horizontal(|ui| {
                if w::button(ui, t, Some(icons::BACK), &back, w::Kind::Ghost, w::Size::Sm).clicked() {
                    self.stack.pop();
                }
            });
            ui.add_space(2.0);
        }
        let salt = format!("hub-scroll-{view:?}");
        egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show(ui, |ui| {
            let pad = if io.overlay { 4.0 } else { 24.0 };
            match &view {
                View::Hub { app, tab } => {
                    self.hub_header(ui, t, io, *app, *tab);
                    w::centered(ui, 1240.0, pad, |ui| files::hub_tab(self, ui, t, io, *app, *tab));
                }
                View::Guides(app) => {
                    self.hub_header(ui, t, io, *app, HubTab::Guides);
                    w::centered(ui, 1240.0, pad, |ui| files::guides(self, ui, t, io, *app));
                }
                View::Item(id) => w::centered(ui, 1240.0, pad, |ui| files::item(self, ui, t, io, *id)),
                View::Forum { app, forum } => {
                    self.hub_header(ui, t, io, *app, HubTab::Discussions);
                    w::centered(ui, 1240.0, pad, |ui| forum::forum(self, ui, t, io, *app, forum));
                }
                View::Thread(url) => {
                    if let Some(app) = view.app() {
                        self.hub_header(ui, t, io, app, HubTab::Discussions);
                    }
                    w::centered(ui, 1240.0, pad, |ui| forum::thread(self, ui, t, io, url));
                }
                View::Achievements { app, friend } => w::centered(ui, 1100.0, pad, |ui| achievements::show(self, ui, t, io, *app, *friend)),
                View::UserFiles { user, kind } => w::centered(ui, 1240.0, pad, |ui| files::user_files(self, ui, t, io, *user, *kind)),
                View::Page(..) => {}
            }
            ui.add_space(30.0);
        });
        self.show_lightbox(ui, t);
    }

    /// A picture shown large over the page.
    fn show_lightbox(&mut self, ui: &mut Ui, t: &Theme) {
        let Some(url) = self.lightbox.clone() else { return };
        let screen = ui.max_rect().expand(8.0);
        let mut close = false;
        egui::Area::new(egui::Id::unique(("hub-lightbox", ui.scope_id()))).order(egui::Order::Foreground).fixed_pos(screen.min).show(ui.ctx(), |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(225));
            let inner = r.shrink(28.0);
            if !w::paint_contain(ui, inner, &url, Align2::CENTER_CENTER) {
                ui.put(Rect::from_center_size(r.center(), vec2(40.0, 40.0)), egui::Spinner::new().size(32.0).color(t.accent));
            }
            let x = Rect::from_min_size(egui::pos2(r.right() - 48.0, r.top() + 12.0), vec2(36.0, 36.0));
            ui.painter().text(x.center(), Align2::CENTER_CENTER, icons::X, w::font(22.0), Color32::WHITE);
            close = resp.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
        if close {
            self.lightbox = None;
        }
    }

    /// The hub's banner and tab row, shared by the hub, guides and discussions.
    fn hub_header(&mut self, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId, active: HubTab) {
        let name = io.state.app_name(app).map(String::from).unwrap_or_default();
        let full = ui.available_width();
        let h = if io.overlay { 92.0 } else { 150.0 };
        let (banner, _) = ui.allocate_exact_size(vec2(full, h), Sense::hover());
        if !w::paint_cover(ui, banner, &art::hero(app), CornerRadius::ZERO) {
            w::gradient(ui.painter(), banner, t.accent.gamma_multiply(0.3), t.accent_2.gamma_multiply(0.2), true);
        }
        w::fade_down(ui, banner, t.bg, 0.0);
        let pad = if io.overlay { 12.0 } else { 32.0 };
        let cap_h = if io.overlay { 44.0 } else { 62.0 };
        let cap = Rect::from_min_size(egui::pos2(banner.left() + pad, banner.bottom() - cap_h - 12.0), vec2(cap_h * 460.0 / 215.0, cap_h));
        w::image_or_title(ui, t, cap, &art::header(app), &name, CornerRadius::same(4));
        let x = cap.right() + 14.0;
        ui.painter().text(egui::pos2(x, cap.top() + 4.0), Align2::LEFT_TOP, &name, w::font_bold(if io.overlay { 18.0 } else { 26.0 }), Color32::WHITE);
        ui.painter().text(egui::pos2(x, cap.bottom()), Align2::LEFT_BOTTOM, t!("Community hub"), w::font_medium(13.0), t.text_dim);
        // Store page and Workshop on the right, like Steam's hub header.
        let right = Rect::from_min_max(egui::pos2(banner.right() - 360.0, cap.top()), egui::pos2(banner.right() - pad, cap.bottom()));
        let mut r = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)));
        if w::button(&mut r, t, Some(icons::STOREFRONT), t!("Store page"), w::Kind::Secondary, w::Size::Sm).clicked() {
            io.go(View::Page("store".into(), Some(format!("app:{app}"))));
        }
        ui.add_space(6.0);
        // Tabs.
        let side = if io.overlay { 4.0 } else { 24.0 };
        w::centered(ui, 1240.0, side, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
                for tab in HubTab::ALL {
                    if w::nav_tab(ui, t, &tab.label(), tab == active, 34.0).clicked() && tab != active {
                        io.go(match tab {
                            HubTab::Discussions => View::Forum { app, forum: String::new() },
                            HubTab::Guides => View::Guides(app),
                            HubTab::Workshop => View::Page("workshop".into(), Some(app.to_string())),
                            tab => View::Hub { app, tab },
                        });
                    }
                }
            });
            let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
        });
        ui.add_space(12.0);
    }
}

/// Two columns side by side when there's room (main + sidebar), stacked otherwise.
fn columns<'a>(ui: &mut Ui, side: f32, hv: &mut HubView, io: &mut Io<'a>, main: impl FnOnce(&mut Ui, &mut HubView, &mut Io<'a>), right: impl FnOnce(&mut Ui, &mut HubView, &mut Io<'a>)) {
    let total = ui.available_width();
    if total > 820.0 {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            ui.vertical(|ui| {
                ui.set_width(total - side - 24.0);
                main(ui, hv, io);
            });
            ui.vertical(|ui| {
                ui.set_width(side);
                right(ui, hv, io);
            });
        });
    } else {
        main(ui, hv, io);
        ui.add_space(16.0);
        right(ui, hv, io);
    }
}

/// Paging row: Previous · Page x of y · Next. Returns the new page.
fn pager(ui: &mut Ui, t: &Theme, page: u32, pages: u32) -> Option<u32> {
    let mut to = None;
    if pages <= 1 {
        return None;
    }
    ui.horizontal(|ui| {
        ui.add_enabled_ui(page > 1, |ui| {
            if w::button(ui, t, Some(icons::CARET_LEFT), t!("Previous"), w::Kind::Secondary, w::Size::Sm).clicked() {
                to = Some(page - 1);
            }
        });
        ui.label(w::text(t!("Page {page} of {pages}", page = page, pages = pages), 13.0, t.text_dim));
        ui.add_enabled_ui(page < pages, |ui| {
            if w::button(ui, t, Some(icons::CARET_RIGHT), t!("Next"), w::Kind::Secondary, w::Size::Sm).clicked() {
                to = Some(page + 1);
            }
        });
    });
    to
}

/// Steam's 0–5 stars, or "Not enough ratings".
fn stars(ui: &mut Ui, t: &Theme, up: u32, down: u32, size: f32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        match hub::stars(up, down) {
            Some(n) => {
                for i in 0..5 {
                    ui.label(w::text(icons::STAR, size, if i < n { t.away } else { t.border }));
                }
            }
            None => {
                ui.label(w::text(t!("Not enough ratings"), size - 1.0, t.text_faint));
            }
        }
    });
}

/// An error box with "Try again".
fn error(ui: &mut Ui, t: &Theme, msg: &str) -> bool {
    let mut again = false;
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(w::text(icons::WARNING, 34.0, t.text_faint));
        ui.add_space(6.0);
        ui.label(w::text(i18n::tr_dyn(msg), 14.0, t.text_dim));
        ui.add_space(8.0);
        again = w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Sm).clicked();
    });
    again
}

/// Avatar and name of a Steam user (name from the persona once known).
fn author(ui: &mut Ui, t: &Theme, io: &mut Io, id: SteamId, fallback: &str, size: f32) {
    let p = io.state.persona(id);
    let name = if p.name.is_empty() { fallback.to_string() } else { p.name.clone() };
    let r = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), size, None);
        ui.label(w::text_semibold(if name.is_empty() { "…".to_string() } else { name }, (size * 0.5).clamp(11.5, 14.0), t.text));
    });
    if id != 0 && r.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("View profile")).clicked() {
        io.go(View::Page("profile".into(), Some(id.to_string())));
    }
}

/// The extension that owns the community pages in the main window.
#[derive(Default)]
pub struct Hub {
    view: HubView,
}

impl Extension for Hub {
    fn manifest(&self) -> Manifest {
        Manifest { id: "community_hub", name: "Community hub", author: "SteamInRust", description: "Workshop items, guides, discussions and achievements", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        PAGES.iter().map(|p| PageSpec::new(*p, *p, icons::USERS).hidden().order(960)).collect()
    }

    fn on_navigate(&mut self, page: &str, arg: Option<&str>) {
        if let Some(v) = View::from_page(page, arg) {
            self.view.open(v, true);
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut Ctx) {
        match event {
            Event::WorkshopSubscribed { file_id, result: Ok(()) } => set_subscribed(*file_id, true),
            Event::WorkshopUnsubscribed { file_id, result } => match result {
                Ok(()) => {
                    set_subscribed(*file_id, false);
                    ctx.toast(t!("Unsubscribed. Steam will remove it from the game."));
                }
                Err(e) => ctx.toast(e.clone()),
            },
            Event::Hub(hub::Response::Voted { result: Err(e), .. }) => ctx.toast(t!("Couldn't save your rating: {error}", error = e)),
            Event::Hub(hub::Response::Favorited { add, result, .. }) => ctx.toast(match (result, add) {
                (Ok(()), true) => t!("Added to your favorites").to_string(),
                (Ok(()), false) => t!("Removed from your favorites").to_string(),
                (Err(e), _) => t!("Couldn't change favorites: {error}", error = e),
            }),
            _ => {}
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        let (cmds, actions, open) = {
            let mut io = Io::new(ctx.state, ctx.local, false);
            self.view.show(ui, &t, &mut io);
            (io.cmds, io.actions, io.open)
        };
        for c in cmds {
            ctx.send(c);
        }
        for a in actions {
            ctx.act(a);
        }
        for v in open {
            let (page, arg) = v.to_page();
            ctx.navigate(&page, arg);
        }
    }
}

/// Requests for the `Files` list of a hub tab.
fn file_query(app: AppId, kind: FileKind, l: &ListState, per_page: u32, page: u32, lang: String) -> FileQuery {
    let mut tags: Vec<String> = l.tags.iter().cloned().collect();
    if l.my_language && kind == FileKind::Guides {
        let mut name = i18n::language().steam_name().to_string();
        name[..1].make_ascii_uppercase();
        tags.push(name);
    }
    FileQuery {
        app_id: app,
        kind,
        sort: l.sort,
        days: if l.sort == 3 { l.days } else { 0 },
        page,
        per_page,
        search: l.submitted.clone(),
        tags,
        language: lang,
        mature: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_steam_links() {
        assert_eq!(route("https://steamcommunity.com/sharedfiles/filedetails/?id=123456&searchtext="), Some(View::Item(123456)));
        assert_eq!(route("https://steamcommunity.com/workshop/filedetails/?id=9"), Some(View::Item(9)));
        assert_eq!(route("https://steamcommunity.com/app/413150/guides/"), Some(View::Guides(413150)));
        assert_eq!(route("https://steamcommunity.com/app/413150"), Some(View::Hub { app: 413150, tab: HubTab::All }));
        assert_eq!(route("https://steamcommunity.com/app/413150/screenshots/?p=1"), Some(View::Hub { app: 413150, tab: HubTab::Screenshots }));
        assert_eq!(route("https://steamcommunity.com/app/413150/discussions/"), Some(View::Forum { app: 413150, forum: String::new() }));
        assert_eq!(route("https://steamcommunity.com/app/413150/discussions/1/"), Some(View::Forum { app: 413150, forum: "discussions/1/".into() }));
        assert_eq!(
            route("https://steamcommunity.com/app/413150/discussions/0/564798618066955213/?ctp=2#c1"),
            Some(View::Thread("https://steamcommunity.com/app/413150/discussions/0/564798618066955213/".into()))
        );
        assert_eq!(route("https://steamcommunity.com/app/413150/tradingforum/"), Some(View::Forum { app: 413150, forum: "tradingforum/".into() }));
        assert_eq!(route("https://steamcommunity.com/app/440/workshop/"), Some(View::Page("workshop".into(), Some("440".into()))));
        assert_eq!(route("https://store.steampowered.com/app/620/Portal_2/"), Some(View::Page("store".into(), Some("app:620".into()))));
        assert_eq!(route("https://steamcommunity.com/profiles/76561197960265729"), Some(View::Page("profile".into(), Some("76561197960265729".into()))));
        assert_eq!(route("https://steamcommunity.com/profiles/76561197960265729/screenshots/"), Some(View::UserFiles { user: 76561197960265729, kind: FileKind::Screenshots }));
        assert_eq!(route("https://steamcommunity.com/profiles/76561197960265729/myworkshopfiles/?section=guides"), Some(View::UserFiles { user: 76561197960265729, kind: FileKind::Guides }));
        assert_eq!(route("https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fstardewvalleywiki.com"), None);
        assert_eq!(route("https://stardewvalleywiki.com/Tools"), None);
        assert_eq!(route("https://store.steampowered.com/checkout/"), None);
    }

    #[test]
    fn pages_round_trip() {
        for v in [
            View::Hub { app: 1, tab: HubTab::All },
            View::Hub { app: 1, tab: HubTab::Screenshots },
            View::Item(5),
            View::Guides(7),
            View::Forum { app: 1, forum: String::new() },
            View::Forum { app: 1, forum: "discussions/2/".into() },
            View::Thread("https://steamcommunity.com/app/1/discussions/0/2/".into()),
            View::Achievements { app: 3, friend: None },
            View::Achievements { app: 3, friend: Some(76561197960265729) },
            View::UserFiles { user: 76561197960265729, kind: FileKind::Guides },
        ] {
            let (page, arg) = v.to_page();
            assert_eq!(View::from_page(&page, arg.as_deref()), Some(v));
        }
        // Only Steam's own site can be a thread.
        assert_eq!(View::from_page("thread", Some("https://evil.example/x")), None);
    }
}

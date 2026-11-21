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
mod group;
pub mod post;
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
    Broadcasts,
    Reviews,
}

impl HubTab {
    /// In the design's order (Figma `495:6863`).
    const ALL: [HubTab; 10] = [HubTab::All, HubTab::Discussions, HubTab::Screenshots, HubTab::Artwork, HubTab::Workshop, HubTab::Broadcasts, HubTab::Videos, HubTab::News, HubTab::Guides, HubTab::Reviews];

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
            HubTab::Broadcasts => t!("Broadcasts"),
            HubTab::Reviews => t!("Reviews"),
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
            HubTab::Broadcasts => "broadcasts",
            HubTab::Reviews => "reviews",
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
    /// A Steam group by its 64-bit id or address name.
    Group(String),
    /// Another page of the app (store, Workshop browser, profile).
    Page(String, Option<String>),
}

/// The pages this extension draws.
pub const PAGES: [&str; 8] = ["hub", "workshop_item", "guides", "discussions", "thread", "achievements", "user_files", "group"];

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
            View::Group(key) => (p("group"), Some(key.clone())),
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
            "group" => View::Group(hub::group_path(arg).map(|_| arg.to_string())?),
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
    pub(super) fn app(&self) -> Option<AppId> {
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
            View::Group(key) => match state.hub.groups.get(key) {
                Some(Ok(g)) => g.name.clone(),
                _ => t!("Group").to_string(),
            },
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
                    ["broadcasts", ..] => View::Hub { app, tab: HubTab::Broadcasts },
                    ["reviews", ..] => View::Hub { app, tab: HubTab::Reviews },
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
            // A group's discussion thread; anything else of a group is its page.
            ["groups", name, "discussions", n, topic, ..] if topic.chars().all(|c| c.is_ascii_digit()) && n.chars().all(|c| c.is_ascii_digit()) => {
                hub::group_path(name)?;
                Some(View::Thread(format!("https://steamcommunity.com/groups/{name}/discussions/{n}/{topic}/")))
            }
            ["groups", name, ..] => Some(View::Group(hub::group_path(name).map(|_| name.to_string())?)),
            ["gid", id, ..] => Some(View::Group(num(Some(id))?.to_string())),
            ["profiles", id, "inventory", ..] => Some(View::Page("inventory".into(), Some(num(Some(id))?.to_string()))),
            ["profiles", id, ..] => Some(View::Page("profile".into(), Some(num(Some(id))?.to_string()))),
            _ => None,
        },
        "store.steampowered.com" => match parts.as_slice() {
            ["app", id, ..] => Some(View::Page("store".into(), Some(format!("app:{}", id.parse::<AppId>().ok()?)))),
            ["bundle", id, ..] => Some(View::Page("store".into(), Some(format!("bundle:{}", id.parse::<u32>().ok()?)))),
            ["sub", id, ..] => Some(View::Page("store".into(), Some(format!("sub:{}", id.parse::<u32>().ok()?)))),
            // /curator/1850-PC-Gamer/: the number comes first.
            ["curator", id, ..] => Some(View::Page("store".into(), Some(format!("curator:{}", id.split('-').next()?.parse::<u32>().ok()?)))),
            ["curators", ..] => Some(View::Page("store".into(), Some("curators".into()))),
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
    /// Reply drafts per thread.
    reply_text: HashMap<String, String>,
    /// Threads with a reply on its way, and the page to show once it's posted.
    reply_sending: HashMap<String, u32>,
    /// How many reply answers per thread were already handled.
    reply_seen: HashMap<String, u32>,
    /// A hub feed shows the most recent posts instead of the most popular.
    feed_recent: HashMap<AppId, bool>,
    /// Pages of a hub feed shown.
    feed_pages: HashMap<(AppId, HubTab), u32>,
    /// Search in a hub feed: (typed, submitted).
    feed_search: HashMap<AppId, (String, String)>,
    /// The game's store info for the hub (main window only).
    pub store: Option<StoreInfo>,
}

/// Loads a game's store details for the hub's info card.
pub struct StoreInfo {
    pub rt: tokio::runtime::Handle,
    pub ctx: egui::Context,
    slots: HashMap<AppId, std::sync::Arc<Mutex<Option<Option<store::Details>>>>>,
}

impl StoreInfo {
    pub fn new(rt: tokio::runtime::Handle, ctx: egui::Context) -> Self {
        Self { rt, ctx, slots: HashMap::new() }
    }

    /// The details once loaded (`Some(None)`: Steam had none).
    fn get(&mut self, app: AppId, state: &State) -> Option<Option<store::Details>> {
        let slot = self.slots.entry(app).or_insert_with(|| {
            let slot = std::sync::Arc::new(Mutex::new(None));
            let country = model::store_country(state.wallet.map(|w| w.1), &state.country).unwrap_or_else(|| "us".into());
            let api = store::StoreClient::new(&country, i18n::language().steam_name());
            let (out, ctx) = (slot.clone(), self.ctx.clone());
            self.rt.spawn(async move {
                let d = api.details(app).await.ok();
                *out.lock().unwrap() = Some(d);
                ctx.request_repaint();
            });
            slot
        });
        slot.lock().unwrap().clone()
    }
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
    fn follow(&mut self, io: &mut Io, click: rich::Click) {
        match click {
            rich::Click::Image(img) => {
                if crate::links::validate_http_url(&img).is_some() {
                    self.lightbox = Some(img);
                }
            }
            rich::Click::Link(url) => {
                if let Some(v) = route(&url) {
                    io.go(v);
                } else {
                    crate::links::open_http(&url);
                }
            }
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
                    backdrop(ui, *app);
                    w::centered(ui, 1564.0, pad, |ui| {
                        self.hub_header(ui, t, io, *app, *tab);
                        if *tab == HubTab::All && !io.overlay {
                            self.store_card(ui, t, io, *app);
                        }
                        files::hub_tab(self, ui, t, io, *app, *tab)
                    });
                }
                View::Guides(app) => {
                    backdrop(ui, *app);
                    w::centered(ui, 1564.0, pad, |ui| {
                        self.hub_header(ui, t, io, *app, HubTab::Guides);
                        files::guides(self, ui, t, io, *app)
                    });
                }
                View::Item(id) => w::centered(ui, 1240.0, pad, |ui| files::item(self, ui, t, io, *id)),
                View::Forum { app, forum } => {
                    backdrop(ui, *app);
                    w::centered(ui, 1564.0, pad, |ui| {
                        self.hub_header(ui, t, io, *app, HubTab::Discussions);
                        forum::forum(self, ui, t, io, *app, forum)
                    });
                }
                View::Thread(url) => w::centered(ui, 1564.0, pad, |ui| forum::thread(self, ui, t, io, url)),
                View::Achievements { app, friend } => w::centered(ui, 1100.0, pad, |ui| achievements::show(self, ui, t, io, *app, *friend)),
                View::UserFiles { user, kind } => w::centered(ui, 1240.0, pad, |ui| files::user_files(self, ui, t, io, *user, *kind)),
                View::Group(key) => w::centered(ui, 1240.0, pad, |ui| group::show(self, ui, t, io, key)),
                View::Page(..) => {}
            }
            ui.add_space(30.0);
        });
        self.show_lightbox(ui, t);
    }

    /// A picture shown large over the page.
    fn show_lightbox(&mut self, ui: &mut Ui, t: &Theme) {
        let Some(url) = self.lightbox.clone() else { return };
        if crate::links::validate_http_url(&url).is_none() {
            self.lightbox = None;
            return;
        }
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

    /// The hub's header card (Figma `495:6863`), shared by the hub, guides
    /// and discussions.
    fn hub_header(&mut self, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId, active: HubTab) {
        if let Some(v) = header(ui, t, io.state, app, active, io.overlay) {
            io.go(v);
        }
    }

    /// The game's store info under the header: picture, blurb, genres,
    /// price and a way to the store (Figma `495:6863`).
    fn store_card(&mut self, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId) {
        let Some(info) = self.store.as_mut() else { return };
        let Some(Some(d)) = info.get(app, io.state) else { return };
        let owned = io.state.apps.as_ref().is_some_and(|a| a.contains_key(&app));
        w::post_card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.md;
                let (img, _) = ui.allocate_exact_size(vec2(278.0, 130.0), Sense::hover());
                let url = if d.header.is_empty() { art::header(app) } else { d.header.clone() };
                w::image_or_title(ui, t, img, &url, &d.name, t.corner_small());
                let right = 260.0f32.min(ui.available_width() * 0.3);
                ui.allocate_ui_with_layout(vec2((ui.available_width() - right).max(120.0), 130.0), Layout::top_down(Align::Min), |ui| {
                    ui.add(egui::Label::new(w::body_m(t, &d.short_description, t.text)).wrap());
                    ui.add_space(t.spacing.sm);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
                        for g in d.genres.iter().take(5) {
                            w::tag(ui, t, g, None, w::TagState::Plain);
                        }
                    });
                });
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 130.0), Layout::right_to_left(Align::Max), |ui| {
                    ui.spacing_mut().item_spacing.x = t.spacing.md;
                    let label = if owned { t!("Store page") } else if d.is_free { t!("Play for free") } else { t!("Buy now") };
                    if w::button(ui, t, None, label, w::Kind::Primary, w::Size::Md).clicked() {
                        io.go(View::Page("store".into(), Some(format!("app:{app}"))));
                    }
                    if owned {
                        ui.label(w::body_m(t, t!("In your library"), t.text_dim));
                    } else if let Some(p) = &d.price {
                        if let Some(f) = p.final_ {
                            let old = p.initial.filter(|i| *i > f).map(|i| store::Price::format(i, &p.currency));
                            w::price_tag(ui, t, &store::Price::format(f, &p.currency), old.as_deref(), p.discount);
                        }
                    }
                });
            });
        });
        ui.add_space(t.spacing.xl - t.spacing.xs);
    }
}

/// A game's art behind the top of its hub pages, softened into the page.
pub fn backdrop(ui: &Ui, app: AppId) {
    let top = ui.cursor().top();
    let rect = Rect::from_min_size(egui::pos2(ui.max_rect().left(), top), vec2(ui.max_rect().width(), 520.0));
    w::paint_cover_fading(ui, rect, &art::hero(app), 0.16);
}

/// A game hub's header card (Figma `495:6863`): the game's name, View
/// stats and Store page, and the hub's tabs. Returns the page to open.
pub fn header(ui: &mut Ui, t: &Theme, state: &State, app: AppId, active: HubTab, overlay: bool) -> Option<View> {
    let name = state.app_name(app).map(String::from).unwrap_or_default();
    let mut go = None;
    ui.add_space(if overlay { t.spacing.sm } else { t.spacing.xl + t.spacing.lg });
    let size = if overlay { w::Size::Sm } else { w::Size::Md };
    w::filter_bar(t).inner_margin(egui::Margin::symmetric(t.spacing.xl as i8, t.spacing.lg as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let title = if overlay { w::heading_s(t, &name, t.text) } else { w::heading_l(t, &name, t.text) };
            ui.add(egui::Label::new(title).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.sm + 2.0;
                if w::button(ui, t, None, t!("Store page"), w::Kind::Secondary, size).clicked() {
                    go = Some(View::Page("store".into(), Some(format!("app:{app}"))));
                }
                if w::button(ui, t, None, t!("View stats"), w::Kind::Tertiary, size).clicked() {
                    go = Some(View::Achievements { app, friend: None });
                }
            });
        });
        ui.add_space(t.spacing.md);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.xs + 1.0, t.spacing.xs + 1.0);
            for tab in HubTab::ALL {
                if w::sort_chip(ui, t, &tab.label(), tab == active).clicked() && tab != active {
                    go = Some(match tab {
                        HubTab::Discussions => View::Forum { app, forum: String::new() },
                        HubTab::Guides => View::Guides(app),
                        HubTab::Workshop => View::Page("workshop".into(), Some(app.to_string())),
                        tab => View::Hub { app, tab },
                    });
                }
            }
        });
    });
    ui.add_space(t.spacing.xl - t.spacing.xs);
    go
}

/// A game hub's feed (All, Reviews, Broadcasts): "Sort by" and a search in
/// the filter bar, then posts in two columns, like the Community home.
fn feed_tab(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId, tab: HubTab) {
    let section = match tab {
        HubTab::Reviews => client::feed::Section::Reviews,
        HubTab::Broadcasts => client::feed::Section::Broadcasts,
        _ => client::feed::Section::All,
    };
    let mut recent = hv.feed_recent.get(&app).copied().unwrap_or(false);
    let (mut typed, mut submitted) = hv.feed_search.get(&app).cloned().unwrap_or_default();
    let mut changed = false;
    w::filter_bar(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let popular = t!("Most Popular").to_string();
            let newest = t!("Most Recent").to_string();
            changed |= w::sort_by(ui, t, &mut recent, &[(false, popular.as_str()), (true, newest.as_str())]);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let width = 374.0f32.min(ui.available_width() - 8.0).max(120.0);
                let r = w::search(ui, t, &mut typed, t!("Search…"), width);
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && typed.trim() != submitted {
                    submitted = typed.trim().to_string();
                    changed = true;
                }
                if typed.is_empty() && !submitted.is_empty() {
                    submitted.clear();
                    changed = true;
                }
            });
        });
    });
    ui.add_space(t.spacing.xl - t.spacing.xs);
    hv.feed_recent.insert(app, recent);
    hv.feed_search.insert(app, (typed, submitted.clone()));
    if changed {
        hv.feed_pages.insert((app, tab), 1);
    }
    let pages = hv.feed_pages.get(&(app, tab)).copied().unwrap_or(1);
    let lang = io.lang();
    let mut posts: Vec<&client::feed::Post> = Vec::new();
    let mut loading = false;
    let mut failed = None;
    let mut last_full = false;
    let state = io.state;
    for page in 1..=pages {
        let q = client::feed::ExploreQuery { section, recent, page, language: lang.clone(), app, search: submitted.clone() };
        let key = q.key();
        match state.hub.explore.get(&key) {
            None => {
                hv.ask(io, format!("feed:{key}"), false, || Request::Explore(q));
                loading = true;
                break;
            }
            Some(Err(e)) => {
                failed = Some((e.clone(), q));
                break;
            }
            Some(Ok(f)) => {
                last_full = !f.posts.is_empty();
                for p in &f.posts {
                    if !posts.iter().any(|x| x.url == p.url && x.file_id == p.file_id) {
                        posts.push(p);
                    }
                }
            }
        }
    }
    if posts.is_empty() {
        if let Some((e, q)) = failed {
            if error(ui, t, &e) {
                hv.retry(io, Request::Explore(q));
            }
        } else if loading {
            w::loading(ui, t, &t!("Loading…"));
        } else if tab == HubTab::Broadcasts {
            w::empty(ui, t, icons::MONITOR, t!("No one is broadcasting right now"), t!("Check back later."));
        } else {
            w::empty(ui, t, icons::USERS, t!("Nothing here yet"), t!("This game's community hasn't shared anything yet."));
        }
        return;
    }
    if let Some(c) = post::masonry(ui, t, state, &posts, &format!("hub-{app}-{tab:?}"), false) {
        match c {
            post::Click::Open(url) => hv.follow(io, rich::Click::Link(url)),
            post::Click::Hub(a) => io.go(View::Hub { app: a, tab: HubTab::All }),
            post::Click::Store(a) => io.go(View::Page("store".into(), Some(format!("app:{a}")))),
            post::Click::Profile(id) => io.go(View::Page("profile".into(), Some(id.to_string()))),
            post::Click::News(a) => io.go(View::Hub { app: a, tab: HubTab::News }),
        }
    }
    if loading {
        w::loading(ui, t, &t!("Loading…"));
    } else if failed.is_none() && last_full && pages < 10 {
        ui.vertical_centered(|ui| {
            if w::button(ui, t, Some(icons::CARET_DOWN), t!("See more"), w::Kind::Secondary, w::Size::Md).clicked() {
                hv.feed_pages.insert((app, tab), pages + 1);
            }
        });
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

/// Numbered paging (Figma `505:8934`). Returns the new page.
fn pager(ui: &mut Ui, t: &Theme, page: u32, pages: u32) -> Option<u32> {
    w::page_numbers(ui, t, page, pages)
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
    bar_search: String,
}

impl Hub {
    pub fn new(rt: tokio::runtime::Handle, ctx: egui::Context) -> Self {
        let mut hub = Hub::default();
        hub.view.store = Some(StoreInfo::new(rt, ctx));
        hub
    }
}

impl Extension for Hub {
    fn manifest(&self) -> Manifest {
        Manifest { id: "community_hub", name: "Community hub", author: "SteamInRust", description: "Workshop items, guides, discussions and achievements", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        PAGES.iter().map(|p| PageSpec::new(*p, *p, icons::USERS).hidden().order(960)).collect()
    }

    fn page_bar(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        use crate::builtin::community_home::{BarTab, page_bar};
        let tab = match self.view.current() {
            Some(View::Forum { .. } | View::Thread(_)) => BarTab::Discussions,
            _ => BarTab::Other,
        };
        page_bar(ui, ctx, tab, &mut self.bar_search);
        true
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
            Event::Hub(hub::Response::Replied { result, .. }) => ctx.toast(match result {
                Ok(()) => t!("Reply posted").to_string(),
                Err(e) => t!("Couldn't post your reply: {error}", error = i18n::tr_dyn(e)),
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
        assert_eq!(route("https://store.steampowered.com/bundle/232/Valve_Complete_Pack/"), Some(View::Page("store".into(), Some("bundle:232".into()))));
        assert_eq!(route("https://store.steampowered.com/sub/7877/"), Some(View::Page("store".into(), Some("sub:7877".into()))));
        assert_eq!(route("https://store.steampowered.com/curator/1850-PC-Gamer/"), Some(View::Page("store".into(), Some("curator:1850".into()))));
        assert_eq!(route("https://store.steampowered.com/curator/1850/"), Some(View::Page("store".into(), Some("curator:1850".into()))));
        assert_eq!(route("https://store.steampowered.com/curators/"), Some(View::Page("store".into(), Some("curators".into()))));
        assert_eq!(route("https://store.steampowered.com/curator/not-a-number/"), None);
        assert_eq!(route("https://steamcommunity.com/groups/SteamClientBeta"), Some(View::Group("steamclientbeta".into())));
        assert_eq!(route("https://steamcommunity.com/groups/SteamClientBeta/announcements"), Some(View::Group("steamclientbeta".into())));
        assert_eq!(route("https://steamcommunity.com/gid/103582791433918461/"), Some(View::Group("103582791433918461".into())));
        assert_eq!(
            route("https://steamcommunity.com/groups/x/discussions/0/586188528590954637/"),
            Some(View::Thread("https://steamcommunity.com/groups/x/discussions/0/586188528590954637/".into()))
        );
        assert_eq!(route("https://steamcommunity.com/groups/a.b/"), None);
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
            View::Group("103582791433918461".into()),
            View::Group("steamclientbeta".into()),
        ] {
            let (page, arg) = v.to_page();
            assert_eq!(View::from_page(&page, arg.as_deref()), Some(v));
        }
        // Only Steam's own site can be a thread.
        assert_eq!(View::from_page("thread", Some("https://evil.example/x")), None);
    }

    #[test]
    fn image_prefix_link_cannot_open_lightbox() {
        let mut hv = HubView::default();
        let state = client::State::default();
        let local_state = local::LocalState::default();
        let mut io = Io::new(&state, &local_state, false);
        hv.follow(&mut io, rich::Click::Link("image:file://attacker/share/a.png".into()));
        assert_eq!(hv.lightbox, None);
        hv.follow(&mut io, rich::Click::Image("file://attacker/share/a.png".into()));
        assert_eq!(hv.lightbox, None);
        hv.follow(&mut io, rich::Click::Image("https://steamuserimages-a.akamaihd.net/x.png".into()));
        assert_eq!(hv.lightbox.as_deref(), Some("https://steamuserimages-a.akamaihd.net/x.png"));
    }
}

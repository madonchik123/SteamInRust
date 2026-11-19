//! Library: sidebar game list, library home with shelves, and game pages.

use ui::widgets::EditMenu;
use i18n::t;
use super::{Entry, entries};

#[path = "library_news.rs"]
mod news;
#[path = "library_props.rs"]
mod props;
#[path = "library_views.rs"]
mod views;
use client::Command;
use egui::{Align, Align2, Color32, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use local::manage::{Progress, VerifyReport};
use model::{AppId, InstalledGame, art};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use ui::{Theme, icons, widgets as w};

/// Per-game options the user can change.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct GameSettings {
    /// Extra launch arguments.
    args: String,
    /// DLC the user turned off.
    disabled_dlc: BTreeSet<AppId>,
    /// DLC the user turned on after installing (added on the next update).
    enabled_dlc: BTreeSet<AppId>,
    /// Version branch ("" = installed/public).
    branch: String,
    beta_code: String,
    /// 0 always, 1 on launch, 2 high priority.
    auto_update: u8,
    /// 0 global, 1 allow, 2 never.
    background: u8,
    /// 0 default, 1 force on, 2 force off.
    controller: u8,
    #[serde(default = "yes")]
    overlay_on: bool,
    private: bool,
    /// Download language ("" = interface language).
    language: String,
    notes: String,
    /// Steam Cloud turned off for this game (on by default, like Steam).
    cloud_off: bool,
}

fn yes() -> bool {
    true
}

enum Dialog {
    Uninstall,
}

/// Library filters like Steam's: any choice within a group, every group must match.
#[derive(Clone, Default)]
struct AdvFilters {
    /// 0 installed, 1 not installed, 2 played, 3 never played.
    play: BTreeSet<u8>,
    kinds: BTreeSet<String>,
    genres: BTreeSet<u32>,
    features: BTreeSet<u32>,
    deck: BTreeSet<u8>,
    /// Systems the game has a version for ("windows", "macos", "linux").
    systems: BTreeSet<&'static str>,
}

impl AdvFilters {
    fn count(&self) -> usize {
        self.play.len() + self.kinds.len() + self.genres.len() + self.features.len() + self.deck.len() + self.systems.len()
    }
}

/// The game card's width (Figma "game card" `363:5954`: 191 x 282).
const TILE: f32 = 191.0;

/// Seconds since 1970.
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The three systems Steam ships games for, with the names players know.
const SYSTEMS: [&str; 3] = ["windows", "macos", "linux"];

const GENRES: &[(u32, &str)] = &[
    (1, "Action"), (25, "Adventure"), (4, "Casual"), (23, "Indie"), (29, "Massively Multiplayer"), (9, "Racing"),
    (3, "RPG"), (28, "Simulation"), (18, "Sports"), (2, "Strategy"), (37, "Free to Play"), (70, "Early Access"),
    (56, "Utilities"), (59, "Game Development"), (51, "Animation & Modeling"), (57, "Video Production"),
];

const FEATURES: &[(u32, &str)] = &[
    (2, "Single-player"), (1, "Multi-player"), (9, "Co-op"), (38, "Online co-op"), (24, "Shared/split screen"),
    (28, "Full controller support"), (22, "Steam Achievements"), (23, "Steam Cloud"), (29, "Steam Trading Cards"),
    (30, "Steam Workshop"), (44, "Remote Play Together"), (41, "Remote Play on TV"),
];

/// A named group of games the player made, like Steam's collections.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Collection {
    name: String,
    games: BTreeSet<AppId>,
    /// Set when it came from Steam (its id there), so it stays in step.
    #[serde(default)]
    steam_id: Option<String>,
}

/// A launch held back because saves changed both here and in Steam Cloud.
struct CloudHold {
    app: AppId,
    args: client::LaunchArgs,
    report: model::CloudReport,
}

/// A game added by hand (not from Steam).
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct CustomGame {
    name: String,
    exe: String,
    args: String,
}

/// The page bar's tabs.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Home,
    Collections,
    News,
}

/// How the library home shows your games (the page bar's view buttons).
#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
enum View {
    /// What's new, recent games, then every game.
    #[default]
    Home,
    Grid,
    List,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    tile: f32,
    view: View,
    games: HashMap<AppId, GameSettings>,
    favorites: BTreeSet<AppId>,
    hidden: BTreeSet<AppId>,
    custom: Vec<CustomGame>,
    collections: Vec<Collection>,
    /// Kept across restarts so a shortcut still appears if the app was
    /// closed during the download.
    pending_shortcuts: BTreeMap<AppId, std::path::PathBuf>,
    /// What Steam had at the last sync. See [`Library::steam_base`].
    steam_base: Collections,
}

/// Collections by Steam id ("favorite", "hidden", "uc-…"): name and games.
type Collections = BTreeMap<String, (String, BTreeSet<AppId>)>;

pub(crate) use news::Reader;

#[derive(Clone, Copy)]
enum MenuAct {
    Play,
    Stop,
    Install,
    PauseDownload,
    ResumeDownload,
    CancelDownload,
    Favorite,
    Hide,
    Shortcut,
    Browse,
    Verify,
    Uninstall,
    Store,
    Help,
    Properties,
    ToggleCollection(usize),
    NewCollection,
    Update,
    /// Open the Downloads page.
    Downloads,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    All,
    /// Ready to play: installed games only.
    Installed,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Sort {
    #[default]
    Name,
    Recent,
    Playtime,
    Size,
}

/// Where a game's install stands: one answer used by the list, the game
/// page, the right-click menu and Properties, so they always agree.
#[derive(Clone)]
pub(crate) enum Setup {
    NotInstalled,
    Queued,
    Downloading(model::DownloadStatus),
    Installed,
}

/// A game's main button, see [`Library::main_action`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum MainAction {
    Play,
    Running,
    Update,
    Install,
    Syncing,
    /// Running the game's install scripts before it starts.
    FirstTimeSetup,
    Queued,
    Uninstalling,
    /// Downloading or verifying.
    Busy { fraction: f32, verifying: bool, paused: bool },
    /// Not owned: only the store page.
    StorePage,
}

pub(crate) fn setup_state(ctx: &Ctx, id: AppId, installed: bool) -> Setup {
    if let Some(d) = ctx.state.downloads.get(&id).filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)) {
        return Setup::Downloading(d.clone());
    }
    if ctx.state.download_queue.contains(&id) {
        return Setup::Queued;
    }
    if installed { Setup::Installed } else { Setup::NotInstalled }
}

#[derive(Default)]
pub struct Library {
    /// This frame's downloads (paused?) and queue, for the right-click menu.
    downloading: HashMap<AppId, bool>,
    queued: BTreeSet<AppId>,
    rt: Option<tokio::runtime::Handle>,
    egui: Option<egui::Context>,
    favorites: BTreeSet<AppId>,
    hidden: BTreeSet<AppId>,
    show_hidden: bool,
    custom: Vec<CustomGame>,
    /// Index into `custom` when a non-Steam game is open.
    custom_selected: Option<usize>,
    adding: Option<CustomGame>,
    selected: Option<AppId>,
    search: String,
    filter: Filter,
    sort: Sort,
    tile: f32,
    games: HashMap<AppId, GameSettings>,
    dialog: Option<Dialog>,
    /// Games being uninstalled in the background: (name, progress, result).
    uninstalling: HashMap<AppId, Uninstalling>,
    /// Installed build and branch per game, read from disk now and then.
    builds: HashMap<AppId, (std::time::Instant, u64, String)>,
    reports: HashMap<AppId, VerifyReport>,
    pending: Option<String>,
    events: HashMap<AppId, news::EventsSlot>,
    /// People playing each game right now: (last asked, count).
    players: HashMap<AppId, (std::time::Instant, news::PlayersSlot)>,
    reader: Option<Reader>,
    props: Option<props::Props>,
    install_dialog: Option<InstallDialog>,
    /// Games to put on the desktop once their install finishes.
    collections: Vec<Collection>,
    /// What Steam had at the last sync, kept across restarts. Comparing it
    /// with ours shows what changed here (sent to Steam), and with Steam's
    /// what changed elsewhere (taken from Steam).
    steam_base: Collections,
    /// Steam's collections have loaded this session; edits go to Steam.
    steam_loaded: bool,
    /// Steam's dynamic collections (name, filters): their games are worked
    /// out from the filters, like Steam does.
    dynamic: Vec<(String, client::collections::DynamicFilter)>,
    adv: AdvFilters,
    /// Naming a new collection (with games to put in it), or renaming one.
    naming: Option<(Option<usize>, String, Vec<AppId>)>,
    tab: Tab,
    view: View,
    /// The game list grouped by the month you last played.
    recent: bool,
    /// Groups closed in the Collections tab.
    collapsed: BTreeSet<String>,
    /// Games whose achievements were asked for this session.
    asked_achievements: BTreeSet<AppId>,
    /// "DLC" in a game's links: scroll down to its DLC.
    scroll_to_dlc: bool,
    /// Where the page starts, right of the game list (for the filters popup).
    page_left: f32,
    filters_open: bool,
    /// The game page's facts that didn't fit in its play bar (label, value).
    more_facts: Vec<(String, String)>,
    /// Launch waiting on a Steam Cloud conflict choice.
    cloud_hold: Option<CloudHold>,
    flags: Option<super::settings::SharedFlags>,
    /// Updates already queued automatically this session.
    auto_queued: BTreeSet<AppId>,
    /// Games whose cloud usage was looked up this session.
    cloud_checked: BTreeSet<AppId>,
    /// A right-click menu choice, applied after the frame's drawing.
    menu_request: Option<(AppId, MenuAct)>,
    /// Games to put on the desktop once installed, with their library folder.
    shortcut_after_install: BTreeMap<AppId, std::path::PathBuf>,
    seen_finished: BTreeSet<AppId>,
}

impl Extension for Library {
    fn manifest(&self) -> Manifest {
        Manifest { id: "library", name: "Library", author: "SteamInRust", description: "Your games, owned and installed", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("library", "Library", icons::BOOKS).main().order(20)]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        self.custom_selected = None;
        self.dialog = None;
        self.reader = None;
        // Links can ask for an action: "install:ID", "uninstall:ID", "verify:ID", "play:ID".
        match arg.and_then(|a| a.split_once(':')) {
            Some((action, id)) => {
                self.selected = id.parse().ok();
                self.pending = Some(action.to_string());
            }
            None => self.selected = arg.and_then(|a| a.parse().ok()),
        }
    }

    fn on_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        // An uninstall can end while another page is open.
        self.finish_uninstalls(ctx);
        match event {
            client::Event::VerifyFinished { app_id, result } => self.verify_finished(ctx, *app_id, result),
            client::Event::LoggedIn { steam_id, .. } => {
                self.send_cloud_settings(ctx);
                ctx.send(Command::LoadSteamCollections);
                // Account-wide play history, for "Recent games".
                ctx.send(Command::LoadPlayerGames { steam_id: *steam_id, language: i18n::language().steam_name().into() });
            }
            client::Event::Apps(_) => {
                self.send_cloud_settings(ctx);
                self.auto_update(ctx);
            }
            client::Event::GameStopped { .. } => self.auto_update(ctx),
            client::Event::SteamCollections(Ok(list)) => {
                self.merge_steam_collections(list);
                self.sync_collections(ctx);
            }
            client::Event::SteamCollectionsSaved(Err(e)) => {
                ctx.toast(t!("Couldn't save your collections to Steam: {error}", error = e));
            }
            client::Event::CloudReport { report, held_launch } => {
                let name = ctx.state.app_name(report.app_id).unwrap_or_default().to_string();
                if let Some(args) = held_launch {
                    self.cloud_hold = Some(CloudHold { app: report.app_id, args: args.clone(), report: report.clone() });
                } else if report.uploaded > 0 {
                    ctx.toast(t!("{name}: saves uploaded to Steam Cloud", name = name));
                } else if report.downloaded > 0 {
                    ctx.toast(t!("{name}: newer saves downloaded from Steam Cloud", name = name));
                }
                if let Some((_, e)) = report.errors.first() {
                    ctx.toast(t!("Steam Cloud: {error}", error = i18n::tr_dyn(e)));
                }
            }
            _ => {}
        }
        if let client::Event::DownloadFinished { app_id, result } = event {
            {
                self.seen_finished.insert(*app_id);
                match result {
                    Ok(_) => {
                        ctx.toast(t!("{name} is ready to play", name = ctx.state.app_name(*app_id).unwrap_or_default()));
                        ctx.act(extensions::Action::ReloadLocal);
                        self.shortcut_for_finished(ctx, *app_id);
                    }
                    Err(e) => ctx.toast(t!("Download failed: {error}", error = e)),
                }
            }
        }
    }

    fn page_bar(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        let t = ctx.theme.clone();
        self.bar(ui, &t, ctx);
        true
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        self.finish_uninstalls(ctx);
        if !self.uninstalling.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.downloading = ctx.state.downloads.values().filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).map(|d| (d.app_id, d.paused)).collect();
        self.queued = ctx.state.download_queue.iter().copied().collect();
        if self.tile == 0.0 {
            self.tile = TILE;
        }
        // The game list beside the page (Figma: 390 of 1920 px), darker
        // at the top and fading into the page's background.
        let side = (ui.available_width() * 0.2).clamp(270.0, 390.0);
        egui::Panel::left("library-list")
            .exact_size(side)
            .resizable(false)
            .frame(egui::Frame::new())
            .show(ui, |ui| {
                let r = ui.max_rect();
                self.page_left = r.right();
                w::gradient(ui.painter(), r, t.raised, t.raised.gamma_multiply(0.0), false);
                egui::Frame::new().inner_margin(Margin { left: 11, right: 0, top: 10, bottom: 8 }).show(ui, |ui| self.sidebar(ui, &t, ctx));
            });
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| self.main(ui, &t, ctx));
        self.add_dialog(ui, &t, ctx);
        self.reader(ui, &t, ctx);
        self.install_window(ui, &t, ctx);
        self.properties_window(ui, &t, ctx);
        self.run_pending(ctx);
        self.cloud_conflict_dialog(ui, &t, ctx);
        self.naming_dialog(ui, &t);
        if let Some((id, act)) = self.menu_request.take() {
            self.apply_menu(ctx, id, act);
        }
        self.sync_collections(ctx);
    }

    fn save(&self) -> Option<String> {
        serde_json::to_string(&Saved {
            tile: self.tile,
            view: self.view,
            games: self.games.clone(),
            favorites: self.favorites.clone(),
            hidden: self.hidden.clone(),
            custom: self.custom.clone(),
            collections: self.collections.clone(),
            pending_shortcuts: self.shortcut_after_install.clone(),
            steam_base: self.steam_base.clone(),
        })
        .ok()
    }

    fn as_any(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn load(&mut self, data: &str) {
        let saved: Saved = serde_json::from_str(data).unwrap_or_default();
        self.tile = if saved.tile > 0.0 { saved.tile } else { TILE };
        self.view = saved.view;
        self.games = saved.games;
        self.favorites = saved.favorites;
        self.hidden = saved.hidden;
        self.custom = saved.custom;
        self.collections = saved.collections;
        self.shortcut_after_install = saved.pending_shortcuts;
        self.steam_base = saved.steam_base;
    }
}

impl Library {
    /// Act on a link request once the library knows the game.
    fn run_pending(&mut self, ctx: &mut Ctx) {
        let (Some(action), Some(id)) = (self.pending.clone(), self.selected) else { return };
        if ctx.state.apps.is_none() && ctx.local.games.iter().all(|g| g.app_id != id) {
            return; // wait for the library to load
        }
        self.pending = None;
        let installed = ctx.local.games.iter().find(|g| g.app_id == id).cloned();
        match (action.as_str(), installed) {
            ("install", None) => self.install_dialog = Some(InstallDialog::new(id, ctx)),
            ("uninstall", Some(_)) => self.dialog = Some(Dialog::Uninstall),
            ("verify", Some(g)) => self.start_verify(ctx, &g),
            ("update", Some(g)) => self.queue_install(ctx, id, Some(g.library.clone())),
            ("play", Some(g)) => self.launch_game(ctx, id, g.install_dir.clone()),
            ("props", _) => self.props = Some(props::Props { app: id, tab: props::PropTab::General, ..Default::default() }),
            ("news", _) => self.reader = Some(Reader { app: id, index: 0 }),
            ("resetargs", _) => {
                if let Some(s) = self.games.get_mut(&id) {
                    s.args.clear();
                }
                ctx.toast(t!("Launch options cleared"));
            }
            _ => {}
        }
    }

    /// Tell the client where Steam's `userdata` is and which games have
    /// Steam Cloud turned off.
    pub(super) fn send_cloud_settings(&self, ctx: &Ctx) {
        // Settings → Cloud can turn it off for every game.
        let all_off = self.flags.as_ref().is_some_and(|f| !f.lock().unwrap().cloud);
        let disabled = if all_off {
            ctx.state.apps.as_ref().map(|a| a.keys().copied().collect()).unwrap_or_default()
        } else {
            self.games.iter().filter(|(_, s)| s.cloud_off).map(|(id, _)| *id).collect()
        };
        let root = Some(ctx.local.steam_root.clone()).filter(|p| !p.as_os_str().is_empty());
        ctx.send(Command::CloudSettings { steam_root: root, disabled });
    }

    /// Our collections by Steam id. Collections made here get an id first.
    fn collections_now(&mut self) -> Collections {
        for c in self.collections.iter_mut().filter(|c| c.steam_id.is_none()) {
            c.steam_id = Some(client::collections::new_id());
        }
        let name_of = |id: &str, fallback: &str| self.steam_base.get(id).map(|(n, _)| n.clone()).unwrap_or_else(|| fallback.to_string());
        let mut now = Collections::new();
        // Favorites and Hidden always exist; an empty one Steam doesn't have
        // yet needn't be made.
        if !self.favorites.is_empty() || self.steam_base.contains_key("favorite") {
            now.insert("favorite".into(), (name_of("favorite", "Favorites"), self.favorites.clone()));
        }
        if !self.hidden.is_empty() || self.steam_base.contains_key("hidden") {
            now.insert("hidden".into(), (name_of("hidden", "Hidden"), self.hidden.clone()));
        }
        for c in &self.collections {
            now.insert(c.steam_id.clone().expect("set above"), (c.name.clone(), c.games.clone()));
        }
        now
    }

    /// Steam's collections arrived (Favorites, Hidden and your own; ones
    /// built from filters are skipped). Anything changed here since the last
    /// sync is kept and sent to Steam; everything else follows Steam, so edits
    /// from the official client or another computer show up, removals too.
    /// The very first time, both sides are merged so nothing is lost.
    fn merge_steam_collections(&mut self, list: &[model::SteamCollection]) {
        self.dynamic = list.iter().filter(|c| c.dynamic).filter_map(|c| Some((c.name.clone(), client::collections::DynamicFilter::parse(c.filter.as_deref()?)?))).collect();
        let steam: Collections = list.iter().filter(|c| !c.dynamic).map(|c| (c.id.clone(), (c.name.clone(), c.games.iter().copied().collect()))).collect();
        let ours = self.collections_now();
        let first = self.steam_base.is_empty();
        let ids: BTreeSet<&String> = steam.keys().chain(ours.keys()).chain(self.steam_base.keys()).collect();
        let mut merged = Collections::new();
        for id in ids {
            let (mine, theirs, base) = (ours.get(id), steam.get(id), self.steam_base.get(id));
            let pick = if first {
                match (mine, theirs) {
                    (Some((_, a)), Some((name, b))) => Some((name.clone(), a | b)),
                    (a, b) => a.or(b).cloned(),
                }
            } else if mine != base {
                mine.cloned()
            } else {
                theirs.cloned()
            };
            if let Some(v) = pick {
                merged.insert(id.clone(), v);
            }
        }
        self.favorites = merged.get("favorite").map(|(_, g)| g.clone()).unwrap_or_default();
        self.hidden = merged.get("hidden").map(|(_, g)| g.clone()).unwrap_or_default();
        // Keep the order collections are listed in; new ones go at the end.
        let mut rest = merged;
        rest.remove("favorite");
        rest.remove("hidden");
        self.collections.retain_mut(|c| match c.steam_id.as_ref().and_then(|id| rest.remove(id)) {
            Some((name, games)) => {
                c.name = name;
                c.games = games;
                true
            }
            None => false,
        });
        self.collections.extend(rest.into_iter().map(|(id, (name, games))| Collection { name, games, steam_id: Some(id) }));
        self.steam_base = steam;
        self.steam_loaded = true;
    }

    /// Send Steam whatever changed here since it last had our collections.
    fn sync_collections(&mut self, ctx: &mut Ctx) {
        if !self.steam_loaded {
            return;
        }
        let now = self.collections_now();
        if now == self.steam_base {
            return;
        }
        let mut edits: Vec<model::CollectionEdit> = now
            .iter()
            .filter(|(id, v)| self.steam_base.get(*id) != Some(*v))
            .map(|(id, (name, games))| model::CollectionEdit { id: id.clone(), name: name.clone(), games: games.iter().copied().collect(), deleted: false })
            .collect();
        edits.extend(self.steam_base.keys().filter(|id| !now.contains_key(*id)).map(|id| model::CollectionEdit { id: id.clone(), deleted: true, ..Default::default() }));
        self.steam_base = now;
        ctx.send(Command::SaveSteamCollections(edits));
    }

    /// Name a new collection, or rename one.
    fn naming_dialog(&mut self, ui: &mut Ui, t: &Theme) {
        let Some((index, name, _)) = &mut self.naming else { return };
        let mut done = None;
        egui::Modal::new(egui::Id::unique("collection-name")).show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            ui.label(w::heading_s(t, if index.is_some() { t!("Rename collection") } else { t!("New collection") }, t.text));
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(t!("Name, e.g. Co-op with friends")).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
            if !r.has_focus() && name.is_empty() {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = !name.trim().is_empty();
                let save = ui.add_enabled_ui(ok, |ui| w::button(ui, t, Some(icons::CHECK), t!("Save"), w::Kind::Primary, w::Size::Md)).inner.clicked();
                if ok && (save || enter) {
                    done = Some(true);
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                    done = Some(false);
                }
            });
        });
        match done {
            Some(true) => {
                let (index, name, game) = self.naming.take().expect("open");
                let name = name.trim().to_string();
                match index {
                    Some(i) => {
                        if let Some(c) = self.collections.get_mut(i) {
                            c.name = name;
                        }
                    }
                    None => {
                        self.collections.push(Collection { name, games: game.into_iter().collect(), steam_id: None });
                        self.tab = Tab::Collections;
                        self.selected = None;
                        self.custom_selected = None;
                    }
                }
            }
            Some(false) => self.naming = None,
            None if ui.input(|i| i.key_pressed(egui::Key::Escape)) => self.naming = None,
            None => {}
        }
    }

    /// Saves changed both here and in the cloud: ask which to keep, like Steam.
    fn cloud_conflict_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(hold) = &self.cloud_hold else { return };
        let app = hold.app;
        let name = ctx.state.app_name(app).unwrap_or_default().to_string();
        let dir = ctx.local.games.iter().find(|g| g.app_id == app).map(|g| g.install_dir.clone());
        let names: Vec<String> = hold.report.conflicts.iter().map(|c| c.name.clone()).collect();
        let mut choice = None;
        egui::Modal::new(egui::Id::unique(("cloud-conflict", app))).show(ui.ctx(), |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(w::text(icons::CLOUD_CHECK, t.typography.heading_l.size, t.away));
                ui.vertical(|ui| {
                    ui.label(w::heading_s(t, t!("Steam Cloud conflict"), t.text));
                    ui.label(w::body_m(t, &name, t.text_dim));
                });
            });
            ui.add_space(6.0);
            ui.add(egui::Label::new(w::body_m(t, t!("Your saves on this PC and in Steam Cloud both changed since the last sync. Pick which to keep. The other copy is backed up, not lost."), t.text_dim)).wrap());
            ui.add_space(10.0);
            for c in hold.report.conflicts.iter().take(6) {
                w::panel(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let file = c.name.rsplit(['/', '%']).next().unwrap_or(&c.name);
                    ui.label(w::body_m(t, file, t.text)).on_hover_text(&c.name);
                    ui.columns(2, |cols| {
                        cols[0].label(w::styled(t!("ON THIS PC"), t.typography.caption, t.text_faint));
                        cols[0].label(w::body_s(t, format!("{} · {}", ui::date(c.local_time), ui::bytes(c.local_size)), t.text));
                        cols[1].label(w::styled(t!("IN STEAM CLOUD"), t.typography.caption, t.text_faint));
                        cols[1].label(w::body_s(t, format!("{} · {}", ui::date(c.cloud_time), ui::bytes(c.cloud_size)), t.text));
                    });
                });
                ui.add_space(4.0);
            }
            if hold.report.conflicts.len() > 6 {
                ui.label(w::body_s(t, t!("and {n} more files", n = hold.report.conflicts.len() - 6), t.text_faint));
            }
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Use the cloud's saves"), w::Kind::Primary, w::Size::Md).clicked() {
                    choice = Some(1);
                }
                if w::button(ui, t, Some(icons::DESKTOP), t!("Keep this PC's saves"), w::Kind::Secondary, w::Size::Md).clicked() {
                    choice = Some(2);
                }
                if w::button(ui, t, Some(icons::PLAY), t!("Play without syncing"), w::Kind::Ghost, w::Size::Md).clicked() {
                    choice = Some(3);
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                    choice = Some(0);
                }
            });
        });
        let Some(choice) = choice else { return };
        let hold = self.cloud_hold.take().expect("checked above");
        let Some(dir) = dir else { return };
        match choice {
            1 => ctx.send(Command::CloudSync { app_id: app, install_dir: dir, mode: client::cloud::Mode::Sync, keep_local: vec![], keep_cloud: names, then_launch: Some(hold.args) }),
            2 => ctx.send(Command::CloudSync { app_id: app, install_dir: dir, mode: client::cloud::Mode::Sync, keep_local: names, keep_cloud: vec![], then_launch: Some(hold.args) }),
            3 => ctx.send(Command::LaunchWithoutCloud { app_id: app, install_dir: dir, args: hold.args }),
            _ => {}
        }
    }

    /// "What's new": the latest update posts from games you play, newest first.
    fn whats_new(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, ids: &[AppId]) {
        let (posts, loading) = self.whats_new_posts(ids);
        if posts.is_empty() && !loading {
            return;
        }
        let now = now();
        ui.label(w::heading_s(t, t!("What's new").to_uppercase(), t.text));
        ui.add_space(t.spacing.md);
        let card = vec2(300.0, 230.0);
        egui::ScrollArea::horizontal().id_salt("whats-new").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.lg;
                if posts.is_empty() {
                    for _ in 0..4 {
                        let (r, _) = ui.allocate_exact_size(card, Sense::hover());
                        w::skeleton(ui, t, r, t.corner());
                    }
                }
                for (id, index, e) in posts.iter().take(12) {
                    self.news_card(ui, t, ctx, *id, *index, e, card, now);
                }
            });
        });
        ui.add_space(t.spacing.xl * 1.5);
    }

    /// One post: when, its picture, its title and the game's icon and name.
    #[allow(clippy::too_many_arguments)]
    fn news_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx, id: AppId, index: usize, e: &store::events::GameEvent, card: egui::Vec2, now: u64) {
        let (r, resp) = ui.allocate_exact_size(card, Sense::click());
        if !ui.is_rect_visible(r) {
            return;
        }
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
        let when = match now.saturating_sub(e.time) / 86_400 {
            0 => t!("Today").to_string(),
            1 => t!("Yesterday").to_string(),
            2..=6 => t!("This week").to_string(),
            _ => ui::date(e.time),
        };
        let small = t.typography.body_s.font();
        ui.painter().text(r.left_top(), Align2::LEFT_TOP, when, small.clone(), t.text_dim);
        let img = Rect::from_min_size(r.min + vec2(0.0, small.size + t.spacing.sm - 2.0), vec2(card.x, (card.x * 0.5).round()));
        match e.capsule.as_deref().or(e.header.as_deref()) {
            Some(u) => w::image_or_title(ui, t, img, u, &e.title, t.corner_small()),
            None => w::image_or_title(ui, t, img, &art::header(id), &e.title, t.corner_small()),
        }
        if hover > 0.0 {
            ui.painter().rect_stroke(img, t.corner_small(), Stroke::new(2.0, t.accent.gamma_multiply(hover)), egui::StrokeKind::Inside);
        }
        let text_rect = Rect::from_min_max(pos2(r.left(), img.bottom() + t.spacing.sm - 2.0), r.max);
        let mut job = egui::text::LayoutJob::simple(e.title.clone(), w::font_semibold(t.typography.body_m.size), t.text, card.x);
        job.wrap.max_rows = 2;
        job.wrap.overflow_character = Some('…');
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(text_rect.min, galley, t.text);
        let game = ctx.state.app_name(id).unwrap_or_default().to_string();
        let s = t.typography.body_l.size;
        let icon = Rect::from_min_size(pos2(r.left(), r.bottom() - s - 2.0), egui::Vec2::splat(s));
        let ok = ctx.state.apps.as_ref().and_then(|a| a.get(&id)).and_then(|o| o.icon.as_deref()).is_some_and(|h| w::paint_cover(ui, icon, &art::icon(id, h), t.corner_small()));
        let x = if ok { icon.right() + t.spacing.sm - 2.0 } else { r.left() };
        ui.painter().text(pos2(x, icon.center().y), Align2::LEFT_CENTER, game, small, t.text_dim);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.reader = Some(Reader { app: id, index });
        }
    }

    /// Right-click menu for a game anywhere in the library, like Steam's.
    fn game_menu(&mut self, t: &Theme, resp: &egui::Response, id: AppId, installed: bool, running: bool) {
        let fav = self.favorites.contains(&id);
        let hid = self.hidden.contains(&id);
        let pick = std::cell::Cell::new(None);
        resp.context_menu(|ui| {
            ui.set_min_width(220.0);
            let item = |ui: &mut Ui, icon: &str, label: &str, act: MenuAct| {
                if w::menu_item(ui, t, icon, label).clicked() {
                    pick.set(Some(act));
                    ui.close();
                }
            };
            if running {
                item(ui, icons::STOP, t!("Stop"), MenuAct::Stop);
            } else if let Some(paused) = self.downloading.get(&id) {
                if *paused {
                    item(ui, icons::PLAY, t!("Resume download"), MenuAct::ResumeDownload);
                } else {
                    item(ui, "\u{E39E}", t!("Pause download"), MenuAct::PauseDownload);
                }
                item(ui, icons::X, t!("Cancel download"), MenuAct::CancelDownload);
            } else if self.queued.contains(&id) {
                item(ui, icons::X, t!("Remove from download queue"), MenuAct::CancelDownload);
            } else if installed {
                item(ui, icons::PLAY, t!("Play"), MenuAct::Play);
            } else {
                item(ui, icons::DOWNLOAD, t!("Install"), MenuAct::Install);
            }
            ui.separator();
            item(ui, icons::STAR, if fav { t!("Remove from favorites") } else { t!("Add to favorites") }, MenuAct::Favorite);
            item(ui, icons::EYE, if hid { t!("Unhide") } else { t!("Hide this game") }, MenuAct::Hide);
            if installed {
                let r = w::submenu_item(ui, t, icons::GEAR, t!("Manage"));
                egui::containers::menu::SubMenu::new().show(ui, &r, |ui| {
                    ui.set_min_width(220.0);
                    let sub = item;
                    sub(ui, icons::DESKTOP, t!("Add desktop shortcut"), MenuAct::Shortcut);
                    sub(ui, icons::FOLDER, t!("Browse local files"), MenuAct::Browse);
                    sub(ui, icons::SHIELD, t!("Verify game files"), MenuAct::Verify);
                    sub(ui, icons::TRASH, t!("Uninstall…"), MenuAct::Uninstall);
                });
            }
            let r = w::submenu_item(ui, t, icons::STACK, t!("Add to collection"));
            egui::containers::menu::SubMenu::new().show(ui, &r, |ui| {
                ui.set_min_width(220.0);
                for (i, c) in self.collections.iter().enumerate() {
                    // A check for collections the game is already in.
                    let on = c.games.contains(&id);
                    if w::menu_item(ui, t, if on { icons::CHECK } else { "" }, &c.name).clicked() {
                        pick.set(Some(MenuAct::ToggleCollection(i)));
                        ui.close();
                    }
                }
                if !self.collections.is_empty() {
                    ui.separator();
                }
                item(ui, icons::PLUS, t!("New collection…"), MenuAct::NewCollection);
            });
            ui.separator();
            item(ui, icons::STORE, t!("Store page"), MenuAct::Store);
            item(ui, icons::INFO, t!("Get help with this game"), MenuAct::Help);
            item(ui, icons::SLIDERS, t!("Properties…"), MenuAct::Properties);
        });
        if let Some(act) = pick.get() {
            self.menu_request = Some((id, act));
        }
    }

    fn apply_menu(&mut self, ctx: &mut Ctx, id: AppId, act: MenuAct) {
        let local = ctx.local.games.iter().find(|g| g.app_id == id).cloned();
        match act {
            MenuAct::Play => {
                if let Some(g) = local {
                    self.launch_game(ctx, id, g.install_dir.clone());
                }
            }
            MenuAct::Stop => ctx.send(Command::StopGame { app_id: id }),
            MenuAct::PauseDownload => ctx.send(Command::PauseDownload { app_id: id }),
            MenuAct::ResumeDownload => ctx.send(Command::ResumeDownload { app_id: id }),
            MenuAct::CancelDownload => ctx.send(Command::CancelDownload { app_id: id }),
            MenuAct::Install if ctx.state.apps.as_ref().and_then(|a| a.get(&id)).is_some_and(|o| !o.runs_on_this_os()) => {
                ctx.toast(t!("This game doesn't run on {os}.", os = client::download::os_name(model::THIS_OS)));
            }
            MenuAct::Install => self.install_dialog = Some(InstallDialog::new(id, ctx)),
            MenuAct::Favorite => {
                if !self.favorites.remove(&id) {
                    self.favorites.insert(id);
                }
            }
            MenuAct::Hide => {
                if !self.hidden.remove(&id) {
                    self.hidden.insert(id);
                }
            }
            MenuAct::Shortcut => {
                if let Some(g) = local {
                    self.add_shortcut(ctx, id, &g);
                }
            }
            MenuAct::Browse => {
                if let Some(g) = local {
                    local::open_folder(&g.install_dir);
                }
            }
            MenuAct::Verify => {
                if let Some(g) = local {
                    self.start_verify(ctx, &g);
                }
            }
            MenuAct::Uninstall => {
                self.selected = Some(id);
                self.custom_selected = None;
                self.dialog = Some(Dialog::Uninstall);
            }
            MenuAct::Store => ctx.navigate("store", Some(format!("app:{id}"))),
            MenuAct::Help => ctx.navigate("support", Some(format!("game:{id}"))),
            MenuAct::Properties => self.props = Some(props::Props { app: id, tab: props::PropTab::General, ..Default::default() }),
            MenuAct::ToggleCollection(i) => {
                if let Some(c) = self.collections.get_mut(i) {
                    if !c.games.remove(&id) {
                        c.games.insert(id);
                        ctx.toast(t!("Added to {name}", name = c.name));
                    }
                }
            }
            MenuAct::NewCollection => self.naming = Some((None, String::new(), vec![id])),
            MenuAct::Update => {
                if let Some(g) = local {
                    self.queue_install(ctx, id, Some(g.library.clone()));
                }
            }
            MenuAct::Downloads => ctx.navigate("downloads", None),
        }
    }

    /// Put a just-installed game on the desktop, if the install asked for it.
    /// Runs from the download event, so it works whatever page is open.
    fn shortcut_for_finished(&mut self, ctx: &mut Ctx, id: AppId) {
        let Some(library) = self.shortcut_after_install.remove(&id) else { return };
        let owned = ctx.state.apps.as_ref().and_then(|a| a.get(&id));
        let name = owned.map(|o| o.name.clone()).or_else(|| ctx.local.games.iter().find(|g| g.app_id == id).map(|g| g.name.clone())).unwrap_or_else(|| id.to_string());
        let folder = owned.map(|o| o.install_dir.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| name.clone());
        let dir = library.join("steamapps").join("common").join(folder);
        match crate::links::desktop_shortcut(id, &name, crate::links::game_icon(owned, &dir)) {
            Ok(_) => ctx.toast(t!("Shortcut added to your desktop")),
            Err(e) => ctx.toast(t!("Couldn't add a desktop shortcut: {error}", error = e)),
        }
    }

    pub fn new(rt: tokio::runtime::Handle, egui: egui::Context, flags: super::settings::SharedFlags) -> Self {
        Self { rt: Some(rt), egui: Some(egui), tile: TILE, flags: Some(flags), ..Default::default() }
    }

    /// Like Steam: queue games that need an update, unless auto-updates are
    /// off (globally, or "only when I launch it" for the game).
    fn auto_update(&mut self, ctx: &mut Ctx) {
        let on = self.flags.as_ref().is_some_and(|f| f.lock().unwrap().auto_updates);
        if !on || ctx.state.apps.is_none() {
            return;
        }
        let due: Vec<(AppId, std::path::PathBuf)> = ctx
            .local
            .games
            .iter()
            .filter(|g| g.state == model::InstallState::UpdateRequired)
            .filter(|g| self.games.get(&g.app_id).is_none_or(|s| s.auto_update != 1))
            .filter(|g| !ctx.state.downloads.contains_key(&g.app_id) && !ctx.state.download_queue.contains(&g.app_id))
            .filter(|g| self.auto_queued.insert(g.app_id))
            .map(|g| (g.app_id, g.library.clone()))
            .collect();
        for (id, lib) in due {
            self.queue_install(ctx, id, Some(lib));
        }
    }

    fn filtered<'a>(&self, ctx: &'a Ctx) -> Vec<Entry<'a>> {
        self.filtered_in(ctx.state, ctx.local, self.filter)
    }

    /// The games that match the search, `filter` and the advanced filters,
    /// in the chosen order.
    fn filtered_in<'a>(&self, state: &'a client::State, local: &'a local::LocalState, filter: Filter) -> Vec<Entry<'a>> {
        let mut items = entries(state, local);
        let q = self.search.to_lowercase();
        items.retain(|e| {
            (q.is_empty() || e.name.to_lowercase().contains(&q))
                && match filter {
                    Filter::All => true,
                    Filter::Installed => e.installed.is_some(),
                }
                && (self.show_hidden || !self.hidden.contains(&e.app_id))
                && self.adv_match(state, e)
        });
        match self.sort {
            Sort::Name => items.sort_by_key(|e| e.name.to_lowercase()),
            Sort::Recent => items.sort_by_key(|e| std::cmp::Reverse(e.installed.and_then(|g| g.last_played))),
            Sort::Playtime => items.sort_by_key(|e| std::cmp::Reverse(e.installed.and_then(|g| g.playtime_minutes))),
            Sort::Size => items.sort_by_key(|e| std::cmp::Reverse(e.installed.map(|g| g.size_on_disk))),
        }
        items
    }

    /// Whether a game passes the advanced filters.
    fn adv_match(&self, state: &client::State, e: &Entry) -> bool {
        let f = &self.adv;
        if f.count() == 0 {
            return true;
        }
        let o = e.owned;
        let minutes = e
            .installed
            .and_then(|g| g.playtime_minutes)
            .or_else(|| match state.player_games.get(&state.steam_id) {
                Some(Ok(list)) => list.iter().find(|g| g.app_id == e.app_id).map(|g| g.playtime_minutes),
                _ => None,
            })
            .unwrap_or(0);
        let play_ok = f.play.is_empty()
            || f.play.iter().any(|p| match p {
                0 => e.installed.is_some(),
                1 => e.installed.is_none(),
                2 => minutes > 0,
                _ => minutes == 0,
            });
        let kind_ok = f.kinds.is_empty() || o.is_some_and(|o| f.kinds.contains(&o.kind));
        let genre_ok = f.genres.is_empty() || o.is_some_and(|o| o.genres.iter().any(|g| f.genres.contains(g)));
        let feature_ok = f.features.is_empty() || o.is_some_and(|o| f.features.iter().all(|c| o.categories.contains(c)));
        let deck_ok = f.deck.is_empty() || f.deck.contains(&o.map(|o| o.deck).unwrap_or(0));
        // Installed games with no app info yet count as runnable here.
        let system_ok = f.systems.is_empty() || f.systems.iter().any(|os| o.map_or(*os == model::THIS_OS && e.installed.is_some(), |o| o.runs_on(os)));
        play_ok && kind_ok && genre_ok && feature_ok && deck_ok && system_ok
    }

    /// The filter popup (Figma "library home filters" `727:39670`): the
    /// choices in three columns, then clear and "Create Collection" (a
    /// collection of the games the filters leave).
    fn adv_panel(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx) {
        // Only offer what this library has.
        let apps: Vec<&model::OwnedApp> = ctx.state.apps.as_ref().map(|a| a.values().filter(|a| !a.is_dlc()).collect()).unwrap_or_default();
        let has_genre = |g: u32| apps.iter().any(|a| a.genres.contains(&g));
        let has_feature = |c: u32| apps.iter().any(|a| a.categories.contains(&c));
        let mut kinds: Vec<String> = apps.iter().map(|a| a.kind.clone()).filter(|k| !k.is_empty()).collect();
        kinds.sort();
        kinds.dedup();
        let kind_label = |k: &str| match k {
            "game" => t!("Games").to_string(),
            "application" => t!("Software").to_string(),
            "tool" => t!("Tools").to_string(),
            "demo" => t!("Demos").to_string(),
            "music" => t!("Soundtracks").to_string(),
            other => other.to_string(),
        };
        let genres: Vec<&(u32, &str)> = GENRES.iter().filter(|(g, _)| has_genre(*g)).collect();
        let features: Vec<&(u32, &str)> = FEATURES.iter().filter(|(c, _)| has_feature(*c)).collect();
        let play = [(0u8, t!("Ready to play")), (1, t!("Not installed")), (2, t!("Played")), (3, t!("Never played"))];
        let deck = [(3u8, t!("Verified")), (2, t!("Playable")), (1, t!("Unsupported")), (0, t!("Unknown"))];
        let mut create = false;
        let f = &mut self.adv;
        let gap = t.spacing.xl;
        let col = ((ui.available_width() - gap * 2.0) / 3.0).max(160.0);
        ui.add_space(t.spacing.xs);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.vertical(|ui| {
                ui.set_width(col);
                if !kinds.is_empty() {
                    let hit = check_list(ui, t, t!("Type"), kinds.iter().map(|k| (f.kinds.contains(k), kind_label(k))));
                    toggle_set(&mut f.kinds, hit.map(|i| kinds[i].clone()));
                }
                let hit = check_list(ui, t, t!("Play state"), play.iter().map(|(v, l)| (f.play.contains(v), l.to_string())));
                toggle_set(&mut f.play, hit.map(|i| play[i].0));
            });
            ui.vertical(|ui| {
                ui.set_width(col);
                let hit = check_list(ui, t, t!("Runs on"), SYSTEMS.iter().map(|os| (f.systems.contains(os), client::download::os_name(os).to_string())));
                toggle_set(&mut f.systems, hit.map(|i| SYSTEMS[i]));
                let hit = check_list(ui, t, t!("Steam Deck"), deck.iter().map(|(v, l)| (f.deck.contains(v), l.to_string())));
                toggle_set(&mut f.deck, hit.map(|i| deck[i].0));
                if !features.is_empty() {
                    let hit = check_list(ui, t, t!("Features"), features.iter().map(|(c, l)| (f.features.contains(c), i18n::tr_dyn(l).to_string())));
                    toggle_set(&mut f.features, hit.map(|i| features[i].0));
                }
            });
            ui.vertical(|ui| {
                ui.set_width(col);
                if !genres.is_empty() {
                    let hit = check_list(ui, t, t!("Genre"), genres.iter().map(|(g, l)| (f.genres.contains(g), i18n::tr_dyn(l).to_string())));
                    toggle_set(&mut f.genres, hit.map(|i| genres[i].0));
                }
            });
        });
        ui.add_space(t.spacing.md);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.md;
            let any = f.count() > 0;
            if ui.add_enabled_ui(any, |ui| w::button(ui, t, None, t!("Create Collection"), w::Kind::Primary, w::Size::Md)).inner.on_hover_text(t!("A new collection of the games these filters show")).clicked() {
                create = true;
            }
            if ui.add_enabled_ui(any, |ui| w::icon_box(ui, t, icons::X, t!("Clear all"), false)).inner.clicked() {
                *f = AdvFilters::default();
            }
        });
        if create {
            let games = self.filtered(ctx).iter().map(|e| e.app_id).collect();
            self.naming = Some((None, String::new(), games));
            ui.close();
        }
    }

    fn add_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(g) = &mut self.adding else { return };
        let mut close = false;
        let mut add = false;
        egui::Modal::new(egui::Id::unique("add-game")).show(ui.ctx(), |ui| {
            ui.set_width(500.0);
            ui.label(w::heading_s(t, t!("Add a non-Steam game"), t.text));
            ui.add_space(8.0);
            ui.label(w::body_s(t, t!("Program"), t.text_dim));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut g.exe).hint_text(t!("Path to the game's .exe")).desired_width(360.0).margin(vec2(10.0, 8.0))).with_edit_menu();
                if w::button(ui, t, Some(icons::FOLDER), t!("Choose…"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Programs", &["exe", "bat", "cmd", "lnk"]).pick_file() {
                        g.exe = path.display().to_string();
                        if g.name.is_empty() {
                            g.name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                        }
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(w::body_s(t, t!("Display name"), t.text_dim));
            ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
            ui.add_space(6.0);
            ui.label(w::body_s(t, t!("Launch options"), t.text_dim));
            ui.add(egui::TextEdit::singleline(&mut g.args).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let ok = !g.exe.trim().is_empty() && !g.name.trim().is_empty();
                if ui.add_enabled_ui(ok, |ui| w::button(ui, t, Some(icons::PLUS), t!("Add to library"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                    add = true;
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked() {
                    close = true;
                }
            });
        });
        if add {
            let g = self.adding.take().unwrap();
            ctx.toast(t!("{name} was added to your library", name = g.name));
            self.custom.push(g);
            self.custom_selected = Some(self.custom.len() - 1);
            self.selected = None;
        } else if close {
            self.adding = None;
        }
    }

    fn custom_page(&mut self, i: usize, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(g) = self.custom.get(i).cloned() else {
            self.custom_selected = None;
            return;
        };
        let mut remove = false;
        ui.add_space(t.spacing.xl + t.spacing.xs);
        w::centered(ui, 900.0, t.spacing.xl + t.spacing.sm, |ui| {
            ui.label(w::body_s(t, t!("Non-Steam game").to_uppercase(), t.text_dim));
            ui.label(w::styled(&g.name, ui::theme::TextStyle::new(t.typography.heading_l.size * 1.25, t.typography.heading_l.weight), t.text));
            ui.label(w::body_m(t, &g.exe, t.text_faint).monospace());
            ui.add_space(t.spacing.lg);
            ui.horizontal(|ui| {
                if w::play_button(ui, t, t!("Play")).clicked() {
                    let exe = std::path::PathBuf::from(&g.exe);
                    let mut cmd = std::process::Command::new(&exe);
                    cmd.args(g.args.split_whitespace());
                    if let Some(dir) = exe.parent() {
                        cmd.current_dir(dir);
                    }
                    if let Err(e) = cmd.spawn() {
                        ctx.toast(e.to_string());
                    }
                }
                if let Some(dir) = std::path::Path::new(&g.exe).parent() {
                    if w::button(ui, t, Some(icons::FOLDER), t!("Browse local files"), w::Kind::Secondary, w::Size::Lg).clicked() {
                        local::open_folder(dir);
                    }
                }
                if w::button(ui, t, Some(icons::TRASH), t!("Remove from library"), w::Kind::Danger, w::Size::Lg).clicked() {
                    remove = true;
                }
            });
        });
        if remove {
            self.custom.remove(i);
            self.custom_selected = None;
        }
    }

    /// Library home: what's new, the game to pick up again, recent games,
    /// then every game.
    fn home(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let mut recent: Vec<InstalledGame> = ctx.local.games.iter().filter(|g| g.last_played.is_some()).cloned().collect();
        recent.sort_by_key(|g| std::cmp::Reverse(g.last_played));
        let ids = self.recent_ids(ctx);
        let (state, local) = (ctx.state, ctx.local);
        Self::page_scroll(ui, t, "lib-home", |ui| {
            self.whats_new(ui, t, ctx, &ids);
            // Continue playing.
            if let Some(g) = recent.first() {
                let width = ui.available_width();
                let (rect, resp) = ui.allocate_exact_size(vec2(width, t.dense(300.0)), Sense::click());
                let r = t.corner();
                if !w::paint_cover_first(ui, rect, &[&art::hero(g.app_id), &art::header(g.app_id)], r) {
                    w::skeleton(ui, t, rect, r);
                }
                w::fade_right(ui, rect, t.overlay_dark(3), 0.65);
                w::fade_down(ui, rect, t.overlay_dark(2), 0.4);
                let inner = rect.shrink(t.spacing.xl + t.spacing.sm);
                ui.painter().text(inner.left_top(), Align2::LEFT_TOP, t!("CONTINUE PLAYING"), t.typography.caption.font(), t.accent);
                let btn_h = (t.height_of(&t.components.button_play, 40.0) * 1.325).round();
                let logo = Rect::from_min_max(inner.left_top() + vec2(0.0, t.spacing.xl + t.spacing.xs), inner.left_bottom() + vec2(380.0, -btn_h - t.spacing.lg));
                if !w::paint_contain(ui, logo, &art::logo(g.app_id), Align2::LEFT_BOTTOM) {
                    ui.painter().text(logo.left_bottom(), Align2::LEFT_BOTTOM, &g.name, egui::FontId::new(t.typography.heading_l.size * 1.3, t.typography.heading_l.weight.family()), t.text);
                }
                let btn = Rect::from_min_size(inner.left_bottom() - vec2(0.0, btn_h), vec2(600.0, btn_h));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(btn).layout(Layout::left_to_right(Align::Center)));
                let playing = ctx.state.playing == Some(g.app_id);
                self.play_button(&mut c, t, ctx, g);
                c.add_space(t.spacing.md);
                if let Some(m) = g.playtime_minutes {
                    c.label(w::body_m(t, t!("{time} played", time = ui::playtime(m)), t.text));
                }
                self.game_menu(t, &resp, g.app_id, true, playing);
                if resp.clicked() {
                    self.selected = Some(g.app_id);
                }
                ui.add_space(t.spacing.xl * 1.5);
            }

            // Recent games: everything played lately, installed or not, one row.
            let first = recent.first().map(|g| g.app_id);
            let all = entries(state, local);
            let shelf: Vec<Entry> = ids.iter().filter(|id| Some(**id) != first).filter_map(|id| all.iter().find(|e| e.app_id == *id)).map(|e| Entry { app_id: e.app_id, name: e.name, installed: e.installed, owned: e.owned }).collect();
            if !shelf.is_empty() {
                ui.label(w::heading_s(t, t!("Recent games").to_uppercase(), t.text));
                ui.add_space(t.spacing.xl);
                let size = w::game_card_size(t, self.tile);
                let per = (((ui.available_width() + t.spacing.xl) / (size.x + t.spacing.xl)).floor() as usize).max(1);
                self.games_grid(ui, t, ctx, &shelf[..per.min(shelf.len())]);
                ui.add_space(t.spacing.md);
            }

            // Every game.
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
            self.games_grid(ui, t, ctx, &items);
        });
    }

    fn download_bar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, d: &model::DownloadStatus) {
        let label = i18n::tr_dyn(d.activity());
        // Narrower when the play bar is short of room.
        let width = (ui.available_width() * 0.45).clamp(200.0, 380.0);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(w::body_m(t, label, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Like Steam: an update shows only its own size.
                    let (done, total) = d.shown();
                    ui.label(w::body_s(t, format!("{} / {}", ui::bytes(done), ui::bytes(total)), t.text_dim));
                });
            });
            w::progress(ui, t, d.fraction(), width);
            // Network and disk separately (they differ a lot: reused data
            // and buffered writes hit the disk without coming over the network).
            if !d.verifying() {
                ui.label(w::body_s(t, t!("Network {net}/s · Disk {disk}/s", net = ui::bytes(d.speed), disk = ui::bytes(d.disk_speed)), t.text_faint));
            }
        });
        if d.verifying() {
            if w::icon_button(ui, t, icons::X, t!("Cancel"), false, 0).clicked() {
                ctx.send(Command::CancelVerify { app_id: id });
            }
            return;
        }
        if d.paused {
            if w::icon_button(ui, t, icons::PLAY, t!("Resume"), false, 0).clicked() {
                ctx.send(Command::ResumeDownload { app_id: id });
            }
        } else if w::icon_button(ui, t, "\u{E39E}", t!("Pause"), false, 0).clicked() {
            ctx.send(Command::PauseDownload { app_id: id });
        }
        if w::icon_button(ui, t, icons::X, t!("Cancel"), false, 0).clicked() {
            ctx.send(Command::CancelDownload { app_id: id });
        }
    }

    fn manage_menu(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, installed: Option<&InstalledGame>) {
        ui.set_min_width(240.0);
        let item = |ui: &mut Ui, icon: &str, label: &str, color: Color32| w::menu_item_tinted(ui, t, icon, label, (color != t.text).then_some(color)).clicked();
        if let Some(g) = installed {
            if item(ui, icons::DESKTOP, t!("Add desktop shortcut"), t.text) {
                self.add_shortcut(ctx, id, g);
            }
            if item(ui, icons::FOLDER, t!("Browse local files"), t.text) {
                local::open_folder(&g.install_dir);
            }
            if item(ui, icons::SHIELD, t!("Verify game files"), t.text) {
                self.start_verify(ctx, g);
            }
        }
        if item(ui, icons::INFO, t!("Get help with this game"), t.text) {
            ctx.navigate("support", Some(format!("game:{id}")));
        }
        if item(ui, icons::SLIDERS, t!("Properties…"), t.text) {
            self.props = Some(props::Props { app: id, tab: props::PropTab::General, ..Default::default() });
        }
        if installed.is_some() {
            ui.separator();
            if item(ui, icons::TRASH, t!("Uninstall…"), t.danger) {
                self.dialog = Some(Dialog::Uninstall);
            }
        }
    }

    /// DLC owned for this game, as a strip of their pictures.
    fn dlc_strip(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId) {
        let dlc: Vec<(AppId, String)> = ctx
            .state
            .apps
            .as_ref()
            // Like Steam: only DLC with a store page. Free "DLC" such as Dota 2's
            // Vulkan support stays in Properties, not on the game's page.
            .map(|a| a.values().filter(|d| d.is_dlc() && d.parent == Some(id) && d.store_assets).map(|d| (d.app_id, d.name.clone())).collect())
            .unwrap_or_default();
        if dlc.is_empty() {
            return;
        }
        let resp = w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::body_l(t, t!("DLC ({n})", n = dlc.len()), t.text));
            ui.add_space(t.spacing.xs);
            egui::ScrollArea::horizontal().id_salt(("dlc", id)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = t.spacing.sm;
                    for (d, name) in &dlc {
                        let (r, resp) = ui.allocate_exact_size(vec2(220.0, 103.0), Sense::click());
                        // DLC without its own art shows the game's, with a DLC badge and the name, like Steam.
                        let url = art::header(*d);
                        if !w::paint_cover(ui, r, &url, t.corner_small()) {
                            if w::image_state(ui, &url) == w::ImageState::Missing {
                                w::image_or_skeleton(ui, t, r, &art::header(id), t.corner_small());
                                let shade = Rect::from_min_max(pos2(r.left(), r.bottom() - 30.0), r.max);
                                ui.painter().rect_filled(shade, egui::CornerRadius { sw: t.corner_small().sw, se: t.corner_small().se, nw: 0, ne: 0 }, t.overlay_dark(3));
                                let galley = ui.painter().layout(name.clone(), w::font_semibold(t.typography.body_s.size), t.text, r.width() - 16.0);
                                ui.painter().galley(pos2(r.left() + t.spacing.sm, shade.center().y - galley.size().y / 2.0), galley, t.text);
                                let badge = Rect::from_min_size(r.min, vec2(34.0, 17.0));
                                ui.painter().rect_filled(badge, egui::CornerRadius { nw: t.corner_small().nw, se: t.corner_small().se, ne: 0, sw: 0 }, t.accent_2);
                                ui.painter().text(badge.center(), Align2::CENTER_CENTER, "DLC", t.typography.caption.font(), t.text_on(t.accent_2));
                            } else {
                                w::skeleton(ui, t, r, t.corner_small());
                            }
                        }
                        let off = self.games.get(&id).is_some_and(|g| g.disabled_dlc.contains(d));
                        if off {
                            ui.painter().rect_filled(r, t.corner_small(), t.overlay_dark(2));
                        }
                        if resp.on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            ctx.navigate("store", Some(format!("app:{d}")));
                        }
                    }
                });
            });
        });
        if std::mem::take(&mut self.scroll_to_dlc) {
            resp.response.scroll_to_me(Some(Align::Min));
        }
    }

    fn queue_install(&mut self, ctx: &mut Ctx, id: AppId, library: Option<std::path::PathBuf>) {
        let s = self.games.get(&id).cloned().unwrap_or_default();
        let library = library.or_else(|| ctx.local.libraries.first().cloned()).unwrap_or_else(|| ctx.local.steam_root.clone());
        let steam_root = if ctx.local.steam_root.as_os_str().is_empty() { library.clone() } else { ctx.local.steam_root.clone() };
        let language = if s.language.is_empty() { i18n::language().steam_name().to_string() } else { s.language.clone() };
        ctx.send(Command::Install(client::download::InstallRequest {
            app_id: id,
            library,
            steam_root,
            branch: if s.branch.is_empty() { "public".into() } else { s.branch.clone() },
            language,
            skip_dlc: s.disabled_dlc.iter().copied().collect(),
            add_dlc: s.enabled_dlc.iter().copied().collect(),
        }));
        ctx.toast(t!("Added to downloads"));
    }

    fn add_shortcut(&mut self, ctx: &mut Ctx, id: AppId, g: &InstalledGame) {
        let owned = ctx.state.apps.as_ref().and_then(|a| a.get(&id));
        match crate::links::desktop_shortcut(id, &g.name, crate::links::game_icon(owned, &g.install_dir)) {
            Ok(_) => ctx.toast(t!("Shortcut added to your desktop")),
            Err(e) => ctx.toast(e),
        }
    }

    fn install_window(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(dlg) = &mut self.install_dialog else { return };
        let id = dlg.app;
        let name = ctx.state.app_name(id).unwrap_or_default().to_string();
        let mut go = false;
        let mut close = false;
        let mut found: Option<std::path::PathBuf> = None;
        egui::Modal::new(egui::Id::unique(("install", id))).show(ui.ctx(), |ui| {
            ui.set_width(520.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(150.0, 70.0), Sense::hover());
                w::image_or_skeleton(ui, t, r, &art::header(id), t.corner_small());
                ui.vertical(|ui| {
                    ui.label(w::body_m(t, t!("Install"), t.text_dim));
                    ui.label(w::heading_s(t, &name, t.text));
                });
            });
            ui.add_space(12.0);
            ui.label(w::body_s(t, t!("Install to"), t.text_dim));
            for lib in &dlg.libraries {
                let free = fs4_free(lib);
                let label = format!("{}   ·   {}", lib.display(), t!("{size} free", size = free.map(ui::bytes).unwrap_or_else(|| "?".into())));
                w::radio_value(ui, t, &mut dlg.library, lib.clone(), w::body_m(t, label, t.text));
            }
            if w::button(ui, t, Some(icons::FOLDER), t!("Choose another folder…"), w::Kind::Ghost, w::Size::Sm).clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    // Picked its steamapps or the game's folder: the library.
                    let p = local::library_root_of(&p);
                    if local::game_in_library(&p, id).is_some() {
                        // Already installed there (by Steam, or a download
                        // from before this folder was remembered).
                        found = Some(p);
                    } else {
                        let _ = std::fs::create_dir_all(p.join("steamapps"));
                        if !dlg.libraries.iter().any(|l| local::same_path(l, &p)) {
                            dlg.libraries.push(p.clone());
                        }
                        dlg.library = p;
                    }
                }
            }
            ui.add_space(8.0);
            if dlg.size > 0 {
                ui.label(w::body_m(t, t!("Space required: {size}", size = ui::bytes(dlg.size)), t.text));
                if fs4_free(&dlg.library).is_some_and(|f| f < dlg.size) {
                    ui.label(w::body_m(t, format!("{}  {}", icons::WARNING, t!("Not enough free space on this drive.")), t.danger));
                }
            }
            ui.add_space(6.0);
            w::check(ui, t, &mut dlg.desktop, t!("Create a desktop shortcut"));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Install"), w::Kind::Primary, w::Size::Md).clicked() {
                    go = true;
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked() {
                    close = true;
                }
            });
        });
        if let Some(lib) = found {
            // Nothing to download: remember the folder and show the game.
            client::store::remember_library(&lib);
            ctx.act(extensions::Action::ReloadLocal);
            ctx.toast(t!("{name} is already installed in {folder}", name = name, folder = lib.display().to_string()));
            self.install_dialog = None;
        } else if go {
            let d = self.install_dialog.take().unwrap();
            if d.desktop {
                self.shortcut_after_install.insert(id, d.library.clone());
            }
            self.queue_install(ctx, id, Some(d.library));
        } else if close {
            self.install_dialog = None;
        }
    }

    fn start_verify(&mut self, ctx: &mut Ctx, g: &InstalledGame) {
        ctx.send(Command::VerifyGame { game: g.clone(), steam_root: ctx.local.steam_root.clone() });
        ctx.navigate("downloads", None);
    }

    /// Like Steam: report the check, and download damaged or missing files again.
    fn verify_finished(&mut self, ctx: &mut Ctx, id: AppId, result: &Result<local::manage::VerifyReport, String>) {
        let name = ctx.state.app_name(id).map(String::from).or_else(|| ctx.local.games.iter().find(|g| g.app_id == id).map(|g| g.name.clone())).unwrap_or_default();
        let report = match result {
            Ok(r) => r.clone(),
            Err(e) => {
                ctx.toast(i18n::tr_dyn(e).to_string());
                return;
            }
        };
        if report.cancelled {
            ctx.toast(t!("{name}: verification cancelled.", name = name));
        } else if !report.unchecked_depots.is_empty() && report.files == 0 {
            ctx.toast(t!("{name}: Steam's file list for this game isn't on this PC, so it can't be checked yet.", name = name));
        } else if report.healthy() {
            ctx.toast(t!("{name}: all {n} files are OK.", name = name, n = report.ok));
        } else {
            let bad = report.missing.len() + report.corrupt.len();
            ctx.toast(t!("{name}: {n} files failed to validate and will be downloaded again.", name = name, n = bad));
            if let Some(g) = ctx.local.games.iter().find(|g| g.app_id == id).cloned() {
                self.queue_install(ctx, id, Some(g.library.clone()));
            }
        }
        self.reports.insert(id, report);
    }

    /// Start an installed game with its saved launch options.
    fn launch_game(&mut self, ctx: &mut Ctx, id: AppId, install_dir: std::path::PathBuf) {
        let args = client::LaunchArgs::user(self.games.get(&id).map(|s| s.args.clone()).unwrap_or_default());
        ctx.send(Command::Launch { app_id: id, install_dir, args });
    }

    /// Recent news posts (last 45 days) from these games, newest first, as
    /// (game, index in its feed, post); `true` while some are still loading.
    pub(crate) fn whats_new_posts(&mut self, ids: &[AppId]) -> (Vec<(AppId, usize, store::events::GameEvent)>, bool) {
        let mut posts: Vec<(AppId, usize, store::events::GameEvent)> = Vec::new();
        let mut loading = false;
        for id in ids.iter().take(10) {
            match self.events(*id) {
                Some(list) => posts.extend(list.into_iter().enumerate().take(3).map(|(i, e)| (*id, i, e))),
                None => loading = true,
            }
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        posts.retain(|p| now.saturating_sub(p.2.time) < 45 * 86_400);
        posts.sort_by_key(|p| std::cmp::Reverse(p.2.time));
        (posts, loading)
    }

    /// What the game's main button does right now (the library page and Big
    /// Picture show the same thing).
    pub(crate) fn main_action(&mut self, ctx: &Ctx, id: AppId) -> MainAction {
        if let Some(d) = ctx.state.downloads.get(&id).filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)) {
            return MainAction::Busy { fraction: d.fraction(), verifying: d.verifying(), paused: d.paused };
        }
        if ctx.state.download_queue.contains(&id) {
            return MainAction::Queued;
        }
        let installed = ctx.local.games.iter().find(|g| g.app_id == id).cloned();
        match installed {
            Some(_) if self.uninstalling.contains_key(&id) => MainAction::Uninstalling,
            Some(_) if ctx.state.playing == Some(id) => MainAction::Running,
            Some(g) if self.needs_update(ctx, &g) => MainAction::Update,
            Some(_) if ctx.state.cloud_syncing.contains(&id) => MainAction::Syncing,
            Some(_) if ctx.state.first_time_setup.contains_key(&id) => MainAction::FirstTimeSetup,
            Some(_) => MainAction::Play,
            None if ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&id)) => MainAction::Install,
            None => MainAction::StorePage,
        }
    }

    /// Do what the main button says: play, stop, update or install (to the
    /// drive with the most free space, without asking: for Big Picture).
    pub(crate) fn run_main_action(&mut self, ctx: &mut Ctx, id: AppId) {
        let installed = ctx.local.games.iter().find(|g| g.app_id == id).cloned();
        match (self.main_action(ctx, id), installed) {
            (MainAction::Play, Some(g)) => self.launch_game(ctx, id, g.install_dir.clone()),
            (MainAction::Running, _) => ctx.send(Command::StopGame { app_id: id }),
            (MainAction::Update, Some(g)) => self.queue_install(ctx, id, Some(g.library.clone())),
            (MainAction::Install, _) => {
                let dialog = InstallDialog::new(id, ctx);
                self.queue_install(ctx, id, Some(dialog.library));
            }
            (MainAction::Busy { paused: true, .. }, _) => ctx.send(Command::ResumeDownload { app_id: id }),
            (MainAction::Busy { verifying: false, .. }, _) => ctx.send(Command::PauseDownload { app_id: id }),
            _ => {}
        }
    }

    /// Hidden from the library by the user.
    pub(crate) fn is_hidden(&self, id: AppId) -> bool {
        self.hidden.contains(&id)
    }

    /// Questions the library may need answered while another screen is up
    /// (a Steam Cloud conflict before a game starts).
    pub(crate) fn background_dialogs(&mut self, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        self.cloud_conflict_dialog(ui, &t, ctx);
    }

    /// Is a newer build of the installed version out? (Cached: it reads the
    /// game's manifest from disk.)
    fn needs_update(&mut self, ctx: &Ctx, g: &InstalledGame) -> bool {
        if g.state == model::InstallState::UpdateRequired {
            return true;
        }
        let now = std::time::Instant::now();
        let stale = self.builds.get(&g.app_id).is_none_or(|(t, _, _)| now.duration_since(*t).as_secs() > 10);
        if stale {
            let (build, branch) = local::manage::installed_build(g);
            self.builds.insert(g.app_id, (now, build, branch));
        }
        let (_, build, branch) = &self.builds[&g.app_id];
        let latest = ctx.state.apps.as_ref().and_then(|a| a.get(&g.app_id)).and_then(|o| o.branches.iter().find(|b| &b.name == branch)).map(|b| b.build_id).unwrap_or(0);
        *build != 0 && latest != 0 && latest != *build
    }

    /// The game's main button, like Steam's: Play, Update, or what's running.
    fn play_button(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, g: &InstalledGame) {
        let id = g.app_id;
        let busy = ctx.state.downloads.get(&id).filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).cloned();
        if let Some(d) = busy {
            let what = if d.verifying() { t!("Verifying…") } else { t!("Updating…") };
            let label = format!("{what} {}%", (d.fraction() * 100.0) as u32);
            if ui.add_enabled_ui(true, |ui| w::button(ui, t, Some(icons::DOWNLOAD), &label, w::Kind::Secondary, w::Size::Lg)).inner.on_hover_text(t!("Open Downloads")).clicked() {
                ctx.navigate("downloads", None);
            }
        } else if ctx.state.download_queue.contains(&id) {
            ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::CLOCK), t!("Queued"), w::Kind::Secondary, w::Size::Lg));
        } else if ctx.state.playing == Some(id) {
            ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::PLAY), t!("Running"), w::Kind::Play, w::Size::Lg));
        } else if self.needs_update(ctx, g) {
            if w::button(ui, t, Some(icons::DOWNLOAD), t!("Update"), w::Kind::Primary, w::Size::Lg).clicked() {
                self.queue_install(ctx, id, Some(g.library.clone()));
            }
        } else if w::button(ui, t, Some(icons::PLAY), t!("Play"), w::Kind::Play, w::Size::Lg).clicked() {
            self.launch_game(ctx, id, g.install_dir.clone());
        }
    }

    fn dialogs(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, name: &str, installed: Option<&InstalledGame>) {
        if !matches!(self.dialog, Some(Dialog::Uninstall)) {
            return;
        }
        let mut close = false;
        let Some(g) = installed else {
            self.dialog = None;
            return;
        };
        if self.uninstalling.contains_key(&id) {
            self.dialog = None;
            return;
        }
        let running = ctx.state.playing == Some(id);
        egui::Modal::new(egui::Id::unique(("uninstall", id))).show(ui.ctx(), |ui| {
            ui.set_width(460.0);
            ui.label(w::text(icons::TRASH, t.typography.heading_l.size, t.danger));
            ui.label(w::heading_s(t, t!("Uninstall {name}?", name = name), t.text));
            ui.add_space(4.0);
            let workshop = local::manage::workshop_size(g);
            ui.add(egui::Label::new(w::body_m(t, t!("This deletes {size} from:", size = ui::bytes(g.size_on_disk + workshop)), t.text_dim)).wrap());
            ui.label(w::body_m(t, g.install_dir.display().to_string(), t.text).monospace());
            if workshop > 0 {
                ui.add(egui::Label::new(w::body_s(t, t!("Its Workshop items ({size}) are removed too.", size = ui::bytes(workshop)), t.text_dim)).wrap());
            }
            ui.add_space(4.0);
            ui.add(egui::Label::new(w::body_s(t, t!("Saves kept inside the game folder are deleted too. Steam Cloud saves aren't affected."), t.text_faint)).wrap());
            if running {
                ui.label(w::body_m(t, format!("{}  {}", icons::WARNING, t!("Close the game first.")), t.away));
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let go = ui.add_enabled_ui(!running, |ui| w::button(ui, t, Some(icons::TRASH), t!("Uninstall"), w::Kind::Danger, w::Size::Md)).inner.clicked();
                if go {
                    // Deleting a big game takes a while: do it in the
                    // background, with progress on the game's button.
                    let job = Uninstalling { name: name.to_string(), progress: Arc::new(Progress::default()), result: Arc::new(Mutex::new(None)) };
                    let (game, progress, result, egui_ctx) = (g.clone(), job.progress.clone(), job.result.clone(), ui.ctx().clone());
                    std::thread::spawn(move || {
                        let r = local::manage::uninstall_with(&game, &progress);
                        *result.lock().unwrap() = Some(r);
                        egui_ctx.request_repaint();
                    });
                    self.uninstalling.insert(id, job);
                    close = true;
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.dialog = None;
        }
    }
}

/// A game being uninstalled in the background.
struct Uninstalling {
    name: String,
    progress: Arc<Progress>,
    result: Arc<Mutex<Option<Result<(), String>>>>,
}

impl Library {
    /// Report uninstalls that ended, and refresh the installed games.
    fn finish_uninstalls(&mut self, ctx: &mut Ctx) {
        let done: Vec<AppId> = self.uninstalling.iter().filter(|(_, j)| j.result.lock().unwrap().is_some()).map(|(id, _)| *id).collect();
        for id in done {
            let Some(job) = self.uninstalling.remove(&id) else { continue };
            let name = job.name;
            match job.result.lock().unwrap().take() {
                Some(Ok(())) => {
                    // Its desktop shortcut would only lead nowhere now.
                    if crate::links::remove_desktop_shortcuts(id) > 0 {
                        ctx.toast(t!("{name} was uninstalled and its desktop shortcut removed", name = name));
                    } else {
                        ctx.toast(t!("{name} was uninstalled", name = name));
                    }
                }
                Some(Err(e)) => ctx.toast(e),
                None => {}
            }
            ctx.act(extensions::Action::ReloadLocal);
        }
    }
}

/// The play button while a game is being deleted: red, filling up, like Steam.
fn uninstall_bar(ui: &mut Ui, t: &Theme, fraction: f32) {
    let label = t!("Uninstalling… {pct}%", pct = (fraction.clamp(0.0, 1.0) * 100.0).round() as u32);
    let galley = ui.painter().layout_no_wrap(format!("{}  {label}", icons::TRASH), w::font_semibold(t.typography.button.size + 1.0), t.danger);
    let (rect, _) = ui.allocate_exact_size(vec2((galley.size().x + 48.0).max(220.0), 48.0), Sense::hover());
    let r = t.corner_small();
    let p = ui.painter();
    p.rect_filled(rect, r, t.danger.gamma_multiply(0.14));
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()));
    p.rect_filled(fill, r, t.danger.gamma_multiply(0.35));
    p.rect_stroke(rect, r, Stroke::new(1.0, t.danger.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    p.galley(pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0), galley, t.danger);
}

/// A titled list of check boxes (a group of the filter popup); returns the
/// index of the one clicked.
fn check_list(ui: &mut Ui, t: &Theme, title: &str, items: impl Iterator<Item = (bool, String)>) -> Option<usize> {
    let mut hit = None;
    ui.label(w::body_l(t, title, t.text));
    ui.add_space(t.spacing.xs);
    ui.horizontal(|ui| {
        ui.add_space(t.spacing.md);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = t.spacing.xs;
            for (i, (on, label)) in items.enumerate() {
                let mut v = on;
                if w::check(ui, t, &mut v, w::body_s(t, label, t.text)).changed() {
                    hit = Some(i);
                }
            }
        });
    });
    ui.add_space(t.spacing.md);
    hit
}

/// Add `v` to the set, or take it out if it's there.
fn toggle_set<T: Ord>(set: &mut BTreeSet<T>, v: Option<T>) {
    if let Some(v) = v
        && !set.remove(&v)
    {
        set.insert(v);
    }
}

fn fs4_free(path: &std::path::Path) -> Option<u64> {
    fs4::available_space(path).ok()
}

pub(crate) struct InstallDialog {
    app: AppId,
    libraries: Vec<std::path::PathBuf>,
    library: std::path::PathBuf,
    size: u64,
    desktop: bool,
}

impl InstallDialog {
    pub(crate) fn new(app: AppId, ctx: &Ctx) -> Self {
        let mut libraries = ctx.local.libraries.clone();
        if libraries.is_empty() {
            // No Steam here: offer Steam's usual folder, like its installer
            // would use (our own folder if that one can't be written).
            let fallback = client::store::config_dir().join("Library");
            let default = local::default_steam_root().filter(|d| std::fs::create_dir_all(d.join("steamapps")).is_ok()).unwrap_or_else(|| {
                let _ = std::fs::create_dir_all(fallback.join("steamapps"));
                fallback
            });
            libraries.push(default);
        }
        let size = ctx
            .state
            .apps
            .as_ref()
            .and_then(|a| a.get(&app))
            .map(|o| o.depots.iter().filter(|d| d.dlc_app.is_none() && !d.shared_install && model::os_matches(&d.os, model::THIS_OS)).filter_map(|d| d.manifest("public").map(|m| m.1)).sum())
            .unwrap_or(0);
        // The library that already has the game's folder (files left by an
        // earlier install: installing there checks and keeps them), else
        // the one with the most free space.
        let folder = ctx.state.apps.as_ref().and_then(|a| a.get(&app)).map(|o| if o.install_dir.is_empty() { o.name.clone() } else { o.install_dir.clone() });
        let has_files = |l: &std::path::PathBuf| folder.as_deref().filter(|f| local::is_valid_folder_name(f)).is_some_and(|f| l.join("steamapps").join("common").join(f).is_dir());
        let library = libraries.iter().find(|l| has_files(l)).or_else(|| libraries.iter().max_by_key(|l| fs4_free(l).unwrap_or(0))).cloned().unwrap_or_default();
        Self { app, libraries, library, size, desktop: true }
    }
}


/// What a dynamic collection's filters look at for one game.
fn facts<'a>(e: &'a Entry) -> client::collections::GameFacts<'a> {
    client::collections::GameFacts {
        name: e.name,
        installed: e.installed.is_some(),
        played: e.installed.and_then(|g| g.last_played).is_some_and(|t| t > 0),
        runs_here: e.owned.is_none_or(|o| o.runs_on_this_os()),
        categories: e.owned.map(|o| o.categories.as_slice()).unwrap_or_default(),
        tags: e.owned.map(|o| o.tags.as_slice()).unwrap_or_default(),
        deck: e.owned.map(|o| o.deck).unwrap_or(0),
    }
}

#[cfg(test)]
mod collection_sync_tests {
    use super::*;

    fn steam(list: &[(&str, &str, &[AppId])]) -> Vec<model::SteamCollection> {
        list.iter().map(|(id, name, games)| model::SteamCollection { id: id.to_string(), name: name.to_string(), games: games.to_vec(), dynamic: false, filter: None }).collect()
    }

    #[test]
    fn first_sync_merges_both_sides() {
        let mut l = Library::default();
        l.favorites = [10, 20].into();
        l.collections.push(Collection { name: "Mine".into(), games: [5].into(), steam_id: None });
        l.merge_steam_collections(&steam(&[("favorite", "Favorites", &[20, 30]), ("uc-a", "Theirs", &[7])]));
        assert_eq!(l.favorites, [10, 20, 30].into());
        assert_eq!(l.collections.len(), 2);
        // What Steam lacks is still to be sent.
        assert_ne!(l.collections_now(), l.steam_base);
    }

    #[test]
    fn steam_wins_where_nothing_changed_here() {
        let mut l = Library::default();
        l.merge_steam_collections(&steam(&[("favorite", "Favorites", &[1, 2]), ("uc-a", "A", &[3])]));
        // Elsewhere: 2 un-favorited, collection A deleted, B made.
        l.merge_steam_collections(&steam(&[("favorite", "Favorites", &[1]), ("uc-b", "B", &[4])]));
        assert_eq!(l.favorites, [1].into());
        assert_eq!(l.collections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["B"]);
        assert_eq!(l.collections_now(), l.steam_base);
    }

    #[test]
    fn edits_made_before_steam_answers_are_kept() {
        let mut l = Library::default();
        l.merge_steam_collections(&steam(&[("favorite", "Favorites", &[1]), ("uc-a", "A", &[3])]));
        l.steam_loaded = false;
        // Next start: edits before Steam's copy arrives.
        l.favorites.insert(2);
        l.collections.retain(|c| c.name != "A");
        l.merge_steam_collections(&steam(&[("favorite", "Favorites", &[1]), ("uc-a", "A", &[3])]));
        assert_eq!(l.favorites, [1, 2].into());
        assert!(l.collections.is_empty());
        // Both changes go to Steam.
        let now = l.collections_now();
        assert_eq!(now.get("favorite").map(|(_, g)| g.clone()), Some([1, 2].into()));
        assert!(!now.contains_key("uc-a") && l.steam_base.contains_key("uc-a"));
    }
}

/// What installing a game needs (for Big Picture's install screen).
pub(crate) struct InstallPlan {
    /// Library folders with their free space.
    pub libraries: Vec<(std::path::PathBuf, u64)>,
    /// The one with the most free space.
    pub default: std::path::PathBuf,
    /// Size on disk.
    pub size: u64,
}

impl Library {
    pub(crate) fn install_plan(&self, ctx: &Ctx, id: AppId) -> InstallPlan {
        let d = InstallDialog::new(id, ctx);
        let libraries = d.libraries.iter().map(|l| (l.clone(), fs4_free(l).unwrap_or(0))).collect();
        InstallPlan { libraries, default: d.library, size: d.size }
    }

    /// Install to a chosen folder, with a desktop shortcut afterwards if asked.
    pub(crate) fn install_to(&mut self, ctx: &mut Ctx, id: AppId, library: std::path::PathBuf, shortcut: bool) {
        if shortcut {
            self.shortcut_after_install.insert(id, library.clone());
        }
        self.queue_install(ctx, id, Some(library));
    }
}

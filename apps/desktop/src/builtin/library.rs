//! Library: sidebar game list, library home with shelves, and game pages.

use i18n::t;
use super::{Entry, entries};

#[path = "library_news.rs"]
mod news;
#[path = "library_props.rs"]
mod props;
use client::Command;
use egui::{Align, Align2, Color32, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
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
}

impl AdvFilters {
    fn count(&self) -> usize {
        self.play.len() + self.kinds.len() + self.genres.len() + self.features.len() + self.deck.len()
    }
}

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
    args: String,
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

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    tile: f32,
    games: HashMap<AppId, GameSettings>,
    favorites: BTreeSet<AppId>,
    hidden: BTreeSet<AppId>,
    custom: Vec<CustomGame>,
    collections: Vec<Collection>,
    /// Kept across restarts so a shortcut still appears if the app was
    /// closed during the download.
    pending_shortcuts: BTreeMap<AppId, std::path::PathBuf>,
}

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
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    All,
    Installed,
    Playing,
    Favorites,
    Collection(usize),
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
    adv: AdvFilters,
    /// Naming a new collection (with a game to put in it), or renaming one.
    naming: Option<(Option<usize>, String, Option<AppId>)>,
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
        // Links can ask for an action: "install:ID", "uninstall:ID", "verify:ID".
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
            client::Event::SteamCollections(Ok(list)) => self.merge_steam_collections(list),
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

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        self.finish_uninstalls(ctx);
        if !self.uninstalling.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.downloading = ctx.state.downloads.values().filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).map(|d| (d.app_id, d.paused)).collect();
        self.queued = ctx.state.download_queue.iter().copied().collect();
        if self.tile == 0.0 {
            self.tile = 160.0;
        }
        egui::Panel::left("library-list")
            .exact_size(280.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin { left: 12, right: 6, top: 14, bottom: 8 }))
            .show(ui, |ui| self.sidebar(ui, &t, ctx));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| match (self.custom_selected, self.selected) {
            (Some(i), _) => self.custom_page(i, ui, &t, ctx),
            (None, Some(id)) => self.game_page(id, ui, &t, ctx),
            (None, None) => self.home(ui, &t, ctx),
        });
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
    }

    fn save(&self) -> Option<String> {
        serde_json::to_string(&Saved {
            tile: self.tile,
            games: self.games.clone(),
            favorites: self.favorites.clone(),
            hidden: self.hidden.clone(),
            custom: self.custom.clone(),
            collections: self.collections.clone(),
            pending_shortcuts: self.shortcut_after_install.clone(),
        })
        .ok()
    }

    fn load(&mut self, data: &str) {
        let saved: Saved = serde_json::from_str(data).unwrap_or_default();
        self.tile = if saved.tile > 0.0 { saved.tile } else { 160.0 };
        self.games = saved.games;
        self.favorites = saved.favorites;
        self.hidden = saved.hidden;
        self.custom = saved.custom;
        self.collections = saved.collections;
        self.shortcut_after_install = saved.pending_shortcuts;
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

    /// Bring in the collections made in the official Steam client: Favorites
    /// and Hidden fill ours, the rest show up as collections here.
    fn merge_steam_collections(&mut self, list: &[model::SteamCollection]) {
        for c in list {
            match c.id.as_str() {
                "favorite" => self.favorites.extend(c.games.iter().copied()),
                "hidden" => self.hidden.extend(c.games.iter().copied()),
                _ if c.dynamic => {}
                _ => match self.collections.iter_mut().find(|x| x.steam_id.as_deref() == Some(c.id.as_str())) {
                    Some(mine) => {
                        mine.name = c.name.clone();
                        mine.games = c.games.iter().copied().collect();
                    }
                    None => self.collections.push(Collection { name: c.name.clone(), games: c.games.iter().copied().collect(), steam_id: Some(c.id.clone()) }),
                },
            }
        }
    }

    /// Name a new collection, or rename one.
    fn naming_dialog(&mut self, ui: &mut Ui, t: &Theme) {
        let Some((index, name, _)) = &mut self.naming else { return };
        let mut done = None;
        egui::Modal::new(egui::Id::unique("collection-name")).show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            ui.label(w::text_bold(if index.is_some() { t!("Rename collection") } else { t!("New collection") }, 18.0, t.text));
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(t!("Name, e.g. Co-op with friends")).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0)));
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
                        self.filter = Filter::Collection(self.collections.len() - 1);
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
                ui.label(w::text(icons::CLOUD_CHECK, 28.0, t.away));
                ui.vertical(|ui| {
                    ui.label(w::text_bold(t!("Steam Cloud conflict"), 19.0, t.text));
                    ui.label(w::text(&name, 13.5, t.text_dim));
                });
            });
            ui.add_space(6.0);
            ui.add(egui::Label::new(w::text(t!("Your saves on this PC and in Steam Cloud both changed since the last sync. Pick which to keep. The other copy is backed up, not lost."), 13.5, t.text_dim)).wrap());
            ui.add_space(10.0);
            for c in hold.report.conflicts.iter().take(6) {
                w::panel(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let file = c.name.rsplit(['/', '%']).next().unwrap_or(&c.name);
                    ui.label(w::text_semibold(file, 13.5, t.text)).on_hover_text(&c.name);
                    ui.columns(2, |cols| {
                        cols[0].label(w::text_bold(t!("ON THIS PC"), 10.5, t.text_faint));
                        cols[0].label(w::text(format!("{} · {}", ui::date(c.local_time), ui::bytes(c.local_size)), 12.5, t.text));
                        cols[1].label(w::text_bold(t!("IN STEAM CLOUD"), 10.5, t.text_faint));
                        cols[1].label(w::text(format!("{} · {}", ui::date(c.cloud_time), ui::bytes(c.cloud_size)), 12.5, t.text));
                    });
                });
                ui.add_space(4.0);
            }
            if hold.report.conflicts.len() > 6 {
                ui.label(w::text(t!("and {n} more files", n = hold.report.conflicts.len() - 6), 12.5, t.text_faint));
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
    fn whats_new(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, recent: &[(AppId, String, u64)]) {
        let mut posts: Vec<(AppId, usize, store::events::GameEvent)> = Vec::new();
        let mut loading = false;
        for (id, _, _) in recent.iter().take(10) {
            match self.events(*id) {
                Some(list) => posts.extend(list.into_iter().enumerate().take(3).map(|(i, e)| (*id, i, e))),
                None => loading = true,
            }
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        posts.retain(|p| now.saturating_sub(p.2.time) < 45 * 86_400);
        posts.sort_by_key(|p| std::cmp::Reverse(p.2.time));
        if posts.is_empty() && !loading {
            return;
        }
        w::section(ui, t, t!("What's new"), None);
        let card = vec2(300.0, 230.0);
        egui::ScrollArea::horizontal().id_salt("whats-new").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                if posts.is_empty() {
                    for _ in 0..4 {
                        let (r, _) = ui.allocate_exact_size(card, Sense::hover());
                        w::skeleton(ui, t, r, t.corner());
                    }
                }
                for (id, index, e) in posts.iter().take(12) {
                    let (r, resp) = ui.allocate_exact_size(card, Sense::click());
                    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                    let age = now.saturating_sub(e.time) / 86_400;
                    let when = match age {
                        0 => t!("Today").to_string(),
                        1 => t!("Yesterday").to_string(),
                        2..=6 => t!("This week").to_string(),
                        _ => ui::date(e.time),
                    };
                    ui.painter().text(r.left_top(), Align2::LEFT_TOP, when, w::font(12.0), t.text_faint);
                    let img = Rect::from_min_size(r.min + vec2(0.0, 18.0), vec2(card.x, 150.0));
                    match e.capsule.as_deref().or(e.header.as_deref()) {
                        Some(u) => w::image_or_title(ui, t, img, u, &e.title, t.corner_small()),
                        None => w::image_or_title(ui, t, img, &art::header(*id), &e.title, t.corner_small()),
                    }
                    if hover > 0.0 {
                        ui.painter().rect_stroke(img, t.corner_small(), Stroke::new(2.0, t.accent.gamma_multiply(hover)), egui::StrokeKind::Inside);
                    }
                    let text_rect = Rect::from_min_max(pos2(r.left(), img.bottom() + 6.0), r.max);
                    let mut job = egui::text::LayoutJob::simple(e.title.clone(), w::font_semibold(13.5), t.text, card.x);
                    job.wrap.max_rows = 2;
                    job.wrap.overflow_character = Some('…');
                    let galley = ui.painter().layout_job(job);
                    ui.painter().galley(text_rect.min, galley, t.text);
                    let game = ctx.state.app_name(*id).unwrap_or_default().to_string();
                    let icon = Rect::from_min_size(pos2(r.left(), r.bottom() - 18.0), vec2(16.0, 16.0));
                    let ok = ctx.state.apps.as_ref().and_then(|a| a.get(id)).and_then(|o| o.icon.as_deref()).is_some_and(|h| w::paint_cover(ui, icon, &art::icon(*id, h), CornerRadius::same(3)));
                    let x = if ok { icon.right() + 6.0 } else { r.left() };
                    ui.painter().text(pos2(x, icon.center().y), Align2::LEFT_CENTER, game, w::font(12.0), t.text_dim);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.reader = Some(Reader { app: *id, index: *index });
                    }
                }
            });
        });
        ui.add_space(26.0);
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
                ui.menu_button(format!("{}   {}", icons::GEAR, t!("Manage")), |ui| {
                    ui.set_min_width(220.0);
                    let sub = item;
                    sub(ui, icons::DESKTOP, t!("Add desktop shortcut"), MenuAct::Shortcut);
                    sub(ui, icons::FOLDER, t!("Browse local files"), MenuAct::Browse);
                    sub(ui, icons::SHIELD, t!("Verify game files"), MenuAct::Verify);
                    sub(ui, icons::TRASH, t!("Uninstall…"), MenuAct::Uninstall);
                });
            }
            ui.menu_button(format!("{}   {}", icons::STACK, t!("Add to collection")), |ui| {
                ui.set_min_width(200.0);
                for (i, c) in self.collections.iter().enumerate() {
                    let mut on = c.games.contains(&id);
                    if ui.checkbox(&mut on, &c.name).clicked() {
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
                    let args = self.games.get(&id).map(|s| s.args.clone()).unwrap_or_default();
                    ctx.send(Command::Launch { app_id: id, install_dir: g.install_dir.clone(), args });
                }
            }
            MenuAct::Stop => ctx.send(Command::StopGame { app_id: id }),
            MenuAct::PauseDownload => ctx.send(Command::PauseDownload { app_id: id }),
            MenuAct::ResumeDownload => ctx.send(Command::ResumeDownload { app_id: id }),
            MenuAct::CancelDownload => ctx.send(Command::CancelDownload { app_id: id }),
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
            MenuAct::NewCollection => self.naming = Some((None, String::new(), Some(id))),
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
        Self { rt: Some(rt), egui: Some(egui), tile: 160.0, flags: Some(flags), ..Default::default() }
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
        let mut items = entries(ctx.state, ctx.local);
        let q = self.search.to_lowercase();
        items.retain(|e| {
            (q.is_empty() || e.name.to_lowercase().contains(&q))
                && match self.filter {
                    Filter::All => true,
                    Filter::Installed => e.installed.is_some(),
                    Filter::Playing => e.installed.and_then(|g| g.last_played).is_some(),
                    Filter::Favorites => self.favorites.contains(&e.app_id),
                    Filter::Collection(i) => self.collections.get(i).is_some_and(|c| c.games.contains(&e.app_id)),
                }
                && (self.show_hidden || !self.hidden.contains(&e.app_id))
                && self.adv_match(ctx, e)
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
    fn adv_match(&self, ctx: &Ctx, e: &Entry) -> bool {
        let f = &self.adv;
        if f.count() == 0 {
            return true;
        }
        let o = e.owned;
        let minutes = e
            .installed
            .and_then(|g| g.playtime_minutes)
            .or_else(|| match ctx.state.player_games.get(&ctx.state.steam_id) {
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
        play_ok && kind_ok && genre_ok && feature_ok && deck_ok
    }

    /// The filter popup, like Steam's library filters.
    fn adv_panel(&mut self, ui: &mut Ui, t: &Theme, ctx: &Ctx) {
        // Only offer what this library has.
        let apps: Vec<&model::OwnedApp> = ctx.state.apps.as_ref().map(|a| a.values().filter(|a| !a.is_dlc()).collect()).unwrap_or_default();
        let has_genre = |g: u32| apps.iter().any(|a| a.genres.contains(&g));
        let has_feature = |c: u32| apps.iter().any(|a| a.categories.contains(&c));
        let mut kinds: Vec<String> = apps.iter().map(|a| a.kind.clone()).filter(|k| !k.is_empty()).collect();
        kinds.sort();
        kinds.dedup();
        let group = |ui: &mut Ui, title: &str| {
            ui.add_space(6.0);
            ui.label(w::text_bold(title.to_uppercase(), 11.0, t.text_faint));
        };
        let f = &mut self.adv;
        ui.horizontal(|ui| {
            ui.label(w::text_bold(t!("Filters"), 15.0, t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if f.count() > 0 && ui.link(w::text(t!("Clear all"), 12.5, t.accent)).clicked() {
                    *f = AdvFilters::default();
                }
            });
        });
        // Checkboxes in columns, compact enough to fit the window.
        {
            group(ui, t!("Play state"));
            let play = [(0u8, t!("Ready to play")), (1, t!("Not installed")), (2, t!("Played")), (3, t!("Never played"))];
            if let Some(i) = check_grid(ui, t, "f-play", 4, play.iter().map(|(v, l)| (f.play.contains(v), l.to_string()))) {
                let v = play[i].0;
                if !f.play.remove(&v) {
                    f.play.insert(v);
                }
            }
            group(ui, t!("Type"));
            let kind_label = |k: &str| match k {
                "game" => t!("Games").to_string(),
                "application" => t!("Software").to_string(),
                "tool" => t!("Tools").to_string(),
                "demo" => t!("Demos").to_string(),
                "music" => t!("Soundtracks").to_string(),
                other => other.to_string(),
            };
            if let Some(i) = check_grid(ui, t, "f-kind", 5, kinds.iter().map(|k| (f.kinds.contains(k), kind_label(k)))) {
                let k = &kinds[i];
                if !f.kinds.remove(k) {
                    f.kinds.insert(k.clone());
                }
            }
            group(ui, t!("Genre"));
            let genres: Vec<&(u32, &str)> = GENRES.iter().filter(|(g, _)| has_genre(*g)).collect();
            if let Some(i) = check_grid(ui, t, "f-genre", 3, genres.iter().map(|(g, l)| (f.genres.contains(g), i18n::tr_dyn(l).to_string()))) {
                let g = genres[i].0;
                if !f.genres.remove(&g) {
                    f.genres.insert(g);
                }
            }
            group(ui, t!("Features"));
            let features: Vec<&(u32, &str)> = FEATURES.iter().filter(|(c, _)| has_feature(*c)).collect();
            if let Some(i) = check_grid(ui, t, "f-feature", 3, features.iter().map(|(c, l)| (f.features.contains(c), i18n::tr_dyn(l).to_string()))) {
                let c = features[i].0;
                if !f.features.remove(&c) {
                    f.features.insert(c);
                }
            }
            group(ui, t!("Steam Deck"));
            let deck = [(3u8, t!("Verified")), (2, t!("Playable")), (1, t!("Unsupported")), (0, t!("Unknown"))];
            if let Some(i) = check_grid(ui, t, "f-deck", 4, deck.iter().map(|(v, l)| (f.deck.contains(v), l.to_string()))) {
                let v = deck[i].0;
                if !f.deck.remove(&v) {
                    f.deck.insert(v);
                }
            }
        }
    }

    fn sidebar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.horizontal(|ui| {
            let home = ui.add(egui::Button::selectable(self.selected.is_none() && self.custom_selected.is_none(), w::text_semibold(format!("{}   {}", icons::HOUSE, t!("Library home")), 14.0, t.text)).min_size(vec2(ui.available_width() - 48.0, 36.0)));
            if home.clicked() {
                self.selected = None;
                self.custom_selected = None;
            }
            if w::icon_button(ui, t, icons::PLUS, t!("Add a non-Steam game"), false, 0).clicked() {
                self.adding = Some(CustomGame::default());
            }
        });
        ui.add_space(8.0);
        let width = ui.available_width() - 6.0;
        w::search(ui, t, &mut self.search, t!("Search games"), width);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (f, label) in [(Filter::All, t!("All")), (Filter::Installed, t!("Installed")), (Filter::Favorites, icons::STAR), (Filter::Playing, t!("Played"))] {
                if w::chip(ui, t, label, self.filter == f).clicked() {
                    self.filter = f;
                }
            }
            // Steam-style filters in a popup.
            let n = self.adv.count();
            let label = if n > 0 { format!("{}  {} ({n})", icons::FUNNEL, t!("Filters")) } else { format!("{}  {}", icons::FUNNEL, t!("Filters")) };
            let r = w::chip(ui, t, &label, n > 0);
            egui::Popup::from_toggle_button_response(&r)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(720.0)
                .show(|ui| self.adv_panel(ui, t, ctx));
        });
        // Collections.
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let mut remove = None;
            for (i, c) in self.collections.iter().enumerate() {
                let r = w::chip(ui, t, &format!("{}  {}  {}", icons::STACK, c.name, c.games.len()), self.filter == Filter::Collection(i));
                if r.clicked() {
                    self.filter = if self.filter == Filter::Collection(i) { Filter::All } else { Filter::Collection(i) };
                }
                r.context_menu(|ui| {
                    if ui.button(format!("{}   {}", icons::NOTE, t!("Rename"))).clicked() {
                        self.naming = Some((Some(i), c.name.clone(), None));
                        ui.close();
                    }
                    if ui.button(format!("{}   {}", icons::TRASH, t!("Delete collection"))).clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
            if w::chip(ui, t, &format!("{}  {}", icons::PLUS, t!("Collection")), false).on_hover_text(t!("Make a new collection")).clicked() {
                self.naming = Some((None, String::new(), None));
            }
            if let Some(i) = remove {
                self.collections.remove(i);
                self.filter = Filter::All;
            }
        });
        ui.add_space(8.0);
        let items = self.filtered(ctx);
        let favs: Vec<&Entry> = items.iter().filter(|e| self.favorites.contains(&e.app_id)).collect();
        let (installed, rest): (Vec<_>, Vec<_>) = items.iter().filter(|e| !self.favorites.contains(&e.app_id)).partition(|e| e.installed.is_some());
        let mut groups: Vec<(String, Vec<&Entry>)> = Vec::new();
        if let Filter::Collection(i) = self.filter {
            let name = self.collections.get(i).map(|c| c.name.clone()).unwrap_or_default();
            groups.push((format!("{name} — {}", items.len()), items.iter().collect()));
        } else if !favs.is_empty() {
            groups.push((t!("Favorites — {n}", n = favs.len()), favs));
        }
        if !matches!(self.filter, Filter::Collection(_)) {
            groups.push((t!("Installed — {n}", n = installed.len()), installed));
        }
        if !rest.is_empty() && !matches!(self.filter, Filter::Collection(_)) {
            groups.push((t!("Not installed — {n}", n = rest.len()), rest));
        }
        if ctx.state.apps.is_none() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0).color(t.accent));
                ui.label(w::text(t!("Loading owned games…"), 12.0, t.text_faint));
            });
        }
        egui::ScrollArea::vertical().id_salt("lib-side").auto_shrink([false, false]).show(ui, |ui| {
            if items.is_empty() && ctx.state.apps.is_none() {
                for i in 0..10 {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width() - 6.0, 30.0), Sense::hover());
                    w::skeleton(ui, t, Rect::from_min_size(r.min + vec2(6.0, 3.0), vec2(24.0, 24.0)), CornerRadius::same(4));
                    w::skeleton(ui, t, Rect::from_min_size(r.min + vec2(40.0, 9.0), vec2(80.0 + (i * 37 % 90) as f32, 12.0)), CornerRadius::same(4));
                }
            }
            for (title, list) in groups {
                w::caption(ui, t, &title);
                ui.add_space(2.0);
                for e in list {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 6.0, 34.0), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let sel = self.selected == Some(e.app_id);
                    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                    if sel {
                        ui.painter().rect_filled(rect, t.corner_small(), t.accent.gamma_multiply(0.16));
                    } else if hover > 0.0 {
                        ui.painter().rect_filled(rect, t.corner_small(), w::mix(Color32::TRANSPARENT, t.surface, hover));
                    }
                    let icon = Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(24.0, 24.0));
                    let url = e.owned.and_then(|o| o.icon.as_deref()).map(|h| art::icon(e.app_id, h));
                    // The big header only when the game has no icon at all.
                    let header = art::header(e.app_id);
                    let urls: Vec<&str> = url.as_deref().into_iter().chain([header.as_str()]).collect();
                    if !w::paint_cover_first(ui, icon, &urls, CornerRadius::same(4)) {
                        ui.painter().rect_filled(icon, CornerRadius::same(4), t.surface_hover);
                    }
                    let setup = setup_state(ctx, e.app_id, e.installed.is_some());
                    let busy = matches!(setup, Setup::Downloading(_) | Setup::Queued);
                    let color = if busy { t.accent } else if sel { t.text } else if e.installed.is_some() { w::mix(t.text_dim, t.text, hover) } else { t.text_faint };
                    let galley = ui.painter().layout(e.name.to_string(), w::font_medium(13.5), color, rect.width() - 50.0);
                    let galley = if galley.rows.len() > 1 {
                        let mut job = egui::text::LayoutJob::simple_singleline(e.name.to_string(), w::font_medium(13.5), color);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 50.0);
                        ui.painter().layout_job(job)
                    } else {
                        galley
                    };
                    ui.painter().galley(pos2(icon.right() + 10.0, rect.center().y - galley.size().y / 2.0), galley, color);
                    if ctx.state.playing == Some(e.app_id) {
                        ui.painter().circle_filled(rect.right_center() - vec2(10.0, 0.0), 4.0, t.in_game);
                    }
                    // Installing: a thin bar along the bottom of the row, like Steam.
                    if let Setup::Downloading(d) = &setup {
                        let frac = d.fraction();
                        let track = Rect::from_min_max(pos2(icon.right() + 10.0, rect.bottom() - 4.0), pos2(rect.right() - 8.0, rect.bottom() - 2.0));
                        ui.painter().rect_filled(track, CornerRadius::same(1), t.raised);
                        let fill = Rect::from_min_size(track.min, vec2(track.width() * frac, track.height()));
                        ui.painter().rect_filled(fill, CornerRadius::same(1), if d.paused { t.text_faint } else { t.accent });
                    }
                    let id = e.app_id;
                    self.game_menu(t, &resp, id, e.installed.is_some(), ctx.state.playing == Some(id));
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.selected = Some(id);
                        self.custom_selected = None;
                    }
                }
                ui.add_space(10.0);
            }
            if !self.custom.is_empty() {
                w::caption(ui, t, &t!("Non-Steam — {n}", n = self.custom.len()));
                for (i, g) in self.custom.iter().enumerate() {
                    let sel = self.custom_selected == Some(i);
                    let r = ui.add(egui::Button::selectable(sel, w::text_medium(format!("{}   {}", icons::CONTROLLER, g.name), 13.5, t.text_dim)).min_size(vec2(ui.available_width() - 6.0, 32.0)));
                    if r.clicked() {
                        self.custom_selected = Some(i);
                        self.selected = None;
                    }
                }
                ui.add_space(10.0);
            }
            if !self.hidden.is_empty() {
                w::check(ui, t, &mut self.show_hidden, w::text(t!("Show hidden games"), 12.5, t.text_faint));
            }
        });
        drop(items);
    }

    fn add_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(g) = &mut self.adding else { return };
        let mut close = false;
        let mut add = false;
        egui::Modal::new(egui::Id::unique("add-game")).show(ui.ctx(), |ui| {
            ui.set_width(500.0);
            ui.label(w::text_bold(t!("Add a non-Steam game"), 20.0, t.text));
            ui.add_space(8.0);
            ui.label(w::text_medium(t!("Program"), 12.5, t.text_dim));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut g.exe).hint_text(t!("Path to the game's .exe")).desired_width(360.0).margin(vec2(10.0, 8.0)));
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
            ui.label(w::text_medium(t!("Display name"), 12.5, t.text_dim));
            ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0)));
            ui.add_space(6.0);
            ui.label(w::text_medium(t!("Launch options"), 12.5, t.text_dim));
            ui.add(egui::TextEdit::singleline(&mut g.args).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0)));
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
        ui.add_space(28.0);
        w::centered(ui, 900.0, 32.0, |ui| {
            w::pill(ui, t!("Non-Steam game"), t.text_dim, t.raised);
            ui.label(w::text_bold(&g.name, 32.0, t.text));
            ui.label(w::text(&g.exe, 13.0, t.text_faint).monospace());
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if w::button(ui, t, Some(icons::PLAY), t!("Play"), w::Kind::Play, w::Size::Lg).clicked() {
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

    fn home(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let mut recent: Vec<_> = ctx.local.games.iter().filter(|g| g.last_played.is_some()).collect();
        recent.sort_by_key(|g| std::cmp::Reverse(g.last_played));
        // Recent games across the whole account (installed or not), like Steam.
        let mut account_recent: Vec<(AppId, String, u64)> = match ctx.state.player_games.get(&ctx.state.steam_id) {
            Some(Ok(list)) if !list.is_empty() => list.iter().filter_map(|g| Some((g.app_id, g.name.clone(), g.last_played?))).collect(),
            _ => recent.iter().filter_map(|g| Some((g.app_id, g.name.clone(), g.last_played?))).collect(),
        };
        account_recent.sort_by_key(|g| std::cmp::Reverse(g.2));

        egui::ScrollArea::vertical().id_salt("lib-home").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(24.0);
            w::centered(ui, 1400.0, 28.0, |ui| {
                self.whats_new(ui, t, ctx, &account_recent);
                // Continue playing.
                if let Some(g) = recent.first() {
                    let width = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(vec2(width, 300.0), Sense::click());
                    let r = t.corner();
                    if !w::paint_cover_first(ui, rect, &[&art::hero(g.app_id), &art::header(g.app_id)], r) {
                        w::skeleton(ui, t, rect, r);
                    }
                    w::fade_right(ui, rect, Color32::from_black_alpha(200), 0.65);
                    w::fade_down(ui, rect, Color32::from_black_alpha(160), 0.4);
                    let inner = rect.shrink(32.0);
                    ui.painter().text(inner.left_top(), Align2::LEFT_TOP, t!("CONTINUE PLAYING"), w::font_bold(12.0), t.accent);
                    let logo = Rect::from_min_max(inner.left_top() + vec2(0.0, 28.0), inner.left_bottom() + vec2(380.0, -66.0));
                    if !w::paint_contain(ui, logo, &art::logo(g.app_id), Align2::LEFT_BOTTOM) {
                        ui.painter().text(logo.left_bottom(), Align2::LEFT_BOTTOM, &g.name, w::font_bold(34.0), Color32::WHITE);
                    }
                    let btn = Rect::from_min_size(inner.left_bottom() - vec2(0.0, 48.0), vec2(150.0, 48.0));
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(btn.min, vec2(600.0, 48.0))).layout(Layout::left_to_right(Align::Center)));
                    let playing = ctx.state.playing == Some(g.app_id);
                    let g: InstalledGame = (*g).clone();
                    self.play_button(&mut c, t, ctx, &g);
                    c.add_space(12.0);
                    if let Some(m) = g.playtime_minutes {
                        c.label(w::text_medium(t!("{time} played", time = ui::playtime(m)), 14.0, Color32::from_white_alpha(220)));
                    }
                    self.game_menu(t, &resp, g.app_id, true, playing);
                    if resp.clicked() {
                        self.selected = Some(g.app_id);
                    }
                    ui.add_space(30.0);
                }

                // Recent games shelf: everything played lately, installed or not.
                let first = recent.first().map(|g| g.app_id);
                let shelf: Vec<&(AppId, String, u64)> = account_recent.iter().filter(|g| Some(g.0) != first).collect();
                if !shelf.is_empty() {
                    w::section(ui, t, t!("Recent games"), None);
                    let size = vec2(170.0, 255.0);
                    let per = (((ui.available_width() + 16.0) / (size.x + 16.0)).floor() as usize).max(1);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        for (id, name, when) in shelf.into_iter().take(per) {
                            let installed = ctx.local.games.iter().any(|g| g.app_id == *id);
                            ui.vertical(|ui| {
                                ui.set_width(size.x);
                                let r = w::game_tile(ui, t, &art::capsule(*id), &art::header(*id), name, installed, size);
                                self.game_menu(t, &r, *id, installed, ctx.state.playing == Some(*id));
                                if r.clicked() {
                                    self.selected = Some(*id);
                                }
                                ui.label(w::text(ui::ago(*when), 12.0, t.text_faint));
                            });
                        }
                    });
                    ui.add_space(26.0);
                }

                // All games grid.
                let items = self.filtered(ctx);
                ui.horizontal(|ui| {
                    ui.label(w::text_bold(t!("All games ({n})", n = items.len()), 18.0, t.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(egui::Slider::new(&mut self.tile, 110.0..=220.0).show_value(false)).on_hover_text(t!("Tile size"));
                        ui.label(w::text(icons::GRID, 15.0, t.text_faint));
                        ui.add_space(10.0);
                        w::segmented(ui, t, &mut self.sort, &[(Sort::Name, t!("Name")), (Sort::Recent, t!("Recent")), (Sort::Playtime, t!("Hours")), (Sort::Size, t!("Size"))]);
                        ui.label(w::text(t!("Sort"), 13.0, t.text_faint));
                    });
                });
                ui.add_space(12.0);
                if items.is_empty() {
                    if ctx.state.apps.is_none() {
                        w::loading(ui, t, t!("Loading your library…"));
                    } else {
                        w::empty(ui, t, icons::SEARCH, t!("No games match"), t!("Try a different search or filter."));
                    }
                    return;
                }
                let size = vec2(self.tile, self.tile * 1.5);
                let gap = 16.0;
                let cols = (((ui.available_width() + gap) / (size.x + gap)).floor() as usize).max(1);
                for row in items.chunks(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for e in row {
                            ui.vertical(|ui| {
                                ui.set_width(size.x);
                                ui.spacing_mut().item_spacing.y = 6.0;
                                let r = w::game_tile(ui, t, &art::capsule(e.app_id), &art::header(e.app_id), e.name, e.installed.is_some(), size);
                                self.game_menu(t, &r, e.app_id, e.installed.is_some(), ctx.state.playing == Some(e.app_id));
                                if r.clicked() {
                                    self.selected = Some(e.app_id);
                                }
                                ui.add(egui::Label::new(w::text_medium(e.name, 13.0, if e.installed.is_some() { t.text } else { t.text_dim })).truncate());
                            });
                        }
                    });
                    ui.add_space(10.0);
                }
                ui.add_space(30.0);
            });
        });
    }

    fn game_page(&mut self, id: AppId, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let all = entries(ctx.state, ctx.local);
        let Some(e) = all.iter().find(|e| e.app_id == id) else {
            self.selected = None;
            return;
        };
        let (name, installed, owned) = (e.name.to_string(), e.installed.cloned(), e.owned.cloned());
        drop(all);
        let download = ctx.state.downloads.get(&id).cloned().filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed));
        let queued = ctx.state.download_queue.contains(&id);

        egui::ScrollArea::vertical().id_salt(("game", id)).auto_shrink([false, false]).show(ui, |ui| {
            // Hero with logo.
            let full = ui.available_width();
            let (hero, _) = ui.allocate_exact_size(vec2(full, 340.0), Sense::hover());
            if !w::paint_cover_first(ui, hero, &[&props::hero(id), &art::header(id)], CornerRadius::ZERO) {
                w::placeholder(ui, t, hero, "", CornerRadius::ZERO);
            }
            w::fade_down(ui, hero, t.bg, 0.5);
            let logo = Rect::from_min_max(hero.left_top() + vec2(40.0, 60.0), hero.left_bottom() + vec2(460.0, -30.0));
            if !w::paint_contain(ui, logo, &props::logo(id), Align2::LEFT_BOTTOM) {
                ui.painter().text(logo.left_bottom(), Align2::LEFT_BOTTOM, &name, w::font_bold(40.0), Color32::WHITE);
            }

            // Play bar.
            egui::Frame::new().fill(t.chrome).corner_radius(t.corner()).inner_margin(Margin::symmetric(20, 14)).outer_margin(Margin::symmetric(24, 0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let playing = ctx.state.playing == Some(id);
                    if let Some(d) = &download {
                        self.download_bar(ui, t, ctx, id, d);
                    } else if queued {
                        ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::CLOCK), t!("Queued"), w::Kind::Secondary, w::Size::Lg));
                    } else {
                        match &installed {
                            Some(_) if self.uninstalling.contains_key(&id) => {
                                let p = self.uninstalling[&id].progress.fraction();
                                uninstall_bar(ui, t, p);
                            }
                            Some(_) if playing => {
                                if w::button(ui, t, Some(icons::STOP), t!("Stop"), w::Kind::Danger, w::Size::Lg).on_hover_text(t!("Close the game")).clicked() {
                                    ctx.send(Command::StopGame { app_id: id });
                                }
                            }
                            Some(g) if self.needs_update(ctx, g) => {
                                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Update"), w::Kind::Primary, w::Size::Lg).clicked() {
                                    self.queue_install(ctx, id, Some(g.library.clone()));
                                }
                            }
                            Some(_) if ctx.state.cloud_syncing.contains(&id) => {
                                // Like Steam: the game starts once its saves are in sync.
                                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::CLOUD_CHECK), t!("Syncing Steam Cloud…"), w::Kind::Play, w::Size::Lg));
                                ui.add(egui::Spinner::new().color(t.accent));
                            }
                            Some(g) => {
                                let r = w::button(ui, t, Some(icons::PLAY), t!("Play"), w::Kind::Play, w::Size::Lg);
                                if r.clicked() {
                                    let args = self.games.get(&id).map(|s| s.args.clone()).unwrap_or_default();
                                    ctx.send(Command::Launch { app_id: id, install_dir: g.install_dir.clone(), args });
                                }
                            }
                            None if owned.is_some() => {
                                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Install"), w::Kind::Primary, w::Size::Lg).clicked() {
                                    self.install_dialog = Some(InstallDialog::new(id, ctx));
                                }
                            }
                            None => {
                                if w::button(ui, t, Some(icons::STORE), t!("Store page"), w::Kind::Primary, w::Size::Lg).clicked() {
                                    ctx.navigate("store", Some(format!("app:{id}")));
                                }
                            }
                        }
                        ui.add_space(22.0);
                        if let Some(g) = &installed {
                            w::stat(ui, t, t!("Last played"), &g.last_played.map(ui::ago).unwrap_or_else(|| t!("Never").to_string()));
                            ui.add_space(20.0);
                            crate::offline_ui::playtime_stat(ui, t, ctx.state, id, g.playtime_minutes);
                            ui.add_space(20.0);
                            w::stat(ui, t, t!("Size"), &ui::bytes(g.size_on_disk));
                            // Where the game's Steam features come from (see docs/steam-compatibility.md).
                            if let Some(o) = &owned {
                                let (mode, why) = match client::launch_mode_cached(o, &g.install_dir) {
                                    client::LaunchMode::SteamInRust => (t!("SteamInRust"), t!("Achievements, cloud saves and friends work through SteamInRust.").to_string()),
                                    client::LaunchMode::OfficialSteam { anti_cheat } => (t!("Runs through Steam (anti-cheat)"), t!("Uses {name}: starts through the official Steam client, which opens in the background.", name = anti_cheat)),
                                    client::LaunchMode::NoSteam { reason } => (t!("Starts without Steam"), i18n::tr_dyn(&reason)),
                                };
                                ui.add_space(20.0);
                                ui.scope(|ui| w::stat(ui, t, t!("Steam features"), mode)).response.on_hover_text(why);
                            }
                        } else if let Some(o) = &owned {
                            let size: u64 = o.depots.iter().filter(|d| d.dlc_app.is_none() && (d.os.is_empty() || d.os.contains("windows"))).filter_map(|d| d.manifest("public").map(|m| m.1)).sum();
                            if size > 0 {
                                w::stat(ui, t, t!("Download size"), &format!("≈ {}", ui::bytes(size)));
                            }
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let fav = self.favorites.contains(&id);
                        let heart = w::icon_button(ui, t, icons::HEART, if fav { t!("Remove from favorites") } else { t!("Add to favorites") }, fav, 0);
                        if heart.clicked() && !self.favorites.remove(&id) {
                            self.favorites.insert(id);
                        }
                        if w::icon_button(ui, t, icons::GEAR, t!("Properties…"), false, 0).clicked() {
                            self.props = Some(props::Props { app: id, tab: props::PropTab::General, ..Default::default() });
                        }
                        let more = w::icon_button(ui, t, icons::DOTS, t!("Manage"), false, 0);
                        egui::Popup::menu(&more).width(240.0).show(|ui| self.manage_menu(ui, t, ctx, id, installed.as_ref()));
                        ctx.game_actions(id, ui);
                        crate::offline_ui::cloud_pending(ui, t, ctx.state, id);
                    });
                });
            });
            ui.add_space(10.0);

            // Link row (like Steam's under the play bar).
            egui::Frame::new().outer_margin(Margin::symmetric(24, 0)).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    let link = |ui: &mut Ui, label: &str| w::button(ui, t, None, label, w::Kind::Secondary, w::Size::Sm).clicked();
                    if link(ui, t!("Store page")) {
                        ctx.navigate("store", Some(format!("app:{id}")));
                    }
                    if link(ui, t!("Workshop")) {
                        ctx.navigate("workshop", Some(id.to_string()));
                    }
                    for (label, page) in [
                        (t!("Community hub"), "hub"),
                        (t!("Discussions"), "discussions"),
                        (t!("Guides"), "guides"),
                        (t!("Achievements"), "achievements"),
                    ] {
                        if link(ui, label) {
                            ctx.navigate(page, Some(id.to_string()));
                        }
                    }
                    if link(ui, t!("Support")) {
                        ctx.navigate("support", Some(format!("game:{id}")));
                    }
                    // The Market has no native page yet.
                    if w::button(ui, t, Some(icons::EXTERNAL), t!("Market"), w::Kind::Ghost, w::Size::Sm).on_hover_text(t!("Opens in your browser")).clicked() {
                        let _ = open::that_detached(format!("https://steamcommunity.com/market/search?appid={id}"));
                    }
                });
            });
            ui.add_space(18.0);

            w::centered(ui, 1300.0, 24.0, |ui| {
                // Two columns when there's room, stacked in a narrow window
                // (the window can also be tiny for a moment, e.g. while restoring).
                let total = ui.available_width();
                let side = (total * 0.32).clamp(280.0, 400.0);
                if total >= 720.0 {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 24.0;
                        ui.vertical(|ui| {
                            ui.set_width(total - side - 24.0);
                            self.dlc_strip(ui, t, ctx, id);
                            self.updates_feed(ui, t, id);
                        });
                        ui.vertical(|ui| {
                            ui.set_width(side);
                            self.side_panels(ui, t, ctx, id, installed.as_ref(), owned.as_ref());
                        });
                    });
                } else {
                    self.side_panels(ui, t, ctx, id, installed.as_ref(), owned.as_ref());
                    ui.add_space(16.0);
                    self.dlc_strip(ui, t, ctx, id);
                    self.updates_feed(ui, t, id);
                }
                ui.add_space(40.0);
            });
        });
        self.dialogs(ui, t, ctx, id, &name, installed.as_ref());
    }

    fn download_bar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, d: &model::DownloadStatus) {
        let label = i18n::tr_dyn(d.activity());
        ui.vertical(|ui| {
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                ui.label(w::text_semibold(label, 14.0, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Like Steam: an update shows only its own size.
                    let (done, total) = d.shown();
                    ui.label(w::text(format!("{} / {}", ui::bytes(done), ui::bytes(total)), 12.5, t.text_dim));
                });
            });
            w::progress(ui, t, d.fraction(), 380.0);
            // Network and disk separately (they differ a lot: reused data
            // and buffered writes hit the disk without coming over the network).
            if !d.verifying() {
                ui.label(w::text(t!("Network {net}/s · Disk {disk}/s", net = ui::bytes(d.speed), disk = ui::bytes(d.disk_speed)), 12.0, t.text_faint));
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

    /// DLC owned for this game, as a horizontal strip.
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
        egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.accent_2.gamma_multiply(0.5))).corner_radius(t.corner()).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::text_bold(t!("YOUR DLC ({n})", n = dlc.len()), 12.0, t.text_dim));
            ui.add_space(6.0);
            egui::ScrollArea::horizontal().id_salt(("dlc", id)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (d, name) in &dlc {
                        let (r, resp) = ui.allocate_exact_size(vec2(220.0, 103.0), Sense::click());
                        // DLC without its own art shows the game's, with a DLC badge and the name, like Steam.
                        let url = art::header(*d);
                        if !w::paint_cover(ui, r, &url, t.corner_small()) {
                            if w::image_state(ui, &url) == w::ImageState::Missing {
                                w::image_or_skeleton(ui, t, r, &art::header(id), t.corner_small());
                                let shade = egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 30.0), r.max);
                                ui.painter().rect_filled(shade, egui::CornerRadius { sw: t.corner_small().sw, se: t.corner_small().se, nw: 0, ne: 0 }, Color32::from_black_alpha(190));
                                let galley = ui.painter().layout(name.clone(), w::font_semibold(12.5), Color32::WHITE, r.width() - 16.0);
                                ui.painter().galley(egui::pos2(r.left() + 8.0, shade.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
                                let badge = egui::Rect::from_min_size(r.min, vec2(34.0, 17.0));
                                ui.painter().rect_filled(badge, egui::CornerRadius { nw: t.corner_small().nw, se: 4, ne: 0, sw: 0 }, t.accent_2);
                                ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, "DLC", w::font_bold(10.5), Color32::WHITE);
                            } else {
                                w::skeleton(ui, t, r, t.corner_small());
                            }
                        }
                        let off = self.games.get(&id).is_some_and(|g| g.disabled_dlc.contains(d));
                        if off {
                            ui.painter().rect_filled(r, t.corner_small(), Color32::from_black_alpha(150));
                        }
                        if resp.on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            ctx.navigate("store", Some(format!("app:{d}")));
                        }
                    }
                });
            });
        });
        ui.add_space(16.0);
    }

    fn side_panels(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, installed: Option<&InstalledGame>, owned: Option<&model::OwnedApp>) {
        // Notes.
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::text_bold(t!("NOTES"), 12.0, t.text_faint));
            ui.add_space(4.0);
            let s = self.games.entry(id).or_default();
            ui.add(egui::TextEdit::multiline(&mut s.notes).hint_text(t!("Write a note for yourself…")).desired_rows(3).desired_width(f32::INFINITY));
        });
        ui.add_space(12.0);

        // Friends who are playing.
        let friends: Vec<_> = ctx.state.friend_list().into_iter().filter(|p| p.game_app_id == Some(id)).collect();
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::text_bold(t!("FRIENDS PLAYING NOW"), 12.0, t.text_faint));
            ui.add_space(4.0);
            if friends.is_empty() {
                ui.label(w::text(t!("None of your friends are playing this right now."), 13.0, t.text_faint));
            }
            ui.horizontal_wrapped(|ui| {
                for p in friends {
                    w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 36.0, Some(t.in_game)).on_hover_text(&p.name);
                }
            });
        });
        ui.add_space(12.0);

        // Workshop.
        if let Some(g) = installed {
            let items = local::manage::workshop_items(g);
            if !items.is_empty() {
                w::card(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(w::text_bold(t!("STEAM WORKSHOP"), 12.0, t.text_faint));
                    ui.add_space(4.0);
                    let total: u64 = items.iter().map(|i| i.size).sum();
                    ui.label(w::text(t!("{n} items · {size}", n = items.len(), size = ui::bytes(total)), 13.5, t.text));
                    ui.horizontal(|ui| {
                        if w::button(ui, t, None, t!("Manage"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            self.props = Some(props::Props { app: id, tab: props::PropTab::Workshop, ..Default::default() });
                        }
                        if w::button(ui, t, None, t!("Browse the Workshop"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            ctx.navigate("workshop", Some(id.to_string()));
                        }
                    });
                });
                ui.add_space(12.0);
            }
        }

        // Details.
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::text_bold(t!("DETAILS"), 12.0, t.text_faint));
            ui.add_space(4.0);
            egui::Grid::new("game-details").num_columns(2).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
                ui.label(w::text_bold(t!("APP ID"), 10.5, t.text_faint));
                ui.label(w::text(id.to_string(), 13.0, t.text));
                ui.end_row();
                if let Some(n) = self.current_players(id) {
                    ui.label(w::text_bold(t!("PLAYING NOW"), 10.5, t.text_faint));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 4.0, t.play);
                        ui.label(w::text(super::store::thousands(n as u64), 13.0, t.text));
                    })
                    .response
                    .on_hover_text(t!("People playing on Steam right now. Updates every minute."));
                    ui.end_row();
                }
                if let Some(g) = installed {
                    let (build, branch) = local::manage::installed_build(g);
                    ui.label(w::text_bold(t!("BUILD"), 10.5, t.text_faint));
                    ui.label(w::text(format!("{build}{}", if branch != "public" { format!(" ({branch})") } else { String::new() }), 13.0, t.text));
                    ui.end_row();
                    ui.label(w::text_bold(t!("LAST UPDATED"), 10.5, t.text_faint));
                    ui.label(w::text(g.last_updated.map(news::date).unwrap_or_else(|| "—".into()), 13.0, t.text));
                    ui.end_row();
                }
                if let Some(o) = owned {
                    if !o.branches.is_empty() {
                        ui.label(w::text_bold(t!("VERSIONS"), 10.5, t.text_faint));
                        ui.label(w::text(o.branches.len().to_string(), 13.0, t.text));
                        ui.end_row();
                    }
                }
            });
        });
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
        egui::Modal::new(egui::Id::unique(("install", id))).show(ui.ctx(), |ui| {
            ui.set_width(520.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(150.0, 70.0), Sense::hover());
                w::image_or_skeleton(ui, t, r, &art::header(id), t.corner_small());
                ui.vertical(|ui| {
                    ui.label(w::text(t!("Install"), 13.0, t.text_dim));
                    ui.label(w::text_bold(&name, 20.0, t.text));
                });
            });
            ui.add_space(12.0);
            ui.label(w::text_medium(t!("Install to"), 12.5, t.text_dim));
            for lib in &dlg.libraries {
                let free = fs4_free(lib);
                let label = format!("{}   ·   {}", lib.display(), t!("{size} free", size = free.map(ui::bytes).unwrap_or_else(|| "?".into())));
                w::radio_value(ui, t, &mut dlg.library, lib.clone(), w::text(label, 13.5, t.text));
            }
            if w::button(ui, t, Some(icons::FOLDER), t!("Choose another folder…"), w::Kind::Ghost, w::Size::Sm).clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    let _ = std::fs::create_dir_all(p.join("steamapps"));
                    dlg.libraries.push(p.clone());
                    dlg.library = p;
                }
            }
            ui.add_space(8.0);
            if dlg.size > 0 {
                ui.label(w::text(t!("Space required: {size}", size = ui::bytes(dlg.size)), 13.5, t.text));
                if fs4_free(&dlg.library).is_some_and(|f| f < dlg.size) {
                    ui.label(w::text(format!("{}  {}", icons::WARNING, t!("Not enough free space on this drive.")), 13.0, t.danger));
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
        if go {
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
            let args = self.games.get(&id).map(|s| s.args.clone()).unwrap_or_default();
            ctx.send(Command::Launch { app_id: id, install_dir: g.install_dir.clone(), args });
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
            ui.label(w::text(icons::TRASH, 28.0, t.danger));
            ui.label(w::text_bold(t!("Uninstall {name}?", name = name), 20.0, t.text));
            ui.add_space(4.0);
            ui.add(egui::Label::new(w::text(t!("This deletes {size} from:", size = ui::bytes(g.size_on_disk)), 14.0, t.text_dim)).wrap());
            ui.label(w::text(g.install_dir.display().to_string(), 13.0, t.text).monospace());
            ui.add_space(4.0);
            ui.add(egui::Label::new(w::text(t!("Saves kept inside the game folder are deleted too. Steam Cloud saves aren't affected."), 12.5, t.text_faint)).wrap());
            if running {
                ui.label(w::text(format!("{}  {}", icons::WARNING, t!("Close the game first.")), 13.0, t.away));
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
    let galley = ui.painter().layout_no_wrap(format!("{}  {label}", icons::TRASH), w::font_semibold(16.0), t.danger);
    let (rect, _) = ui.allocate_exact_size(vec2((galley.size().x + 48.0).max(220.0), 48.0), Sense::hover());
    let r = t.corner_small();
    let p = ui.painter();
    p.rect_filled(rect, r, t.danger.gamma_multiply(0.14));
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()));
    p.rect_filled(fill, r, t.danger.gamma_multiply(0.35));
    p.rect_stroke(rect, r, Stroke::new(1.0, t.danger.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    p.galley(pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0), galley, t.danger);
}

/// Checkboxes in `cols` columns; returns the index of the one clicked.
fn check_grid(ui: &mut Ui, t: &Theme, id: &str, cols: usize, items: impl Iterator<Item = (bool, String)>) -> Option<usize> {
    let mut hit = None;
    let col_w = (ui.available_width() - 16.0 * (cols as f32 - 1.0)) / cols as f32;
    egui::Grid::new(id).num_columns(cols).spacing(vec2(16.0, 2.0)).min_col_width(col_w).max_col_width(col_w).show(ui, |ui| {
        for (i, (on, label)) in items.enumerate() {
            let mut v = on;
            if w::check(ui, t, &mut v, w::text(label, 13.0, t.text)).changed() {
                hit = Some(i);
            }
            if i % cols == cols - 1 {
                ui.end_row();
            }
        }
    });
    hit
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
            let default = client::store::config_dir().join("Library");
            let _ = std::fs::create_dir_all(default.join("steamapps"));
            libraries.push(default);
        }
        let size = ctx
            .state
            .apps
            .as_ref()
            .and_then(|a| a.get(&app))
            .map(|o| o.depots.iter().filter(|d| d.dlc_app.is_none() && !d.shared_install && (d.os.is_empty() || d.os.contains("windows"))).filter_map(|d| d.manifest("public").map(|m| m.1)).sum())
            .unwrap_or(0);
        // Pick the library with the most free space.
        let library = libraries.iter().max_by_key(|l| fs4_free(l).unwrap_or(0)).cloned().unwrap_or_default();
        Self { app, libraries, library, size, desktop: true }
    }
}


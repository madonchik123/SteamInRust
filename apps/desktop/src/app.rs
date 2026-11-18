//! App root: owns the engine, the state snapshot, extensions, prefs and the overlay.

use client::{Client, Command, Event, SavedAccount, State, state::Phase};
use extensions::{Action, Env, Registry};
use local::LocalState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use ui::Theme;

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The theme in use (from `skin`; not saved itself).
    #[serde(skip)]
    pub theme: Theme,
    /// Which theme the user picked, with their accent and roundness.
    pub skin: ui::ThemeChoice,
    /// The whole theme, as app versions before theme packages saved it:
    /// read once to pick the matching theme package, never written.
    #[serde(rename = "theme", skip_serializing)]
    legacy_theme: ui::themes::LegacyTheme,
    pub zoom: f32,
    pub disabled: Vec<String>,
    pub ext_data: HashMap<String, String>,
    pub page: String,
    pub overlay_enabled: bool,
    /// Account to sign in automatically at startup (cleared on log out).
    pub auto_login: Option<String>,
    /// Interface language code; `None` follows the system.
    pub language: Option<String>,
    pub close_to_tray: bool,
    /// Show the account picker at startup instead of signing in automatically.
    pub ask_account: bool,
    #[serde(default = "yes")]
    pub auto_updates: bool,
    #[serde(default = "yes")]
    pub cloud: bool,
    #[serde(default = "yes")]
    pub toast_messages: bool,
    #[serde(default = "yes")]
    pub toast_notifications: bool,
    /// Offline mode was on at exit: start offline again, like Steam.
    pub wants_offline: bool,
    /// Start in offline mode by itself when Steam can't be reached.
    pub auto_offline: bool,
    /// Friends pinned to the top of the list ("Favorites"), like Steam.
    #[serde(default)]
    pub favorite_friends: Vec<u64>,
    /// Big Picture's interface size (1.0 = sized for a 1080p TV).
    pub bigpicture_scale: f32,
    /// Open in Big Picture at startup.
    pub bigpicture_start: bool,
    /// Which version of the logo the tray icon shows.
    pub tray_icon: ui::brand::Logo,
    /// The one-time "try the new look" question has been answered (or
    /// wasn't needed: a new install starts on the new look).
    pub new_look_offered: bool,
}

/// `--offscreen`: keep the window far off the screen (previews and
/// screenshots of a test copy, which then never shows on the desktop).
pub fn offscreen() -> bool {
    std::env::args().any(|a| a == "--offscreen")
}

/// Where an `--offscreen` window sits: left of any monitor.
pub const OFFSCREEN_POS: [f32; 2] = [-12000.0, 40.0];

/// `--window-size=1920x1080`: the main window's size (previews).
fn window_size_flag() -> Option<[f32; 2]> {
    let v = std::env::args().find_map(|a| a.strip_prefix("--window-size=").map(String::from))?;
    let (w, h) = v.split_once('x')?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

impl Prefs {
    /// Pick the theme: the saved choice; or, for settings from before theme
    /// packages, the package matching the saved theme; or the default.
    /// A new install starts on the default theme. Someone who used the app
    /// before themes keeps the look they had, and is asked once whether to
    /// try the new one ([`Prefs::wants_new_look_offer`]).
    pub fn settle_theme(&mut self) {
        let old = std::mem::take(&mut self.legacy_theme);
        if self.skin.id.is_empty() {
            if old.is_set() {
                self.skin = old.to_choice();
            } else {
                self.skin = ui::ThemeChoice { id: ui::themes::DEFAULT_ID.into(), ..Default::default() };
                self.new_look_offered = true;
            }
        }
        if self.skin.id == ui::themes::DEFAULT_ID {
            self.new_look_offered = true;
        }
        self.theme = ui::themes::resolve(&self.skin);
    }

    /// Whether to ask "try the new look?": not asked yet, and on an older theme.
    pub fn wants_new_look_offer(&self) -> bool {
        !self.new_look_offered && self.skin.id != ui::themes::DEFAULT_ID
    }
}

fn yes() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            // Empty: filled in at startup (from older settings, or the default).
            skin: ui::ThemeChoice::default(),
            legacy_theme: Default::default(),
            zoom: 1.0,
            disabled: Vec::new(),
            ext_data: HashMap::new(),
            page: "library".into(),
            overlay_enabled: true,
            auto_login: None,
            language: None,
            close_to_tray: true,
            ask_account: false,
            auto_updates: true,
            cloud: true,
            toast_messages: true,
            toast_notifications: true,
            wants_offline: false,
            auto_offline: false,
            favorite_friends: Vec::new(),
            bigpicture_scale: 1.0,
            bigpicture_start: false,
            tray_icon: ui::brand::Logo::default(),
            new_look_offered: false,
        }
    }
}

#[derive(Default)]
pub struct Activate {
    pub key: String,
    pub busy: bool,
    pub result: Option<Result<Vec<String>, String>>,
}

pub struct Toast {
    pub title: String,
    pub body: String,
    pub until: f64,
}

pub struct SteamApp {
    pub(crate) _rt: tokio::runtime::Runtime,
    pub client: Client,
    events: mpsc::Receiver<Event>,
    pub state: State,
    pub local: LocalState,
    pub registry: Registry,
    pub prefs: Prefs,
    pub saved: Vec<SavedAccount>,
    pub login: crate::login::LoginForm,
    pub toasts: Vec<Toast>,
    pub friend_filter: String,
    /// `--friends=<what>`: what to open in the friends window once signed in.
    pub pending_friends: Option<String>,
    /// An open friend dialog (nickname, server info, remove, block) and the
    /// window it belongs to.
    pub friend_dialog: Option<(egui::ViewportId, crate::shell::FriendDialog)>,
    pub search: String,
    /// Back/forward navigation stacks of (page, arg).
    pub back: Vec<(String, Option<String>)>,
    pub forward: Vec<(String, Option<String>)>,
    pub current_arg: Option<String>,
    pub overlay: crate::overlay::Overlay,
    pub demo: bool,
    pub search_open: bool,
    pub activate: Option<Activate>,
    /// Set by "Exit" so closing really quits instead of hiding to the tray.
    pub exiting: bool,
    /// Items in the Steam cart last time we heard (for "added" toasts).
    cart_count: Option<usize>,
    /// "A download is in progress, exit anyway?" is showing.
    confirm_exit: bool,
    window_mode: Option<WindowMode>,
    pub tray: crate::tray::Tray,
    flags: crate::builtin::settings::SharedFlags,
    pub friends_window: crate::friends_window::FriendsWindow,
    links: std::sync::mpsc::Receiver<String>,
    pub launch_popup: Option<crate::launch_popup::LaunchPopup>,
    /// Links waiting for sign-in to finish.
    pending_links: Vec<String>,
    /// Started by a game shortcut: once signed in, only the "Launching…"
    /// window shows, like Steam (the library stays in the Dock / tray).
    launch_only: bool,
    /// Main window is hidden in the tray.
    hidden: bool,
    applied_theme: Option<Theme>,
    /// The look as last written to disk, and a change waiting to settle.
    saved_look: Option<String>,
    look_pending: Option<(String, f64)>,
    /// `--new-look-prompt`: show the "try the new look" question (previews).
    pub force_new_look_prompt: bool,
    applied_zoom: f32,
    /// Sharp window icons for the display's scaling (Windows).
    window_icon: crate::window_icon::WindowIcon,
    /// Big Picture (fullscreen TV mode) and its controller input.
    pub bigpicture: crate::bigpicture::BigPicture,
    bigpicture_was_on: bool,
    /// A two-finger sideways swipe in progress (Mac trackpads): how far it
    /// went, when it last moved, and whether it already went back/forward.
    swipe: (egui::Vec2, f64, bool),
    /// When the first frame ran, and when we last asked to leave fullscreen.
    started_at: Option<f64>,
    unfullscreen_asked: f64,
    /// Download progress on the taskbar button / Dock icon.
    taskbar: crate::taskbar::Taskbar,
}

impl SteamApp {
    pub fn new(cc: &eframe::CreationContext<'_>, link: Option<String>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        ui::install_fonts(&cc.egui_ctx);
        let mut prefs: Prefs = cc.storage.and_then(|s| eframe::get_value(s, "prefs")).unwrap_or_default();
        // The user's own themes (Settings → Appearance lists them).
        ui::themes::set_user_dir(client::store::config_dir().join("themes"));
        // Settings from before theme packages: the matching package.
        prefs.settle_theme();
        // `--lang=en` / `--lang=ru` overrides the saved choice (screenshots, testing).
        let lang_flag = std::env::args().find_map(|a| a.strip_prefix("--lang=").map(String::from));
        // The language picked in setup counts until one is chosen in the app.
        let from_setup = || std::fs::read_to_string(client::store::config_dir().join("language")).ok().map(|s| s.trim().to_string());
        i18n::set_language(lang_flag.or_else(|| prefs.language.clone()).or_else(from_setup).as_deref().and_then(i18n::Lang::from_code).unwrap_or_else(i18n::Lang::detect));

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .enable_all()
            .build()
            .expect("tokio runtime");
        // Pictures come from the disk cache when possible (added after the
        // standard loaders, so it's asked first).
        crate::image_cache::ImageCache::install(&cc.egui_ctx, rt.handle().clone());
        let (tx, events) = mpsc::channel();
        let demo_tx = tx.clone();
        let repaint = cc.egui_ctx.clone();
        let client = Client::spawn(
            rt.handle(),
            Arc::new(move |e| {
                let _ = tx.send(e);
                repaint.request_repaint();
            }),
        );

        // `--no-local`: don't read this PC's Steam library (clean demo screenshots).
        let local = if std::env::args().any(|a| a == "--no-local") {
            LocalState::default()
        } else {
            local::load(None).unwrap_or_else(|e| {
                tracing::warn!("local Steam library: {e}");
                LocalState::default()
            })
        };

        let flags = std::sync::Arc::new(std::sync::Mutex::new(crate::builtin::settings::AppFlags {
            close_to_tray: prefs.close_to_tray,
            ask_account: prefs.ask_account,
            auto_updates: prefs.auto_updates,
            cloud: prefs.cloud,
            toast_messages: prefs.toast_messages,
            toast_notifications: prefs.toast_notifications,
            tray_icon: prefs.tray_icon,
        }));
        let mut registry = crate::builtin::registry(rt.handle().clone(), cc.egui_ctx.clone(), flags.clone());
        for id in &prefs.disabled {
            registry.set_enabled(id, false);
        }
        let demo = std::env::args().any(|a| a == "--demo");
        // The demo shows only its own made-up account: not your saved
        // library state (favorites, collections), and it never saves over it.
        if !demo {
            for (id, data) in &prefs.ext_data {
                registry.load(id, data);
            }
        }

        let saved = client::store::load();
        let mut auto_account = None;
        if demo {
            for e in crate::demo::events(&local) {
                let _ = demo_tx.send(e);
            }
        } else if let Some(acc) = prefs.auto_login.as_ref().filter(|_| !prefs.ask_account && !std::env::args().any(|a| a == "--no-auto-login")) {
            if saved.iter().any(|s| &s.account_name == acc) {
                auto_account = Some(acc.clone());
                // Offline mode sticks across restarts until "Go online".
                client.send(if prefs.wants_offline { Command::StartOffline { account: acc.clone() } } else { Command::LoginSaved { account: acc.clone() } });
            }
        }

        let overlay = crate::overlay::Overlay::new(&cc.egui_ctx);
        let mut bigpicture = crate::bigpicture::BigPicture::new(&cc.egui_ctx);
        if prefs.bigpicture_start {
            bigpicture.active = true;
        }
        let tray_icon = prefs.tray_icon;
        let mut app = Self {
            _rt: rt,
            client,
            events,
            state: State::default(),
            local,
            registry,
            prefs,
            saved,
            login: Default::default(),
            toasts: Vec::new(),
            friend_filter: String::new(),
            pending_friends: None,
            friend_dialog: None,
            search: String::new(),
            back: Vec::new(),
            forward: Vec::new(),
            current_arg: None,
            overlay,
            demo,
            search_open: false,
            activate: None,
            exiting: false,
            cart_count: None,
            confirm_exit: false,
            window_mode: None,
            tray: crate::tray::Tray::new(&cc.egui_ctx, tray_icon),
            flags,
            friends_window: Default::default(),
            links: crate::links::listen(cc.egui_ctx.clone()),
            launch_popup: None,
            launch_only: link.as_deref().is_some_and(|l| matches!(crate::links::parse(l), Some(crate::links::Link::Run(_)))),
            pending_links: link.into_iter().collect(),
            hidden: false,
            applied_theme: None,
            saved_look: None,
            look_pending: None,
            force_new_look_prompt: std::env::args().any(|a| a == "--new-look-prompt"),
            applied_zoom: 0.0,
            window_icon: Default::default(),
            bigpicture,
            bigpicture_was_on: false,
            swipe: (egui::Vec2::ZERO, 0.0, false),
            started_at: None,
            unfullscreen_asked: f64::MIN,
            taskbar: Default::default(),
        };
        // `--page=inventory --arg=listings`: open a page at startup (handy with --demo).
        let flag = |name: &str| std::env::args().find_map(|a| a.strip_prefix(&format!("--{name}=")).map(String::from));
        if let Some(page) = flag("page") {
            app.go(page, flag("arg"));
        }
        // `--theme=classic`: show a built-in theme (previews; not saved).
        if let Some(theme) = flag("theme").and_then(|n| ui::themes::builtins().into_iter().find(|p| p.id.eq_ignore_ascii_case(&n) || p.name.eq_ignore_ascii_case(&n))) {
            app.prefs.theme = theme;
        }
        // `--theme-dir <folder>`: try a theme package from a folder (for
        // theme authors; not saved). A theme that can't be read falls back
        // to the default, with the reason in the log.
        let args: Vec<String> = std::env::args().collect();
        let dir = args.iter().position(|a| a == "--theme-dir").and_then(|i| args.get(i + 1).cloned()).or_else(|| flag("theme-dir"));
        if let Some(dir) = dir {
            app.prefs.theme = ui::themes::load_dir_or_default(std::path::Path::new(&dir));
        }
        app.login.signing_in = auto_account;
        // `--overlay` opens the Shift+Tab overlay right away (for previews).
        // `--friends` opens the Friends & Chat window right away (for
        // previews); `--friends=<what>` opens it on a friends link's target
        // (a SteamID, `group:<id>`, `add`, `invites`, `activity`…), and
        // `--friends=chat` / `--friends=group` on the first conversation.
        if std::env::args().any(|a| a == "--friends") {
            app.friends_window.raise();
        }
        if let Some(what) = flag("friends") {
            app.pending_friends = Some(what);
        }
        // `--tray-menu` opens the tray menu near the screen corner (previews).
        if std::env::args().any(|a| a == "--tray-menu") {
            app.tray.open_menu(egui::pos2(900.0, 700.0));
        }
        if std::env::args().any(|a| a == "--overlay") {
            if demo {
                // A pretend running game, so the preview has an overview.
                app.overlay.game_started(1145360, false);
            }
            app.overlay.open = true;
        }
        app
    }

    pub fn toast(&mut self, now: f64, title: impl Into<String>, body: impl Into<String>) {
        self.toasts.push(Toast { title: title.into(), body: body.into(), until: now + 5.0 });
    }

    fn drain_events(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let now = ctx.input(|i| i.time);
        while let Ok(e) = self.events.try_recv() {
            // Toast-worthy things are decided before state changes.
            if let Event::Message(m) = &e {
                let open = self.friends_window.showing_chat(m.friend);
                if m.from != self.state.steam_id && !open && self.prefs.toast_messages {
                    let name = self.state.persona(m.from).name;
                    self.toast(now, name, m.text.clone());
                }
            }
            if let (Event::NotificationsArrived(list), true) = (&e, self.prefs.toast_notifications) {
                for n in list.iter().filter(|n| !n.read).take(3) {
                    let (_, text) = crate::shell::notification_text(&self.state, n);
                    self.toast(now, i18n::tr_dyn("Notification"), text);
                }
            }
            crate::offline_ui::on_event(self, &e, now);
            // What friends start doing, for the friends window's Activity tab.
            if let Event::Personas(list) = &e {
                self.friends_window.note_personas(&self.state, list);
            }
            self.state.apply(&e);
            match &e {
                Event::LoggedIn { account, .. } => {
                    if !self.demo {
                        self.prefs.auto_login = Some(account.clone());
                    }
                    self.login = Default::default();
                }
                Event::LoggedOut { reason } => {
                    if reason == "Logged out" {
                        self.prefs.auto_login = None;
                    }
                    self.saved = client::store::load();
                }
                // Store actions that failed (they used to be silent).
                Event::StoreError(e) => self.toast(now, i18n::tr_dyn(e), ""),
                // Additions show in the store's "Added to your cart!" popup.
                Event::Cart(lines) => {
                    self.cart_count = Some(lines.len());
                }
                Event::LoginFailed(_) => {
                    self.prefs.auto_login = None;
                    self.saved = client::store::load();
                    self.login.on_failed();
                }
                Event::AccountInfo { .. } => self.saved = client::store::load(),
                Event::GameStarted { app_id } => {
                    self.overlay.game_started(*app_id, self.prefs.overlay_enabled);
                    if let Some(p) = self.launch_popup.as_mut().filter(|p| p.app_id == *app_id) {
                        p.stage = crate::launch_popup::Stage::Running(now);
                    }
                }
                Event::GameStopped { .. } => {
                    self.overlay.game_stopped();
                    if let Ok(l) = local::load(Some(self.local.steam_root.clone())) {
                        self.local = l;
                    }
                }
                Event::Error(msg) => {
                    match self.launch_popup.as_mut().filter(|p| matches!(p.stage, crate::launch_popup::Stage::Launching)) {
                        Some(p) => p.stage = crate::launch_popup::Stage::Failed(msg.clone()),
                        None => self.toast(now, i18n::t!("Something went wrong"), i18n::tr_dyn(msg)),
                    }
                }
                Event::KeyActivated(r) => {
                    if let Some(a) = &mut self.activate {
                        a.busy = false;
                        a.result = Some(r.clone());
                    }
                    if r.is_ok() {
                        self.client.send(Command::SetPersonaState(self.state.me().map(|m| m.state).unwrap_or(model::PersonaState::Online)));
                    }
                }
                _ => {}
            }
            let env = Env {
                state: &self.state,
                local: &self.local,
                theme: &self.prefs.theme,
                zoom: self.prefs.zoom,
                client: &self.client,
                actions: &mut *actions,
            };
            self.registry.broadcast(&e, env);
        }
        // Timers (plugins' on_tick, their web replies).
        let env = Env {
            state: &self.state,
            local: &self.local,
            theme: &self.prefs.theme,
            zoom: self.prefs.zoom,
            client: &self.client,
            actions: &mut *actions,
        };
        self.registry.tick(env);
    }

    pub fn navigate(&mut self, mut page: String, arg: Option<String>) {
        // Friends and chat open in the Friends & Chat window, from any link
        // ("friends", or the old friends page's "community" with a target).
        if page == "friends" || (page == "community" && arg.is_some()) {
            self.friends_window.route(arg.as_deref());
            return;
        }
        // Plain "community" (the tray, plugins, older links) was the friends
        // page: the Community home now when there is one, else the window.
        if page == "community" && !self.registry.pages().iter().any(|p| p.id == "community") {
            if self.registry.pages().iter().any(|p| p.id == "community_home") {
                page = "community_home".into();
            } else {
                self.friends_window.route(None);
                return;
            }
        }
        if page != self.prefs.page || arg != self.current_arg {
            self.back.push((self.prefs.page.clone(), self.current_arg.clone()));
            self.forward.clear();
        }
        self.go(page, arg);
    }

    fn go(&mut self, page: String, arg: Option<String>) {
        self.registry.navigate(&page, arg.as_deref());
        self.prefs.page = page;
        self.current_arg = arg;
    }

    /// Two fingers swiped sideways on a trackpad: back (to the right) or
    /// forward (to the left), like Safari. Only scrolling that nothing on
    /// the page used counts, so carousels and wide lists still scroll.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn swipe_navigation(&mut self, ctx: &egui::Context) {
        const FAR: f32 = 160.0;
        let (now, left, raw) = ctx.input(|i| {
            // This frame's wheel/trackpad movement, before any list used it.
            let raw = i.events.iter().fold(egui::Vec2::ZERO, |acc, e| match e {
                egui::Event::MouseWheel { delta, .. } => acc + *delta,
                _ => acc,
            });
            (i.time, i.smooth_scroll_delta, raw)
        });
        let (total, last, done) = &mut self.swipe;
        if raw == egui::Vec2::ZERO && left == egui::Vec2::ZERO {
            // A pause ends the gesture.
            if now - *last > 0.35 {
                *total = egui::Vec2::ZERO;
                *done = false;
            }
            return;
        }
        *last = now;
        total.x += left.x;
        total.y += raw.y.abs();
        if *done || total.x.abs() < FAR || total.x.abs() < total.y * 2.0 {
            return;
        }
        *done = true;
        if total.x > 0.0 {
            self.go_back();
        } else {
            self.go_forward();
        }
    }

    pub fn go_back(&mut self) {
        if let Some((p, a)) = self.back.pop() {
            self.forward.push((self.prefs.page.clone(), self.current_arg.clone()));
            self.go(p, a);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some((p, a)) = self.forward.pop() {
            self.back.push((self.prefs.page.clone(), self.current_arg.clone()));
            self.go(p, a);
        }
    }

    pub fn apply_actions(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        let now = ctx.input(|i| i.time);
        for a in actions {
            match a {
                // "activate" is the Activate a Product dialog, not a page (the library's bar opens it).
                Action::Navigate { page, .. } if page == "activate" => self.activate = Some(Default::default()),
                Action::Navigate { page, arg } => self.navigate(page, arg),
                Action::Toast(text) => self.toast(now, text, ""),
                Action::SetTheme(t) => {
                    // A built-in theme with the user's changes: remember the
                    // choice (the small radius follows the large one).
                    if ui::themes::find(&t.id).is_some() {
                        self.prefs.skin = ui::themes::choice_for(&t);
                        self.prefs.theme = ui::themes::resolve(&self.prefs.skin);
                    } else {
                        self.prefs.theme = t;
                    }
                }
                Action::SetZoom(z) => self.prefs.zoom = z.clamp(0.7, 2.0),
                Action::SetEnabled { id, enabled } => {
                    self.registry.set_enabled(&id, enabled);
                    self.prefs.disabled.retain(|d| d != &id);
                    if !enabled {
                        self.prefs.disabled.push(id);
                    }
                }
                Action::OpenChat(id) => self.friends_window.show(crate::friends_window::Pane::Chat(id)),
                Action::ReloadLocal => {
                    if let Ok(l) = local::load(Some(self.local.steam_root.clone())) {
                        self.local = l;
                    }
                }
                Action::MarkRead(id) => {
                    self.state.unread.remove(&id);
                }
                Action::ToggleOverlay => self.overlay.toggle(),
                Action::JoinGame { app_id, connect } => {
                    let now = ctx.input(|i| i.time);
                    if let Err(msg) = client::safe_connect_args(&connect) {
                        self.toast(now, i18n::t!("Something went wrong"), i18n::tr_dyn(&msg));
                    } else if self.overlay.game.is_some_and(|(id, _)| id == app_id) {
                        // A second copy would just fight the first one.
                        self.toast(now, i18n::tr("The game is already running"), i18n::tr("Join from the game's own menu."));
                    } else if let Some(g) = self.local.games.iter().find(|g| g.app_id == app_id) {
                        self.launch_popup = Some(crate::launch_popup::LaunchPopup::new(app_id, g.name.clone(), now));
                        self.client.send(Command::Launch { app_id, install_dir: g.install_dir.clone(), args: client::LaunchArgs::connect(connect) });
                    } else {
                        self.navigate("library".into(), Some(format!("install:{app_id}")));
                    }
                }
                Action::SetLanguage(code) => {
                    if let Some(lang) = i18n::Lang::from_code(&code) {
                        i18n::set_language(lang);
                        self.prefs.language = Some(code);
                    }
                }
            }
        }
        let pages = self.registry.pages();
        if !pages.iter().any(|p| p.id == self.prefs.page) {
            self.prefs.page = "library".into();
        }
    }
}

impl SteamApp {
    /// May a window that doesn't exist yet open now? eframe can't create
    /// one while the main window is hidden (it then paints outside the
    /// event loop and gives up), so this brings the main window back first;
    /// the other window opens on the next frame.
    pub(crate) fn can_open_window(&mut self, ctx: &egui::Context, id: egui::ViewportId) -> bool {
        if !self.hidden || ctx.input(|i| i.raw.viewports.contains_key(&id)) {
            return true;
        }
        // Shown but not focused, so macOS stays on the game's space.
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.request_repaint();
        false
    }

    pub(crate) fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn handle_links(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use crate::links::Link;
        while let Ok(l) = self.links.try_recv() {
            if !crate::links::is_allowed_payload(&l) {
                continue;
            }
            // Starting a game only needs the small "Launching…" window.
            if !matches!(crate::links::parse(&l), Some(Link::Run(_))) {
                self.show_window(ctx);
            }
            self.pending_links.push(l);
        }
        if self.state.phase != Phase::LoggedIn {
            return;
        }
        if std::mem::take(&mut self.launch_only) && !self.hidden {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        }
        for raw in std::mem::take(&mut self.pending_links) {
            let nav = |page: &str, arg: Option<String>| Action::Navigate { page: page.into(), arg };
            match crate::links::parse(&raw) {
                Some(Link::Run(id)) => match self.local.games.iter().find(|g| g.app_id == id) {
                    Some(g) => {
                        let now = ctx.input(|i| i.time);
                        self.launch_popup = Some(crate::launch_popup::LaunchPopup::new(id, g.name.clone(), now));
                        self.client.send(Command::Launch { app_id: id, install_dir: g.install_dir.clone(), args: client::LaunchArgs::default() });
                    }
                    None => actions.push(nav("library", Some(format!("install:{id}")))),
                },
                Some(Link::Install(id)) => actions.push(nav("library", Some(format!("install:{id}")))),
                Some(Link::Uninstall(id)) => actions.push(nav("library", Some(format!("uninstall:{id}")))),
                Some(Link::Validate(id)) => actions.push(nav("library", Some(format!("verify:{id}")))),
                Some(Link::Store(id)) => actions.push(nav("store", Some(format!("app:{id}")))),
                Some(Link::Page(p)) => actions.push(nav(p, None)),
                Some(Link::Show) | None => {}
            }
        }
    }

    fn handle_tray(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use crate::tray::TrayCmd;
        for cmd in self.tray.poll(ctx) {
            match cmd {
                TrayCmd::Show => self.show_window(ctx),
                TrayCmd::Menu(..) => {}
                TrayCmd::Page(page) => {
                    self.show_window(ctx);
                    actions.push(Action::Navigate { page: page.into(), arg: None });
                }
                TrayCmd::Friends => self.friends_window.raise(),
                TrayCmd::Status(s) => self.client.send(Command::SetPersonaState(s)),
                TrayCmd::Exit => {
                    self.show_window(ctx);
                    self.request_exit(ctx);
                }
            }
        }
    }
}

impl SteamApp {
    /// eframe restores the last window state, fullscreen included, so after
    /// quitting in Big Picture the next start would be fullscreen. Start in a
    /// normal window (Big Picture asks for fullscreen itself once it shows).
    /// (Saving never stores fullscreen any more, see `save`; this covers
    /// state saved by older builds.) macOS may ignore the request while the
    /// restored window is still going fullscreen, so keep asking for a few
    /// seconds.
    fn leave_restored_fullscreen(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let start = *self.started_at.get_or_insert(now);
        let big_picture = self.bigpicture.active && self.state.phase == Phase::LoggedIn;
        // Not stopped early by what the window reports: on macOS it can read
        // "not fullscreen" while the window is still in a fullscreen space.
        if big_picture || now - start > 4.0 {
            return;
        }
        if now - self.unfullscreen_asked >= 0.25 {
            self.unfullscreen_asked = now;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    /// A download is running, paused or queued.
    pub fn downloading(&self) -> bool {
        !self.state.download_queue.is_empty() || self.state.downloads.values().any(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed))
    }

    /// Exit the app, asking first if a download is in progress (like Steam).
    pub fn request_exit(&mut self, ctx: &egui::Context) {
        if self.downloading() {
            self.confirm_exit = true;
        } else {
            self.exiting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn exit_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_exit {
            return;
        }
        let t = self.prefs.theme.clone();
        let name = self
            .state
            .downloads
            .values()
            .find(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed))
            .map(|d| d.app_id)
            .or_else(|| self.state.download_queue.first().copied())
            .and_then(|id| self.state.app_name(id).map(String::from))
            .unwrap_or_default();
        let (mut exit, mut tray, mut cancel) = (false, false, false);
        egui::Modal::new(egui::Id::unique("confirm-exit")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(ui::widgets::text_bold(i18n::tr("A download is in progress"), 18.0, t.text));
            ui.add_space(6.0);
            let body = i18n::trf("{game} is still downloading. If you exit, it continues the next time you open SteamInRust.", &[("game", name.clone())]);
            ui.add(egui::Label::new(ui::widgets::text(body, 13.5, t.text_dim)).wrap());
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                exit = ui::widgets::button(ui, &t, Some(ui::icons::POWER), i18n::tr("Exit anyway"), ui::widgets::Kind::Danger, ui::widgets::Size::Md).clicked();
                tray = ui::widgets::button(ui, &t, None, i18n::tr("Keep downloading in the tray"), ui::widgets::Kind::Secondary, ui::widgets::Size::Md).clicked();
                cancel = ui::widgets::button(ui, &t, None, i18n::tr("Cancel"), ui::widgets::Kind::Ghost, ui::widgets::Size::Md).clicked();
            });
        });
        if exit {
            self.confirm_exit = false;
            self.exiting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if tray {
            self.confirm_exit = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        } else if cancel {
            self.confirm_exit = false;
        }
    }
}

impl eframe::App for SteamApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent so the overlay window can see through; the main window
        // paints its own background.
        [0.0, 0.0, 0.0, 0.0]
    }

    /// Previews of an `--offscreen` copy can't be pointed at with the real
    /// mouse: `--pointer=x,y` keeps a pretend pointer there (hover states),
    /// and `--click=x,y` clicks there once, 4 seconds in (or
    /// `--click-after=<seconds>`).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if !offscreen() || raw_input.viewport_id != egui::ViewportId::ROOT {
            return;
        }
        let at = |name: &str| -> Option<egui::Pos2> {
            let v = std::env::args().find_map(|a| a.strip_prefix(&format!("--{name}=")).map(String::from))?;
            let (x, y) = v.split_once(',')?;
            Some(egui::pos2(x.parse().ok()?, y.parse().ok()?))
        };
        let time = raw_input.time.unwrap_or(0.0);
        raw_input.events.retain(|e| !matches!(e, egui::Event::PointerGone | egui::Event::PointerMoved(_)));
        if let Some(p) = at("pointer") {
            raw_input.events.push(egui::Event::PointerMoved(p));
        }
        if let Some(p) = at("click") {
            let done = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("preview-clicked"))).unwrap_or(false);
            let after = std::env::args().find_map(|a| a.strip_prefix("--click-after=").and_then(|v| v.parse::<f64>().ok())).unwrap_or(4.0);
            if time > after && !done {
                ctx.data_mut(|d| d.insert_temp(egui::Id::unique("preview-clicked"), true));
                let modifiers = egui::Modifiers::NONE;
                raw_input.events.push(egui::Event::PointerMoved(p));
                raw_input.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers });
                raw_input.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers });
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.save_look_now(&ui.ctx().clone(), frame);
        let ctx = ui.ctx().clone();
        // The running download (not a file check) on the taskbar / Dock.
        let shown = self.state.downloads.values().find(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed) && !d.verifying()).map(|d| (d.fraction(), d.paused));
        self.taskbar.show(frame, shown);
        self.window_icon.show(frame);
        crate::image_cache::trim_decoded(&ctx);
        if self.applied_theme.as_ref() != Some(&self.prefs.theme) {
            self.prefs.theme.apply(&ctx);
            self.applied_theme = Some(self.prefs.theme.clone());
        }
        // The user's zoom times the theme's own size (a theme drawn large,
        // like the redesign, shrinks itself to a comfortable size).
        let zoom = self.prefs.zoom * self.prefs.theme.layout.scale.clamp(0.5, 2.0);
        if self.applied_zoom != zoom {
            ctx.set_zoom_factor(zoom);
            self.applied_zoom = zoom;
        }
        // Update checks (not in demo/preview runs).
        if !self.demo {
            crate::updater::tick(self, &ctx);
        }

        let mut actions = Vec::new();
        {
            let f = *self.flags.lock().unwrap();
            self.prefs.close_to_tray = f.close_to_tray;
            self.prefs.ask_account = f.ask_account;
            self.prefs.auto_updates = f.auto_updates;
            self.prefs.toast_messages = f.toast_messages;
            self.prefs.toast_notifications = f.toast_notifications;
            if self.prefs.tray_icon != f.tray_icon {
                self.prefs.tray_icon = f.tray_icon;
                self.tray.set_icon(f.tray_icon);
            }
            if self.prefs.cloud != f.cloud {
                self.prefs.cloud = f.cloud;
                self.client.send(Command::CloudSettings { steam_root: Some(self.local.steam_root.clone()), disabled: if f.cloud { vec![] } else { self.state.apps.as_ref().map(|a| a.keys().copied().collect()).unwrap_or_default() } });
            }
        }
        self.drain_events(&ctx, &mut actions);
        self.leave_restored_fullscreen(&ctx);
        crate::bigpicture::poll(self, &ctx);
        self.handle_tray(&ctx, &mut actions);
        self.handle_links(&ctx, &mut actions);

        // Closing the window hides it in the tray (like Steam) unless "Exit" was chosen.
        let signed_in_now = self.state.phase == Phase::LoggedIn;
        if ctx.input(|i| i.viewport().close_requested()) && self.prefs.close_to_tray && signed_in_now && !self.exiting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        } else if ctx.input(|i| i.viewport().close_requested()) && !self.exiting && self.downloading() {
            // Closing for real while something downloads: ask first.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_exit = true;
        }
        self.exit_dialog(&ctx);
        if self.hidden {
            // Keep the friends window, overlay and notifications alive while hidden.
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            crate::friends_window::show(self, &ctx, &mut actions);
            crate::overlay::show(self, &ctx, &mut actions);
            crate::launch_popup::show(self, &ctx);
            crate::tray::show(self, &ctx);
            self.apply_actions(&ctx, actions);
            return;
        }

        // Mouse back/forward buttons.
        let (b, f) = ctx.input(|i| (i.pointer.button_pressed(egui::PointerButton::Extra1), i.pointer.button_pressed(egui::PointerButton::Extra2)));
        if b {
            self.go_back();
        }
        if f {
            self.go_forward();
        }

        let signed_in = self.state.phase == Phase::LoggedIn;
        let mode = if signed_in {
            WindowMode::Main
        } else if crate::login::wants_splash(self) {
            WindowMode::Splash
        } else {
            WindowMode::SignIn
        };
        // Big Picture owns the window (fullscreen); size it again when it ends.
        // In a window (`--bigpicture=window`) it gets the normal window's size
        // like any page, instead of keeping the loading screen's.
        let big_picture = signed_in && self.bigpicture.active && !self.bigpicture.is_windowed();
        if big_picture {
            self.window_mode = Some(mode);
        } else if self.bigpicture_was_on {
            self.window_mode = None;
        }
        self.bigpicture_was_on = big_picture;
        if self.window_mode != Some(mode) {
            self.window_mode = Some(mode);
            let main_size = window_size_flag().unwrap_or([1440.0, 900.0]);
            let (size, min, resizable) = match mode {
                WindowMode::Main => (main_size, [1060.0, 680.0], true),
                WindowMode::SignIn => ([720.0, 470.0], [720.0, 470.0], false),
                WindowMode::Splash => ([380.0, 250.0], [380.0, 250.0], false),
            };
            if mode != WindowMode::Main {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(resizable));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min.into()));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
            // Keep it centered on the screen, like Steam's windows (an
            // off-screen preview stays where it is).
            if offscreen() {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(OFFSCREEN_POS.into()));
            } else if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                let pos = ((monitor - egui::Vec2::from(size)) / 2.0).max(egui::Vec2::ZERO);
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.to_pos2()));
            }
            // Back from Big Picture: maximized again if it was before.
            if mode == WindowMode::Main && !big_picture && self.bigpicture.take_was_maximized() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
        }
        if signed_in && self.bigpicture.active {
            crate::bigpicture::show(self, ui, &mut actions);
            crate::friends_window::show(self, &ctx, &mut actions);
            crate::overlay::show(self, &ctx, &mut actions);
            crate::launch_popup::show(self, &ctx);
            crate::tray::show(self, &ctx);
        } else if signed_in {
            crate::shell::show(self, ui, &mut actions);
            crate::shell::activate_dialog(self, &ctx);
            crate::shell::friend_dialogs(self, &ctx);
            crate::shell::new_look_prompt(self, &ctx, &mut actions);
            crate::friends_window::show(self, &ctx, &mut actions);
            crate::overlay::show(self, &ctx, &mut actions);
            crate::launch_popup::show(self, &ctx);
            crate::tray::show(self, &ctx);
            #[cfg(target_os = "macos")]
            crate::mac_permissions::show(&ctx, &self.prefs.theme);
            #[cfg(target_os = "macos")]
            self.swipe_navigation(&ctx);
        } else {
            crate::login::show(self, ui);
            crate::tray::show(self, &ctx);
        }

        self.apply_actions(&ctx, actions);
        crate::shell::toasts(self, &ctx);
        if !(signed_in && self.bigpicture.active) {
            if signed_in {
                crate::chrome::resize_handles(&ctx);
            }
            crate::chrome::border(&ctx, &self.prefs.theme);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.saved_look = Some(self.look_key());
        if self.demo {
            return;
        }
        self.prefs.ext_data = self.registry.save_all().into_iter().collect();
        eframe::set_value(storage, "prefs", &self.prefs);
        // eframe has just saved the window's state (it runs before this).
        // Fullscreen only ever comes from Big Picture, which asks for it
        // again itself: never restore it, or a normal start after quitting
        // in Big Picture opens fullscreen (macOS can't be talked out of it
        // reliably once the window is restored that way).
        if let Some(window) = storage.get_string("window").filter(|w| w.contains("fullscreen:true")) {
            storage.set_string("window", window.replace("fullscreen:true", "fullscreen:false"));
        }
    }
}

impl SteamApp {
    /// What the user picked about the app's look: the theme, their accent
    /// colors and roundness, and the interface size.
    fn look_key(&self) -> String {
        format!("{:?}|{}|{}", self.prefs.skin, self.prefs.zoom, self.prefs.new_look_offered)
    }

    /// Save the settings right away once a change to the look settles
    /// (half a second without further change, so dragging a color picker
    /// doesn't write every frame). eframe's own save comes only every 30
    /// seconds and on a normal exit, and a theme switch was lost when the
    /// app was closed before that.
    fn save_look_now(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let key = self.look_key();
        let now = ctx.input(|i| i.time);
        let Some(saved) = &self.saved_look else {
            self.saved_look = Some(key);
            return;
        };
        if *saved == key {
            self.look_pending = None;
            return;
        }
        match &self.look_pending {
            Some((k, _)) if *k == key => {}
            _ => self.look_pending = Some((key.clone(), now)),
        }
        let since = self.look_pending.as_ref().map(|p| p.1).unwrap_or(now);
        if now - since < 0.5 {
            ctx.request_repaint_after(std::time::Duration::from_millis(550));
            return;
        }
        if let Some(storage) = frame.storage_mut() {
            eframe::App::save(self, storage);
            storage.flush();
        }
        self.saved_look = Some(key);
        self.look_pending = None;
    }
}

/// Window size and style for each stage, like Steam's separate windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WindowMode {
    /// Small "signing in" splash.
    Splash,
    SignIn,
    Main,
}

#[cfg(test)]
mod prefs_tests {
    use super::Prefs;

    impl Prefs {
        fn clone_for_test(&self) -> Prefs {
            ron::from_str(&ron::ser::to_string(self).unwrap()).unwrap()
        }
    }
    use egui::Color32;

    /// Settings saved before theme packages kept the whole theme; they load
    /// as the matching theme package with the user's accent, and nothing
    /// else is lost.
    #[test]
    fn settings_from_before_theme_packages_still_load() {
        #[derive(serde::Serialize)]
        struct OldTheme {
            name: String,
            dark: bool,
            bg: Color32,
            accent: Color32,
            accent_2: Color32,
            radius: u8,
        }
        #[derive(serde::Serialize)]
        struct OldPrefs {
            theme: OldTheme,
            zoom: f32,
            page: String,
        }
        let old = OldPrefs {
            theme: OldTheme { name: "Aurora".into(), dark: true, bg: Color32::from_rgb(8, 17, 18), accent: Color32::from_rgb(200, 10, 10), accent_2: Color32::from_rgb(56, 189, 248), radius: 12 },
            zoom: 1.25,
            page: "store".into(),
        };
        let text = ron::ser::to_string(&old).unwrap();
        let mut prefs: Prefs = ron::from_str(&text).unwrap();
        assert_eq!(prefs.zoom, 1.25);
        assert_eq!(prefs.page, "store");
        let old = prefs.legacy_theme.clone();
        assert!(old.is_set(), "the old theme is read");
        let choice = old.to_choice();
        assert_eq!(choice.id, "aurora");
        assert_eq!(choice.accent, Some(Color32::from_rgb(200, 10, 10)));
        // At startup: the look they had, and the new one is offered once.
        prefs.settle_theme();
        assert_eq!(prefs.theme.id, "aurora");
        assert_eq!(prefs.theme.accent, Color32::from_rgb(200, 10, 10));
        assert!(prefs.wants_new_look_offer());
        // New users get the default theme, and aren't asked.
        let mut fresh = Prefs::default();
        fresh.settle_theme();
        assert_eq!(fresh.theme.id, ui::themes::DEFAULT_ID);
        assert!(!fresh.wants_new_look_offer());
        // Settings saved after the question was answered don't ask again.
        let mut answered = prefs.clone_for_test();
        answered.new_look_offered = true;
        let mut answered: Prefs = ron::from_str(&ron::ser::to_string(&answered).unwrap()).unwrap();
        answered.settle_theme();
        assert!(!answered.wants_new_look_offer());
        assert_eq!(answered.theme.id, "aurora");
        // A saved choice comes back as it was.
        let back: Prefs = ron::from_str(&ron::ser::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(back.skin, prefs.skin);
        // And new settings don't write the old theme back.
        let again = ron::ser::to_string(&prefs).unwrap();
        assert!(!again.contains("legacy"), "{again}");
        assert!(again.contains("skin"));
    }
}

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
    pub theme: Theme,
    pub zoom: f32,
    pub disabled: Vec<String>,
    pub ext_data: HashMap<String, String>,
    pub page: String,
    pub friends_open: bool,
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
}

fn yes() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            zoom: 1.0,
            disabled: Vec::new(),
            ext_data: HashMap::new(),
            page: "library".into(),
            friends_open: true,
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
    _rt: tokio::runtime::Runtime,
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
    window_mode: Option<WindowMode>,
    pub tray: crate::tray::Tray,
    flags: crate::builtin::settings::SharedFlags,
    pub friends_window: crate::friends_window::FriendsWindow,
    links: std::sync::mpsc::Receiver<String>,
    pub launch_popup: Option<crate::launch_popup::LaunchPopup>,
    /// Links waiting for sign-in to finish.
    pending_links: Vec<String>,
    /// Main window is hidden in the tray.
    hidden: bool,
    applied_theme: Option<Theme>,
    applied_zoom: f32,
}

impl SteamApp {
    pub fn new(cc: &eframe::CreationContext<'_>, link: Option<String>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        ui::install_fonts(&cc.egui_ctx);
        let prefs: Prefs = cc.storage.and_then(|s| eframe::get_value(s, "prefs")).unwrap_or_default();
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
        }));
        let mut registry = crate::builtin::registry(rt.handle().clone(), cc.egui_ctx.clone(), flags.clone());
        for id in &prefs.disabled {
            registry.set_enabled(id, false);
        }
        for (id, data) in &prefs.ext_data {
            registry.load(id, data);
        }

        let saved = client::store::load();
        let demo = std::env::args().any(|a| a == "--demo");
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
            search: String::new(),
            back: Vec::new(),
            forward: Vec::new(),
            current_arg: None,
            overlay,
            demo,
            search_open: false,
            activate: None,
            exiting: false,
            window_mode: None,
            tray: crate::tray::Tray::new(&cc.egui_ctx),
            flags,
            friends_window: Default::default(),
            links: crate::links::listen(cc.egui_ctx.clone()),
            launch_popup: None,
            pending_links: link.into_iter().collect(),
            hidden: false,
            applied_theme: None,
            applied_zoom: 0.0,
        };
        // `--page=inventory --arg=listings`: open a page at startup (handy with --demo).
        let flag = |name: &str| std::env::args().find_map(|a| a.strip_prefix(&format!("--{name}=")).map(String::from));
        if let Some(page) = flag("page") {
            app.go(page, flag("arg"));
        }
        app.login.signing_in = auto_account;
        // `--overlay` opens the Shift+Tab overlay right away (for previews).
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
                let open = self.prefs.page == "community" && self.current_arg.as_deref() == Some(&m.friend.to_string());
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
    }

    pub fn navigate(&mut self, page: String, arg: Option<String>) {
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
                Action::Navigate { page, arg } => self.navigate(page, arg),
                Action::Toast(text) => self.toast(now, text, ""),
                Action::SetTheme(t) => self.prefs.theme = t,
                Action::SetZoom(z) => self.prefs.zoom = z.clamp(0.7, 2.0),
                Action::SetEnabled { id, enabled } => {
                    self.registry.set_enabled(&id, enabled);
                    self.prefs.disabled.retain(|d| d != &id);
                    if !enabled {
                        self.prefs.disabled.push(id);
                    }
                }
                Action::OpenChat(id) => self.navigate("community".into(), Some(id.to_string())),
                Action::ReloadLocal => {
                    if let Ok(l) = local::load(Some(self.local.steam_root.clone())) {
                        self.local = l;
                    }
                }
                Action::MarkRead(id) => {
                    self.state.unread.remove(&id);
                }
                Action::ToggleOverlay => self.overlay.toggle(),
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
    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn handle_links(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use crate::links::Link;
        while let Ok(l) = self.links.try_recv() {
            self.show_window(ctx);
            self.pending_links.push(l);
        }
        if self.state.phase != Phase::LoggedIn {
            return;
        }
        for raw in std::mem::take(&mut self.pending_links) {
            let nav = |page: &str, arg: Option<String>| Action::Navigate { page: page.into(), arg };
            match crate::links::parse(&raw) {
                Some(Link::Run(id)) => match self.local.games.iter().find(|g| g.app_id == id) {
                    Some(g) => {
                        let now = ctx.input(|i| i.time);
                        self.launch_popup = Some(crate::launch_popup::LaunchPopup::new(id, g.name.clone(), now));
                        self.client.send(Command::Launch { app_id: id, install_dir: g.install_dir.clone(), args: String::new() });
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
                TrayCmd::Friends => self.friends_window.open = true,
                TrayCmd::Status(s) => self.client.send(Command::SetPersonaState(s)),
                TrayCmd::Exit => {
                    self.exiting = true;
                    self.show_window(ctx);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

impl eframe::App for SteamApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent so the overlay window can see through; the main window
        // paints its own background.
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_theme.as_ref() != Some(&self.prefs.theme) {
            self.prefs.theme.apply(&ctx);
            self.applied_theme = Some(self.prefs.theme.clone());
        }
        if self.applied_zoom != self.prefs.zoom {
            ctx.set_zoom_factor(self.prefs.zoom);
            self.applied_zoom = self.prefs.zoom;
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
            if self.prefs.cloud != f.cloud {
                self.prefs.cloud = f.cloud;
                self.client.send(Command::CloudSettings { steam_root: Some(self.local.steam_root.clone()), disabled: if f.cloud { vec![] } else { self.state.apps.as_ref().map(|a| a.keys().copied().collect()).unwrap_or_default() } });
            }
        }
        self.drain_events(&ctx, &mut actions);
        self.handle_tray(&ctx, &mut actions);
        self.handle_links(&ctx, &mut actions);

        // Closing the window hides it in the tray (like Steam) unless "Exit" was chosen.
        let signed_in_now = self.state.phase == Phase::LoggedIn;
        if ctx.input(|i| i.viewport().close_requested()) && self.prefs.close_to_tray && signed_in_now && !self.exiting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        }
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
        if self.window_mode != Some(mode) {
            self.window_mode = Some(mode);
            let (size, min, resizable) = match mode {
                WindowMode::Main => ([1440.0, 900.0], [1060.0, 680.0], true),
                WindowMode::SignIn => ([720.0, 470.0], [720.0, 470.0], false),
                WindowMode::Splash => ([380.0, 250.0], [380.0, 250.0], false),
            };
            if mode != WindowMode::Main {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(resizable));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min.into()));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
            // Keep it centered on the screen, like Steam's windows.
            if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                let pos = ((monitor - egui::Vec2::from(size)) / 2.0).max(egui::Vec2::ZERO);
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.to_pos2()));
            }
        }
        if signed_in {
            crate::shell::show(self, ui, &mut actions);
            crate::shell::activate_dialog(self, &ctx);
            crate::friends_window::show(self, &ctx, &mut actions);
            crate::overlay::show(self, &ctx, &mut actions);
            crate::launch_popup::show(self, &ctx);
            crate::tray::show(self, &ctx);
        } else {
            crate::login::show(self, ui);
            crate::tray::show(self, &ctx);
        }

        self.apply_actions(&ctx, actions);
        crate::shell::toasts(self, &ctx);
        if signed_in {
            crate::chrome::resize_handles(&ctx);
        }
        crate::chrome::border(&ctx, &self.prefs.theme);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.prefs.ext_data = self.registry.save_all().into_iter().collect();
        eframe::set_value(storage, "prefs", &self.prefs);
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

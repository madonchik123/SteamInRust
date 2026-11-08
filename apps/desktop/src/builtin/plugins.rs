//! Hosts user plugins (sandboxed Rhai scripts) inside the app: pages,
//! events, the plugin manager in Settings, and the permission prompt.

use client::{Command, Event};
use egui::{Align, Layout, Sense, Ui, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, PersonaState, SteamId, art};
use plugins::{HostEvent, Permission, Plugin, Request, Snapshot, Widget};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use ui::{Theme, icons, widgets as w};

pub struct Plugins {
    dir: PathBuf,
    data_dir: PathBuf,
    loaded: bool,
    list: Vec<Plugin>,
    broken: Vec<(PathBuf, String)>,
    enabled: BTreeSet<String>,
    /// Plugin waiting for the user to approve its permissions.
    consent: Option<String>,
    show_logs: Option<String>,
    seen: HashMap<SteamId, (bool, Option<AppId>, Option<String>)>,
}

impl Plugins {
    pub fn new() -> Self {
        let base = client::store::config_dir();
        Self {
            dir: base.join("plugins"),
            data_dir: base.join("plugin-data"),
            loaded: false,
            list: Vec::new(),
            broken: Vec::new(),
            enabled: BTreeSet::new(),
            consent: None,
            show_logs: None,
            seen: HashMap::new(),
        }
    }

    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let _ = std::fs::create_dir_all(&self.dir);
        plugins::install_examples(&self.dir);
        self.list.clear();
        self.broken.clear();
        for r in plugins::discover(&self.dir) {
            match r {
                Ok(p) => self.list.push(p),
                Err((path, e)) => self.broken.push((path, e.to_string())),
            }
        }
        let ids: Vec<String> = self.enabled.iter().cloned().collect();
        for id in ids {
            self.start(&id);
        }
    }

    fn storage(&self, id: &str) -> BTreeMap<String, String> {
        std::fs::read_to_string(self.data_dir.join(format!("{id}.json")))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn start(&mut self, id: &str) {
        let storage = self.storage(id);
        if let Some(p) = self.list.iter_mut().find(|p| p.manifest.id == id) {
            p.start(storage);
        }
    }

    fn persist(&self, p: &Plugin) {
        if let Some(data) = p.take_storage() {
            let _ = std::fs::create_dir_all(&self.data_dir);
            if let Ok(json) = serde_json::to_string_pretty(&data) {
                let _ = std::fs::write(self.data_dir.join(format!("{}.json", p.manifest.id)), json);
            }
        }
    }

    fn handle(&self, p: &Plugin, ctx: &mut Ctx) {
        for r in p.drain() {
            match r {
                Request::Notify(text) => ctx.toast(format!("{}: {text}", p.manifest.name)),
                Request::SetStatus(s) => ctx.send(Command::SetPersonaState(s)),
                Request::SendMessage(to, text) => ctx.send(Command::SendMessage { to, text }),
            }
        }
        self.persist(p);
    }
}

fn snapshot(ctx: &Ctx) -> Snapshot {
    let me = ctx.state.me().map(|p| {
        (
            p.steam_id,
            ctx.state.persona_name.clone(),
            p.state.label().to_string(),
        )
    });
    let friends = ctx
        .state
        .friend_list()
        .into_iter()
        .map(|p| {
            let game = p.game_name.clone().or_else(|| {
                p.game_app_id
                    .and_then(|id| ctx.state.app_name(id).map(String::from))
            });
            (
                p.steam_id,
                p.name.clone(),
                p.state.label().to_string(),
                game,
            )
        })
        .collect();
    let installed: HashMap<AppId, u64> = ctx
        .local
        .games
        .iter()
        .map(|g| (g.app_id, g.playtime_minutes.unwrap_or(0)))
        .collect();
    let library = super::entries(ctx.state, ctx.local)
        .into_iter()
        .map(|e| {
            (
                e.app_id,
                e.name.to_string(),
                e.installed.is_some(),
                installed.get(&e.app_id).copied().unwrap_or(0),
            )
        })
        .collect();
    Snapshot {
        me,
        friends,
        library,
    }
}

impl Extension for Plugins {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "plugins",
            name: "Plugin host",
            author: "SteamInRust",
            description: "Runs sandboxed user plugins from your plugins folder",
            core: true,
            ..Manifest::DEFAULT
        }
    }

    fn pages(&self) -> Vec<PageSpec> {
        self.list
            .iter()
            .filter(|p| p.is_running())
            .filter_map(|p| {
                let page = p.manifest.page.as_ref()?;
                Some(
                    PageSpec::new(
                        format!("plugin:{}", p.manifest.id),
                        page.title.clone(),
                        icons::by_name(&page.icon),
                    )
                    .order(200),
                )
            })
            .collect()
    }

    fn on_event(&mut self, event: &Event, ctx: &mut Ctx) {
        self.ensure_loaded();
        let mut host_events = Vec::new();
        match event {
            Event::LoggedIn { .. } => host_events.push(HostEvent::LoggedIn),
            Event::GameStarted { app_id } => {
                host_events.push(HostEvent::GameStarted { app_id: *app_id })
            }
            Event::GameStopped { app_id } => {
                host_events.push(HostEvent::GameStopped { app_id: *app_id })
            }
            Event::Message(m) if m.from != ctx.state.steam_id => {
                host_events.push(HostEvent::Message {
                    steam_id: m.from,
                    name: ctx.state.persona(m.from).name,
                    text: m.text.clone(),
                });
            }
            Event::Personas(list) => {
                for p in list.iter().filter(|p| p.steam_id != ctx.state.steam_id) {
                    let online = p.state != PersonaState::Offline;
                    let now = (online, p.game_app_id, p.game_name.clone());
                    let Some(prev) = self.seen.insert(p.steam_id, now.clone()) else {
                        continue;
                    };
                    let (steam_id, name) = (p.steam_id, p.name.clone());
                    if !prev.0 && online {
                        host_events.push(HostEvent::FriendOnline { steam_id, name });
                    } else if prev.0 && !online {
                        host_events.push(HostEvent::FriendOffline { steam_id, name });
                    } else if (prev.1, &prev.2) != (now.1, &now.2) {
                        let game = now.2.clone().or_else(|| {
                            now.1
                                .and_then(|id| ctx.state.app_name(id).map(String::from))
                        });
                        host_events.push(HostEvent::FriendGame {
                            steam_id,
                            name,
                            game,
                        });
                    }
                }
            }
            _ => {}
        }
        if host_events.is_empty() || !self.list.iter().any(|p| p.is_running()) {
            return;
        }
        let snap = snapshot(ctx);
        for p in self.list.iter_mut().filter(|p| p.is_running()) {
            p.set_snapshot(snap.clone());
            for e in &host_events {
                p.event(e);
            }
        }
        for p in &self.list {
            self.handle(p, ctx);
        }
    }

    fn page(&mut self, page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        self.ensure_loaded();
        let t = ctx.theme.clone();
        let id = page.trim_start_matches("plugin:").to_string();
        let snap = snapshot(ctx);
        let Some(idx) = self.list.iter().position(|p| p.manifest.id == id) else {
            return;
        };
        self.list[idx].set_snapshot(snap);
        let widgets = self.list[idx].page();
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(26.0);
                w::centered(ui, 900.0, 32.0, |ui| {
                    let p = &self.list[idx];
                    // Who made this and what it can do, always visible.
                    ui.horizontal(|ui| {
                        w::pill(ui, t!("PLUGIN"), t.accent, t.accent.gamma_multiply(0.15));
                        ui.label(w::text(
                            t!(
                                "{name} {version} by {author}",
                                name = p.manifest.name,
                                version = p.manifest.version,
                                author = if p.manifest.author.is_empty() {
                                    t!("an unknown author")
                                } else {
                                    &p.manifest.author
                                }
                            ),
                            12.5,
                            t.text_faint,
                        ));
                        for perm in &p.manifest.permissions {
                            w::pill(ui, perm.key(), t.text_dim, t.raised);
                        }
                    });
                    if let Some(e) = &p.error {
                        ui.label(w::text(format!("{}  {e}", icons::WARNING), 13.0, t.danger));
                    }
                    ui.add_space(10.0);
                    render(ui, &t, ctx, &widgets, &mut clicked);
                });
            });
        if let Some(b) = clicked {
            self.list[idx].click(&b);
        }
        let p = &self.list[idx];
        self.handle(p, ctx);
    }

    fn settings(&mut self, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        self.ensure_loaded();
        let t = ctx.theme.clone();
        ui.horizontal(|ui| {
            if w::button(
                ui,
                &t,
                Some(icons::FOLDER),
                t!("Open plugins folder"),
                w::Kind::Secondary,
                w::Size::Sm,
            )
            .clicked()
            {
                local::open_folder(&self.dir);
            }
            if w::button(
                ui,
                &t,
                Some(icons::REFRESH),
                t!("Reload"),
                w::Kind::Secondary,
                w::Size::Sm,
            )
            .clicked()
            {
                self.loaded = false;
                self.ensure_loaded();
                ctx.toast(t!("Plugins reloaded"));
            }
        });
        ui.label(w::text(self.dir.display().to_string(), 12.0, t.text_faint));
        ui.add_space(10.0);

        let mut toggle_req: Option<(String, bool)> = None;
        for p in &self.list {
            let m = &p.manifest;
            let on = self.enabled.contains(&m.id);
            w::card(&t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(w::text_semibold(&m.name, 15.5, t.text));
                            ui.label(w::text(format!("v{}", m.version), 12.0, t.text_faint));
                            if !m.author.is_empty() {
                                ui.label(w::text(
                                    t!("by {author}", author = m.author),
                                    12.0,
                                    t.text_faint,
                                ));
                            }
                        });
                        ui.add(egui::Label::new(w::text(&m.description, 13.0, t.text_dim)).wrap());
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut v = on;
                        if ui.add(super::settings::toggle(&mut v)).changed() {
                            toggle_req = Some((m.id.clone(), v));
                        }
                    });
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if m.permissions.is_empty() {
                        ui.label(w::text(t!("No permissions"), 12.0, t.text_faint));
                    }
                    for perm in &m.permissions {
                        let (fg, bg) = if perm.sensitive() {
                            (t.away, t.away.gamma_multiply(0.15))
                        } else {
                            (t.text_dim, t.raised)
                        };
                        w::pill(ui, perm.key(), fg, bg);
                    }
                });
                if let Some(e) = &p.error {
                    ui.add_space(4.0);
                    ui.label(w::text(format!("{}  {e}", icons::WARNING), 12.5, t.danger));
                }
                let logs = p.logs();
                if !logs.is_empty() {
                    let open = self.show_logs.as_deref() == Some(&m.id);
                    if ui
                        .link(w::text(
                            if open { t!("Hide log") } else { t!("Show log") },
                            12.5,
                            t.text_dim,
                        ))
                        .clicked()
                    {
                        self.show_logs = if open { None } else { Some(m.id.clone()) };
                    }
                    if open {
                        w::panel(&t).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for l in logs.iter().rev().take(30) {
                                ui.label(w::text(l, 12.0, t.text_dim).monospace());
                            }
                        });
                    }
                }
            });
            ui.add_space(8.0);
        }
        for (path, e) in &self.broken {
            w::card(&t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(w::text_semibold(
                    path.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    15.0,
                    t.text,
                ));
                ui.label(w::text(
                    format!(
                        "{}  {}",
                        icons::WARNING,
                        t!("Can't load: {error}", error = e)
                    ),
                    12.5,
                    t.danger,
                ));
            });
            ui.add_space(8.0);
        }
        if self.list.is_empty() && self.broken.is_empty() {
            w::empty(
                ui,
                &t,
                icons::PLUGS,
                t!("No plugins installed"),
                t!("Drop a plugin folder into the plugins folder and press Reload."),
            );
        }

        match toggle_req {
            Some((id, true)) => self.consent = Some(id),
            Some((id, false)) => {
                self.enabled.remove(&id);
                if let Some(p) = self.list.iter_mut().find(|p| p.manifest.id == id) {
                    p.stop();
                }
            }
            None => {}
        }
        self.consent_dialog(ui, &t);
        true
    }

    fn save(&self) -> Option<String> {
        serde_json::to_string(&self.enabled).ok()
    }

    fn load(&mut self, data: &str) {
        self.enabled = serde_json::from_str(data).unwrap_or_default();
    }
}

impl Plugins {
    fn consent_dialog(&mut self, ui: &mut Ui, t: &Theme) {
        let Some(id) = self.consent.clone() else {
            return;
        };
        let Some(p) = self.list.iter().find(|p| p.manifest.id == id) else {
            self.consent = None;
            return;
        };
        let (name, perms) = (p.manifest.name.clone(), p.manifest.permissions.clone());
        let mut decision = None;
        egui::Modal::new(egui::Id::unique(("plugin-consent", &id))).show(ui.ctx(), |ui| {
            ui.set_width(440.0);
            ui.label(w::text(icons::SHIELD, 30.0, t.accent));
            ui.label(w::text_bold(
                t!("Enable {name}?", name = name),
                20.0,
                t.text,
            ));
            ui.add_space(4.0);
            ui.label(w::text(
                t!("This plugin will be able to:"),
                14.0,
                t.text_dim,
            ));
            ui.add_space(6.0);
            if perms.is_empty() {
                ui.label(w::text(
                    t!("Nothing beyond showing its own page."),
                    14.0,
                    t.text,
                ));
            }
            for perm in &perms {
                ui.horizontal(|ui| {
                    let c = if perm.sensitive() { t.away } else { t.accent };
                    ui.label(w::text(
                        if perm.sensitive() {
                            icons::WARNING
                        } else {
                            icons::CHECK
                        },
                        14.0,
                        c,
                    ));
                    ui.label(w::text(perm.describe(), 14.0, t.text));
                });
            }
            ui.add_space(6.0);
            ui.label(w::text(
                t!("It can't read your files, access the network, or run programs."),
                12.5,
                t.text_faint,
            ));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if w::button(ui, t, None, t!("Enable"), w::Kind::Primary, w::Size::Md).clicked() {
                    decision = Some(true);
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked() {
                    decision = Some(false);
                }
            });
        });
        match decision {
            Some(true) => {
                self.enabled.insert(id.clone());
                self.start(&id);
                self.consent = None;
            }
            Some(false) => self.consent = None,
            None => {}
        }
        let _ = Permission::ALL;
    }
}

fn render(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, widgets: &[Widget], clicked: &mut Option<String>) {
    for wd in widgets {
        match wd {
            Widget::Heading(s) => {
                ui.add_space(6.0);
                ui.label(w::text_bold(s, 22.0, t.text));
            }
            Widget::Text(s) => {
                ui.add(egui::Label::new(w::text(s, 14.0, t.text)).wrap());
            }
            Widget::Muted(s) => {
                ui.add(egui::Label::new(w::text(s, 13.5, t.text_dim)).wrap());
            }
            Widget::Button { id, text } => {
                if w::button(ui, t, None, text, w::Kind::Secondary, w::Size::Md).clicked() {
                    *clicked = Some(id.clone());
                }
            }
            Widget::Stat { label, value } => {
                w::card(t).show(ui, |ui| w::stat(ui, t, label, value));
            }
            Widget::Separator => {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }
            Widget::Progress(v) => {
                let width = ui.available_width().min(500.0);
                w::progress(ui, t, *v, width);
            }
            Widget::Row(items) => {
                ui.horizontal_wrapped(|ui| render(ui, t, ctx, items, clicked));
            }
            Widget::Game(id) => {
                let name = ctx
                    .state
                    .app_name(*id)
                    .map(String::from)
                    .or_else(|| {
                        ctx.local
                            .games
                            .iter()
                            .find(|g| g.app_id == *id)
                            .map(|g| g.name.clone())
                    })
                    .unwrap_or_else(|| t!("App {id}", id = id));
                let (rect, resp) = ui.allocate_exact_size(
                    vec2(ui.available_width().min(520.0), 90.0),
                    Sense::click(),
                );
                ui.painter().rect_filled(rect, t.corner(), t.surface);
                let img = egui::Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(150.0, 70.0));
                w::image_or_skeleton(ui, t, img, &art::header(*id), t.corner_small());
                ui.painter().text(
                    img.right_center() + vec2(16.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    name,
                    w::font_semibold(16.0),
                    t.text,
                );
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    ctx.navigate("library", Some(id.to_string()));
                }
            }
        }
    }
}

//! Hosts user plugins (sandboxed Rhai scripts) inside the app: pages,
//! events, timers, web requests, buttons on game pages, items in friends'
//! menus, the plugin manager in Settings, and the permission prompt.

use i18n::t;
use client::{Command, Event};
use egui::{Align, Layout, Sense, Ui, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use model::{AppId, Persona, PersonaState, SteamId, art};
use plugins::{HostEvent, Permission, Plugin, Request, Snapshot, Widget};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

/// A web request's answer, waiting to go back to its plugin:
/// (plugin id, request id, HTTP status or 0, body or error).
type HttpResult = (String, u64, u16, String);

/// Largest answer a plugin gets (the rest is cut off).
const MAX_BODY: usize = 1024 * 1024;

/// Hosts any plugin may show images from (Steam's own image servers).
const STEAM_IMAGES: [&str; 5] = ["steamstatic.com", "steampowered.com", "steamusercontent.com", "steamcommunity.com", "akamaihd.net"];

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
    rt: tokio::runtime::Handle,
    egui: egui::Context,
    http: reqwest::Client,
    replies: Arc<Mutex<Vec<HttpResult>>>,
    /// When each plugin's next `on_tick` is due (seconds since the epoch).
    next_tick: HashMap<String, u64>,
    /// The data plugins see, rebuilt at most once a second.
    snapshot: Option<(std::time::Instant, Snapshot)>,
    /// Text fields on plugin pages: (plugin, field) -> what's typed.
    inputs: HashMap<(String, String), String>,
    /// Trade offers seen so far and their state (`None` until the first list),
    /// to tell plugins about new and changed ones.
    trades_seen: Option<HashMap<u64, model::TradeState>>,
    /// When trade offers were last asked for on plugins' behalf.
    trades_polled: Option<std::time::Instant>,
    /// Prices plugins asked for: already asked, and games searched in bulk.
    prices: Mutex<(std::collections::HashSet<(AppId, String)>, std::collections::HashSet<AppId>)>,
}

/// Something done on a plugin's page.
enum PageEvent {
    Click(String),
    Input(String, String),
    Toggle(String, bool),
}

impl Plugins {
    pub fn new(rt: tokio::runtime::Handle, egui: egui::Context) -> Self {
        let base = client::store::config_dir();
        let http = reqwest::Client::builder()
            .user_agent(concat!("SteamInRust/", env!("CARGO_PKG_VERSION"), " (plugin)"))
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
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
            rt,
            egui,
            http,
            replies: Default::default(),
            next_tick: HashMap::new(),
            snapshot: None,
            inputs: HashMap::new(),
            trades_seen: None,
            trades_polled: None,
            prices: Default::default(),
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
        self.next_tick.remove(id);
    }

    fn persist(&self, p: &Plugin) {
        if let Some(data) = p.take_storage() {
            let _ = std::fs::create_dir_all(&self.data_dir);
            if let Ok(json) = serde_json::to_string_pretty(&data) {
                let _ = std::fs::write(self.data_dir.join(format!("{}.json", p.manifest.id)), json);
            }
        }
    }

    /// Carry out what plugin `idx` asked for.
    fn handle(&self, idx: usize, ctx: &mut Ctx) {
        let p = &self.list[idx];
        for r in p.drain() {
            match r {
                Request::Notify(text) => ctx.toast(format!("{}: {text}", p.manifest.name)),
                Request::SetStatus(s) => ctx.send(Command::SetPersonaState(s)),
                Request::SendMessage(to, text) => ctx.send(Command::SendMessage { to, text }),
                Request::Launch(app_id) => {
                    if let Some(g) = ctx.local.games.iter().find(|g| g.app_id == app_id) {
                        ctx.send(Command::Launch { app_id, install_dir: g.install_dir.clone(), args: String::new() });
                    }
                }
                Request::Navigate(page, arg) => ctx.navigate(&page, arg),
                Request::Copy(text) => self.egui.copy_text(text),
                Request::Http { id, method, url, body, headers } => self.fetch(p.manifest.id.clone(), id, method, url, body, headers),
                Request::LoadTrades => ctx.send(Command::LoadTradeOffers { language: i18n::language().steam_name().to_string() }),
                Request::RespondTrade(id, action) => {
                    let partner = match &ctx.state.trade_offers {
                        Some(Ok(o)) => o.received.iter().chain(o.sent.iter()).find(|x| x.id == id).map(|x| x.partner),
                        _ => None,
                    };
                    if let Some(partner) = partner {
                        ctx.send(Command::RespondTradeOffer { id, partner, action });
                    }
                }
                Request::ReviewTrade(id) => ctx.navigate("inventory", Some(format!("trade-review:{id}"))),
                Request::ProposeTrade { partner, give, get, message } => {
                    ctx.navigate("inventory", Some(super::inventory::trade_link(partner, &give, &get, &message)));
                }
                Request::LoadInventory(app_id, context_id) => {
                    let steam_id = ctx.state.steam_id;
                    if !ctx.state.inventories.contains_key(&(steam_id, app_id, context_id)) {
                        ctx.send(Command::LoadInventory { steam_id, app_id, context_id, language: i18n::language().steam_name().to_string() });
                    }
                }
                Request::Prices(app_id, names) => {
                    // Each price once; a game's first ones as a bulk search.
                    let mut asked = self.prices.lock().unwrap();
                    let names: Vec<String> = names.into_iter().filter(|n| asked.0.insert((app_id, n.clone()))).collect();
                    if !names.is_empty() && (app_id == 753 || asked.1.insert(app_id)) {
                        ctx.send(Command::MarketPrices { app_id, names, currency: super::inventory::wallet_currency(ctx) });
                    }
                }
            }
        }
        self.persist(p);
    }

    /// Run a plugin's web request in the background; the answer is handed
    /// back on a later frame (`tick`).
    fn fetch(&self, plugin: String, id: u64, method: String, url: String, body: String, headers: Vec<(String, String)>) {
        let (http, replies, egui) = (self.http.clone(), self.replies.clone(), self.egui.clone());
        self.rt.spawn(async move {
            let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
            let mut req = http.request(method, &url);
            for (k, v) in headers {
                req = req.header(k, v);
            }
            if !body.is_empty() {
                req = req.body(body);
            }
            let (status, text) = match req.send().await {
                Ok(mut resp) => {
                    let status = resp.status().as_u16();
                    let mut buf = Vec::new();
                    while let Ok(Some(chunk)) = resp.chunk().await {
                        buf.extend_from_slice(&chunk);
                        if buf.len() >= MAX_BODY {
                            buf.truncate(MAX_BODY);
                            break;
                        }
                    }
                    (status, String::from_utf8_lossy(&buf).into_owned())
                }
                Err(e) => (0, e.to_string()),
            };
            replies.lock().unwrap().push((plugin, id, status, text));
            egui.request_repaint();
        });
    }

    /// The data plugins see (rebuilt at most once a second: the library can be big).
    fn snapshot(&mut self, ctx: &Ctx) -> Snapshot {
        if let Some((at, s)) = &self.snapshot {
            if at.elapsed() < std::time::Duration::from_secs(1) {
                return s.clone();
            }
        }
        let wants = |perm| self.list.iter().any(|p| p.is_running() && p.manifest.permissions.contains(&perm));
        let s = snapshot(ctx, wants(plugins::Permission::TradesRead), wants(plugins::Permission::InventoryRead));
        self.snapshot = Some((std::time::Instant::now(), s.clone()));
        s
    }
}

fn snapshot(ctx: &Ctx, trades: bool, inventory: bool) -> Snapshot {
    let me = ctx.state.me().map(|p| (p.steam_id, ctx.state.persona_name.clone(), p.state.label().to_string()));
    let friends = ctx
        .state
        .friend_list()
        .into_iter()
        .map(|p| {
            let game = p.game_name.clone().or_else(|| p.game_app_id.and_then(|id| ctx.state.app_name(id).map(String::from)));
            (p.steam_id, p.name.clone(), p.state.label().to_string(), game)
        })
        .collect();
    let installed: HashMap<AppId, u64> = ctx.local.games.iter().map(|g| (g.app_id, g.playtime_minutes.unwrap_or(0))).collect();
    let library = super::entries(ctx.state, ctx.local)
        .into_iter()
        .map(|e| (e.app_id, e.name.to_string(), e.installed.is_some(), installed.get(&e.app_id).copied().unwrap_or(0)))
        .collect();
    let name = |id: AppId| ctx.state.app_name(id).map(String::from).unwrap_or_default();
    let mut downloads: Vec<(u32, String, f32, u64, bool, bool)> = ctx
        .state
        .downloads
        .values()
        .filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed))
        .map(|d| (d.app_id, name(d.app_id), d.fraction(), d.speed, d.paused, false))
        .collect();
    downloads.extend(ctx.state.download_queue.iter().map(|&id| (id, name(id), ctx.state.set_aside.get(&id).copied().unwrap_or(0.0), 0, false, true)));
    let wishlist = ctx.state.wishlist.as_ref().map(|w| w.iter().map(|(id, _)| *id).collect()).unwrap_or_default();
    // Trades, inventories and prices only when a running plugin may see them
    // (they can be large).
    let trades = match (&ctx.state.trade_offers, trades) {
        (Some(Ok(o)), true) => o.received.iter().chain(o.sent.iter()).map(|x| (x.clone(), ctx.state.persona(x.partner).name)).collect(),
        _ => Vec::new(),
    };
    let me_id = ctx.state.steam_id;
    let (inv, prices) = if inventory || trades.iter().len() > 0 {
        let inv = if inventory {
            ctx.state.inventories.iter().filter(|((o, _, _), _)| *o == me_id).map(|((_, a, c), i)| ((*a, *c), i.items.clone())).collect()
        } else {
            HashMap::new()
        };
        let prices = ctx
            .state
            .market_prices
            .iter()
            .filter_map(|(k, v)| v.as_ref().ok().and_then(|p| p.lowest_sell.or_else(|| p.lowest.as_deref().and_then(model::parse_money))).map(|c| (k.clone(), c)))
            .collect();
        (inv, prices)
    } else {
        (HashMap::new(), HashMap::new())
    };
    let currency = model::currency(super::inventory::wallet_currency(ctx)).0.to_string();
    Snapshot {
        me,
        friends,
        library,
        downloads,
        wishlist,
        language: i18n::language().code().to_string(),
        trades: Arc::new(trades),
        inventory: Arc::new(inv),
        prices: Arc::new(prices),
        currency,
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
                Some(PageSpec::new(format!("plugin:{}", p.manifest.id), page.title.clone(), icons::by_name(&page.icon)).order(200))
            })
            .collect()
    }

    fn on_event(&mut self, event: &Event, ctx: &mut Ctx) {
        self.ensure_loaded();
        let mut host_events = Vec::new();
        match event {
            Event::LoggedIn { .. } => host_events.push(HostEvent::LoggedIn),
            Event::GameStarted { app_id } => host_events.push(HostEvent::GameStarted { app_id: *app_id }),
            Event::GameStopped { app_id } => host_events.push(HostEvent::GameStopped { app_id: *app_id }),
            Event::DownloadFinished { app_id, result } => host_events.push(HostEvent::DownloadFinished { app_id: *app_id, ok: result.is_ok() }),
            Event::Message(m) if m.from != ctx.state.steam_id => {
                host_events.push(HostEvent::Message { steam_id: m.from, name: ctx.state.persona(m.from).name, text: m.text.clone() });
            }
            Event::TradeOffers(Ok(o)) => {
                let first = self.trades_seen.is_none();
                let seen = self.trades_seen.get_or_insert_with(HashMap::new);
                for x in o.received.iter().chain(o.sent.iter()) {
                    let before = seen.insert(x.id, x.state);
                    if first {
                        continue;
                    }
                    match before {
                        None if !x.ours && x.state == model::TradeState::Active => {
                            host_events.push(HostEvent::TradeOffer { id: x.id, partner: x.partner, name: ctx.state.persona(x.partner).name });
                        }
                        Some(s) if s != x.state => host_events.push(HostEvent::TradeChanged { id: x.id, state: x.state }),
                        _ => {}
                    }
                }
            }
            Event::Personas(list) => {
                for p in list.iter().filter(|p| p.steam_id != ctx.state.steam_id) {
                    let online = p.state != PersonaState::Offline;
                    let now = (online, p.game_app_id, p.game_name.clone());
                    let Some(prev) = self.seen.insert(p.steam_id, now.clone()) else { continue };
                    let (steam_id, name) = (p.steam_id, p.name.clone());
                    if !prev.0 && online {
                        host_events.push(HostEvent::FriendOnline { steam_id, name });
                    } else if prev.0 && !online {
                        host_events.push(HostEvent::FriendOffline { steam_id, name });
                    } else if (prev.1, &prev.2) != (now.1, &now.2) {
                        let game = now.2.clone().or_else(|| now.1.and_then(|id| ctx.state.app_name(id).map(String::from)));
                        host_events.push(HostEvent::FriendGame { steam_id, name, game });
                    }
                }
            }
            _ => {}
        }
        if host_events.is_empty() || !self.list.iter().any(|p| p.is_running()) {
            return;
        }
        let snap = self.snapshot(ctx);
        for p in self.list.iter_mut().filter(|p| p.is_running()) {
            p.set_snapshot(snap.clone());
            for e in &host_events {
                p.event(e);
            }
        }
        for i in 0..self.list.len() {
            self.handle(i, ctx);
        }
    }

    fn tick(&mut self, ctx: &mut Ctx) {
        if !self.loaded {
            return;
        }
        // Keep trade offers fresh for plugins that watch them (every 2 minutes).
        if self.list.iter().any(|p| p.is_running() && p.manifest.permissions.contains(&plugins::Permission::TradesRead))
            && self.trades_polled.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(120))
        {
            self.trades_polled = Some(std::time::Instant::now());
            ctx.send(Command::LoadTradeOffers { language: i18n::language().steam_name().to_string() });
        }
        // Answers to web requests.
        let replies = std::mem::take(&mut *self.replies.lock().unwrap());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let due: Vec<usize> = (0..self.list.len())
            .filter(|&i| {
                let p = &self.list[i];
                p.is_running() && p.manifest.tick.is_some() && self.next_tick.get(&p.manifest.id).is_none_or(|t| now >= *t)
            })
            .collect();
        if replies.is_empty() && due.is_empty() {
            return;
        }
        let snap = self.snapshot(ctx);
        for (plugin, id, status, body) in replies {
            if let Some(i) = self.list.iter().position(|p| p.manifest.id == plugin && p.is_running()) {
                self.list[i].set_snapshot(snap.clone());
                self.list[i].http_done(id, status, body);
                self.handle(i, ctx);
            }
        }
        for i in due {
            self.list[i].set_snapshot(snap.clone());
            if let Some(next) = self.list[i].tick(now) {
                self.next_tick.insert(self.list[i].manifest.id.clone(), next);
            }
            self.handle(i, ctx);
        }
        // Wake up for the next tick (the app doesn't redraw when idle).
        if let Some(next) = self.next_tick.values().min() {
            self.egui.request_repaint_after(std::time::Duration::from_secs(next.saturating_sub(now).max(1)));
        }
    }

    fn game_actions(&mut self, app_id: AppId, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        for i in 0..self.list.len() {
            if !self.list[i].defines("game_actions") {
                continue;
            }
            let snap = self.snapshot(ctx);
            self.list[i].set_snapshot(snap);
            for item in self.list[i].game_actions(app_id) {
                let r = w::button(ui, &t, Some(icons::PLUGS), &item.text, w::Kind::Secondary, w::Size::Md).on_hover_text(t!("From the {name} plugin", name = self.list[i].manifest.name));
                if r.clicked() {
                    self.list[i].game_action(&item.id, app_id);
                }
            }
            self.handle(i, ctx);
        }
    }

    fn friend_menu(&mut self, friend: &Persona, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        for i in 0..self.list.len() {
            if !self.list[i].defines("friend_menu") {
                continue;
            }
            let snap = self.snapshot(ctx);
            self.list[i].set_snapshot(snap);
            for item in self.list[i].friend_menu(friend.steam_id) {
                if w::menu_item(ui, &t, icons::PLUGS, &item.text).clicked() {
                    self.list[i].friend_menu_click(&item.id, friend.steam_id);
                    ui.close();
                }
            }
            self.handle(i, ctx);
        }
    }

    fn page(&mut self, page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        self.ensure_loaded();
        let t = ctx.theme.clone();
        let id = page.trim_start_matches("plugin:").to_string();
        let snap = self.snapshot(ctx);
        let Some(idx) = self.list.iter().position(|p| p.manifest.id == id) else { return };
        self.list[idx].set_snapshot(snap);
        let widgets = self.list[idx].page();
        let mut events = Vec::new();
        let hosts = self.list[idx].manifest.hosts.clone();
        let mut inputs = std::mem::take(&mut self.inputs);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(26.0);
            w::centered(ui, 900.0, 32.0, |ui| {
                let p = &self.list[idx];
                // Who made this and what it can do, always visible.
                ui.horizontal(|ui| {
                    w::pill(ui, t!("PLUGIN"), t.accent, t.accent.gamma_multiply(0.15));
                    ui.label(w::text(t!("{name} {version} by {author}", name = p.manifest.name, version = p.manifest.version, author = if p.manifest.author.is_empty() { t!("an unknown author") } else { &p.manifest.author }), 12.5, t.text_faint));
                    for perm in &p.manifest.permissions {
                        w::pill(ui, perm.key(), t.text_dim, t.raised);
                    }
                });
                if let Some(e) = &p.error {
                    ui.label(w::text(format!("{}  {e}", icons::WARNING), 13.0, t.danger));
                }
                ui.add_space(10.0);
                let mut r = Render { plugin: &id, hosts: &hosts, inputs: &mut inputs, events: &mut events };
                render(ui, &t, ctx, &widgets, &mut r);
            });
        });
        self.inputs = inputs;
        for e in events {
            match e {
                PageEvent::Click(b) => self.list[idx].click(&b),
                PageEvent::Input(f, text) => self.list[idx].input(&f, &text),
                PageEvent::Toggle(f, on) => self.list[idx].toggle(&f, on),
            }
        }
        self.handle(idx, ctx);
    }

    fn settings(&mut self, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        self.ensure_loaded();
        let t = ctx.theme.clone();
        ui.horizontal(|ui| {
            if w::button(ui, &t, Some(icons::FOLDER), t!("Open plugins folder"), w::Kind::Secondary, w::Size::Sm).clicked() {
                local::open_folder(&self.dir);
            }
            if w::button(ui, &t, Some(icons::REFRESH), t!("Reload"), w::Kind::Secondary, w::Size::Sm).clicked() {
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
                                ui.label(w::text(t!("by {author}", author = m.author), 12.0, t.text_faint));
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
                        let (fg, bg) = if perm.sensitive() { (t.away, t.away.gamma_multiply(0.15)) } else { (t.text_dim, t.raised) };
                        w::pill(ui, perm.key(), fg, bg);
                    }
                });
                if m.permissions.contains(&Permission::Network) && !m.hosts.is_empty() {
                    ui.label(w::text(t!("Connects to: {hosts}", hosts = m.hosts.join(", ")), 12.0, t.text_faint));
                }
                if let Some(e) = &p.error {
                    ui.add_space(4.0);
                    ui.label(w::text(format!("{}  {e}", icons::WARNING), 12.5, t.danger));
                }
                let logs = p.logs();
                if !logs.is_empty() {
                    let open = self.show_logs.as_deref() == Some(&m.id);
                    if ui.link(w::text(if open { t!("Hide log") } else { t!("Show log") }, 12.5, t.text_dim)).clicked() {
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
                ui.label(w::text_semibold(path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(), 15.0, t.text));
                ui.label(w::text(format!("{}  {}", icons::WARNING, t!("Can't load: {error}", error = e)), 12.5, t.danger));
            });
            ui.add_space(8.0);
        }
        if self.list.is_empty() && self.broken.is_empty() {
            w::empty(ui, &t, icons::PLUGS, t!("No plugins installed"), t!("Drop a plugin folder into the plugins folder and press Reload."));
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
        let Some(id) = self.consent.clone() else { return };
        let Some(p) = self.list.iter().find(|p| p.manifest.id == id) else {
            self.consent = None;
            return;
        };
        let (name, perms, hosts) = (p.manifest.name.clone(), p.manifest.permissions.clone(), p.manifest.hosts.clone());
        let network = perms.contains(&Permission::Network);
        let mut decision = None;
        egui::Modal::new(egui::Id::unique(("plugin-consent", &id))).show(ui.ctx(), |ui| {
            ui.set_width(440.0);
            ui.label(w::text(icons::SHIELD, 30.0, t.accent));
            ui.label(w::text_bold(t!("Enable {name}?", name = name), 20.0, t.text));
            ui.add_space(4.0);
            ui.label(w::text(t!("This plugin will be able to:"), 14.0, t.text_dim));
            ui.add_space(6.0);
            if perms.is_empty() {
                ui.label(w::text(t!("Nothing beyond showing its own page."), 14.0, t.text));
            }
            for perm in &perms {
                ui.horizontal(|ui| {
                    let c = if perm.sensitive() { t.away } else { t.accent };
                    ui.label(w::text(if perm.sensitive() { icons::WARNING } else { icons::CHECK }, 14.0, c));
                    ui.label(w::text(i18n::tr_dyn(perm.describe()), 14.0, t.text));
                });
                if *perm == Permission::Network {
                    let list = if hosts.is_empty() { t!("none listed, so nothing").to_string() } else { hosts.join(", ") };
                    ui.label(w::text(t!("Websites: {hosts}", hosts = list), 12.5, t.text_dim));
                }
            }
            ui.add_space(6.0);
            let limits = if network { t!("It can't read your files or run programs.") } else { t!("It can't read your files, access the network, or run programs.") };
            ui.label(w::text(limits, 12.5, t.text_faint));
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
    }
}

/// What `render` needs besides the widgets.
struct Render<'a> {
    plugin: &'a str,
    hosts: &'a [String],
    inputs: &'a mut HashMap<(String, String), String>,
    events: &'a mut Vec<PageEvent>,
}

/// Images come from Steam's image servers, or the plugin's own hosts.
fn image_allowed(url: &str, hosts: &[String]) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or_default().to_ascii_lowercase();
    if host.contains('@') {
        return false;
    }
    let under = |base: &str| host == base || host.ends_with(&format!(".{base}"));
    STEAM_IMAGES.iter().any(|b| under(b)) || hosts.iter().any(|h| match h.strip_prefix("*.") {
        Some(base) => under(base),
        None => host == *h,
    })
}

fn render(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, widgets: &[Widget], r: &mut Render) {
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
                    r.events.push(PageEvent::Click(id.clone()));
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
                ui.horizontal_wrapped(|ui| render(ui, t, ctx, items, r));
            }
            Widget::Game(id) => {
                let name = ctx.state.app_name(*id).map(String::from).or_else(|| ctx.local.games.iter().find(|g| g.app_id == *id).map(|g| g.name.clone())).unwrap_or_else(|| t!("App {id}", id = id));
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(520.0), 90.0), Sense::click());
                ui.painter().rect_filled(rect, t.corner(), t.surface);
                let img = egui::Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(150.0, 70.0));
                w::image_or_skeleton(ui, t, img, &art::header(*id), t.corner_small());
                ui.painter().text(img.right_center() + vec2(16.0, 0.0), egui::Align2::LEFT_CENTER, name, w::font_semibold(16.0), t.text);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    ctx.navigate("library", Some(id.to_string()));
                }
            }
            Widget::Input { id, value, hint } => {
                // What's typed stays until the plugin is told (Enter, or leaving the field).
                let key = (r.plugin.to_string(), id.clone());
                let text = r.inputs.entry(key.clone()).or_insert_with(|| value.clone());
                let resp = ui.add(egui::TextEdit::singleline(text).hint_text(hint.as_str()).desired_width(ui.available_width().min(420.0)).margin(vec2(10.0, 8.0))).with_edit_menu();
                if resp.lost_focus() && *text != *value {
                    r.events.push(PageEvent::Input(id.clone(), text.clone()));
                } else if !resp.has_focus() && *text != *value && !resp.changed() {
                    // The plugin changed the value itself.
                    *text = value.clone();
                }
            }
            Widget::Toggle { id, text, value } => {
                let mut v = *value;
                if w::check(ui, t, &mut v, w::text(text, 14.0, t.text)).changed() {
                    r.events.push(PageEvent::Toggle(id.clone(), v));
                }
            }
            Widget::Link { text, page, arg } => {
                if ui.link(w::text(text, 14.0, t.accent)).clicked() {
                    let allowed = ["store", "library", "community", "profile", "downloads", "settings"];
                    if allowed.contains(&page.as_str()) || page.starts_with("plugin:") {
                        ctx.navigate(page, arg.clone());
                    }
                }
            }
            Widget::Pill(s) => w::pill(ui, s, t.text_dim, t.raised),
            Widget::Table { columns, rows } => {
                w::card(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::Grid::new(ui.next_auto_id()).striped(true).spacing(vec2(18.0, 6.0)).show(ui, |ui| {
                        for c in columns {
                            ui.label(w::text_semibold(c, 13.0, t.text_dim));
                        }
                        if !columns.is_empty() {
                            ui.end_row();
                        }
                        for row in rows {
                            for cell in row {
                                ui.label(w::text(cell, 13.5, t.text));
                            }
                            ui.end_row();
                        }
                    });
                });
            }
            Widget::Image { url, width, height } => {
                let (rect, _) = ui.allocate_exact_size(vec2(width.min(ui.available_width()), *height), Sense::hover());
                if image_allowed(url, r.hosts) {
                    w::image_or_skeleton(ui, t, rect, url, t.corner_small());
                } else {
                    ui.painter().rect_filled(rect, t.corner_small(), t.raised);
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, t!("Image not allowed"), w::font(12.0), t.text_faint);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// The Wishlist Sales example against Steam's real store (needs the
    /// internet): `cargo test -p steam-rs wishlist_sales_live -- --ignored`.
    #[test]
    #[ignore]
    fn wishlist_sales_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let host = super::Plugins::new(rt.handle().clone(), egui::Context::default());
        let dir = std::env::temp_dir().join(format!("sir-ws-{}", std::process::id()));
        let (_, manifest, script) = plugins::EXAMPLES.iter().find(|e| e.0 == "wishlist-sales").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(dir.join("main.rhai"), script).unwrap();
        let mut p = plugins::Plugin::load(&dir).unwrap();
        // A wishlist of three well-known games.
        p.set_snapshot(plugins::Snapshot { wishlist: vec![440, 620, 1145360], ..Default::default() });
        p.start(Default::default());
        assert!(p.error.is_none(), "{:?}", p.error);
        let reqs = p.drain();
        let Some(plugins::Request::Http { id, method, url, body, headers }) = reqs.into_iter().find(|r| matches!(r, plugins::Request::Http { .. })) else { panic!("no request") };
        host.fetch("wishlist-sales".into(), id, method, url, body, headers);
        let reply = (0..100)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                host.replies.lock().unwrap().pop()
            })
            .expect("an answer");
        assert_eq!(reply.2, 200, "{}", reply.3);
        p.http_done(reply.1, reply.2, reply.3);
        assert!(p.error.is_none(), "{:?}", p.error);
        let saved = p.take_storage().unwrap();
        assert!(saved.contains_key("checked") && saved.contains_key("sales"), "{saved:?}");
        println!("sales: {}", saved["sales"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn images_only_from_allowed_hosts() {
        let hosts = vec!["api.example.com".to_string(), "*.cdn.example.com".to_string()];
        assert!(super::image_allowed("https://shared.akamai.steamstatic.com/x.jpg", &hosts));
        assert!(super::image_allowed("https://img.cdn.example.com/a.png", &hosts));
        assert!(super::image_allowed("https://api.example.com/a.png", &hosts));
        assert!(!super::image_allowed("http://cdn.cloudflare.steamstatic.com/x.jpg", &hosts));
        assert!(!super::image_allowed("https://steamstatic.com.evil.com/x.jpg", &hosts));
        assert!(!super::image_allowed("https://evil.com/x.jpg", &hosts));
        assert!(!super::image_allowed("https://user@evil.com/x.jpg", &hosts));
    }
}

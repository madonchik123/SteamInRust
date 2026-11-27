//! # User plugins
//!
//! The SteamInRust answer to SteamBrew/Millennium, without injecting into
//! anything: plugins are folders in the user's plugin directory:
//!
//! ```text
//! plugins/friend-alerts/
//!   plugin.toml   id, name, version, permissions, optional page
//!   main.rhai     the script
//! ```
//!
//! Scripts are [Rhai](https://rhai.rs), a scripting language written in pure
//! Rust. They are sandboxed:
//! - no file, network or process access at all, only the API below;
//! - every API call is checked against the permissions the user approved;
//! - CPU (operation count), recursion, string, array and map sizes are capped.
//!
//! The full list of functions, hooks, events, permissions and widgets is in
//! [`docs/plugin-api.md`](../../../docs/plugin-api.md). Keep it up to date
//! when adding to the API (and bump [`API_VERSION`]).

use rhai::{AST, Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Scope};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod achievements;
mod downloads;
mod items;
mod library;
mod net;
mod storage;
mod store;
mod timers;
mod widgets;
pub use widgets::{ChartKind, MAX_WIDGET_DEPTH, MAX_WIDGETS, Tone, Widget};
use widgets::widgets_bounded;
pub use net::blocked_ip;
pub use achievements::AchievementWatch;
pub use store::{STORE_LOOKUPS_PER_MINUTE, StoreApp, StoreHit, StorePrice, search_key, wishlist_changes};
pub use downloads::{DownloadInfo, DownloadWatch, phase as download_phase};
pub use library::GameInfo;

/// What `api_version()` answers. Bump it when functions are added, so
/// plugins can check before calling them.
pub const API_VERSION: i64 = 4;

/// Web requests a plugin can have waiting at once.
pub const MAX_IN_FLIGHT: usize = 8;
/// Longest a web request may take, in seconds.
pub const MAX_TIMEOUT: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Permission {
    AccountRead,
    FriendsRead,
    LibraryRead,
    ChatRead,
    ChatWrite,
    StatusWrite,
    Notifications,
    Storage,
    Network,
    GamesLaunch,
    DownloadsRead,
    StoreRead,
    Clipboard,
    InventoryRead,
    TradesRead,
    TradesWrite,
    AchievementsRead,
    DownloadsWrite,
}

impl Permission {
    pub const ALL: [Permission; 18] = [
        Self::AccountRead,
        Self::FriendsRead,
        Self::LibraryRead,
        Self::ChatRead,
        Self::ChatWrite,
        Self::StatusWrite,
        Self::Notifications,
        Self::Storage,
        Self::Network,
        Self::GamesLaunch,
        Self::DownloadsRead,
        Self::StoreRead,
        Self::Clipboard,
        Self::InventoryRead,
        Self::TradesRead,
        Self::TradesWrite,
        Self::AchievementsRead,
        Self::DownloadsWrite,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::AccountRead => "account.read",
            Self::FriendsRead => "friends.read",
            Self::LibraryRead => "library.read",
            Self::ChatRead => "chat.read",
            Self::ChatWrite => "chat.write",
            Self::StatusWrite => "status.write",
            Self::Notifications => "notifications",
            Self::Storage => "storage",
            Self::Network => "network",
            Self::GamesLaunch => "games.launch",
            Self::DownloadsRead => "downloads.read",
            Self::StoreRead => "store.read",
            Self::Clipboard => "clipboard",
            Self::InventoryRead => "inventory.read",
            Self::TradesRead => "trades.read",
            Self::TradesWrite => "trades.write",
            Self::AchievementsRead => "achievements.read",
            Self::DownloadsWrite => "downloads.write",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            Self::AccountRead => "See your profile name and SteamID",
            Self::FriendsRead => "See your friends and what they're playing",
            Self::LibraryRead => "See your games and playtime",
            Self::ChatRead => "Read incoming chat messages",
            Self::ChatWrite => "Send chat messages as you",
            Self::StatusWrite => "Change your online status",
            Self::Notifications => "Show notifications",
            Self::Storage => "Save its own settings",
            Self::Network => "Connect to the websites it lists",
            Self::GamesLaunch => "Start your installed games",
            Self::DownloadsRead => "See your downloads",
            Self::StoreRead => "See your wishlist, and Steam store pages and prices",
            Self::Clipboard => "Copy text to your clipboard",
            Self::InventoryRead => "See your inventory and items' market prices",
            Self::TradesRead => "See your trade offers",
            Self::TradesWrite => "Decline or cancel trade offers, and suggest trades for you to review",
            Self::AchievementsRead => "See your achievements",
            Self::DownloadsWrite => "Pause, resume, cancel and reorder your downloads",
        }
    }
    /// Permissions that act on your behalf or send data out get a warning in the UI.
    pub fn sensitive(self) -> bool {
        matches!(self, Self::ChatRead | Self::ChatWrite | Self::StatusWrite | Self::Network | Self::TradesWrite | Self::DownloadsWrite)
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == s)
    }
}

impl Serialize for Permission {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for Permission {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown permission '{s}'")))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawManifest {
    id: String,
    name: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default = "default_main")]
    main: String,
    page: Option<PageDecl>,
    /// Hosts `network` may reach ("api.example.com", "*.example.com").
    #[serde(default)]
    hosts: Vec<String>,
    /// Seconds between `on_tick()` calls.
    tick: Option<u64>,
}

fn default_version() -> String {
    "0.1.0".into()
}
fn default_main() -> String {
    "main.rhai".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageDecl {
    pub title: String,
    #[serde(default)]
    pub icon: String,
    /// Also show the page in Big Picture (its Plugins tab). Off unless the
    /// plugin says its page works with a controller.
    #[serde(default)]
    pub bigpicture: bool,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub permissions: Vec<Permission>,
    pub page: Option<PageDecl>,
    /// Hosts the plugin may connect to (with `network`).
    pub hosts: Vec<String>,
    /// Seconds between `on_tick()` calls (at least 10).
    pub tick: Option<u64>,
    main: String,
}

/// Data the host shares with plugins. Filled by the app each frame a plugin runs.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub me: Option<(u64, String, String)>,
    /// (steam_id, name, status, game)
    pub friends: Vec<(u64, String, String, Option<String>)>,
    /// Your games: owned and installed ones.
    pub library: Arc<Vec<GameInfo>>,
    /// Running downloads, then the queue in order.
    pub downloads: Vec<DownloadInfo>,
    /// Your wishlist in your order: (app_id, added in Unix seconds).
    pub wishlist: Vec<(u32, u64)>,
    /// Store pages loaded for plugins.
    pub store: Arc<HashMap<u32, Result<StoreApp, String>>>,
    /// Store searches loaded for plugins, by [`search_key`].
    pub searches: Arc<HashMap<String, Result<Vec<StoreHit>, String>>>,
    /// Names of games the app knows (for wishlist items).
    pub app_names: Arc<HashMap<u32, String>>,
    /// The app's language code ("en", "ru"...).
    pub language: String,
    /// Your trade offers, with the partner's name.
    pub trades: Arc<Vec<(model::TradeOffer, String)>>,
    /// Your loaded inventories by (app, context).
    pub inventory: Arc<std::collections::HashMap<(u32, u64), Vec<model::InventoryItem>>>,
    /// Market prices in cents of your wallet currency, by (app, market hash name).
    pub prices: Arc<std::collections::HashMap<(u32, String), u64>>,
    /// Your wallet's currency, as Steam formats it ("USD", "KZT"...).
    pub currency: String,
    /// Achievements (unlocked, total) by game, for the games loaded so far.
    pub achievement_progress: Arc<HashMap<u32, (u32, u32)>>,
    /// Achievement lists by game, for the games loaded so far.
    pub achievements: Arc<HashMap<u32, Result<Vec<model::Achievement>, String>>>,
}

/// Things a plugin asked the host to do.
#[derive(Debug, Clone)]
pub enum Request {
    Notify(String),
    SetStatus(model::PersonaState),
    SendMessage(u64, String),
    /// A web request; the answer goes to `Plugin::http_done`.
    Http { id: u64, method: String, url: String, body: String, headers: Vec<(String, String)>, timeout: u64 },
    /// Stop a web request (its answer is dropped).
    HttpCancel(u64),
    Launch(u32),
    Navigate(String, Option<String>),
    Copy(String),
    /// Fetch your trade offers again.
    LoadTrades,
    /// Decline or cancel an offer (accepting always goes through the app's review).
    RespondTrade(u64, model::TradeAction),
    /// Show the app's "Accept this trade?" review for an offer.
    ReviewTrade(u64),
    /// Open a new offer with these items for you to review and send.
    ProposeTrade { partner: u64, give: Vec<model::TradeAsset>, get: Vec<model::TradeAsset>, message: String },
    /// Load one of your inventories (app, context).
    LoadInventory(u32, u64),
    /// Look up market prices (app, market hash names).
    Prices(u32, Vec<String>),
    /// Load a game's achievement list.
    LoadAchievements(u32),
    /// Load achievement progress (unlocked / total) for these games.
    LoadAchievementProgress(Vec<u32>),
    PauseDownload(u32),
    ResumeDownload(u32),
    /// Cancel a download, or take it out of the queue.
    CancelDownload(u32),
    /// Move a queued download to this place (0 = next).
    MoveDownload(u32, usize),
    /// Start a queued download now (the running one waits).
    DownloadNow(u32),
    /// Load a game's store page.
    StoreDetails(u32),
    /// Search the store (a [`search_key`]).
    StoreSearch(String),
}

/// Events delivered to `on_event`.
#[derive(Debug, Clone)]
pub enum HostEvent {
    LoggedIn,
    FriendOnline { steam_id: u64, name: String },
    FriendOffline { steam_id: u64, name: String },
    FriendGame { steam_id: u64, name: String, game: Option<String> },
    Message { steam_id: u64, name: String, text: String },
    GameStarted { app_id: u32 },
    GameStopped { app_id: u32 },
    DownloadFinished { app_id: u32, ok: bool, error: Option<String> },
    /// A download started (left the queue).
    DownloadStarted { app_id: u32 },
    DownloadPaused { app_id: u32 },
    DownloadResumed { app_id: u32 },
    /// Someone sent you a trade offer.
    TradeOffer { id: u64, partner: u64, name: String },
    /// One of your offers changed (accepted, declined...).
    TradeChanged { id: u64, state: model::TradeState },
    /// You unlocked an achievement (seen when the game's list reloads: when
    /// the game closes, or when you open its achievements).
    AchievementUnlocked { app_id: u32, name: String, description: String, unlock_time: u64 },
    /// Games were added to or removed from your wishlist.
    WishlistChanged { added: Vec<u32>, removed: Vec<u32> },
    /// A store page asked for with `app_details` / `store_price` arrived.
    AppDetailsLoaded { app_id: u32 },
    /// A store search asked for with `store_search` arrived.
    StoreSearchLoaded { term: String },
    /// A game finished installing (or was found on disk).
    GameInstalled { app_id: u32, name: String },
    /// A game was uninstalled.
    GameUninstalled { app_id: u32, name: String },
}

/// Where a plugin's page is being shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Window,
    BigPicture,
}

impl View {
    pub fn key(self) -> &'static str {
        match self {
            View::Window => "window",
            View::BigPicture => "bigpicture",
        }
    }
}

/// An item a plugin adds to a game's page or a friend's menu.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub id: String,
    pub text: String,
}

const MAX_OUTBOX: usize = 32;

#[derive(Default)]
struct Host {
    granted: HashSet<Permission>,
    snapshot: Snapshot,
    outbox: Vec<Request>,
    hook_outbox: usize,
    storage: BTreeMap<String, String>,
    storage_dirty: bool,
    logs: VecDeque<String>,
    hosts: Vec<String>,
    next_request: u64,
    /// When recent requests were made (at most 30 a minute).
    request_times: VecDeque<std::time::Instant>,
    /// Store lookups asked for in the last minute.
    store_asked: HashMap<String, std::time::Instant>,
    /// The store limit was hit and logged (once until it's under again).
    store_limit_logged: bool,
    timers: Vec<timers::Timer>,
    next_timer: u64,
    /// Web requests waiting for an answer.
    in_flight: HashSet<u64>,
    /// About this plugin, for `plugin_info()`.
    info: Map,
    /// Where its page was last shown (`view_mode()`).
    view: View,
}

impl Host {
    fn need(&self, p: Permission) -> Result<(), Box<EvalAltResult>> {
        if self.granted.contains(&p) {
            Ok(())
        } else {
            Err(format!("permission denied: this plugin didn't request '{}'", p.key()).into())
        }
    }
    fn push(&mut self, r: Request) -> Result<(), Box<EvalAltResult>> {
        // Price lookups for one game join the request already queued (a page
        // asking for each item's price would otherwise hit the limit).
        if let Request::Prices(app, names) = &r {
            if let Some(Request::Prices(_, queued)) = self.outbox.iter_mut().find(|q| matches!(q, Request::Prices(a, _) if a == app)) {
                for n in names {
                    if queued.len() < 500 && !queued.contains(n) {
                        queued.push(n.clone());
                    }
                }
                return Ok(());
            }
        }
        if self.hook_outbox >= MAX_OUTBOX || self.outbox.len() >= MAX_OUTBOX {
            return Err(format!("rate limit exceeded: max {MAX_OUTBOX} actions per hook").into());
        }
        self.hook_outbox += 1;
        self.outbox.push(r);
        Ok(())
    }
    /// Ask the host to load data. Never fails: a request that's already
    /// queued isn't repeated, and past the outbox limit it's simply asked
    /// again on a later call.
    fn push_load(&mut self, r: Request) {
        let merge = |q: &mut Request| -> bool {
            match (q, &r) {
                (Request::Prices(a, queued), Request::Prices(b, names)) if a == b => {
                    for n in names {
                        if queued.len() < 500 && !queued.contains(n) {
                            queued.push(n.clone());
                        }
                    }
                    true
                }
                (Request::LoadAchievementProgress(queued), Request::LoadAchievementProgress(ids)) => {
                    for id in ids {
                        if queued.len() < 500 && !queued.contains(id) {
                            queued.push(*id);
                        }
                    }
                    true
                }
                (Request::LoadAchievements(a), Request::LoadAchievements(b)) => *a == *b,
                (Request::LoadInventory(a, c), Request::LoadInventory(b, d)) => *a == *b && *c == *d,
                (Request::StoreDetails(a), Request::StoreDetails(b)) => *a == *b,
                (Request::StoreSearch(a), Request::StoreSearch(b)) => *a == *b,
                _ => false,
            }
        };
        if self.outbox.iter_mut().any(merge) {
            return;
        }
        if self.outbox.len() < MAX_OUTBOX {
            self.outbox.push(r);
        }
    }
    fn log(&mut self, s: String) {
        self.logs.push_back(s);
        while self.logs.len() > 200 {
            self.logs.pop_front();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("plugin.toml: {0}")]
    Manifest(String),
    #[error("script error: {0}")]
    Script(String),
}

pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
    engine: Engine,
    ast: Option<AST>,
    scope: Scope<'static>,
    host: Arc<Mutex<Host>>,
    running: bool,
    /// Last error (compile or runtime), shown in the plugin manager.
    pub error: Option<String>,
    /// When `on_tick` last ran (seconds since the epoch).
    last_tick: u64,
}

const MAX_OPERATIONS: u64 = 2_000_000;

fn resolve_main_path(dir: &Path, main: &str) -> Result<(PathBuf, PathBuf), Error> {
    if main.is_empty() || main.contains('\0') || main.contains(':') || main.starts_with('/') || main.starts_with('\\') {
        return Err(Error::Manifest("invalid main script path".into()));
    }
    let mut parts = Vec::new();
    for seg in main.split(['/', '\\']) {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(Error::Manifest("invalid main script path".into()));
        }
        parts.push(seg);
    }
    for comp in Path::new(main).components() {
        if !matches!(comp, std::path::Component::Normal(_)) {
            return Err(Error::Manifest("invalid main script path".into()));
        }
    }
    let canon_dir = std::fs::canonicalize(dir).map_err(|e| Error::Io(dir.to_path_buf(), e))?;
    let mut cur = canon_dir.clone();
    for seg in &parts {
        cur.push(seg);
        let meta = std::fs::symlink_metadata(&cur).map_err(|e| Error::Io(cur.clone(), e))?;
        if meta.file_type().is_symlink() {
            return Err(Error::Manifest("main script path must not use symlinks".into()));
        }
    }
    let canon_file = std::fs::canonicalize(&cur).map_err(|e| Error::Io(cur, e))?;
    if !canon_file.starts_with(&canon_dir) || canon_file == canon_dir || !canon_file.is_file() {
        return Err(Error::Manifest("main script must be a file inside plugin folder".into()));
    }
    Ok((canon_dir, canon_file))
}

impl Plugin {
    pub fn load(dir: &Path) -> Result<Plugin, Error> {
        let manifest_path = dir.join("plugin.toml");
        let text = std::fs::read_to_string(&manifest_path).map_err(|e| Error::Io(manifest_path.clone(), e))?;
        let raw: RawManifest = toml::from_str(&text).map_err(|e| Error::Manifest(e.to_string()))?;
        if raw.id.is_empty() || !raw.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(Error::Manifest("id must be letters, digits, '-' or '_'".into()));
        }
        let (canon_dir, _) = resolve_main_path(dir, &raw.main)?;
        let mut permissions = Vec::new();
        for p in &raw.permissions {
            permissions.push(Permission::parse(p).ok_or_else(|| Error::Manifest(format!("unknown permission '{p}'")))?);
        }
        permissions.sort();
        permissions.dedup();
        let manifest = Manifest {
            id: raw.id,
            name: raw.name,
            version: raw.version,
            author: raw.author,
            description: raw.description,
            permissions,
            page: raw.page,
            hosts: raw.hosts.iter().map(|h| h.trim().to_ascii_lowercase()).filter(|h| !h.is_empty()).collect(),
            tick: raw.tick.map(|t| t.max(10)),
            main: raw.main,
        };
        let host = Arc::new(Mutex::new(Host::default()));
        let engine = sandboxed_engine(host.clone());
        Ok(Plugin { manifest, dir: canon_dir, engine, ast: None, scope: Scope::new(), host, running: false, error: None, last_tick: 0 })
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Compile and run the script only if every declared manifest permission is
    /// in `approved`. `storage` is the plugin's saved data.
    pub fn start(&mut self, approved: &BTreeSet<Permission>, storage: BTreeMap<String, String>) {
        self.stop();
        if let Some(missing) = self.manifest.permissions.iter().find(|p| !approved.contains(p)) {
            self.error = Some(format!("unapproved permission '{}'", missing.key()));
            return;
        }
        let (_, path) = match resolve_main_path(&self.dir, &self.manifest.main) {
            Ok(paths) => paths,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(format!("{}: {e}", path.display()));
                return;
            }
        };
        {
            let mut h = self.host.lock().unwrap();
            h.granted = self.manifest.permissions.iter().copied().filter(|p| approved.contains(p)).collect();
            h.hosts = self.manifest.hosts.clone();
            let m = &self.manifest;
            h.view = View::Window;
            h.info = map([
                ("id", m.id.clone().into()),
                ("name", m.name.clone().into()),
                ("version", m.version.clone().into()),
                ("author", m.author.clone().into()),
                ("permissions", Dynamic::from_array(m.permissions.iter().map(|p| Dynamic::from(p.key())).collect())),
                ("hosts", Dynamic::from_array(m.hosts.iter().map(|x| Dynamic::from(x.clone())).collect())),
                ("bigpicture", m.page.as_ref().is_some_and(|p| p.bigpicture).into()),
            ]);
            h.storage = storage;
            h.storage_dirty = false;
            h.outbox.clear();
            h.hook_outbox = 0;
        }
        match self.engine.compile(&src) {
            Ok(ast) => {
                self.scope = Scope::new();
                self.host.lock().unwrap().hook_outbox = 0;
                if let Err(e) = self.engine.run_ast_with_scope(&mut self.scope, &ast) {
                    self.error = Some(e.to_string());
                    self.host.lock().unwrap().granted.clear();
                    return;
                }
                self.ast = Some(ast);
                self.running = true;
                self.error = None;
                self.call("on_load", ());
            }
            Err(e) => {
                self.host.lock().unwrap().granted.clear();
                self.error = Some(format!("{e}"));
            }
        }
    }

    /// Stop the script: `on_unload()` runs first (it can still save), then
    /// its timers and waiting requests are dropped and permissions revoked.
    pub fn stop(&mut self) {
        if self.running {
            self.call("on_unload", ());
        }
        self.running = false;
        self.ast = None;
        let mut h = self.host.lock().unwrap();
        h.granted.clear();
        h.timers.clear();
        h.in_flight.clear();
    }

    /// When the next timer is due (Unix seconds).
    pub fn next_timer(&self) -> Option<u64> {
        if self.running { self.host.lock().unwrap().next_timer_due() } else { None }
    }

    /// Run the timers due at `now` (Unix seconds). Returns when the next is due.
    pub fn timers(&mut self, now: u64) -> Option<u64> {
        if !self.running {
            return None;
        }
        let due = self.host.lock().unwrap().due_timers(now);
        for id in due {
            self.call("on_timer", (id.to_string(),));
        }
        self.host.lock().unwrap().next_timer_due()
    }

    /// How many parameters a hook takes, if the script defines it.
    fn params(&self, name: &str) -> Option<usize> {
        self.ast.as_ref()?.iter_functions().find(|f| f.name == name).map(|f| f.params.len())
    }

    fn has_fn(&self, name: &str) -> bool {
        self.ast.as_ref().is_some_and(|a| a.iter_functions().any(|f| f.name == name))
    }

    fn call(&mut self, name: &str, args: impl rhai::FuncArgs) -> Option<Dynamic> {
        if !self.running || !self.has_fn(name) {
            return None;
        }
        let ast = self.ast.as_ref()?;
        self.host.lock().unwrap().hook_outbox = 0;
        match self.engine.call_fn::<Dynamic>(&mut self.scope, ast, name, args) {
            Ok(v) => Some(v),
            Err(e) => {
                let msg = format!("{name}(): {e}");
                self.host.lock().unwrap().log(format!("error: {msg}"));
                self.error = Some(msg);
                None
            }
        }
    }

    pub fn set_snapshot(&self, s: Snapshot) {
        self.host.lock().unwrap().snapshot = s;
    }

    pub fn event(&mut self, e: &HostEvent) {
        let granted = self.host.lock().unwrap().granted.clone();
        let friends = granted.contains(&Permission::FriendsRead);
        let (name, data): (&str, Map) = match e {
            HostEvent::LoggedIn => ("logged_in", Map::new()),
            HostEvent::GameStarted { app_id } => ("game_started", map([("app_id", (*app_id as i64).into())])),
            HostEvent::GameStopped { app_id } => ("game_stopped", map([("app_id", (*app_id as i64).into())])),
            HostEvent::FriendOnline { steam_id, name } if friends => {
                ("friend_online", map([("steam_id", steam_id.to_string().into()), ("name", name.clone().into())]))
            }
            HostEvent::FriendOffline { steam_id, name } if friends => {
                ("friend_offline", map([("steam_id", steam_id.to_string().into()), ("name", name.clone().into())]))
            }
            HostEvent::FriendGame { steam_id, name, game } if friends => (
                "friend_game",
                map([
                    ("steam_id", steam_id.to_string().into()),
                    ("name", name.clone().into()),
                    ("game", game.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT)),
                ]),
            ),
            HostEvent::DownloadFinished { app_id, ok, error } if granted.contains(&Permission::DownloadsRead) => (
                "download_finished",
                map([("app_id", (*app_id as i64).into()), ("ok", (*ok).into()), ("error", error.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT))]),
            ),
            HostEvent::DownloadStarted { app_id } if granted.contains(&Permission::DownloadsRead) => ("download_started", map([("app_id", (*app_id as i64).into())])),
            HostEvent::DownloadPaused { app_id } if granted.contains(&Permission::DownloadsRead) => ("download_paused", map([("app_id", (*app_id as i64).into())])),
            HostEvent::DownloadResumed { app_id } if granted.contains(&Permission::DownloadsRead) => ("download_resumed", map([("app_id", (*app_id as i64).into())])),
            HostEvent::TradeOffer { id, partner, name } if granted.contains(&Permission::TradesRead) => {
                ("trade_offer", map([("id", id.to_string().into()), ("partner", partner.to_string().into()), ("name", name.clone().into())]))
            }
            HostEvent::TradeChanged { id, state } if granted.contains(&Permission::TradesRead) => {
                ("trade_changed", map([("id", id.to_string().into()), ("state", trade_state(*state).into())]))
            }
            HostEvent::AchievementUnlocked { app_id, name, description, unlock_time } if granted.contains(&Permission::AchievementsRead) => (
                "achievement_unlocked",
                map([
                    ("app_id", (*app_id as i64).into()),
                    ("name", name.clone().into()),
                    ("description", description.clone().into()),
                    ("unlock_time", (*unlock_time as i64).into()),
                ]),
            ),
            HostEvent::WishlistChanged { added, removed } if granted.contains(&Permission::StoreRead) => {
                let ids = |v: &[u32]| Dynamic::from_array(v.iter().map(|x| Dynamic::from(*x as i64)).collect());
                ("wishlist_changed", map([("added", ids(added)), ("removed", ids(removed))]))
            }
            HostEvent::AppDetailsLoaded { app_id } if granted.contains(&Permission::StoreRead) => ("app_details_loaded", map([("app_id", (*app_id as i64).into())])),
            HostEvent::StoreSearchLoaded { term } if granted.contains(&Permission::StoreRead) => ("store_search_loaded", map([("term", term.clone().into())])),
            HostEvent::GameInstalled { app_id, name } if granted.contains(&Permission::LibraryRead) => {
                ("game_installed", map([("app_id", (*app_id as i64).into()), ("name", name.clone().into())]))
            }
            HostEvent::GameUninstalled { app_id, name } if granted.contains(&Permission::LibraryRead) => {
                ("game_uninstalled", map([("app_id", (*app_id as i64).into()), ("name", name.clone().into())]))
            }
            HostEvent::Message { steam_id, name, text } if granted.contains(&Permission::ChatRead) => (
                "message",
                map([("steam_id", steam_id.to_string().into()), ("name", name.clone().into()), ("text", text.clone().into())]),
            ),
            _ => return,
        };
        self.call("on_event", (ImmutableString::from(name), data));
    }

    pub fn has_page(&self) -> bool {
        self.manifest.page.is_some()
    }

    pub fn page(&mut self) -> Vec<Widget> {
        self.page_in(View::Window)
    }

    /// The page's widgets for the normal window or Big Picture. In Big
    /// Picture, `bigpicture_page()` is used if the script has one, else
    /// `page()` (which can ask `view_mode()`).
    pub fn page_in(&mut self, view: View) -> Vec<Widget> {
        self.host.lock().unwrap().view = view;
        let hook = if view == View::BigPicture && self.has_fn("bigpicture_page") { "bigpicture_page" } else { "page" };
        self.call(hook, ()).map(|d| widgets_bounded(d, 0, &mut 0)).unwrap_or_default()
    }

    /// Whether the page should be in Big Picture: the manifest says so.
    pub fn in_bigpicture(&self) -> bool {
        self.running && self.manifest.page.as_ref().is_some_and(|p| p.bigpicture)
    }

    /// A select or tabs widget changed: `on_select(id, value)`, or
    /// `on_input(id, value)` for a script without it.
    pub fn select(&mut self, id: &str, value: &str) {
        let hook = if self.has_fn("on_select") { "on_select" } else { "on_input" };
        self.call(hook, (ImmutableString::from(id), ImmutableString::from(value)));
    }

    /// A slider moved: `on_slider(id, value)`, or `on_input(id, "value")`.
    pub fn slider(&mut self, id: &str, value: f64) {
        if self.has_fn("on_slider") {
            self.call("on_slider", (ImmutableString::from(id), value));
        } else {
            self.call("on_input", (ImmutableString::from(id), ImmutableString::from(format_number(value))));
        }
    }

    pub fn click(&mut self, id: &str) {
        self.call("on_click", (ImmutableString::from(id),));
    }

    pub fn input(&mut self, id: &str, text: &str) {
        self.call("on_input", (ImmutableString::from(id), ImmutableString::from(text)));
    }

    pub fn toggle(&mut self, id: &str, on: bool) {
        self.call("on_toggle", (ImmutableString::from(id), on));
    }

    /// The answer to a request made with `http_get` and friends.
    /// `on_http(id, status, body)`, or `on_http(id, status, body, headers)`
    /// if the script takes the headers (a map, names in lower case). The
    /// answer to a cancelled request is dropped.
    pub fn http_done(&mut self, id: u64, status: u16, body: String, headers: Vec<(String, String)>) {
        if !self.host.lock().unwrap().in_flight.remove(&id) {
            return;
        }
        if self.params("on_http") == Some(4) {
            let headers: Map = headers.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
            self.call("on_http", (id.to_string(), status as i64, body, headers));
        } else {
            self.call("on_http", (id.to_string(), status as i64, body));
        }
    }

    /// Call `on_tick` if it's due (`now` in seconds since the epoch).
    /// Returns when it's due next.
    pub fn tick(&mut self, now: u64) -> Option<u64> {
        let every = self.manifest.tick?;
        if !self.running {
            return None;
        }
        if self.last_tick == 0 {
            // The first tick comes one period after start.
            self.last_tick = now;
        } else if now >= self.last_tick + every {
            self.last_tick = now;
            self.call("on_tick", ());
        }
        Some(self.last_tick + every)
    }

    /// Buttons for a game's page in the library.
    pub fn game_actions(&mut self, app_id: u32) -> Vec<MenuItem> {
        self.call("game_actions", (app_id as i64,)).map(menu_items).unwrap_or_default()
    }

    pub fn game_action(&mut self, id: &str, app_id: u32) {
        self.call("on_game_action", (ImmutableString::from(id), app_id as i64));
    }

    /// Items for a friend's right-click menu.
    pub fn friend_menu(&mut self, steam_id: u64) -> Vec<MenuItem> {
        self.call("friend_menu", (steam_id.to_string(),)).map(menu_items).unwrap_or_default()
    }

    pub fn friend_menu_click(&mut self, id: &str, steam_id: u64) {
        self.call("on_friend_menu", (ImmutableString::from(id), steam_id.to_string()));
    }

    /// Whether the script defines a hook (so the host can skip asking).
    pub fn defines(&self, name: &str) -> bool {
        self.running && self.has_fn(name)
    }

    /// Take the requests made since the last call.
    pub fn drain(&self) -> Vec<Request> {
        std::mem::take(&mut self.host.lock().unwrap().outbox)
    }

    pub fn logs(&self) -> Vec<String> {
        self.host.lock().unwrap().logs.iter().cloned().collect()
    }

    /// Storage to persist, if it changed.
    pub fn take_storage(&self) -> Option<BTreeMap<String, String>> {
        let mut h = self.host.lock().unwrap();
        if h.storage_dirty {
            h.storage_dirty = false;
            Some(h.storage.clone())
        } else {
            None
        }
    }
}

/// A number as a plugin would write it: "5", not "5.0".
pub fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

fn menu_items(d: Dynamic) -> Vec<MenuItem> {
    let Some(arr) = d.try_cast::<Array>() else { return Vec::new() };
    arr.into_iter()
        .filter_map(|v| {
            let m = v.try_cast::<Map>()?;
            let s = |k: &str| m.get(k).map(|v| v.to_string()).unwrap_or_default();
            let (id, text) = (s("id"), s("text"));
            (!id.is_empty() && !text.is_empty()).then(|| MenuItem { id, text: text.chars().take(60).collect() })
        })
        .take(8)
        .collect()
}

/// Whether `url` may be reached: https, to one of the plugin's hosts.
fn host_allowed(url: &str, hosts: &[String]) -> Result<(), String> {
    if url.contains('\\') || url.chars().any(char::is_control) {
        return Err("invalid URL".into());
    }
    let rest = url.strip_prefix("https://").ok_or("only https:// addresses are allowed")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err("userinfo not allowed in plugin URLs".into());
    }
    let host = net::host_of(authority);
    net::public_host(&host)?;
    let ok = hosts.iter().any(|h| match h.strip_prefix("*.") {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => host == *h,
    });
    if ok { Ok(()) } else { Err(format!("'{host}' isn't in this plugin's hosts")) }
}

/// JSON text to Rhai values (maps, arrays, strings, numbers, bools, ()).
fn from_json(v: serde_json::Value) -> Dynamic {
    use serde_json::Value as J;
    match v {
        J::Null => Dynamic::UNIT,
        J::Bool(b) => b.into(),
        J::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64().unwrap_or(0.0).into(),
        },
        J::String(s) => s.into(),
        J::Array(a) => Dynamic::from_array(a.into_iter().map(from_json).collect()),
        J::Object(o) => Dynamic::from_map(o.into_iter().map(|(k, v)| (k.into(), from_json(v))).collect()),
    }
}

fn to_json(d: &Dynamic) -> serde_json::Value {
    use serde_json::Value as J;
    if d.is_unit() {
        J::Null
    } else if let Ok(b) = d.as_bool() {
        J::Bool(b)
    } else if let Ok(i) = d.as_int() {
        J::from(i)
    } else if let Ok(f) = d.as_float() {
        J::from(f)
    } else if let Some(a) = d.clone().try_cast::<Array>() {
        J::Array(a.iter().map(to_json).collect())
    } else if let Some(m) = d.clone().try_cast::<Map>() {
        J::Object(m.iter().map(|(k, v)| (k.to_string(), to_json(v))).collect())
    } else {
        J::String(d.to_string())
    }
}

/// A trade offer state as plugins see it.
pub fn trade_state(s: model::TradeState) -> &'static str {
    use model::TradeState as S;
    match s {
        S::Active => "active",
        S::Accepted => "accepted",
        S::Countered => "countered",
        S::Expired => "expired",
        S::Cancelled => "cancelled",
        S::Declined => "declined",
        S::InvalidItems => "invalid_items",
        S::NeedsConfirmation => "needs_confirmation",
        S::InEscrow => "on_hold",
        S::Other => "other",
    }
}

/// A trade offer as plugins see it.
fn offer_map(s: &Snapshot, o: &model::TradeOffer, name: &str) -> Dynamic {
    let price = |a: u32, i: &model::InventoryItem| s.prices.get(&(a, i.market_hash_name.clone())).copied();
    let items = |l: &[model::TradeItem]| Dynamic::from_array(l.iter().map(|x| item_map(x.app_id, &x.item, price(x.app_id, &x.item))).collect());
    Dynamic::from_map(map([
        ("id", o.id.to_string().into()),
        ("partner", o.partner.to_string().into()),
        ("partner_name", name.to_string().into()),
        ("message", o.message.clone().into()),
        ("state", trade_state(o.state).into()),
        ("ours", o.ours.into()),
        ("created", (o.created as i64).into()),
        ("expires", (o.expires as i64).into()),
        ("held_until", (o.escrow_end as i64).into()),
        ("give", items(&o.give)),
        ("get", items(&o.get)),
    ]))
}

fn item_map(app_id: u32, i: &model::InventoryItem, price: Option<u64>) -> Dynamic {
    let tags: Array = i.tags.iter().map(|(c, v, _)| Dynamic::from_map(map([("category", c.clone().into()), ("name", v.clone().into())]))).collect();
    let mut m = map([
        ("app_id", (app_id as i64).into()),
        ("context_id", i.context_id.to_string().into()),
        ("asset_id", i.asset_id.to_string().into()),
        ("amount", (i.amount as i64).into()),
        ("name", i.name.clone().into()),
        ("type", i.kind.clone().into()),
        ("market_hash_name", i.market_hash_name.clone().into()),
        ("tradable", i.tradable.into()),
        ("marketable", i.marketable.into()),
        ("color", i.name_color.clone().unwrap_or_default().into()),
        ("icon", if i.icon.is_empty() { String::new() } else { model::art::econ(&i.icon, 192) }.into()),
        ("tags", Dynamic::from_array(tags)),
        ("class_id", i.class_id.to_string().into()),
        ("icon_large", i.icon_large.as_deref().filter(|s| !s.is_empty()).map(|s| model::art::econ(s, 512)).unwrap_or_default().into()),
        ("description", Dynamic::from_array(i.description.iter().map(|(text, _)| Dynamic::from(text.clone())).filter(|d| !d.to_string().trim().is_empty()).take(50).collect())),
        ("trade_hold_days", (i.trade_hold_days as i64).into()),
        ("market_hold_days", (i.market_hold_days as i64).into()),
        ("commodity", i.commodity.into()),
    ]);
    m.insert("price".into(), price.map(|p| Dynamic::from(p as i64)).unwrap_or(Dynamic::UNIT));
    Dynamic::from_map(m)
}

/// `[#{app_id, context_id, asset_id, amount?}, ...]` from a plugin.
fn assets(list: Array) -> Result<Vec<model::TradeAsset>, Box<EvalAltResult>> {
    let num = |m: &Map, k: &str| -> Option<u64> {
        let v = m.get(k)?;
        v.as_int().ok().map(|i| i as u64).or_else(|| v.clone().into_string().ok().and_then(|s| s.parse().ok()))
    };
    list.into_iter()
        .take(256)
        .map(|d| {
            let m = d.try_cast::<Map>().ok_or("each item must be a map like #{app_id, context_id, asset_id}")?;
            Ok(model::TradeAsset {
                app_id: num(&m, "app_id").ok_or("item is missing app_id")? as u32,
                context_id: num(&m, "context_id").ok_or("item is missing context_id")?,
                asset_id: num(&m, "asset_id").ok_or("item is missing asset_id")?,
                amount: num(&m, "amount").unwrap_or(1).max(1),
            })
        })
        .collect::<Result<_, &str>>()
        .map_err(|e: &str| e.into())
}

fn map<const N: usize>(pairs: [(&str, Dynamic); N]) -> Map {
    pairs.into_iter().map(|(k, v)| (k.into(), v)).collect()
}

fn sandboxed_engine(host: Arc<Mutex<Host>>) -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(MAX_OPERATIONS);
    e.set_max_call_levels(32);
    e.set_max_expr_depths(64, 32);
    e.set_max_string_size(64 * 1024);
    e.set_max_array_size(20_000);
    e.set_max_map_size(5_000);
    e.disable_symbol("eval");
    e.disable_symbol("import");

    let h = host.clone();
    e.on_print(move |s| h.lock().unwrap().log(s.to_string()));
    let h = host.clone();
    e.on_debug(move |s, _, _| h.lock().unwrap().log(s.to_string()));

    let h = host.clone();
    e.register_fn("log", move |s: &str| h.lock().unwrap().log(s.to_string()));
    let h = host.clone();
    e.register_fn("log_warn", move |s: &str| h.lock().unwrap().log(format!("warning: {s}")));
    let h = host.clone();
    e.register_fn("log_error", move |s: &str| h.lock().unwrap().log(format!("error: {s}")));
    let h = host.clone();
    e.register_fn("plugin_info", move || Dynamic::from_map(h.lock().unwrap().info.clone()));
    let h = host.clone();
    e.register_fn("view_mode", move || h.lock().unwrap().view.key());
    e.register_fn("now", || {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
    });

    let h = host.clone();
    e.register_fn("notify", move |s: &str| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Notifications)?;
        h.push(Request::Notify(s.chars().take(300).collect()))
    });

    let h = host.clone();
    e.register_fn("me", move || -> Result<Dynamic, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::AccountRead)?;
        Ok(match &h.snapshot.me {
            Some((id, name, status)) => {
                map([("steam_id", id.to_string().into()), ("name", name.clone().into()), ("status", status.clone().into())]).into()
            }
            None => Dynamic::UNIT,
        })
    });

    let h = host.clone();
    e.register_fn("friends", move || -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::FriendsRead)?;
        Ok(h.snapshot
            .friends
            .iter()
            .map(|(id, name, status, game)| {
                map([
                    ("steam_id", id.to_string().into()),
                    ("name", name.clone().into()),
                    ("status", status.clone().into()),
                    ("game", game.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT)),
                ])
                .into()
            })
            .collect())
    });

    let h = host.clone();
    e.register_fn("library", move || -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::LibraryRead)?;
        Ok(h.snapshot
            .library
            .iter()
            .map(|g| {
                map([
                    ("app_id", (g.app_id as i64).into()),
                    ("name", g.name.clone().into()),
                    ("installed", g.installed.into()),
                    ("playtime_minutes", (g.playtime_minutes as i64).into()),
                ])
                .into()
            })
            .collect())
    });

    let h = host.clone();
    e.register_fn("set_status", move |s: &str| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::StatusWrite)?;
        let st = match s {
            "online" => model::PersonaState::Online,
            "away" => model::PersonaState::Away,
            "busy" => model::PersonaState::Busy,
            "invisible" => model::PersonaState::Invisible,
            other => return Err(format!("unknown status '{other}'").into()),
        };
        h.push(Request::SetStatus(st))
    });

    let h = host.clone();
    e.register_fn("send_message", move |to: &str, text: &str| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::ChatWrite)?;
        let id: u64 = to.parse().map_err(|_| "steam_id must be a numeric string")?;
        if !h.snapshot.friends.iter().any(|f| f.0 == id) {
            return Err("can only message people on your friends list".into());
        }
        h.push(Request::SendMessage(id, text.chars().take(2000).collect()))
    });

    let h = host.clone();
    e.register_fn("storage_get", move |k: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        Ok(h.storage.get(k).cloned().map(Dynamic::from).unwrap_or(Dynamic::UNIT))
    });

    let h = host.clone();
    let request = move |method: &str, url: &str, body: &str, headers: Map, timeout: u64| -> Result<String, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Network)?;
        host_allowed(url, &h.hosts)?;
        if h.in_flight.len() >= MAX_IN_FLIGHT {
            return Err(format!("too many requests at once: at most {MAX_IN_FLIGHT} waiting for an answer").into());
        }
        let now = std::time::Instant::now();
        while h.request_times.front().is_some_and(|t| now.duration_since(*t).as_secs() >= 60) {
            h.request_times.pop_front();
        }
        if h.request_times.len() >= 30 {
            return Err("too many requests: at most 30 a minute".into());
        }
        let id = h.next_request + 1;
        let headers = headers.into_iter().take(16).map(|(k, v)| (k.to_string(), v.to_string())).collect();
        h.push(Request::Http { id, method: method.to_ascii_uppercase(), url: url.to_string(), body: body.chars().take(256 * 1024).collect(), headers, timeout })?;
        h.in_flight.insert(id);
        h.next_request = id;
        h.request_times.push_back(now);
        Ok(id.to_string())
    };
    let r = request.clone();
    e.register_fn("http_get", move |url: &str| r("GET", url, "", Map::new(), MAX_TIMEOUT));
    let r = request.clone();
    e.register_fn("http_post", move |url: &str, body: &str| r("POST", url, body, Map::new(), MAX_TIMEOUT));
    let check_method = |method: &str| -> Result<(), Box<EvalAltResult>> {
        if ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.to_ascii_uppercase().as_str()) { Ok(()) } else { Err(format!("unknown method '{method}'").into()) }
    };
    let r = request.clone();
    e.register_fn("http_request", move |method: &str, url: &str, body: &str, headers: Map| -> Result<String, Box<EvalAltResult>> {
        check_method(method)?;
        r(method, url, body, headers, MAX_TIMEOUT)
    });
    let r = request;
    e.register_fn("http_fetch", move |opts: Map| -> Result<String, Box<EvalAltResult>> {
        let get = |k: &str| opts.get(k).filter(|v| !v.is_unit());
        let url = get("url").map(|v| v.to_string()).ok_or("http_fetch needs a url")?;
        let mut headers: Map = get("headers").and_then(|v| v.clone().try_cast::<Map>()).unwrap_or_default();
        let body = match get("json") {
            Some(v) => {
                if !headers.keys().any(|k| k.eq_ignore_ascii_case("content-type")) {
                    headers.insert("Content-Type".into(), "application/json".into());
                }
                to_json(v).to_string()
            }
            None => get("body").map(|v| v.to_string()).unwrap_or_default(),
        };
        let method = get("method").map(|v| v.to_string()).unwrap_or_else(|| if body.is_empty() { "GET".into() } else { "POST".into() });
        check_method(&method)?;
        let timeout = get("timeout").and_then(|v| v.as_int().ok()).unwrap_or(MAX_TIMEOUT as i64).clamp(1, MAX_TIMEOUT as i64) as u64;
        r(&method, &url, &body, headers, timeout)
    });

    let h = host.clone();
    e.register_fn("http_cancel", move |id: &str| -> Result<bool, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Network)?;
        let Ok(id) = id.parse::<u64>() else { return Ok(false) };
        if !h.in_flight.remove(&id) {
            return Ok(false);
        }
        h.push_load(Request::HttpCancel(id));
        Ok(true)
    });

    let h = host.clone();
    e.register_fn("launch", move |app_id: i64| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::GamesLaunch)?;
        let id = app_id as u32;
        if !h.snapshot.library.iter().any(|g| g.app_id == id && g.installed) {
            return Err("that game isn't installed".into());
        }
        h.push(Request::Launch(id))
    });

    let h = host.clone();
    e.register_fn("wishlist", move || -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::StoreRead)?;
        Ok(h.snapshot.wishlist.iter().map(|(id, _)| Dynamic::from(*id as i64)).collect())
    });

    // Trades and inventory.
    let h = host.clone();
    e.register_fn("trade_offers", move || -> Result<Array, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::TradesRead)?;
        Ok(h.snapshot.trades.iter().map(|(o, name)| offer_map(&h.snapshot, o, name)).collect())
    });

    let h = host.clone();
    e.register_fn("refresh_trades", move || -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::TradesRead)?;
        h.push(Request::LoadTrades)
    });

    let h = host.clone();
    let respond = move |id: &str, action: model::TradeAction| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::TradesWrite)?;
        let id: u64 = id.parse().map_err(|_| "id must be a trade offer id")?;
        if !h.snapshot.trades.iter().any(|(o, _)| o.id == id) {
            return Err("no such trade offer".into());
        }
        h.push(Request::RespondTrade(id, action))
    };
    let r = respond.clone();
    e.register_fn("decline_trade", move |id: &str| r(id, model::TradeAction::Decline));
    let r = respond;
    e.register_fn("cancel_trade", move |id: &str| r(id, model::TradeAction::Cancel));

    let h = host.clone();
    e.register_fn("review_trade", move |id: &str| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::TradesWrite)?;
        let id: u64 = id.parse().map_err(|_| "id must be a trade offer id")?;
        h.push(Request::ReviewTrade(id))
    });

    let h = host.clone();
    e.register_fn("propose_trade", move |partner: &str, give: Array, get: Array, message: &str| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::TradesWrite)?;
        let partner: u64 = partner.parse().map_err(|_| "partner must be a numeric steam_id")?;
        let (give, get) = (assets(give)?, assets(get)?);
        h.push(Request::ProposeTrade { partner, give, get, message: message.chars().take(128).collect() })
    });

    let h = host.clone();
    e.register_fn("inventory", move |app_id: i64, context_id: i64| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::InventoryRead)?;
        let key = (app_id as u32, context_id as u64);
        match h.snapshot.inventory.get(&key) {
            Some(items) => {
                let list: Array = items.iter().map(|i| item_map(key.0, i, h.snapshot.prices.get(&(key.0, i.market_hash_name.clone())).copied())).collect();
                Ok(Dynamic::from_array(list))
            }
            None => {
                h.push(Request::LoadInventory(key.0, key.1))?;
                Ok(Dynamic::UNIT)
            }
        }
    });

    let h = host.clone();
    e.register_fn("market_price", move |app_id: i64, hash_name: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::InventoryRead)?;
        match h.snapshot.prices.get(&(app_id as u32, hash_name.to_string())) {
            Some(p) => Ok(Dynamic::from(*p as i64)),
            None => {
                h.push(Request::Prices(app_id as u32, vec![hash_name.to_string()]))?;
                Ok(Dynamic::UNIT)
            }
        }
    });

    let h = host.clone();
    e.register_fn("currency", move || h.lock().unwrap().snapshot.currency.clone());

    let h = host.clone();
    e.register_fn("copy", move |text: &str| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Clipboard)?;
        h.push(Request::Copy(text.chars().take(64 * 1024).collect()))
    });

    let h = host.clone();
    e.register_fn("navigate", move |page: &str, arg: Dynamic| -> Result<(), Box<EvalAltResult>> {
        const PAGES: [&str; 6] = ["store", "library", "community", "profile", "downloads", "settings"];
        if !PAGES.contains(&page) && !page.starts_with("plugin:") {
            return Err(format!("unknown page '{page}'").into());
        }
        let arg = (!arg.is_unit()).then(|| arg.to_string());
        h.lock().unwrap().push(Request::Navigate(page.to_string(), arg))
    });

    let h = host.clone();
    e.register_fn("language", move || h.lock().unwrap().snapshot.language.clone());
    e.register_fn("platform", || if cfg!(target_os = "macos") { "macos" } else if cfg!(windows) { "windows" } else { "linux" });
    e.register_fn("json_parse", |text: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        serde_json::from_str::<serde_json::Value>(text).map(from_json).map_err(|e| format!("bad JSON: {e}").into())
    });
    e.register_fn("json_string", |v: Dynamic| to_json(&v).to_string());

    e.register_fn("api_version", || API_VERSION);
    library::register(&mut e, &host);
    achievements::register(&mut e, &host);
    downloads::register(&mut e, &host);
    store::register(&mut e, &host);
    items::register(&mut e, &host);
    timers::register(&mut e, &host);
    storage::register(&mut e, &host);

    let h = host;
    e.register_fn("storage_set", move |k: &str, v: Dynamic| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        h.store(k, v.to_string().chars().take(storage::MAX_VALUE).collect())
    });

    e
}

/// Load every plugin folder in `dir`. Broken plugins are returned as errors
/// so the manager can show them.
pub fn discover(dir: &Path) -> Vec<Result<Plugin, (PathBuf, Error)>> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let loaded: Vec<(PathBuf, Result<Plugin, Error>)> = entries
        .flatten()
        .filter(|e| e.path().join("plugin.toml").is_file())
        .map(|e| {
            let p = e.path();
            let res = Plugin::load(&p);
            (p, res)
        })
        .collect();
    let mut id_counts: HashMap<String, usize> = HashMap::new();
    for (_, res) in &loaded {
        if let Ok(p) = res {
            *id_counts.entry(p.manifest.id.clone()).or_insert(0) += 1;
        }
    }
    let mut out: Vec<Result<Plugin, (PathBuf, Error)>> = loaded
        .into_iter()
        .map(|(path, res)| match res {
            Ok(p) if id_counts.get(&p.manifest.id).copied().unwrap_or(0) > 1 => {
                Err((path, Error::Manifest(format!("duplicate plugin id '{}'", p.manifest.id))))
            }
            Ok(p) => Ok(p),
            Err(err) => Err((path, err)),
        })
        .collect();
    out.sort_by_key(|r| match r {
        Ok(p) => (0u8, p.manifest.name.to_lowercase()),
        Err((p, _)) => (1u8, p.display().to_string()),
    });
    out
}

/// Example plugins bundled with the app, installed on first run.
pub const EXAMPLES: &[(&str, &str, &str)] = &[
    ("friend-alerts", include_str!("../examples/friend-alerts/plugin.toml"), include_str!("../examples/friend-alerts/main.rhai")),
    ("backlog", include_str!("../examples/backlog/plugin.toml"), include_str!("../examples/backlog/main.rhai")),
    ("wishlist-sales", include_str!("../examples/wishlist-sales/plugin.toml"), include_str!("../examples/wishlist-sales/main.rhai")),
    ("trade-helper", include_str!("../examples/trade-helper/plugin.toml"), include_str!("../examples/trade-helper/main.rhai")),
    ("widget-gallery", include_str!("../examples/widget-gallery/plugin.toml"), include_str!("../examples/widget-gallery/main.rhai")),
];

/// Copy the bundled examples into `dir` if they aren't there yet.
pub fn install_examples(dir: &Path) {
    for (id, manifest, script) in EXAMPLES {
        let d = dir.join(id);
        if d.exists() {
            continue;
        }
        if std::fs::create_dir_all(&d).is_ok() {
            let _ = std::fs::write(d.join("plugin.toml"), manifest);
            let _ = std::fs::write(d.join("main.rhai"), script);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn plugin(toml: &str, script: &str) -> Plugin {
        let dir = std::env::temp_dir().join(format!("sir-plugin-test-{}", rand_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.toml"), toml).unwrap();
        std::fs::write(dir.join("main.rhai"), script).unwrap();
        Plugin::load(&dir).unwrap()
    }

    /// Unique per test, even when tests run at the same instant.
    fn rand_id() -> u128 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u128;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        now * 1000 + n + (std::process::id() as u128) * 1_000_000_000
    }

    #[test]
    fn permission_is_enforced() {
        let mut p = plugin("id = \"t\"\nname = \"T\"\n", "fn on_load() { notify(\"hi\"); }");
        p.start(&BTreeSet::new(), Default::default());
        assert!(p.drain().is_empty());
        assert!(p.error.as_deref().unwrap_or("").contains("permission denied"));
    }

    #[test]
    fn granted_permission_works_and_page_renders() {
        let mut p = plugin(
            "id = \"t2\"\nname = \"T\"\npermissions = [\"notifications\"]\n[page]\ntitle = \"T\"\n",
            "fn on_load() { notify(\"hi\"); }\nfn page() { [#{type: \"heading\", text: \"Hello\"}, #{type: \"button\", id: \"b\", text: \"Go\"}] }",
        );
        p.start(&BTreeSet::from([Permission::Notifications]), Default::default());
        assert!(matches!(p.drain().as_slice(), [Request::Notify(s)] if s == "hi"));
        assert_eq!(p.page(), vec![Widget::Heading("Hello".into()), Widget::Button { id: "b".into(), text: "Go".into() }]);
    }

    #[test]
    fn runaway_script_is_stopped() {
        let mut p = plugin("id = \"t3\"\nname = \"T\"\n", "fn on_load() { loop { } }");
        p.start(&BTreeSet::new(), Default::default());
        assert!(p.error.is_some());
    }

    #[test]
    fn network_only_reaches_listed_hosts() {
        let mut p = plugin(
            "id = \"n\"\nname = \"N\"\npermissions = [\"network\"]\nhosts = [\"api.example.com\", \"*.steampowered.com\"]\n",
            "fn on_load() { http_get(\"https://api.example.com/x?y=1\"); http_get(\"https://store.steampowered.com/api\"); }",
        );
        p.start(&BTreeSet::from([Permission::Network]), Default::default());
        assert!(p.error.is_none(), "{:?}", p.error);
        let reqs = p.drain();
        assert_eq!(reqs.len(), 2);
        assert!(matches!(&reqs[0], Request::Http { id: 1, method, url, timeout: 20, .. } if method == "GET" && url == "https://api.example.com/x?y=1"));

        for bad in ["http://api.example.com/", "https://evil.com/", "https://api.example.com.evil.com/", "https://evil.com@api.example.com.x/", "https://evil.com@api.example.com/"] {
            let mut p = plugin(
                "id = \"n2\"\nname = \"N\"\npermissions = [\"network\"]\nhosts = [\"api.example.com\"]\n",
                &format!("fn on_load() {{ http_get(\"{bad}\"); }}"),
            );
            p.start(&BTreeSet::from([Permission::Network]), Default::default());
            assert!(p.error.is_some(), "{bad} should be refused");
            assert!(p.drain().is_empty());
        }
    }

    #[test]
    fn answers_ticks_hooks_and_json() {
        let mut p = plugin(
            "id = \"h\"\nname = \"H\"\npermissions = [\"storage\"]\ntick = 30\n",
            r#"
            fn on_http(id, status, body) { storage_set("got", id + " " + status + " " + json_parse(body).name); }
            fn on_tick() { storage_set("ticks", "1"); }
            fn game_actions(app_id) { [#{id: "a", text: "Look up " + app_id}] }
            fn on_game_action(id, app_id) { storage_set("action", id + app_id); }
            fn friend_menu(steam_id) { [#{id: "f", text: "Hi"}] }
            fn page() { [#{type: "input", id: "q", value: "x", hint: "Search"}, #{type: "table", columns: ["A"], rows: [[1], [2]]}, #{type: "toggle", id: "t", text: "On", value: true}] }
            fn json_round() { json_string(#{a: [1, true, ()]}) }
            "#,
        );
        p.start(&BTreeSet::from([Permission::Storage]), Default::default());
        assert!(p.error.is_none(), "{:?}", p.error);
        p.host.lock().unwrap().in_flight.insert(7);
        p.http_done(7, 200, r#"{"name":"Ferris"}"#.into(), Vec::new());
        assert_eq!(p.tick(1000), Some(1030));
        assert_eq!(p.tick(1031), Some(1061));
        assert_eq!(p.game_actions(440), vec![MenuItem { id: "a".into(), text: "Look up 440".into() }]);
        p.game_action("a", 440);
        assert_eq!(p.friend_menu(76561198000000000).len(), 1);
        let s = p.take_storage().unwrap();
        assert_eq!(s.get("got").map(String::as_str), Some("7 200 Ferris"));
        assert_eq!(s.get("ticks").map(String::as_str), Some("1"));
        assert_eq!(s.get("action").map(String::as_str), Some("a440"));
        assert!(matches!(&p.page()[..], [Widget::Input { .. }, Widget::Table { rows, .. }, Widget::Toggle { value: true, .. }] if rows.len() == 2));
        assert_eq!(p.call("json_round", ()).unwrap().to_string(), r#"{"a":[1,true,null]}"#);
    }

    #[test]
    fn trades_need_permission_and_accepting_goes_through_the_app() {
        let offer = model::TradeOffer {
            id: 77,
            partner: 76561197960265770,
            state: model::TradeState::Active,
            get: vec![model::TradeItem { app_id: 753, item: model::InventoryItem { name: "Card".into(), market_hash_name: "570-Card".into(), marketable: true, amount: 1, ..Default::default() }, missing: false }],
            ..Default::default()
        };
        let snap = || Snapshot { trades: Arc::new(vec![(offer.clone(), "Pal".into())]), prices: Arc::new([((753, "570-Card".to_string()), 12)].into_iter().collect()), ..Default::default() };
        let script = r#"
            fn on_load() {
                let o = trade_offers()[0];
                storage_set("seen", o.partner_name + " " + o.state + " " + o.get[0].price);
                review_trade(o.id);
                decline_trade(o.id);
                propose_trade(o.partner, [#{app_id: 753, context_id: "6", asset_id: "5"}], [], "hi");
            }
        "#;
        let mut p = plugin("id = \"tr\"
name = \"T\"
permissions = [\"trades.read\", \"trades.write\", \"storage\"]
", script);
        p.set_snapshot(snap());
        p.start(&BTreeSet::from([Permission::TradesRead, Permission::TradesWrite, Permission::Storage]), Default::default());
        assert!(p.error.is_none(), "{:?}", p.error);
        let reqs = p.drain();
        assert_eq!(p.take_storage().unwrap().get("seen").map(String::as_str), Some("Pal active 12"));
        assert!(reqs.iter().any(|r| matches!(r, Request::ReviewTrade(77))));
        assert!(reqs.iter().any(|r| matches!(r, Request::RespondTrade(77, model::TradeAction::Decline))));
        assert!(reqs.iter().any(|r| matches!(r, Request::ProposeTrade { give, .. } if give[0].asset_id == 5 && give[0].context_id == 6)));

        // Without the permission, nothing.
        let mut q = plugin("id = \"tr2\"
name = \"T\"
", "fn on_load() { trade_offers(); }");
        q.set_snapshot(snap());
        q.start(&BTreeSet::new(), Default::default());
        assert!(q.error.as_deref().is_some_and(|e| e.contains("trades.read")), "{:?}", q.error);
    }

    #[test]
    fn examples_parse() {
        for (id, manifest, script) in EXAMPLES {
            let mut p = plugin(manifest, script);
            let approved: BTreeSet<Permission> = p.manifest.permissions.iter().copied().collect();
            p.start(&approved, Default::default());
            assert!(p.error.is_none(), "{id}: {:?}", p.error);
        }
    }

    #[test]
    fn unapproved_manifest_permissions_refuse_to_start() {
        let mut p = plugin(
            "id = \"esc\"\nname = \"Esc\"\npermissions = [\"notifications\", \"chat.write\"]\n",
            "fn on_load() { notify(\"hi\"); }",
        );
        p.start(&BTreeSet::from([Permission::Notifications]), Default::default());
        assert!(!p.is_running(), "plugin must not start with unapproved permission");
        assert!(p.drain().is_empty());
        assert!(p.error.as_deref().unwrap_or("").contains("chat.write"));

        // Superset approval never grants undeclared permissions either.
        let mut p2 = plugin("id = \"sub\"\nname = \"Sub\"\n", "fn on_load() { notify(\"hi\"); }");
        p2.start(&BTreeSet::from([Permission::Notifications]), Default::default());
        assert!(p2.drain().is_empty());
        assert!(p2.error.as_deref().unwrap_or("").contains("permission denied"));
    }

    #[test]
    fn import_is_rejected() {
        let dir = std::env::temp_dir().join(format!("sir-plugin-import-{}", rand_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let outside = std::env::temp_dir().join(format!("sir-outside-{}", rand_id()));
        std::fs::write(outside.with_extension("rhai"), "fn pwn() { 42 }").unwrap();
        let outside_stem = outside.display().to_string();

        let mut p1 = plugin("id = \"imp1\"\nname = \"Imp\"\n", &format!("import \"{outside_stem}\" as m;\nfn on_load() {{ log(\"\" + m::pwn()); }}"));
        p1.start(&BTreeSet::new(), Default::default());
        assert!(p1.error.is_some(), "static import should fail");
        assert!(!p1.logs().iter().any(|l| l == "42"), "imported code must not run");

        let mut p2 = plugin(
            "id = \"imp2\"\nname = \"Imp2\"\n",
            &format!("fn on_load() {{ let target = \"{outside_stem}\"; import target as m; log(\"\" + m::pwn()); }}"),
        );
        p2.start(&BTreeSet::new(), Default::default());
        assert!(p2.error.is_some(), "dynamic import should fail");
        assert!(!p2.logs().iter().any(|l| l == "42"), "dynamically imported code must not run");
    }

    #[test]
    fn duplicate_manifest_ids_are_rejected() {
        let root = std::env::temp_dir().join(format!("sir-plugin-dup-{}", rand_id()));
        let a = root.join("aaa-evil");
        let b = root.join("zzz-legit");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("plugin.toml"), "id = \"same\"\nname = \"AAA\"\n").unwrap();
        std::fs::write(a.join("main.rhai"), "").unwrap();
        std::fs::write(b.join("plugin.toml"), "id = \"same\"\nname = \"ZZZ\"\n").unwrap();
        std::fs::write(b.join("main.rhai"), "").unwrap();

        let found = discover(&root);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|r| r.is_err()), "colliding plugin IDs must all fail");
    }

    #[test]
    fn manifest_main_traversal_and_symlink_are_rejected() {
        let base = std::env::temp_dir().join(format!("sir-plugin-main-{}", rand_id()));
        let pdir = base.join("plug");
        std::fs::create_dir_all(&pdir).unwrap();
        let secret = base.join("secret.rhai");
        std::fs::write(&secret, "fn on_load() { log(\"escaped\"); }").unwrap();

        std::fs::write(pdir.join("plugin.toml"), "id = \"p\"\nname = \"P\"\nmain = \"../secret.rhai\"\n").unwrap();
        std::fs::write(pdir.join("main.rhai"), "").unwrap();
        assert!(Plugin::load(&pdir).is_err(), "../ traversal must fail");

        std::fs::write(pdir.join("plugin.toml"), "id = \"p\"\nname = \"P\"\nmain = \"..\\\\secret.rhai\"\n").unwrap();
        assert!(Plugin::load(&pdir).is_err(), "..\\ traversal must fail");

        std::fs::write(pdir.join("plugin.toml"), format!("id = \"p\"\nname = \"P\"\nmain = \"{}\"\n", secret.display().to_string().replace('\\', "/"))).unwrap();
        assert!(Plugin::load(&pdir).is_err(), "absolute main must fail");

        #[cfg(unix)]
        {
            let sdir = base.join("symplug");
            std::fs::create_dir_all(&sdir).unwrap();
            std::fs::write(sdir.join("plugin.toml"), "id = \"s\"\nname = \"S\"\n").unwrap();
            std::os::unix::fs::symlink(&secret, sdir.join("main.rhai")).unwrap();
            assert!(Plugin::load(&sdir).is_err(), "symlink main.rhai must fail");

            let tdir = base.join("toctou");
            std::fs::create_dir_all(&tdir).unwrap();
            std::fs::write(tdir.join("plugin.toml"), "id = \"tc\"\nname = \"TC\"\n").unwrap();
            std::fs::write(tdir.join("main.rhai"), "fn on_load() { log(\"ok\"); }").unwrap();
            let mut tp = Plugin::load(&tdir).unwrap();
            std::fs::remove_file(tp.dir.join("main.rhai")).unwrap();
            std::os::unix::fs::symlink(&secret, tp.dir.join("main.rhai")).unwrap();
            tp.start(&BTreeSet::new(), Default::default());
            assert!(!tp.is_running(), "TOCTOU symlink swap must fail");
            assert!(!tp.logs().iter().any(|l| l == "escaped"));
        }
    }

    #[test]
    fn widget_row_depth_is_capped_at_8() {
        let mut p = plugin(
            "id = \"rowdepth\"\nname = \"R\"\n[page]\ntitle = \"R\"\n",
            "fn page() { let w = #{type: \"text\", text: \"leaf\"}; for i in 0..12 { w = #{type: \"row\", items: [w]}; } [w] }",
        );
        p.start(&BTreeSet::new(), Default::default());
        fn max_row_depth(ws: &[Widget]) -> usize {
            ws.iter()
                .map(|w| match w {
                    Widget::Row(inner) => 1 + max_row_depth(inner),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        }
        assert!(max_row_depth(&p.page()) < MAX_WIDGET_DEPTH);
    }

    #[test]
    fn outbox_is_capped_per_hook() {
        let mut p = plugin(
            "id = \"spam\"\nname = \"Spam\"\npermissions = [\"notifications\"]\n",
            "fn on_load() { for i in 0..100 { notify(\"n\" + i); } }",
        );
        p.start(&BTreeSet::from([Permission::Notifications]), Default::default());
        assert_eq!(p.drain().len(), MAX_OUTBOX);
        assert!(p.error.as_deref().unwrap_or("").contains("rate limit"));

        // Many price lookups for one game are one request, not 100.
        let mut q = plugin(
            "id = \"prices\"\nname = \"P\"\npermissions = [\"inventory.read\"]\n",
            "fn on_load() { for i in 0..100 { market_price(753, \"card \" + i); } market_price(730, \"case\"); }",
        );
        q.start(&BTreeSet::from([Permission::InventoryRead]), Default::default());
        assert!(q.error.is_none(), "{:?}", q.error);
        let reqs = q.drain();
        assert_eq!(reqs.len(), 2);
        assert!(reqs.iter().any(|r| matches!(r, Request::Prices(753, names) if names.len() == 100)));
    }

    fn net_plugin(script: &str) -> Plugin {
        let mut p = plugin("id = \"web\"\nname = \"W\"\nversion = \"1.2.3\"\npermissions = [\"network\", \"storage\"]\nhosts = [\"api.example.com\"]\n", script);
        p.start(&BTreeSet::from([Permission::Network, Permission::Storage]), Default::default());
        p
    }

    #[test]
    fn http_fetch_sends_json_with_a_timeout() {
        let p = net_plugin(r#"fn on_load() { http_fetch(#{url: "https://api.example.com/v1", json: #{a: 1}, timeout: 5}); http_fetch(#{url: "https://api.example.com/v2", method: "delete", timeout: 999}); }"#);
        assert!(p.error.is_none(), "{:?}", p.error);
        let reqs = p.drain();
        assert!(matches!(&reqs[0], Request::Http { method, body, headers, timeout: 5, .. } if method == "POST" && body == r#"{"a":1}"# && headers.iter().any(|(k, v)| k == "Content-Type" && v == "application/json")), "{reqs:?}");
        assert!(matches!(&reqs[1], Request::Http { method, timeout: 20, .. } if method == "DELETE"), "{reqs:?}");
        let bad = net_plugin(r#"fn on_load() { http_fetch(#{url: "https://api.example.com/", method: "TRACE"}); }"#);
        assert!(bad.error.as_deref().is_some_and(|e| e.contains("unknown method")), "{:?}", bad.error);
    }

    #[test]
    fn http_answers_carry_headers_and_cancelled_ones_are_dropped() {
        let mut p = net_plugin(
            r#"
            fn on_load() { storage_set("a", http_get("https://api.example.com/a")); storage_set("b", http_get("https://api.example.com/b")); }
            fn on_http(id, status, body, headers) { storage_set("got", id + " " + status + " " + headers["x-rate"]); }
            fn cancel(id) { http_cancel(id) }
            "#,
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        let s = p.take_storage().unwrap();
        let (a, b) = (s["a"].clone(), s["b"].clone());
        p.drain();
        assert_eq!(p.call("cancel", (b.clone(),)).unwrap().as_bool().unwrap(), true);
        assert!(matches!(p.drain().as_slice(), [Request::HttpCancel(id)] if id.to_string() == b));
        p.http_done(b.parse().unwrap(), 200, "late".into(), Vec::new());
        assert!(p.take_storage().is_none(), "a cancelled answer is dropped");
        p.http_done(a.parse().unwrap(), 200, "ok".into(), vec![("x-rate".into(), "5".into())]);
        assert_eq!(p.take_storage().unwrap()["got"], format!("{a} 200 5"));
        p.http_done(a.parse().unwrap(), 200, "twice".into(), Vec::new());
        assert!(p.take_storage().is_none(), "one answer per request");
    }

    #[test]
    fn requests_waiting_at_once_are_capped() {
        let p = net_plugin("fn on_load() { for i in 0..9 { http_get(\"https://api.example.com/\" + i); } }");
        assert!(p.error.as_deref().is_some_and(|e| e.contains("at most 8")), "{:?}", p.error);
        assert_eq!(p.drain().len(), MAX_IN_FLIGHT);
    }

    #[test]
    fn unload_info_and_log_levels() {
        let mut p = net_plugin(
            r#"
            fn on_load() { let i = plugin_info(); log_warn("careful"); log_error("broke"); storage_set("info", i.id + " " + i.version + " " + i.permissions + " " + api_version()); }
            fn on_unload() { storage_set("bye", "yes"); }
            "#,
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        assert_eq!(p.take_storage().unwrap()["info"], "web 1.2.3 [\"storage\", \"network\"] 4");
        assert!(p.logs().iter().any(|l| l == "warning: careful") && p.logs().iter().any(|l| l == "error: broke"));
        p.stop();
        assert_eq!(p.take_storage().unwrap()["bye"], "yes", "on_unload can still save");
        assert!(!p.is_running());
        p.stop();
        assert!(p.take_storage().is_none(), "on_unload runs once");
    }

    #[test]
    fn install_events_need_library_read() {
        let script = "fn on_event(name, data) { storage_set(name, data.name + \" \" + data.app_id); }";
        let mut p = plugin("id = \"ins\"\nname = \"I\"\npermissions = [\"library.read\", \"storage\"]\n", script);
        p.start(&BTreeSet::from([Permission::LibraryRead, Permission::Storage]), Default::default());
        p.event(&HostEvent::GameInstalled { app_id: 440, name: "TF2".into() });
        p.event(&HostEvent::GameUninstalled { app_id: 70, name: "HL".into() });
        let s = p.take_storage().unwrap();
        assert_eq!(s["game_installed"], "TF2 440");
        assert_eq!(s["game_uninstalled"], "HL 70");
        let mut q = plugin("id = \"ins2\"\nname = \"I\"\npermissions = [\"storage\"]\n", script);
        q.start(&BTreeSet::from([Permission::Storage]), Default::default());
        q.event(&HostEvent::GameInstalled { app_id: 440, name: "TF2".into() });
        assert!(q.take_storage().is_none());
    }

    #[test]
    fn big_picture_pages_and_new_hooks() {
        let mut p = plugin(
            "id = \"bp\"\nname = \"B\"\npermissions = [\"storage\"]\n[page]\ntitle = \"B\"\nbigpicture = true\n",
            r#"
            fn page() { [#{type: "text", text: "page in " + view_mode()}] }
            fn bigpicture_page() { [#{type: "text", text: "couch in " + view_mode()}] }
            fn on_select(id, value) { storage_set(id, value); }
            fn on_input(id, text) { storage_set("input " + id, text); }
            "#,
        );
        p.start(&BTreeSet::from([Permission::Storage]), Default::default());
        assert!(p.in_bigpicture());
        assert_eq!(p.page_in(View::BigPicture), vec![Widget::Text("couch in bigpicture".into())]);
        assert_eq!(p.page(), vec![Widget::Text("page in window".into())]);
        p.select("sort", "time");
        p.slider("vol", 5.0);
        let s = p.take_storage().unwrap();
        assert_eq!(s["sort"], "time");
        assert_eq!(s["input vol"], "5", "a slider falls back to on_input");

        // Without bigpicture_page, page() runs and knows where it is.
        let mut q = plugin("id = \"bp2\"\nname = \"B\"\n[page]\ntitle = \"B\"\n", "fn page() { [#{type: \"text\", text: view_mode()}] }");
        q.start(&BTreeSet::new(), Default::default());
        assert!(!q.in_bigpicture(), "pages stay out of Big Picture unless they ask");
        assert_eq!(q.page_in(View::BigPicture), vec![Widget::Text("bigpicture".into())]);
        assert_eq!(format_number(2.5), "2.5");
    }

    #[test]
    fn widget_gallery_renders_every_tab() {
        let (_, manifest, script) = EXAMPLES.iter().find(|e| e.0 == "widget-gallery").unwrap();
        let mut p = plugin(manifest, script);
        let lib = vec![GameInfo { app_id: 440, name: "TF2".into(), installed: true, playtime_minutes: 600, ..Default::default() }];
        p.set_snapshot(Snapshot { library: Arc::new(lib), ..Default::default() });
        p.start(&p.manifest.permissions.iter().copied().collect(), Default::default());
        assert!(p.in_bigpicture());
        for tab in ["basics", "inputs", "data", "layout"] {
            p.select("tab", tab);
            let w = p.page_in(View::BigPicture);
            assert!(p.error.is_none(), "{tab}: {:?}", p.error);
            assert!(w.len() > 3, "{tab}: {w:?}");
        }
        p.click("count");
        p.slider("level", 7.0);
        let s = p.take_storage().unwrap();
        assert_eq!((s["count"].as_str(), s["level"].as_str()), ("1", "7"));
    }
}

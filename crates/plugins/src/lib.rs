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
//! ## Script API
//! | Function | Permission |
//! |---|---|
//! | `log(text)`, `now()` | none |
//! | `notify(text)` | `notifications` |
//! | `me()` | `account.read` |
//! | `friends()` | `friends.read` |
//! | `library()` | `library.read` |
//! | `set_status("online" / "away" / "busy" / "invisible")` | `status.write` |
//! | `send_message(steam_id, text)` | `chat.write` |
//! | `storage_get(key)`, `storage_set(key, value)` | `storage` |
//!
//! ## Hooks (define any of these functions)
//! - `on_load()`
//! - `on_event(name, data)`: `"friend_online"`, `"friend_offline"`, `"friend_game"`
//!   (need `friends.read`), `"message"` (needs `chat.read`), `"game_started"`,
//!   `"game_stopped"`, `"logged_in"`.
//! - `page()`: returns an array of widgets for the plugin's page (needs `[page]` in the manifest).
//! - `on_click(id)`: a button on the page was clicked.
//!
//! Widgets are maps: `#{type: "heading", text: "Hi"}`, `"text"`, `"muted"`,
//! `#{type: "button", id: "go", text: "Go"}`, `#{type: "stat", label, value}`,
//! `"separator"`, `#{type: "row", items: [...]}`, `#{type: "progress", value: 0.5}`,
//! `#{type: "game", app_id: 440}` (a game card).

use rhai::{AST, Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Scope};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
}

impl Permission {
    pub const ALL: [Permission; 8] = [
        Self::AccountRead,
        Self::FriendsRead,
        Self::LibraryRead,
        Self::ChatRead,
        Self::ChatWrite,
        Self::StatusWrite,
        Self::Notifications,
        Self::Storage,
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
        }
    }
    /// Permissions that act on your behalf get a warning in the UI.
    pub fn sensitive(self) -> bool {
        matches!(self, Self::ChatRead | Self::ChatWrite | Self::StatusWrite)
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == s)
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
    main: String,
}

/// Data the host shares with plugins. Filled by the app each frame a plugin runs.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub me: Option<(u64, String, String)>,
    /// (steam_id, name, status, game)
    pub friends: Vec<(u64, String, String, Option<String>)>,
    /// (app_id, name, installed, playtime minutes)
    pub library: Vec<(u32, String, bool, u64)>,
}

/// Things a plugin asked the host to do.
#[derive(Debug, Clone)]
pub enum Request {
    Notify(String),
    SetStatus(model::PersonaState),
    SendMessage(u64, String),
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum Widget {
    Heading(String),
    Text(String),
    Muted(String),
    Button { id: String, text: String },
    Stat { label: String, value: String },
    Separator,
    Row(Vec<Widget>),
    Progress(f32),
    Game(u32),
}

#[derive(Default)]
struct Host {
    granted: HashSet<Permission>,
    snapshot: Snapshot,
    outbox: Vec<Request>,
    storage: BTreeMap<String, String>,
    storage_dirty: bool,
    logs: VecDeque<String>,
}

impl Host {
    fn need(&self, p: Permission) -> Result<(), Box<EvalAltResult>> {
        if self.granted.contains(&p) {
            Ok(())
        } else {
            Err(format!("permission denied: this plugin didn't request '{}'", p.key()).into())
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
}

const MAX_OPERATIONS: u64 = 2_000_000;

impl Plugin {
    pub fn load(dir: &Path) -> Result<Plugin, Error> {
        let manifest_path = dir.join("plugin.toml");
        let text = std::fs::read_to_string(&manifest_path).map_err(|e| Error::Io(manifest_path.clone(), e))?;
        let raw: RawManifest = toml::from_str(&text).map_err(|e| Error::Manifest(e.to_string()))?;
        if raw.id.is_empty() || !raw.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(Error::Manifest("id must be letters, digits, '-' or '_'".into()));
        }
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
            main: raw.main,
        };
        let host = Arc::new(Mutex::new(Host::default()));
        let engine = sandboxed_engine(host.clone());
        Ok(Plugin { manifest, dir: dir.to_path_buf(), engine, ast: None, scope: Scope::new(), host, running: false, error: None })
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Compile and run the script with the given approved permissions.
    /// `storage` is the plugin's saved data.
    pub fn start(&mut self, storage: BTreeMap<String, String>) {
        self.stop();
        let path = self.dir.join(&self.manifest.main);
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(format!("{}: {e}", path.display()));
                return;
            }
        };
        {
            let mut h = self.host.lock().unwrap();
            h.granted = self.manifest.permissions.iter().copied().collect();
            h.storage = storage;
            h.storage_dirty = false;
        }
        match self.engine.compile(&src) {
            Ok(ast) => {
                self.scope = Scope::new();
                if let Err(e) = self.engine.run_ast_with_scope(&mut self.scope, &ast) {
                    self.error = Some(e.to_string());
                    return;
                }
                self.ast = Some(ast);
                self.running = true;
                self.error = None;
                self.call("on_load", ());
            }
            Err(e) => self.error = Some(format!("{e}")),
        }
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.ast = None;
        self.host.lock().unwrap().granted.clear();
    }

    fn has_fn(&self, name: &str) -> bool {
        self.ast.as_ref().is_some_and(|a| a.iter_functions().any(|f| f.name == name))
    }

    fn call(&mut self, name: &str, args: impl rhai::FuncArgs) -> Option<Dynamic> {
        if !self.running || !self.has_fn(name) {
            return None;
        }
        let ast = self.ast.as_ref()?;
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
        self.call("page", ()).map(|d| widgets(d)).unwrap_or_default()
    }

    pub fn click(&mut self, id: &str) {
        self.call("on_click", (ImmutableString::from(id),));
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

fn map<const N: usize>(pairs: [(&str, Dynamic); N]) -> Map {
    pairs.into_iter().map(|(k, v)| (k.into(), v)).collect()
}

fn widgets(d: Dynamic) -> Vec<Widget> {
    let Some(arr) = d.try_cast::<Array>() else { return Vec::new() };
    arr.into_iter().filter_map(widget).take(500).collect()
}

fn widget(d: Dynamic) -> Option<Widget> {
    let m = d.try_cast::<Map>()?;
    let s = |k: &str| m.get(k).map(|v| v.to_string()).unwrap_or_default();
    Some(match s("type").as_str() {
        "heading" => Widget::Heading(s("text")),
        "text" => Widget::Text(s("text")),
        "muted" => Widget::Muted(s("text")),
        "button" => Widget::Button { id: s("id"), text: s("text") },
        "stat" => Widget::Stat { label: s("label"), value: s("value") },
        "separator" => Widget::Separator,
        "progress" => Widget::Progress(m.get("value").and_then(|v| v.as_float().ok()).unwrap_or(0.0).clamp(0.0, 1.0) as f32),
        "game" => Widget::Game(m.get("app_id").and_then(|v| v.as_int().ok()).unwrap_or(0) as u32),
        "row" => Widget::Row(m.get("items").cloned().map(widgets).unwrap_or_default()),
        _ => return None,
    })
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

    let h = host.clone();
    e.on_print(move |s| h.lock().unwrap().log(s.to_string()));
    let h = host.clone();
    e.on_debug(move |s, _, _| h.lock().unwrap().log(s.to_string()));

    let h = host.clone();
    e.register_fn("log", move |s: &str| h.lock().unwrap().log(s.to_string()));
    e.register_fn("now", || {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
    });

    let h = host.clone();
    e.register_fn("notify", move |s: &str| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Notifications)?;
        h.outbox.push(Request::Notify(s.chars().take(300).collect()));
        Ok(())
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
            .map(|(id, name, installed, minutes)| {
                map([
                    ("app_id", (*id as i64).into()),
                    ("name", name.clone().into()),
                    ("installed", (*installed).into()),
                    ("playtime_minutes", (*minutes as i64).into()),
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
        h.outbox.push(Request::SetStatus(st));
        Ok(())
    });

    let h = host.clone();
    e.register_fn("send_message", move |to: &str, text: &str| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::ChatWrite)?;
        let id: u64 = to.parse().map_err(|_| "steam_id must be a numeric string")?;
        if !h.snapshot.friends.iter().any(|f| f.0 == id) {
            return Err("can only message people on your friends list".into());
        }
        h.outbox.push(Request::SendMessage(id, text.chars().take(2000).collect()));
        Ok(())
    });

    let h = host.clone();
    e.register_fn("storage_get", move |k: &str| -> Result<Dynamic, Box<EvalAltResult>> {
        let h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        Ok(h.storage.get(k).cloned().map(Dynamic::from).unwrap_or(Dynamic::UNIT))
    });

    let h = host;
    e.register_fn("storage_set", move |k: &str, v: Dynamic| -> Result<(), Box<EvalAltResult>> {
        let mut h = h.lock().unwrap();
        h.need(Permission::Storage)?;
        if h.storage.len() >= 200 && !h.storage.contains_key(k) {
            return Err("storage is limited to 200 keys".into());
        }
        h.storage.insert(k.chars().take(100).collect(), v.to_string().chars().take(16 * 1024).collect());
        h.storage_dirty = true;
        Ok(())
    });

    e
}

/// Load every plugin folder in `dir`. Broken plugins are returned as errors
/// so the manager can show them.
pub fn discover(dir: &Path) -> Vec<Result<Plugin, (PathBuf, Error)>> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().join("plugin.toml").is_file())
        .map(|e| Plugin::load(&e.path()).map_err(|err| (e.path(), err)))
        .collect();
    out.sort_by_key(|r| match r {
        Ok(p) => p.manifest.name.to_lowercase(),
        Err((p, _)) => p.display().to_string(),
    });
    out
}

/// Example plugins bundled with the app, installed on first run.
pub const EXAMPLES: &[(&str, &str, &str)] = &[
    ("friend-alerts", include_str!("../examples/friend-alerts/plugin.toml"), include_str!("../examples/friend-alerts/main.rhai")),
    ("backlog", include_str!("../examples/backlog/plugin.toml"), include_str!("../examples/backlog/main.rhai")),
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

    fn plugin(toml: &str, script: &str) -> Plugin {
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
        p.start(Default::default());
        assert!(p.drain().is_empty());
        assert!(p.error.as_deref().unwrap_or("").contains("permission denied"));
    }

    #[test]
    fn granted_permission_works_and_page_renders() {
        let mut p = plugin(
            "id = \"t2\"\nname = \"T\"\npermissions = [\"notifications\"]\n[page]\ntitle = \"T\"\n",
            "fn on_load() { notify(\"hi\"); }\nfn page() { [#{type: \"heading\", text: \"Hello\"}, #{type: \"button\", id: \"b\", text: \"Go\"}] }",
        );
        p.start(Default::default());
        assert!(matches!(p.drain().as_slice(), [Request::Notify(s)] if s == "hi"));
        assert_eq!(p.page(), vec![Widget::Heading("Hello".into()), Widget::Button { id: "b".into(), text: "Go".into() }]);
    }

    #[test]
    fn runaway_script_is_stopped() {
        let mut p = plugin("id = \"t3\"\nname = \"T\"\n", "fn on_load() { loop { } }");
        p.start(Default::default());
        assert!(p.error.is_some());
    }

    #[test]
    fn examples_parse() {
        for (id, manifest, script) in EXAMPLES {
            let mut p = plugin(manifest, script);
            p.start(Default::default());
            assert!(p.error.is_none(), "{id}: {:?}", p.error);
        }
    }
}

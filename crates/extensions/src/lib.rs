//! # Extension API
//!
//! Every page in the app, including Home, Library and Settings, is an
//! [`Extension`]. Third-party extensions use exactly the same API, so anything
//! a built-in page can do, an extension can do too.
//!
//! An extension can:
//! - add pages to the sidebar ([`Extension::pages`] + [`Extension::page`])
//! - react to every Steam event ([`Extension::on_event`])
//! - add buttons to a game's page ([`Extension::game_actions`])
//! - add items to a friend's right-click menu ([`Extension::friend_menu`])
//! - add a section to Settings ([`Extension::settings`])
//! - persist its own data ([`Extension::save`] / [`Extension::load`])
//!
//! ```ignore
//! struct Hello;
//! impl Extension for Hello {
//!     fn manifest(&self) -> Manifest {
//!         Manifest { id: "hello", name: "Hello", ..Manifest::DEFAULT }
//!     }
//!     fn pages(&self) -> Vec<PageSpec> {
//!         vec![PageSpec::new("hello", "Hello", "👋")]
//!     }
//!     fn page(&mut self, _page: &str, ui: &mut egui::Ui, ctx: &mut Ctx) {
//!         ui.label(format!("Hi {}", ctx.state.persona_name));
//!     }
//! }
//! ```
//!
//! Register it in `apps/desktop/src/builtin/mod.rs`.

use client::{Client, Command, Event, State};
use local::LocalState;
use model::{AppId, Persona};
use ui::Theme;

#[derive(Debug, Clone, Copy)]
pub struct Manifest {
    /// Stable id, used for saved settings. Lowercase, no spaces.
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub author: &'static str,
    pub description: &'static str,
    /// Core extensions can't be disabled.
    pub core: bool,
}

impl Manifest {
    pub const DEFAULT: Manifest = Manifest {
        id: "",
        name: "",
        version: "0.1.0",
        author: "",
        description: "",
        core: false,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// A tab in the top navigation bar.
    Main,
    /// Listed in the "Apps" menu (extension and plugin pages).
    Apps,
    /// Reachable only by navigating to it (e.g. Settings, Profile).
    Hidden,
}

#[derive(Debug, Clone)]
pub struct PageSpec {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub placement: Placement,
    /// Lower comes first.
    pub order: i32,
}

impl PageSpec {
    /// A page for the Apps menu.
    pub fn new(id: impl Into<String>, title: impl Into<String>, icon: impl Into<String>) -> Self {
        Self { id: id.into(), title: title.into(), icon: icon.into(), placement: Placement::Apps, order: 100 }
    }
    pub fn order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }
    pub fn main(mut self) -> Self {
        self.placement = Placement::Main;
        self
    }
    pub fn hidden(mut self) -> Self {
        self.placement = Placement::Hidden;
        self
    }
}

/// Requests an extension makes of the app shell. Applied after the frame.
#[derive(Debug, Clone)]
pub enum Action {
    /// Go to a page, with an optional argument (e.g. an app id).
    Navigate { page: String, arg: Option<String> },
    Toast(String),
    SetTheme(Theme),
    SetZoom(f32),
    SetEnabled { id: String, enabled: bool },
    /// Open the friends & chat panel on a conversation.
    OpenChat(model::SteamId),
    /// Clear the unread count for a conversation.
    MarkRead(model::SteamId),
    /// Re-scan the local Steam library (after an uninstall, for example).
    ReloadLocal,
    ToggleOverlay,
    /// Accept a game invite: start the game with `connect` (its launch
    /// arguments, e.g. `+connect_lobby 123`), or install it first.
    JoinGame { app_id: model::AppId, connect: String },
    /// Switch the interface language (`"en"`, `"ru"`).
    SetLanguage(String),
}

/// Read-only info about an installed extension.
#[derive(Debug, Clone)]
pub struct ExtensionInfo {
    pub manifest: Manifest,
    pub enabled: bool,
}

pub trait Extension: 'static {
    fn manifest(&self) -> Manifest;

    fn pages(&self) -> Vec<PageSpec> {
        Vec::new()
    }
    /// Draw one of this extension's pages.
    fn page(&mut self, _page: &str, _ui: &mut egui::Ui, _ctx: &mut Ctx) {}
    /// Called when the user navigates to one of this extension's pages.
    fn on_navigate(&mut self, _page: &str, _arg: Option<&str>) {}

    /// Every event from the Steam engine, after `ctx.state` was updated.
    fn on_event(&mut self, _event: &Event, _ctx: &mut Ctx) {}

    /// Extra buttons on a game's detail view.
    fn game_actions(&mut self, _app_id: AppId, _ui: &mut egui::Ui, _ctx: &mut Ctx) {}
    /// Extra items in a friend's context menu.
    fn friend_menu(&mut self, _friend: &Persona, _ui: &mut egui::Ui, _ctx: &mut Ctx) {}
    /// A section on the Settings page. Return false to show nothing.
    fn settings(&mut self, _ui: &mut egui::Ui, _ctx: &mut Ctx) -> bool {
        false
    }

    /// Serialize state to keep between runs (any format, JSON suggested).
    fn save(&self) -> Option<String> {
        None
    }
    fn load(&mut self, _data: &str) {}
}

/// Everything an extension can see and do during a call.
pub struct Ctx<'a> {
    pub state: &'a State,
    pub local: &'a LocalState,
    pub theme: &'a Theme,
    pub zoom: f32,
    client: &'a Client,
    registry: &'a mut Registry,
    actions: &'a mut Vec<Action>,
}

impl<'a> Ctx<'a> {
    pub fn new(
        state: &'a State,
        local: &'a LocalState,
        theme: &'a Theme,
        zoom: f32,
        client: &'a Client,
        registry: &'a mut Registry,
        actions: &'a mut Vec<Action>,
    ) -> Self {
        Self { state, local, theme, zoom, client, registry, actions }
    }

    pub fn send(&self, cmd: Command) {
        self.client.send(cmd);
    }
    pub fn act(&mut self, a: Action) {
        self.actions.push(a);
    }
    pub fn toast(&mut self, text: impl Into<String>) {
        self.act(Action::Toast(text.into()));
    }
    pub fn navigate(&mut self, page: &str, arg: Option<String>) {
        self.act(Action::Navigate { page: page.into(), arg });
    }
    pub fn extensions(&self) -> Vec<ExtensionInfo> {
        self.registry.info()
    }

    /// Let every other enabled extension add buttons for this game.
    pub fn game_actions(&mut self, app_id: AppId, ui: &mut egui::Ui) {
        self.each(|ext, ctx| ext.game_actions(app_id, ui, ctx));
    }
    pub fn friend_menu(&mut self, friend: &Persona, ui: &mut egui::Ui) {
        self.each(|ext, ctx| ext.friend_menu(friend, ui, ctx));
    }
    /// Render every extension's settings section, each in its own card.
    pub fn settings_sections(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme.clone();
        self.each(|ext, ctx| {
            let name = ext.manifest().name;
            ui::widgets::card(&theme).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(name).strong().size(15.0));
                ui.add_space(4.0);
                if !ext.settings(ui, ctx) {
                    ui.label(egui::RichText::new("No settings").color(theme.text_faint));
                }
            });
            ui.add_space(8.0);
        });
    }

    /// Render one extension's settings section (no card).
    pub fn settings_for(&mut self, id: &str, ui: &mut egui::Ui) -> bool {
        let mut shown = false;
        self.each(|ext, ctx| {
            if ext.manifest().id == id {
                shown = ext.settings(ui, ctx);
            }
        });
        shown
    }

    /// Run `f` on each enabled extension that isn't currently busy (i.e. not
    /// the caller itself).
    fn each(&mut self, mut f: impl FnMut(&mut dyn Extension, &mut Ctx)) {
        for i in 0..self.registry.entries.len() {
            if !self.registry.entries[i].enabled {
                continue;
            }
            let Some(mut ext) = self.registry.entries[i].ext.take() else { continue };
            {
                let mut sub = Ctx {
                    state: self.state,
                    local: self.local,
                    theme: self.theme,
                    zoom: self.zoom,
                    client: self.client,
                    registry: &mut *self.registry,
                    actions: &mut *self.actions,
                };
                f(ext.as_mut(), &mut sub);
            }
            self.registry.entries[i].ext = Some(ext);
        }
    }
}

struct Entry {
    manifest: Manifest,
    enabled: bool,
    /// `None` while the extension is being called (so it can't re-enter itself).
    ext: Option<Box<dyn Extension>>,
}

#[derive(Default)]
pub struct Registry {
    entries: Vec<Entry>,
}

impl Registry {
    pub fn register(&mut self, ext: impl Extension) {
        let manifest = ext.manifest();
        assert!(
            !self.entries.iter().any(|e| e.manifest.id == manifest.id),
            "duplicate extension id {}",
            manifest.id
        );
        self.entries.push(Entry { manifest, enabled: true, ext: Some(Box::new(ext)) });
    }

    pub fn info(&self) -> Vec<ExtensionInfo> {
        self.entries
            .iter()
            .map(|e| ExtensionInfo { manifest: e.manifest, enabled: e.enabled })
            .collect()
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.manifest.id == id) {
            e.enabled = enabled || e.manifest.core;
        }
    }

    /// All pages from enabled extensions, sorted for the sidebar.
    pub fn pages(&self) -> Vec<PageSpec> {
        let mut pages: Vec<PageSpec> = self
            .entries
            .iter()
            .filter(|e| e.enabled)
            .filter_map(|e| e.ext.as_ref())
            .flat_map(|x| x.pages())
            .collect();
        pages.sort_by_key(|p| p.order);
        pages
    }

    fn owner_of(&self, page: &str) -> Option<usize> {
        self.entries.iter().position(|e| {
            e.enabled && e.ext.as_ref().is_some_and(|x| x.pages().iter().any(|p| p.id == page))
        })
    }

    /// Take one extension out, call `f` with a [`Ctx`] that can still reach
    /// all the others, then put it back.
    pub fn with<R>(
        &mut self,
        index: usize,
        env: Env<'_>,
        f: impl FnOnce(&mut dyn Extension, &mut Ctx) -> R,
    ) -> Option<R> {
        let mut ext = self.entries.get_mut(index)?.ext.take()?;
        let r = {
            let mut ctx = Ctx::new(env.state, env.local, env.theme, env.zoom, env.client, self, env.actions);
            f(ext.as_mut(), &mut ctx)
        };
        self.entries[index].ext = Some(ext);
        Some(r)
    }

    pub fn show_page(&mut self, page: &str, ui: &mut egui::Ui, env: Env<'_>) -> bool {
        match self.owner_of(page) {
            Some(i) => self.with(i, env, |ext, ctx| ext.page(page, ui, ctx)).is_some(),
            None => false,
        }
    }

    pub fn navigate(&mut self, page: &str, arg: Option<&str>) {
        if let Some(i) = self.owner_of(page) {
            if let Some(x) = self.entries[i].ext.as_mut() {
                x.on_navigate(page, arg);
            }
        }
    }

    pub fn broadcast(&mut self, event: &Event, env: Env<'_>) {
        let Env { state, local, theme, zoom, client, actions } = env;
        for i in 0..self.entries.len() {
            if self.entries[i].enabled {
                let env = Env { state, local, theme, zoom, client, actions: &mut *actions };
                self.with(i, env, |ext, ctx| ext.on_event(event, ctx));
            }
        }
    }

    pub fn friend_menu(&mut self, friend: &Persona, ui: &mut egui::Ui, env: Env<'_>) {
        let Env { state, local, theme, zoom, client, actions } = env;
        let mut ctx = Ctx::new(state, local, theme, zoom, client, self, actions);
        ctx.friend_menu(friend, ui);
    }

    pub fn save_all(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .filter_map(|e| Some((e.manifest.id.to_string(), e.ext.as_ref()?.save()?)))
            .collect()
    }

    pub fn load(&mut self, id: &str, data: &str) {
        if let Some(x) = self.entries.iter_mut().find(|e| e.manifest.id == id).and_then(|e| e.ext.as_mut()) {
            x.load(data);
        }
    }
}

/// The app data a [`Ctx`] is built from, bundled to keep call sites short.
pub struct Env<'a> {
    pub state: &'a State,
    pub local: &'a LocalState,
    pub theme: &'a Theme,
    pub zoom: f32,
    pub client: &'a Client,
    pub actions: &'a mut Vec<Action>,
}

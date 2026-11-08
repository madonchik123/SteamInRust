//! Built-in extensions. Every page in the app lives here and uses the same
//! public API as third-party extensions. To add your own, write a type that
//! implements `extensions::Extension` and register it below. For scripts that
//! users install themselves, see the `plugins` crate.

pub mod community;
mod downloads;
pub mod hub;
mod inventory;
mod library;
mod plugins;
mod support;
mod profile;
pub mod settings;
mod settings_account;
pub mod store;
mod workshop;

use extensions::Registry;

pub fn registry(rt: tokio::runtime::Handle, ctx: egui::Context, flags: settings::SharedFlags) -> Registry {
    let mut r = Registry::default();
    r.register(store::Store::new(rt.clone(), ctx.clone()));
    r.register(library::Library::new(rt.clone(), ctx.clone(), flags.clone()));
    r.register(community::Community::default());
    r.register(profile::Profile::default());
    r.register(downloads::Downloads::new(flags.clone()));
    r.register(workshop::Workshop::default());
    r.register(hub::Hub::default());
    r.register(inventory::Inventory::default());
    r.register(support::Support::default());
    r.register(plugins::Plugins::new());
    r.register(settings::Settings::new(flags));
    r
}

/// One row of the library: owned and/or installed.
pub struct Entry<'a> {
    pub app_id: model::AppId,
    pub name: &'a str,
    pub installed: Option<&'a model::InstalledGame>,
    pub owned: Option<&'a model::OwnedApp>,
}

/// Merge owned apps (from Steam) with games found on disk.
pub fn entries<'a>(state: &'a client::State, local: &'a local::LocalState) -> Vec<Entry<'a>> {
    let installed: std::collections::HashMap<_, _> = local.games.iter().map(|g| (g.app_id, g)).collect();
    let mut out: Vec<Entry> = Vec::new();
    if let Some(apps) = &state.apps {
        for a in apps.values().filter(|a| !a.is_dlc()) {
            out.push(Entry { app_id: a.app_id, name: &a.name, installed: installed.get(&a.app_id).copied(), owned: Some(a) });
        }
    }
    for g in &local.games {
        if !out.iter().any(|e| e.app_id == g.app_id) {
            out.push(Entry { app_id: g.app_id, name: &g.name, installed: Some(g), owned: None });
        }
    }
    out
}

//! Built-in extensions. Every page in the app lives here and uses the same
//! public API as third-party extensions. To add your own, write a type that
//! implements `extensions::Extension` and register it below. For scripts that
//! users install themselves, see the `plugins` crate.

pub mod community_home;
pub mod friends;
mod downloads;
mod gallery;
pub mod hub;
mod inventory;
pub mod library;
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
    r.register(community_home::CommunityHome::default());
    r.register(friends::Friends::default());
    r.register(profile::Profile::default());
    r.register(downloads::Downloads::new(flags.clone()));
    r.register(workshop::Workshop::default());
    r.register(hub::Hub::new(rt.clone(), ctx.clone()));
    r.register(inventory::Inventory::default());
    r.register(support::Support::default());
    r.register(plugins::Plugins::new(rt.clone(), ctx.clone()));
    r.register(settings::Settings::new(flags));
    // The design gallery, only when asked for (`--page=gallery`).
    if std::env::args().any(|a| a == "--page=gallery") {
        r.register(gallery::Gallery::default());
    }
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

/// A menu of a SteamID's formats (64-bit, SteamID3, STEAM_0, friend code,
/// profile link), each with its value; the clicked one is copied. Returns
/// what was copied, for a "copied" message.
pub(crate) fn steam_id_menu(ui: &mut egui::Ui, t: &ui::Theme, id: u64) -> Option<String> {
    let mut copied = None;
    for (name, value) in model::steam_id_formats(id) {
        let label = match name {
            "friend_code" => i18n::t!("Friend code").to_string(),
            "link" => i18n::t!("Profile link").to_string(),
            other => other.to_string(),
        };
        let (rect, r) = ui.allocate_exact_size(egui::vec2(280.0, 40.0), egui::Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
        }
        let p = ui.painter();
        p.text(rect.left_top() + egui::vec2(10.0, 7.0), egui::Align2::LEFT_TOP, &label, ui::widgets::font_medium(13.0), t.text);
        p.text(rect.left_bottom() + egui::vec2(10.0, -6.0), egui::Align2::LEFT_BOTTOM, &value, ui::widgets::font(11.5), t.text_faint);
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            ui.ctx().copy_text(value);
            copied = Some(label);
            ui.close();
        }
    }
    copied
}

//! Built-in extensions. Every page in the app lives here and uses the same
//! public API as third-party extensions. To add your own, write a type that
//! implements `extensions::Extension` and register it below. For scripts that
//! users install themselves, see the `plugins` crate.

pub mod community_home;
pub mod friends;
pub mod chat;
pub(crate) mod downloads;
mod gallery;
pub mod hub;
mod inventory;
pub mod library;
mod plugins;
mod support;
pub(crate) mod profile;
pub mod settings;
mod settings_account;
mod settings_family;
pub mod store;
mod workshop;

use extensions::Registry;

pub fn registry(rt: tokio::runtime::Handle, ctx: egui::Context, flags: settings::SharedFlags) -> Registry {
    let mut r = Registry::default();
    r.register(store::Store::new(rt.clone(), ctx.clone()));
    r.register(library::Library::new(rt.clone(), ctx.clone(), flags.clone()));
    r.register(community_home::CommunityHome::default());
    chat::init(rt.clone(), ctx.clone());
    r.register(friends::Friends::default());
    profile::set_runtime(rt.clone());
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
    use ui::widgets as w;
    let c = &t.components.menu_item;
    let pad = t.padding_of(c).x;
    let name_font = t.font_of(c);
    let value_font = egui::FontId::new(t.typography.body_s.size, t.typography.body_s.weight.family());
    let rows: Vec<(String, String)> = model::steam_id_formats(id)
        .into_iter()
        .map(|(name, value)| {
            let label = match name {
                "friend_code" => i18n::t!("Friend code").to_string(),
                "link" => i18n::t!("Profile link").to_string(),
                other => other.to_string(),
            };
            (label, value)
        })
        .collect();
    // As wide as the longest value (the profile link), within reason; a
    // value that still doesn't fit is cut with "…" and shown whole on hover.
    let painter = ui.painter().clone();
    let measure = |s: &str, f: &egui::FontId| painter.layout_no_wrap(s.to_string(), f.clone(), egui::Color32::PLACEHOLDER).size().x;
    let widest = rows.iter().map(|(l, v)| measure(l, &name_font).max(measure(v, &value_font))).fold(0.0, f32::max);
    let screen = ui.ctx().content_rect().width();
    let width = (widest + pad * 2.0).clamp(t.dense(240.0), t.dense(460.0).min(screen - 32.0).max(t.dense(200.0)));
    let height = t.height_of(c, 38.0) + t.typography.body_s.size + 2.0;
    let mut copied = None;
    for (label, value) in rows {
        let (rect, r) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        let look = t.look(c, w::interact(ui, &r, false));
        w::paint_box(ui, t, rect, c, &look, 0.0);
        let inner = rect.width() - pad * 2.0;
        let name = crate::builtin::friends::fit(ui, label.clone(), name_font.clone(), look.text, inner);
        let shown = crate::builtin::friends::fit(ui, value.clone(), value_font.clone(), t.text_dim, inner);
        let cut = shown.size().x + 1.0 < measure(&value, &value_font);
        let gap = 2.0;
        let top = rect.center().y - (name.size().y + gap + shown.size().y) / 2.0;
        let nh = name.size().y;
        ui.painter().galley(egui::pos2(rect.left() + pad, top), name, look.text);
        ui.painter().galley(egui::pos2(rect.left() + pad, top + nh + gap), shown, t.text_dim);
        let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
        let r = if cut { r.on_hover_text(&value) } else { r };
        if r.clicked() {
            ui.ctx().copy_text(value);
            copied = Some(label);
            ui.close();
        }
    }
    copied
}

#[cfg(test)]
mod tests {
    /// The SteamID menu is wide enough for the whole profile link (it used
    /// to run past the menu's edge).
    #[test]
    fn steam_id_menu_fits_the_profile_link() {
        let ctx = egui::Context::default();
        let t = ui::Theme::default();
        let id = 76561197960287930u64;
        let mut width = 0.0;
        let mut needed = 0.0;
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            egui::Area::new(egui::Id::unique("menu")).show(ui.ctx(), |ui| {
                super::steam_id_menu(ui, &t, id);
                width = ui.min_rect().width();
                let link = model::steam_id_formats(id).into_iter().find(|(n, _)| *n == "link").map(|(_, v)| v).unwrap_or_default();
                let font = egui::FontId::new(t.typography.body_s.size, t.typography.body_s.weight.family());
                needed = ui.painter().layout_no_wrap(link, font, egui::Color32::WHITE).size().x + 2.0 * t.padding_of(&t.components.menu_item).x;
            });
        });
        out.textures_delta.clear();
        assert!(width + 0.5 >= needed, "menu {width} wide, the link needs {needed}");
    }
}

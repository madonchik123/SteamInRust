//! Preview cards for links in chat, like Steam's: Store pages (picture,
//! name, developer, release date, price, "View Store Page"), Workshop items,
//! profiles, groups and trade links, all opening in the app; and the large
//! picture viewer.

use super::{ChatUi, Env};
use egui::{Margin, Sense, Ui, vec2};
use extensions::Action;
use i18n::t;
use model::{AppId, SteamId};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use ui::{Theme, icons, widgets as w};

/// What a Store card shows.
#[derive(Clone)]
struct StoreCard {
    name: String,
    header: String,
    developer: String,
    release: String,
    /// The price, "Free to Play", or empty.
    price: String,
}

struct Shared {
    rt: tokio::runtime::Handle,
    egui: egui::Context,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
/// Store details per game: `None` while loading.
static STORE: Mutex<Option<HashMap<AppId, Option<Result<StoreCard, String>>>>> = Mutex::new(None);

/// Give the cards a way to fetch Store details (once, when the app starts).
pub fn init(rt: tokio::runtime::Handle, egui: egui::Context) {
    let _ = SHARED.set(Shared { rt, egui });
}

/// The game a Store link is about.
pub fn store_app(url: &str) -> Option<AppId> {
    let rest = url.split_once("store.steampowered.com/app/")?.1;
    rest.split(['/', '?', '#']).next()?.parse().ok()
}

/// The account a `steamcommunity.com/profiles/<id>` link is about.
pub fn profile_id(url: &str) -> Option<SteamId> {
    let rest = url.split_once("steamcommunity.com/profiles/")?.1;
    let id: SteamId = rest.split(['/', '?', '#']).next()?.parse().ok()?;
    (id > 76561197960265728).then_some(id)
}

/// A link straight to a picture.
pub fn is_image(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp"].iter().any(|e| path.ends_with(e))
}

fn store_card(id: AppId, country: &str, language: &str) -> Option<Result<StoreCard, String>> {
    let mut guard = STORE.lock().ok()?;
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(v) = map.get(&id) {
        return v.clone();
    }
    map.insert(id, None);
    let shared = SHARED.get()?;
    let (country, language, ctx) = (country.to_lowercase(), language.to_string(), shared.egui.clone());
    shared.rt.spawn(async move {
        let api = store::StoreClient::new(if country.is_empty() { "us" } else { &country }, &language);
        let r = api
            .details(id)
            .await
            .map(|d| StoreCard {
                price: if d.is_free {
                    t!("Free to Play").to_string()
                } else {
                    d.price_text.clone().or_else(|| d.price.as_ref().and_then(|p| p.final_).map(|c| format!("{:.2}", c as f64 / 100.0))).unwrap_or_default()
                },
                name: d.name,
                header: d.header,
                developer: d.developers.first().cloned().unwrap_or_default(),
                release: d.release_date,
            })
            .map_err(|e| e.to_string());
        if let Ok(mut g) = STORE.lock() {
            g.get_or_insert_with(HashMap::new).insert(id, Some(r));
        }
        ctx.request_repaint();
    });
    None
}

type Found = Result<Vec<(AppId, String)>, String>;
/// Store searches for `/store` by what was typed: `None` while asking.
static SEARCH: Mutex<Option<HashMap<String, Option<Found>>>> = Mutex::new(None);

/// Ask the Store for games matching `q` (once per text).
pub(super) fn search(q: &str, country: &str) {
    let Ok(mut guard) = SEARCH.lock() else { return };
    let map = guard.get_or_insert_with(HashMap::new);
    if map.contains_key(q) {
        return;
    }
    map.insert(q.to_string(), None);
    let Some(shared) = SHARED.get() else { return };
    let (q, country, ctx) = (q.to_string(), country.to_lowercase(), shared.egui.clone());
    shared.rt.spawn(async move {
        let api = store::StoreClient::new(if country.is_empty() { "us" } else { &country }, i18n::language().steam_name());
        let r = api.search(&q).await.map(|items| items.into_iter().take(6).map(|i| (i.app_id, i.name)).collect()).map_err(|e| e.to_string());
        if let Ok(mut g) = SEARCH.lock() {
            g.get_or_insert_with(HashMap::new).insert(q, Some(r));
        }
        ctx.request_repaint();
    });
}

/// What the Store answered for `q`, if it was asked.
pub(super) fn search_cached(q: &str) -> Option<Option<Found>> {
    SEARCH.lock().ok()?.as_ref()?.get(q).cloned()
}

/// The card frame: a picture on the left, lines and a button on the right.
fn frame(ui: &mut Ui, t: &Theme, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let width = ui.available_width().min(t.dense(420.0));
    w::component_frame(t, &t.components.card)
        .inner_margin(Margin::same(t.spacing.sm as i8))
        .show(ui, |ui| {
            ui.set_width((width - t.spacing.lg).max(0.0));
            add(ui)
        })
        .response
}

/// A line of text that's cut with "…" when too long.
fn line(ui: &mut Ui, text: egui::RichText) {
    ui.add(egui::Label::new(text).truncate());
}

/// A preview card for a link, if it's one the app knows.
pub(super) fn card(ui: &mut Ui, t: &Theme, env: &Env, url: &str, out: &mut Vec<Action>) {
    let state = env.state;
    let pic = vec2(t.dense(150.0), t.dense(70.0));
    if let Some(app) = store_app(url) {
        let lang = i18n::language().steam_name().to_string();
        let card = store_card(app, &state.country, &lang);
        frame(ui, t, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(pic, Sense::hover());
                let name = state.app_name(app).map(String::from).or_else(|| card.as_ref().and_then(|c| c.as_ref().ok()).map(|c| c.name.clone())).unwrap_or_else(|| t!("App {id}", id = app).to_string());
                let header = card.as_ref().and_then(|c| c.as_ref().ok()).map(|c| c.header.clone()).filter(|h| !h.is_empty()).unwrap_or_else(|| model::art::header(app));
                w::image_or_title(ui, t, r, &header, &name, t.corner_small());
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    line(ui, crate::builtin::friends::strong(t, &name, t.text));
                    match &card {
                        Some(Ok(c)) => {
                            if !c.developer.is_empty() {
                                line(ui, w::body_s(t, &c.developer, t.text_dim));
                            }
                            if !c.release.is_empty() {
                                line(ui, w::body_s(t, t!("Released {date}", date = c.release), t.text_dim));
                            }
                            if !c.price.is_empty() {
                                line(ui, w::body_s(t, &c.price, t.price));
                            }
                        }
                        Some(Err(_)) => {}
                        None => {
                            ui.spinner();
                        }
                    }
                    if w::button(ui, t, Some(icons::STORE), t!("View Store Page"), w::Kind::Primary, w::Size::Sm).clicked() {
                        out.push(Action::Navigate { page: "store".into(), arg: Some(format!("app:{app}")) });
                    }
                });
            });
        });
        return;
    }
    if let Some((partner, _)) = client::trade::parse_link(url) {
        let p = state.persona(partner);
        let name = if p.name.is_empty() { t!("someone").to_string() } else { p.display_name() };
        frame(ui, t, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icons::SWAP).font(w::font(t.dense(30.0))).color(t.accent));
                ui.vertical(|ui| {
                    line(ui, crate::builtin::friends::strong(t, t!("Trade offer link"), t.text));
                    line(ui, w::body_s(t, t!("Make a trade offer to {name}", name = name), t.text_dim));
                    if w::button(ui, t, Some(icons::SWAP), t!("Make an offer"), w::Kind::Primary, w::Size::Sm).clicked() {
                        out.push(Action::Navigate { page: "inventory".into(), arg: Some(format!("trade:{partner}")) });
                    }
                });
            });
        });
        return;
    }
    if let Some(id) = profile_id(url) {
        let p = state.persona(id);
        if p.name.is_empty() {
            return;
        }
        frame(ui, t, |ui| {
            ui.horizontal(|ui| {
                let s = t.dense(54.0);
                w::presence_avatar(ui, t, &model::art::avatar(p.avatar_hash.as_deref()), s, Some(crate::shell::status_color(t, &p)));
                ui.vertical(|ui| {
                    line(ui, crate::builtin::friends::strong(t, p.display_name(), t.text));
                    line(ui, w::body_s(t, crate::shell::status_text(state, &p), crate::shell::status_color(t, &p)));
                    if w::button(ui, t, Some(icons::USER_CIRCLE), t!("View profile"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        out.push(Action::Navigate { page: "profile".into(), arg: Some(id.to_string()) });
                    }
                });
            });
        });
        return;
    }
    let Some(view) = crate::builtin::hub::route(url) else { return };
    use crate::builtin::hub::View;
    match &view {
        View::Item(file_id) => {
            let lang = i18n::language().steam_name().to_string();
            let item = state.hub.items.get(file_id);
            if item.is_none() {
                let asked = egui::Id::unique(("chat-item-asked", *file_id));
                if !ui.data(|d| d.get_temp::<bool>(asked).unwrap_or(false)) {
                    ui.data_mut(|d| d.insert_temp(asked, true));
                    env.client.send(client::Command::Hub(client::hub::Request::Item { file_id: *file_id, language: lang }));
                }
            }
            let Some(Ok(item)) = item else { return };
            frame(ui, t, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(pic, Sense::hover());
                    w::image_or_title(ui, t, r, item.entry.preview.as_deref().unwrap_or_default(), &item.entry.title, t.corner_small());
                    ui.vertical(|ui| {
                        line(ui, crate::builtin::friends::strong(t, &item.entry.title, t.text));
                        line(ui, w::body_s(t, t!("Workshop item for {game}", game = item.app_name), t.text_dim));
                        if w::button(ui, t, Some(icons::CUBE), t!("View item"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            let (page, arg) = view.to_page();
                            out.push(Action::Navigate { page, arg });
                        }
                    });
                });
            });
        }
        View::Group(key) => {
            let Some(Ok(g)) = state.hub.groups.get(key) else {
                let asked = egui::Id::unique(("chat-group-asked", key.clone()));
                if !ui.data(|d| d.get_temp::<bool>(asked).unwrap_or(false)) {
                    ui.data_mut(|d| d.insert_temp(asked, true));
                    env.client.send(client::Command::Hub(client::hub::Request::Group { key: key.clone() }));
                }
                return;
            };
            frame(ui, t, |ui| {
                ui.horizontal(|ui| {
                    let s = t.dense(54.0);
                    let (r, _) = ui.allocate_exact_size(vec2(s, s), Sense::hover());
                    w::image_or_title(ui, t, r, &g.avatar, &g.name, t.corner_small());
                    ui.vertical(|ui| {
                        line(ui, crate::builtin::friends::strong(t, &g.name, t.text));
                        line(ui, w::body_s(t, t!("Steam group"), t.text_dim));
                        if w::button(ui, t, Some(icons::USERS_THREE), t!("View group"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            let (page, arg) = view.to_page();
                            out.push(Action::Navigate { page, arg });
                        }
                    });
                });
            });
        }
        _ => {}
    }
}

/// What a link is, for a menu row: an icon and a name ("Portal 2").
pub(crate) fn link_label(env: &Env, url: &str) -> (&'static str, String) {
    let state = env.state;
    if let Some(app) = store_app(url) {
        let lang = i18n::language().steam_name().to_string();
        let card = store_card(app, &state.country, &lang);
        let name = state.app_name(app).map(String::from).or_else(|| card.and_then(|c| c.ok()).map(|c| c.name)).unwrap_or_else(|| t!("App {id}", id = app).to_string());
        return (icons::STORE, name);
    }
    if client::trade::parse_link(url).is_some() {
        return (icons::SWAP, t!("Trade offer link").to_string());
    }
    if let Some(id) = profile_id(url) {
        let p = state.persona(id);
        return (icons::USER_CIRCLE, if p.name.is_empty() { t!("View profile").to_string() } else { p.display_name() });
    }
    use crate::builtin::hub::View;
    match crate::builtin::hub::route(url) {
        Some(View::Item(id)) => match state.hub.items.get(&id) {
            Some(Ok(item)) => (icons::CUBE, item.entry.title.clone()),
            _ => (icons::CUBE, t!("View item").to_string()),
        },
        Some(View::Group(key)) => match state.hub.groups.get(&key) {
            Some(Ok(g)) => (icons::USERS_THREE, g.name.clone()),
            _ => (icons::USERS_THREE, t!("View group").to_string()),
        },
        Some(_) => (icons::USERS_THREE, url.split("//").nth(1).unwrap_or(url).trim_end_matches('/').to_string()),
        None => (icons::EXTERNAL, url.split("//").nth(1).unwrap_or(url).trim_end_matches('/').to_string()),
    }
}

/// The picture viewer: the picture as large as the window allows.
pub(super) fn viewer(ui: &mut Ui, t: &Theme, mem: &mut ChatUi) {
    let Some(src) = mem.viewing.clone() else { return };
    let screen = ui.ctx().content_rect();
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("chat-image-viewer")).show(ui.ctx(), |ui| {
        let max = vec2(screen.width() - 96.0, screen.height() - 140.0).max(vec2(200.0, 150.0));
        let sized = crate::sized_images::inline_url(&src, max.x, max.y, ui.ctx().pixels_per_point());
        let r = ui.add(egui::Image::new(sized).max_width(max.x).max_height(max.y).fit_to_original_size(1.0));
        if r.rect.height() < 4.0 {
            ui.spinner();
        }
        ui.add_space(t.spacing.sm);
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::EXTERNAL), t!("Open in browser"), w::Kind::Secondary, w::Size::Sm).clicked() {
                crate::links::open_http(&src);
            }
            if w::button(ui, t, Some(icons::COPY), t!("Copy link"), w::Kind::Ghost, w::Size::Sm).clicked() {
                ui.ctx().copy_text(src.clone());
            }
            if w::button(ui, t, Some(icons::X), t!("Close"), w::Kind::Ghost, w::Size::Sm).clicked() {
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        mem.viewing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_kinds() {
        assert_eq!(store_app("https://store.steampowered.com/app/620/Portal_2/"), Some(620));
        assert_eq!(store_app("https://store.steampowered.com/app/1145360?snr=1"), Some(1145360));
        assert_eq!(store_app("https://store.steampowered.com/sub/1/"), None);
        assert_eq!(profile_id("https://steamcommunity.com/profiles/76561197960287930/"), Some(76561197960287930));
        assert_eq!(profile_id("https://steamcommunity.com/profiles/12"), None);
        assert!(is_image("https://i.example/a/b.PNG?x=1"));
        assert!(!is_image("https://store.steampowered.com/app/620/"));
    }
}

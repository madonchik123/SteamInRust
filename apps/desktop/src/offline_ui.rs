//! Offline mode in the UI, like Steam's: the "Connection error" window at
//! sign-in, the Offline / Reconnecting badge in the top bar, "Go online" in
//! the account menu, and a clean "You're offline" state for pages that need
//! Steam. The engine side lives in `client` (session.rs, offline.rs).

use crate::app::SteamApp;
use client::{Command, Event, Link};
use egui::{Align2, Color32, CornerRadius, Margin, Sense, Stroke, Ui, vec2};
use i18n::t;
use ui::{Theme, icons, widgets as w};

/// A saved login couldn't reach Steam.
#[derive(Clone, Debug)]
pub struct ConnectionError {
    pub account: String,
    pub message: String,
    pub offline_ready: bool,
}

/// Pages that only work with a connection to Steam.
const NEEDS_STEAM: &[&str] = &["store", "community", "profile", "inventory", "workshop", "support"];

fn needs_steam(page: &str) -> bool {
    NEEDS_STEAM.contains(&page) || crate::builtin::hub::PAGES.contains(&page)
}

/// Called for every engine event, before the state snapshot changes.
pub fn on_event(app: &mut SteamApp, e: &Event, now: f64) {
    match e {
        Event::ConnectionError { account, message, offline_ready } => {
            app.login.signing_in = None;
            if *offline_ready && app.prefs.auto_offline {
                tracing::info!("Steam unreachable, starting {account} offline as chosen before");
                app.login.signing_in = Some(account.clone());
                app.client.send(Command::StartOffline { account: account.clone() });
            } else {
                app.login.connection_error = Some(ConnectionError { account: account.clone(), message: message.clone(), offline_ready: *offline_ready });
            }
        }
        Event::LoggedIn { .. } => app.login.connection_error = None,
        Event::LoggedOut { .. } => app.prefs.wants_offline = false,
        Event::Link(link) => {
            let before = app.state.link;
            match link {
                // Offline mode sticks until "Go online", like Steam's.
                Link::Offline => {
                    if !app.demo {
                        app.prefs.wants_offline = true;
                    }
                }
                Link::Online => {
                    app.prefs.wants_offline = false;
                    if before != Link::Online && app.state.phase == client::state::Phase::LoggedIn {
                        app.toast(now, t!("Back online"), t!("Connected to Steam again."));
                    }
                }
                Link::Reconnecting { .. } if before == Link::Online => {
                    app.toast(now, t!("Connection to Steam lost"), t!("Reconnecting… Your games keep running."));
                }
                Link::Reconnecting { .. } => {}
            }
        }
        Event::CloudSkipped { app_id } => {
            let name = app.state.app_name(*app_id).unwrap_or_default().to_string();
            app.toast(now, name, t!("Offline: Steam Cloud wasn't checked. Your saves upload when you're back online."));
        }
        _ => {}
    }
}

/// Steam's "Connection error" window, in the sign-in window.
pub fn connection_error(app: &mut SteamApp, ui: &mut Ui, t: &Theme, err: &ConnectionError) {
    let who = app.saved.iter().find(|s| s.account_name == err.account).cloned();
    ui.vertical_centered(|ui| {
        ui.add_space(6.0);
        ui.label(w::text(icons::PLUGS, 40.0, t.away));
        ui.add_space(4.0);
        ui.label(w::text_bold(t!("Connection error"), 22.0, t.text));
        ui.add_space(2.0);
        ui.label(w::text(t!("SteamInRust can't reach the Steam servers right now."), 13.5, t.text_dim));
        ui.add(egui::Label::new(w::text(i18n::tr_dyn(&err.message), 12.0, t.text_faint)).wrap());
        ui.add_space(14.0);
    });
    let mut retry = false;
    let mut offline = false;
    let mut cancel = false;
    ui.vertical_centered(|ui| {
        if let Some(acc) = &who {
            let name = if acc.persona_name.is_empty() { &acc.account_name } else { &acc.persona_name };
            ui.horizontal(|ui| {
                let row_w = 44.0 + 10.0 + ui.painter().layout_no_wrap(name.clone(), w::font_semibold(14.0), t.text).size().x;
                ui.add_space(((ui.available_width() - row_w) / 2.0).max(0.0));
                w::avatar(ui, t, &model::art::avatar(acc.avatar_hash.as_deref()), 44.0, None);
                ui.label(w::text_semibold(name, 14.0, t.text));
            });
            ui.add_space(12.0);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            // Center the row: each button is its label plus icon and padding.
            let label_w = |s: &str| ui.painter().layout_no_wrap(s.to_string(), w::font_medium(14.0), t.text).size().x + 56.0;
            let width = label_w(&t!("Retry connection")) + label_w(&t!("Start in offline mode")) + label_w(&t!("Cancel")) + 20.0;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            if w::button(ui, t, Some(icons::REFRESH), t!("Retry connection"), w::Kind::Secondary, w::Size::Md).clicked() {
                retry = true;
            }
            let r = ui.add_enabled_ui(err.offline_ready, |ui| w::button(ui, t, Some(icons::PLUGS), t!("Start in offline mode"), w::Kind::Primary, w::Size::Md)).inner;
            let r = if err.offline_ready { r } else { r.on_disabled_hover_text(t!("Sign in online once on this PC first, so your library can be kept for offline use.")) };
            if r.clicked() {
                offline = true;
            }
            if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                cancel = true;
            }
        });
        ui.add_space(14.0);
        if err.offline_ready {
            ui.horizontal(|ui| {
                let label = t!("Next time, start in offline mode by itself when Steam can't be reached");
                let row_w = 40.0 + 10.0 + ui.painter().layout_no_wrap(label.to_string(), w::font(12.5), t.text).size().x;
                ui.add_space(((ui.available_width() - row_w) / 2.0).max(0.0));
                ui.add(crate::builtin::settings::toggle(&mut app.prefs.auto_offline));
                ui.label(w::text(label, 12.5, t.text_dim));
            });
        } else {
            ui.label(w::text(t!("Offline mode needs one online sign-in on this PC first."), 12.5, t.text_faint));
        }
    });
    if retry || offline {
        app.login.connection_error = None;
        app.login.signing_in = Some(err.account.clone());
        app.client.send(if offline { Command::StartOffline { account: err.account.clone() } } else { Command::LoginSaved { account: err.account.clone() } });
    }
    if cancel {
        app.login.connection_error = None;
        app.state.last_error = None;
    }
}

/// "Offline" / "Reconnecting…" in the top bar (nothing while online).
/// Clicking it tries to connect now.
pub fn badge(app: &mut SteamApp, ui: &mut Ui, t: &Theme) {
    let (label, tip, color) = match app.state.link {
        Link::Online => return,
        Link::Offline => (format!("{}  {}", icons::PLUGS, t!("Offline")), t!("Offline mode. Click to go online.").to_string(), t.away),
        Link::Reconnecting { retry_in: 0, .. } => (format!("{}  {}", icons::REFRESH, t!("Reconnecting…")), t!("Connecting to Steam…").to_string(), t.text_dim),
        Link::Reconnecting { retry_in, .. } => (format!("{}  {}", icons::REFRESH, t!("Reconnecting…")), t!("Next try in {n} s. Click to try now.", n = retry_in), t.text_dim),
    };
    let galley = ui.painter().layout_no_wrap(label, w::font_semibold(12.5), color);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, 28.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter().rect(rect, CornerRadius::same(14), w::mix(color.gamma_multiply(0.12), color.gamma_multiply(0.22), hover), Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        app.client.send(Command::GoOnline);
    }
    if matches!(app.state.link, Link::Reconnecting { .. }) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// "Go online" / "Go offline" at the top of the account menu.
pub fn account_items(app: &mut SteamApp, ui: &mut Ui, t: &Theme) {
    let item = |ui: &mut Ui, icon: &str, label: String, color: Color32| ui.add(egui::Button::new(w::text_medium(format!("{icon}   {label}"), 13.5, color)).frame(false).min_size(vec2(210.0, 32.0))).clicked();
    match app.state.link {
        Link::Online => {
            if item(ui, icons::PLUGS, t!("Go offline").to_string(), t.text) {
                app.client.send(Command::GoOffline);
            }
        }
        Link::Offline => {
            ui.label(w::text(format!("{}  {}", icons::PLUGS, t!("You're in offline mode")), 12.5, t.away));
            if item(ui, icons::GLOBE, t!("Go online").to_string(), t.accent) {
                app.client.send(Command::GoOnline);
            }
        }
        Link::Reconnecting { .. } => {
            ui.label(w::text(format!("{}  {}", icons::REFRESH, t!("Reconnecting…")), 12.5, t.text_dim));
            if item(ui, icons::REFRESH, t!("Try again now").to_string(), t.text) {
                app.client.send(Command::GoOnline);
            }
            if item(ui, icons::PLUGS, t!("Go offline").to_string(), t.text) {
                app.client.send(Command::GoOffline);
            }
        }
    }
    ui.separator();
}

/// A page that needs Steam, while offline: "You're offline" instead.
/// Returns true when it drew the page.
pub fn page(app: &mut SteamApp, ui: &mut Ui, page: &str) -> bool {
    if app.state.online() || !needs_steam(page) {
        return false;
    }
    let t = app.prefs.theme.clone();
    let reconnecting = matches!(app.state.link, Link::Reconnecting { .. });
    ui.vertical_centered(|ui| {
        ui.add_space(110.0);
        ui.label(w::text(icons::PLUGS, 52.0, t.text_faint));
        ui.add_space(10.0);
        ui.label(w::text_bold(if reconnecting { t!("Reconnecting to Steam…") } else { t!("You're offline") }, 22.0, t.text));
        ui.add_space(4.0);
        ui.label(w::text(t!("This page needs a connection to Steam."), 14.0, t.text_dim));
        ui.label(w::text(t!("Your library and installed games still work."), 14.0, t.text_dim));
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_space(((ui.available_width() - 340.0) / 2.0).max(0.0));
            let label = if reconnecting { t!("Try again now") } else { t!("Go online") };
            if w::button(ui, &t, Some(icons::GLOBE), label, w::Kind::Primary, w::Size::Md).clicked() {
                app.client.send(Command::GoOnline);
            }
            if w::button(ui, &t, Some(icons::BOOKS), t!("Library"), w::Kind::Secondary, w::Size::Md).clicked() {
                app.navigate("library".into(), None);
            }
        });
    });
    true
}

/// A thin notice strip (friends list, downloads).
fn notice(ui: &mut Ui, t: &Theme, text: String) {
    egui::Frame::new().fill(t.away.gamma_multiply(0.12)).corner_radius(t.corner_small()).inner_margin(Margin::symmetric(10, 7)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(w::text(format!("{}  {text}", icons::PLUGS), 12.5, t.away)).wrap());
    });
    ui.add_space(8.0);
}

/// Friends while offline: their status is unknown.
pub fn friends_notice(app: &SteamApp, ui: &mut Ui, t: &Theme) {
    match app.state.link {
        Link::Online => {}
        Link::Offline => notice(ui, t, t!("Offline: friends' status is unknown and chat is unavailable.").to_string()),
        Link::Reconnecting { .. } => notice(ui, t, t!("Reconnecting… Friends' status may be out of date.").to_string()),
    }
}

/// Downloads wait for the connection.
pub fn downloads_notice(app: &SteamApp, ui: &mut Ui, t: &Theme) {
    if !app.state.online() {
        egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: 16, bottom: 0 }).show(ui, |ui| {
            notice(ui, t, t!("Downloads are paused while you're offline. They continue once you're back online.").to_string());
        });
    }
}

/// In the status bar, next to "Downloads".
pub fn status_note(app: &SteamApp, ui: &mut Ui, t: &Theme) {
    if !app.state.online() {
        ui.label(w::text(t!("· paused while offline"), 12.5, t.away));
    }
}

/// Play time on a game's page, with time played offline in SteamInRust
/// that Steam doesn't have yet.
pub fn playtime_stat(ui: &mut Ui, t: &Theme, state: &client::State, app_id: model::AppId, steam_minutes: Option<u64>) {
    let (offline_total, not_sent) = state.offline_playtime.get(&app_id).copied().unwrap_or((0, 0));
    let extra = not_sent / 60;
    let value = match (steam_minutes, extra) {
        (None, 0) => "—".to_string(),
        (m, extra) => ui::playtime(m.unwrap_or(0) + extra),
    };
    let r = ui.scope(|ui| w::stat(ui, t, t!("Play time"), &value)).response;
    if offline_total >= 60 {
        let tip = if not_sent >= 60 {
            t!("Includes {time} played offline that Steam doesn't have yet. It's sent to your Steam profile when you're back online.", time = ui::playtime(not_sent / 60))
        } else {
            t!("{time} of it was played offline and has been sent to Steam.", time = ui::playtime(offline_total / 60))
        };
        r.on_hover_text(tip);
    }
}

/// Draw a small "cloud upload waiting" mark on a game page's play bar.
pub fn cloud_pending(ui: &mut Ui, t: &Theme, state: &client::State, app_id: model::AppId) {
    if !state.cloud_pending.contains(&app_id) {
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icons::CLOUD_CHECK, w::font(16.0), t.away);
    resp.on_hover_text(t!("Saves changed offline. They upload to Steam Cloud when you're back online."));
}

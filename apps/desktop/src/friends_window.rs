//! The undocked "Friends & Chat" window, like Steam's. It keeps working while
//! the main window is hidden in the tray.

use crate::app::SteamApp;
use egui::{Align, Layout, Margin, Ui};
use extensions::Action;
use i18n::t;
use model::{PersonaState, SteamId, art};
use ui::{icons, widgets as w};

#[derive(Default)]
pub struct FriendsWindow {
    pub open: bool,
    pub chat: Option<SteamId>,
    input: String,
}

pub fn show(app: &mut SteamApp, ctx: &egui::Context, actions: &mut Vec<Action>) {
    if !app.friends_window.open {
        return;
    }
    let width = if app.friends_window.chat.is_some() { 820.0 } else { 340.0 };
    let builder = egui::ViewportBuilder::default()
        .with_title(t!("Friends & Chat"))
        .with_inner_size([width, 720.0])
        .with_min_inner_size([300.0, 420.0]);
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("friends-window"), builder, |ui, _| {
        if ui.input(|i| i.viewport().close_requested()) {
            app.friends_window.open = false;
        }
        let t = app.prefs.theme.clone();
        if app.friends_window.chat.is_some() {
            egui::Panel::right("fw-chat")
                .exact_size(480.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(t.bg).inner_margin(Margin::same(16)))
                .show(ui, |ui| chat(app, ui, actions));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin { left: 14, right: 10, top: 16, bottom: 8 }))
            .show(ui, |ui| list(app, ui, actions));
    });
}

fn list(app: &mut SteamApp, ui: &mut Ui, actions: &mut Vec<Action>) {
    let t = app.prefs.theme.clone();
    let me = app.state.me().cloned().unwrap_or_default();
    ui.horizontal(|ui| {
        w::avatar(ui, &t, &art::avatar(me.avatar_hash.as_deref()), 40.0, Some(crate::shell::status_color(&t, &me)));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(2.0);
            ui.label(w::text_semibold(&app.state.persona_name, 15.0, t.text));
            egui::ComboBox::from_id_salt("fw-status")
                .selected_text(w::text(i18n::tr_dyn(me.state.label()), 12.5, crate::shell::status_color(&t, &me)))
                .width(140.0)
                .show_ui(ui, |ui| {
                    for s in PersonaState::SELECTABLE {
                        if ui.selectable_label(me.state == s, i18n::tr_dyn(s.label())).clicked() {
                            app.client.send(client::Command::SetPersonaState(s));
                        }
                    }
                });
        });
    });
    ui.add_space(10.0);
    let width = ui.available_width() - 4.0;
    w::search(ui, &t, &mut app.friend_filter, t!("Search friends"), width);
    ui.add_space(8.0);
    let friends = app.state.friend_list();
    let selected = app.friends_window.chat;
    // The shared list pushes OpenChat when nothing is selected; capture clicks here instead.
    let mut local_actions = Vec::new();
    if let Some(id) = crate::shell::friend_list(app, ui, &t, &mut local_actions, &friends, Some(selected.unwrap_or(0))) {
        app.friends_window.chat = Some(id);
        app.client.send(client::Command::LoadHistory { friend: id });
        actions.push(Action::MarkRead(id));
    }
    actions.extend(local_actions.into_iter().filter(|a| !matches!(a, Action::OpenChat(_))));
}

fn chat(app: &mut SteamApp, ui: &mut Ui, actions: &mut Vec<Action>) {
    let t = app.prefs.theme.clone();
    let Some(id) = app.friends_window.chat else { return };
    let p = app.state.persona(id);
    if app.state.unread.contains_key(&id) {
        actions.push(Action::MarkRead(id));
    }
    ui.horizontal(|ui| {
        w::avatar(ui, &t, &art::avatar(p.avatar_hash.as_deref()), 36.0, Some(crate::shell::status_color(&t, &p)));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(w::text_semibold(&p.name, 15.0, t.text));
            ui.label(w::text(crate::shell::status_text(&app.state, &p), 12.5, crate::shell::status_color(&t, &p)));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::icon_button(ui, &t, icons::X, t!("Close"), false, 0).clicked() {
                app.friends_window.chat = None;
            }
        });
    });
    ui.separator();
    let h = ui.available_height();
    crate::builtin::community::chat_body(ui, &t, &app.state, &app.client, id, &mut app.friends_window.input, h);
}

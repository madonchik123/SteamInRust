//! Community: Steam group chats, beside the friends list. A group's text
//! channels, its messages (paging back), sending, members, unread marks and
//! game invites, like Steam's Friends & Chat. (No voice.)

use super::{Community, message};
use client::{Command, State};
use egui::{Align, Key, Layout, Margin, Sense, Stroke, Ui, vec2};
use extensions::{Action, Ctx};
use i18n::t;
use model::{ChatGroup, PersonaState, SteamId, art};
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

/// A group chat's name: its own, or (for an unnamed group of friends) its
/// members' names, like Steam shows it.
pub(super) fn group_name(state: &State, g: &ChatGroup) -> String {
    if !g.name.trim().is_empty() {
        return g.name.trim().to_string();
    }
    let names: Vec<String> = state
        .chat_group_members
        .get(&g.id)
        .map(|m| m.iter().filter(|id| **id != state.steam_id).map(|id| state.persona(*id).name).filter(|n| !n.is_empty()).take(4).collect())
        .unwrap_or_default();
    if names.is_empty() { t!("Group chat").to_string() } else { names.join(", ") }
}

fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
}

/// The group's picture, or its initials on a colored tile.
fn group_avatar(ui: &mut Ui, t: &Theme, g: &ChatGroup, name: &str, size: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if !g.avatar.is_empty() && w::paint_cover(ui, r, &g.avatar, t.corner_small()) {
        return;
    }
    let hue = (g.id % 360) as f32 / 360.0;
    let fill = egui::ecolor::Hsva::new(hue, 0.45, 0.45, 1.0);
    ui.painter().rect_filled(r, t.corner_small(), fill);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initials(name), w::font_bold(size * 0.38), egui::Color32::WHITE);
}

impl Community {
    /// The sidebar's group chats; returns the one clicked (its default room).
    pub(super) fn group_list(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) -> Option<(u64, u64)> {
        if !self.groups_asked {
            self.groups_asked = true;
            ctx.send(Command::LoadChatGroups);
        }
        let groups = match &ctx.state.chat_groups {
            None => {
                for _ in 0..4 {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
                    w::skeleton(ui, t, r.shrink(4.0), t.corner_small());
                }
                return None;
            }
            Some(Err(e)) => {
                ui.add(egui::Label::new(w::text(t!("Couldn't load your group chats: {error}", error = i18n::tr_dyn(e)), 12.5, t.text_faint)).wrap());
                return None;
            }
            Some(Ok(g)) => g.clone(),
        };
        let f = self.filter.to_lowercase();
        let mut clicked = None;
        egui::ScrollArea::vertical().id_salt("community-groups").auto_shrink([false, false]).show(ui, |ui| {
            if groups.is_empty() {
                ui.add(egui::Label::new(w::text(t!("You're not in any group chats. Steam's group chats show up here."), 13.0, t.text_faint)).wrap());
            }
            for g in &groups {
                let name = group_name(ctx.state, g);
                if !f.is_empty() && !name.to_lowercase().contains(&f) {
                    continue;
                }
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
                let on = self.group.is_some_and(|(id, _)| id == g.id);
                let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                if on {
                    ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
                } else if hover > 0.0 {
                    ui.painter().rect_filled(rect, t.corner_small(), w::mix(egui::Color32::TRANSPARENT, t.surface, hover));
                }
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 6.0))).layout(Layout::left_to_right(Align::Center)));
                group_avatar(&mut c, t, g, &name, 36.0);
                c.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add_space(2.0);
                    ui.set_max_width((ui.available_width() - 24.0).max(40.0));
                    ui.add(egui::Label::new(w::text_medium(&name, 14.0, t.text)).truncate());
                    let last = g.rooms.iter().max_by_key(|r| r.last_time);
                    let preview = last
                        .filter(|r| !r.last_message.is_empty())
                        .map(|r| {
                            let who = ctx.state.persona(r.last_sender).name;
                            let (text, _) = model::chat::parse(&r.last_message);
                            if who.is_empty() { text } else { format!("{who}: {text}") }
                        })
                        .unwrap_or_else(|| if g.active_members == 1 { t!("1 member online").to_string() } else { t!("{n} members online", n = g.active_members) });
                    ui.add(egui::Label::new(w::text(preview, 12.0, t.text_dim)).truncate());
                });
                if g.unread {
                    ui.painter().circle_filled(rect.right_center() - vec2(14.0, 0.0), 5.0, t.accent);
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    let room = if g.default_room != 0 { g.default_room } else { g.rooms.first().map(|r| r.id).unwrap_or(0) };
                    clicked = Some((g.id, room));
                }
            }
        });
        clicked
    }

    /// A group chat: its channels, messages and members.
    pub(super) fn group_conversation(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, group: u64, room: u64) {
        let Some(g) = ctx.state.chat_groups.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|g| g.id == group)).cloned() else {
            w::loading(ui, t, t!("Loading…"));
            return;
        };
        let name = group_name(ctx.state, &g);
        let key = (group, room);
        if self.history_asked.insert(key) {
            ctx.send(Command::LoadGroupHistory { group, room, before: None });
        }
        // Read: tell Steam once per newest message seen.
        let msgs = ctx.state.group_messages.get(&key).cloned().unwrap_or_default();
        if let Some(last) = msgs.last().map(|m| m.time) {
            if self.acked.get(&key) != Some(&last) {
                self.acked.insert(key, last);
                ctx.send(Command::AckGroupRoom { group, room, time: last });
            }
        }

        // Header: picture, name, tagline, members; channel tabs.
        egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(24, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                group_avatar(ui, t, &g, &name, 44.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add_space(3.0);
                    ui.add(egui::Label::new(w::text_bold(&name, 18.0, t.text)).truncate());
                    let members = ctx.state.chat_group_members.get(&group).map(|m| m.len()).unwrap_or(0);
                    let mut sub = t!("{n} members", n = members).to_string();
                    if !g.tagline.is_empty() {
                        sub = format!("{sub} · {}", g.tagline);
                    }
                    ui.add(egui::Label::new(w::text(sub, 12.5, t.text_dim)).truncate());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::X, t!("Close conversation"), false, 0).clicked() {
                        self.group = None;
                    }
                    if g.clan_id != 0 && w::icon_button(ui, t, icons::USERS_THREE, t!("Steam group page"), false, 0).clicked() {
                        ctx.navigate("group", Some((103582791429521408u64 + g.clan_id as u64).to_string()));
                    }
                });
            });
            if g.rooms.len() > 1 {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for r in &g.rooms {
                        let label = format!("# {}", if r.name.is_empty() { t!("general").to_string() } else { r.name.clone() });
                        let resp = w::chip(ui, t, &label, r.id == room);
                        if r.unread && r.id != room {
                            ui.painter().circle_filled(resp.rect.right_top() + vec2(-4.0, 4.0), 4.0, t.accent);
                        }
                        if resp.clicked() {
                            self.group = Some((group, r.id));
                        }
                    }
                });
            }
        });
        let rect = ui.max_rect();
        ui.painter().hline(rect.x_range(), ui.cursor().top(), Stroke::new(1.0, t.border));

        // Messages on the left, members on the right.
        let members_w = if ui.available_width() > 760.0 { 220.0 } else { 0.0 };
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            let chat_w = ui.available_width() - members_w;
            ui.allocate_ui_with_layout(vec2(chat_w, height), Layout::top_down(Align::Min), |ui| {
                egui::Frame::new().inner_margin(Margin::symmetric(24, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let h = ui.available_height();
                    if let Some(a) = self.group_body(ui, t, ctx, &g, room, &msgs, h) {
                        ctx.act(a);
                    }
                });
            });
            if members_w > 0.0 {
                ui.allocate_ui_with_layout(vec2(members_w, height), Layout::top_down(Align::Min), |ui| {
                    egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(12, 12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(height - 24.0);
                        member_list(ui, t, ctx, group);
                    });
                });
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn group_body(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, g: &ChatGroup, room: u64, msgs: &[model::GroupMessage], height: f32) -> Option<Action> {
        let key = (g.id, room);
        let state = ctx.state;
        let mut join = None;
        let composer_id = ui.scope_id().with(("group-composer", key));
        let composer = ui.data(|d| d.get_temp::<f32>(composer_id)).unwrap_or(46.0);
        let list_h = (height - composer - 8.0 - ui.spacing().item_spacing.y).floor().max(40.0);
        let mut older = None;
        egui::ScrollArea::vertical()
            .id_salt(("group-chat", key))
            .max_height(list_h)
            .min_scrolled_height(list_h)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if state.group_more.get(&key) == Some(&true) {
                    if let Some(first) = msgs.first() {
                        ui.vertical_centered(|ui| {
                            if w::button(ui, t, Some(icons::CARET_UP), t!("Older messages"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                older = Some((first.time, first.ordinal));
                            }
                        });
                    }
                }
                if msgs.is_empty() {
                    if self.history_asked.contains(&key) && state.group_more.contains_key(&key) {
                        w::empty(ui, t, icons::CHAT, t!("No messages yet"), t!("Say hi!"));
                    } else {
                        w::loading(ui, t, t!("Loading…"));
                    }
                }
                let mut last: Option<(SteamId, u32)> = None;
                for m in msgs {
                    let (text, invite) = model::chat::parse(&m.text);
                    if text.is_empty() && invite.is_none() {
                        continue;
                    }
                    let shown = model::ChatMessage { friend: 0, from: m.sender, text, time: m.time as u64, invite };
                    let new_block = last.is_none_or(|(from, time)| from != m.sender || m.time.saturating_sub(time) > 300);
                    if new_block {
                        ui.add_space(10.0);
                        let p = state.persona(m.sender);
                        ui.horizontal_top(|ui| {
                            w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 34.0, None);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.horizontal(|ui| {
                                    let me = m.sender == state.steam_id;
                                    let name = if p.name.is_empty() { m.sender.to_string() } else { p.name.clone() };
                                    ui.label(w::text_semibold(name, 14.0, if me { t.accent } else { t.text }));
                                    ui.label(w::text(ui::ago(m.time as u64), 11.5, t.text_faint));
                                });
                                message(ui, t, state, &shown, &mut join);
                            });
                        });
                    } else {
                        ui.horizontal_top(|ui| {
                            ui.add_space(42.0);
                            ui.vertical(|ui| message(ui, t, state, &shown, &mut join));
                        });
                    }
                    last = Some((m.sender, m.time));
                }
            });
        if let Some(before) = older {
            ctx.send(Command::LoadGroupHistory { group: g.id, room, before: Some(before) });
        }
        ui.add_space(8.0);
        let room_name = g.rooms.iter().find(|r| r.id == room).map(|r| r.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| group_name(state, g));
        let r = egui::Frame::new()
            .fill(t.raised)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(t.corner())
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let r = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.group_input)
                                .hint_text(w::text(t!("Message {name}", name = room_name), 14.0, t.text_faint))
                                .font(w::font(14.5))
                                .frame(egui::Frame::NONE)
                                .desired_width(ui.available_width() - 48.0),
                        )
                        .with_edit_menu();
                    let send = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let clicked = w::icon_button(ui, t, icons::SEND, t!("Send"), false, 0).clicked();
                    if (send || clicked) && !self.group_input.trim().is_empty() {
                        ctx.send(Command::SendGroupMessage { group: g.id, room, text: self.group_input.trim().to_string() });
                        self.group_input.clear();
                        r.request_focus();
                    }
                });
            });
        let measured = r.response.rect.height();
        if (measured - composer).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(composer_id, measured));
            ui.ctx().request_repaint();
        }
        join
    }
}

/// Who's in a group: in game, online, then offline.
fn member_list(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, group: u64) {
    let state = ctx.state;
    let mut people: Vec<model::Persona> = state.chat_group_members.get(&group).map(|m| m.iter().map(|id| state.persona(*id)).collect()).unwrap_or_default();
    people.sort_by_key(|p| (!p.in_game(), p.state == PersonaState::Offline, p.name.to_lowercase()));
    ui.label(w::text_bold(t!("Members"), 13.0, t.text_dim));
    ui.add_space(6.0);
    let mut open = None;
    egui::ScrollArea::vertical().id_salt(("group-members", group)).auto_shrink([false, false]).show(ui, |ui| {
        for p in &people {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
            }
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 4.0))).layout(Layout::left_to_right(Align::Center)));
            w::avatar(&mut c, t, &art::avatar(p.avatar_hash.as_deref()), 28.0, Some(crate::shell::status_color(t, p)));
            c.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let name = if p.name.is_empty() { p.steam_id.to_string() } else { p.name.clone() };
                ui.add(egui::Label::new(w::text_medium(name, 13.0, if p.state == PersonaState::Offline { t.text_faint } else { t.text })).truncate());
                if p.in_game() {
                    ui.add(egui::Label::new(w::text(crate::shell::status_text(state, p), 11.0, crate::shell::status_color(t, p))).truncate());
                }
            });
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Profile")).clicked() {
                open = Some(p.steam_id);
            }
        }
    });
    if let Some(id) = open {
        ctx.navigate("profile", Some(id.to_string()));
    }
}

//! Steam group chats in the Friends & Chat window: a group's text channels,
//! its messages (paging back), sending, members, unread marks and game
//! invites, like Steam's Friends & Chat. (No voice.)

use super::{Friends, composer_box, message, message_block, strong};
use crate::friends_window::{Conversations, Pane};
use client::Command;
use egui::{Align, Layout, Margin, Rect, Sense, Ui, vec2};
use extensions::{Action, Ctx};
use i18n::t;
use model::{ChatGroup, PersonaState, SteamId, art};
use ui::{Theme, icons, widgets as w};

/// A group chat's name: its own, or (for an unnamed group of friends) its
/// members' names, like Steam shows it.
pub(crate) fn group_name(state: &client::State, g: &ChatGroup) -> String {
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

/// The group's picture, or its initials on a tile colored from its id.
pub(crate) fn paint_group_avatar(ui: &Ui, t: &Theme, g: &ChatGroup, name: &str, r: Rect) {
    let radius = t.radius_of(&t.components.friend_avatar);
    let radius = egui::CornerRadius::same(radius.nw.min((r.width() / 2.0) as u8));
    if !g.avatar.is_empty() && w::paint_cover(ui, r, &g.avatar, radius) {
        return;
    }
    // A tile in the accent colors, picked by the group's id, so groups
    // without a picture still look different from each other.
    let fill = match g.id % 3 {
        0 => t.accent_2,
        1 => w::mix(t.accent_2, t.accent, 0.5),
        _ => w::mix(t.surface_hover, t.accent_2, 0.6),
    };
    ui.painter().rect_filled(r, radius, fill);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initials(name), egui::FontId::new(r.height() * 0.38, t.typography.heading_s.weight.family()), t.text_on(fill));
}

impl Friends {
    /// A group chat: its channels, messages and members.
    pub(super) fn group_conversation(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, convs: &mut Conversations, group: u64, room: u64) {
        if !self.groups_asked {
            self.groups_asked = true;
            if ctx.state.chat_groups.is_none() {
                ctx.send(Command::LoadChatGroups);
            }
        }
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
        let x = t.spacing.xl as i8;
        egui::Frame::new().inner_margin(Margin { left: x, right: x, top: t.spacing.sm as i8, bottom: t.spacing.md as i8 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::X, t!("Close conversation"), false, 0).clicked() {
                        convs.close(Pane::Group(group, room));
                    }
                    let wide = ui.max_rect().width() > t.dense(700.0);
                    if !wide && w::icon_button(ui, t, icons::USERS, t!("Members"), self.members, 0).clicked() {
                        self.members = !self.members;
                    }
                    if g.clan_id != 0 && w::icon_button(ui, t, icons::USERS_THREE, t!("Steam group page"), false, 0).clicked() {
                        ctx.navigate("group", Some((103582791429521408u64 + g.clan_id as u64).to_string()));
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let size = t.dense(46.0);
                        let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
                        paint_group_avatar(ui, t, &g, &name, r);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = t.spacing.xs;
                            ui.add(egui::Label::new(w::heading_s(t, &name, t.text)).truncate());
                            let members = ctx.state.chat_group_members.get(&group).map(|m| m.len()).unwrap_or(0);
                            let mut sub = t!("{n} members", n = members).to_string();
                            if !g.tagline.is_empty() {
                                sub = format!("{sub} · {}", g.tagline);
                            }
                            ui.add(egui::Label::new(w::body_m(t, sub, t.text_dim)).truncate());
                        });
                    });
                });
            });
            if g.rooms.len() > 1 {
                ui.add_space(t.spacing.sm);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(t.spacing.xs + 2.0, t.spacing.xs + 2.0);
                    for r in &g.rooms {
                        let label = format!("# {}", if r.name.is_empty() { t!("general").to_string() } else { r.name.clone() });
                        let resp = w::chip(ui, t, &label, r.id == room);
                        if r.unread && r.id != room {
                            ui.painter().circle_filled(resp.rect.right_top() + vec2(-4.0, 4.0), 4.0, t.accent);
                        }
                        if resp.clicked() {
                            convs.show(Pane::Group(group, r.id));
                        }
                    }
                });
            }
        });

        // Messages on the left, members on the right (when there's room).
        let members_w = if ui.available_width() > t.dense(700.0) {
            t.dense(220.0)
        } else if self.members {
            t.dense(190.0)
        } else {
            0.0
        };
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            let chat_w = ui.available_width() - members_w;
            ui.allocate_ui_with_layout(vec2(chat_w, height), Layout::top_down(Align::Min), |ui| {
                egui::Frame::new().inner_margin(Margin { left: x, right: x, top: 0, bottom: t.spacing.lg as i8 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let h = ui.available_height();
                    if let Some(a) = self.group_body(ui, t, ctx, &g, room, &msgs, h) {
                        ctx.act(a);
                    }
                });
            });
            if members_w > 0.0 {
                ui.allocate_ui_with_layout(vec2(members_w, height), Layout::top_down(Align::Min), |ui| {
                    w::component_frame(t, &t.components.friends_box).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(height - t.spacing.xl);
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
        let list_h = (height - composer - t.spacing.sm - ui.spacing().item_spacing.y).floor().max(40.0);
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
                    let first = last.is_none_or(|(from, time)| from != m.sender || m.time.saturating_sub(time) > 300);
                    let p = state.persona(m.sender);
                    let name = if p.name.is_empty() { m.sender.to_string() } else { p.display_name() };
                    message_block(ui, t, state, m.sender, &name, m.time as u64, first, |ui, join| message(ui, t, state, &shown, join), &mut join);
                    last = Some((m.sender, m.time));
                }
            });
        if let Some(before) = older {
            ctx.send(Command::LoadGroupHistory { group: g.id, room, before: Some(before) });
        }
        ui.add_space(t.spacing.sm);
        let room_name = g.rooms.iter().find(|r| r.id == room).map(|r| r.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| group_name(state, g));
        let hint = t!("Message {name}", name = room_name).to_string();
        let mut sent = None;
        let r = composer_box(ui, t, &mut self.group_input, &hint, |text| sent = Some(text), |_| {});
        if let Some(text) = sent {
            ctx.send(Command::SendGroupMessage { group: g.id, room, text });
        }
        let measured = r.height();
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
    ui.label(w::body_m(t, format!("{} ({})", t!("Members"), people.len()), t.text_dim));
    ui.add_space(t.spacing.sm);
    let mut open = None;
    egui::ScrollArea::vertical().id_salt(("group-members", group)).auto_shrink([false, false]).show(ui, |ui| {
        for p in &people {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), t.dense(38.0)), Sense::click());
            let c = &t.components.friend_row;
            w::paint_box(ui, t, rect, c, &t.look(c, w::interact(ui, &resp, false)), 0.0);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 4.0))).layout(Layout::left_to_right(Align::Center)));
            let color = crate::shell::status_color(t, p);
            w::presence_avatar(&mut c, t, &art::avatar(p.avatar_hash.as_deref()), rect.height() - 8.0, Some(color));
            c.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let name = if p.name.is_empty() { p.steam_id.to_string() } else { p.display_name() };
                let off = p.state == PersonaState::Offline;
                ui.add(egui::Label::new(strong(t, name, if off { t.text_dim } else { color })).truncate());
                if p.in_game() {
                    ui.add(egui::Label::new(w::body_s(t, crate::shell::status_text(state, p), color)).truncate());
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

//! A Steam group's page: who it is, how many are around, what it's about,
//! its announcements and some of its members.

use super::{HubView, Io, View, columns, error, files, rich};
use client::hub::{self, Request};
use egui::{Align, Layout, Sense, Ui, vec2};
use i18n::t;
use model::Relationship;
use ui::{Theme, icons, widgets as w};

/// Group ids start here; the rest is the clan's account id.
const CLAN_BASE: u64 = 103582791429521408;

pub(super) fn show(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, key: &str) {
    let req = || Request::Group { key: key.to_string() };
    let have = io.state.hub.groups.contains_key(key);
    hv.ask(io, format!("group:{key}"), have, req);
    let g = match io.state.hub.groups.get(key) {
        None => return w::loading(ui, t, &t!("Loading…")),
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, req());
            }
            return;
        }
        Some(Ok(g)) => g.clone(),
    };
    ui.add_space(16.0);
    header(ui, t, io, &g);
    ui.add_space(18.0);
    columns(
        ui,
        300.0,
        hv,
        io,
        |ui, hv, io| {
            if !g.summary.trim().is_empty() {
                w::card(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(w::text_bold(t!("About"), 15.0, t.text));
                    ui.add_space(6.0);
                    if let Some(click) = rich::show(ui, t, &rich::parse(&g.summary), 14.0) {
                        hv.follow(io, click);
                    }
                });
                ui.add_space(16.0);
            }
            ui.label(w::text_bold(t!("Announcements"), 17.0, t.text));
            ui.add_space(8.0);
            if g.announcements.is_empty() {
                ui.label(w::text(t!("No announcements yet."), 13.0, t.text_faint));
            } else {
                files::news_cards(hv, ui, t, io, &g.announcements);
            }
        },
        |ui, _hv, io| members(ui, t, io, &g),
    );
}

fn header(ui: &mut Ui, t: &Theme, io: &mut Io, g: &hub::Group) {
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
            w::image_or_skeleton(ui, t, r, &g.avatar, t.corner_small());
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add(egui::Label::new(w::text_bold(&g.name, 26.0, t.text)).wrap());
                if !g.headline.is_empty() && g.headline != g.name {
                    ui.add(egui::Label::new(w::text(&g.headline, 14.0, t.text_dim)).wrap());
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let stat = |ui: &mut Ui, n: u32, label: String, color: egui::Color32| {
                        ui.label(w::text_bold(crate::builtin::store::thousands(n as u64), 14.0, color));
                        ui.label(w::text(label, 13.0, t.text_dim));
                        ui.add_space(10.0);
                    };
                    stat(ui, g.members, t!("members").to_string(), t.text);
                    stat(ui, g.in_game, t!("in game").to_string(), t.in_game);
                    stat(ui, g.online, t!("online").to_string(), t.online);
                    stat(ui, g.in_chat, t!("in group chat").to_string(), t.text);
                });
            });
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                let path = hub::group_path(&g.url).unwrap_or_else(|| format!("gid/{}", g.id));
                if w::button(ui, t, Some(icons::EXTERNAL), t!("Open on Steam"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    crate::links::open_http(&format!("https://steamcommunity.com/{path}"));
                }
                // Its group chat, when you're in it.
                let clan = g.id.checked_sub(CLAN_BASE).unwrap_or(0) as u32;
                let chat = io.state.chat_groups.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|c| c.clan_id != 0 && c.clan_id == clan)).map(|c| c.id);
                if let Some(chat) = chat {
                    if w::button(ui, t, Some(icons::CHAT), t!("Open group chat"), w::Kind::Primary, w::Size::Sm).clicked() {
                        io.go(View::Page("community".into(), Some(format!("group:{chat}"))));
                    }
                }
            });
        });
    });
}

/// Friends in the group first, then the first members Steam lists.
fn members(ui: &mut Ui, t: &Theme, io: &mut Io, g: &hub::Group) {
    let friend = |id: &u64| io.state.friends.get(id) == Some(&Relationship::Friend);
    let friends: Vec<u64> = g.member_ids.iter().copied().filter(|id| friend(id)).collect();
    let others: Vec<u64> = g.member_ids.iter().copied().filter(|id| !friend(id)).take(24).collect();
    let mut open = None;
    let mut list = |ui: &mut Ui, title: String, ids: &[u64]| {
        if ids.is_empty() {
            return;
        }
        ui.label(w::text_bold(title, 15.0, t.text));
        ui.add_space(6.0);
        for id in ids {
            let p = io.state.persona(*id);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
            ui.painter().rect_filled(r, t.corner_small(), w::mix(t.bg, t.surface_hover, hover));
            let av = egui::Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(32.0, 32.0));
            let url = model::art::avatar(p.avatar_hash.as_deref());
            w::image_or_skeleton(ui, t, av, &url, t.corner_small());
            let name = if p.name.is_empty() { id.to_string() } else { p.name.clone() };
            let font = w::font(13.5);
            let max = r.width() - 52.0;
            let galley = ui.painter().layout(name, font, t.text, max);
            ui.painter().galley(egui::pos2(av.right() + 10.0, r.center().y - galley.size().y / 2.0), galley, t.text);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                open = Some(*id);
            }
        }
        ui.add_space(12.0);
    };
    list(ui, t!("Friends in this group").to_string(), &friends);
    list(ui, t!("Members").to_string(), &others);
    if let Some(id) = open {
        io.go(View::Page("profile".into(), Some(id.to_string())));
    }
}

//! Profile: someone's friends, reviews and groups as pages in the app
//! (they used to open Steam's website).

use super::{Profile, panel};
use client::Command;
use egui::{Align2, Sense, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::{SteamId, art};
use ui::{Theme, icons, widgets as w};

impl Profile {
    fn back(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId) {
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Back to profile"), w::Kind::Ghost, w::Size::Sm).clicked() {
                ctx.navigate("profile", Some(id.to_string()));
            }
        });
        ui.add_space(8.0);
    }

    fn loading_rows(ui: &mut Ui, t: &Theme, n: usize, h: f32) {
        for _ in 0..n {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
            w::skeleton(ui, t, r.shrink2(vec2(0.0, 4.0)), t.corner_small());
        }
    }

    fn hidden(ui: &mut Ui, t: &Theme, e: &str, what: &str) {
        let body = if e == "private" { t!("Its owner has chosen not to show this.").to_string() } else { i18n::tr_dyn(e).to_string() };
        w::empty(ui, t, icons::LOCK_SIMPLE, what, &body);
    }

    /// Someone's friends list.
    pub(super) fn friends_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, name: &str, mine: bool) {
        if self.lists_requested.insert((id, "friends")) {
            ctx.send(Command::LoadProfileFriends { steam_id: id });
        }
        Self::back(ui, t, ctx, id);
        panel(ui, t, |ui| {
            let title = if mine { t!("Your friends").to_string() } else { t!("{name}'s friends", name = name) };
            ui.label(w::text_bold(title, 24.0, t.text));
            let ids = match ctx.state.profile_friends.get(&id).cloned() {
                None => return Self::loading_rows(ui, t, 8, 56.0),
                Some(Err(e)) => return Self::hidden(ui, t, &e, &t!("These friends are hidden")),
                Some(Ok(ids)) => ids,
            };
            let common = ids.iter().filter(|f| ctx.state.friends.contains_key(f)).count();
            let mut sub = t!("{n} friends", n = ids.len()).to_string();
            if !mine && common > 0 {
                sub = format!("{sub} · {}", t!("{n} in common with you", n = common));
            }
            ui.label(w::text(sub, 13.5, t.text_dim));
            ui.add_space(10.0);
            w::search(ui, t, &mut self.list_search, &t!("Search friends"), 280.0);
            ui.add_space(10.0);
            let q = self.list_search.trim().to_lowercase();
            let mut people: Vec<(SteamId, model::Persona)> = ids.iter().map(|f| (*f, ctx.state.persona(*f))).filter(|(_, p)| q.is_empty() || p.name.to_lowercase().contains(&q)).collect();
            // Online first, then by name, like Steam.
            people.sort_by_key(|(_, p)| (p.state == model::PersonaState::Offline, p.name.to_lowercase()));
            let row_h = 56.0;
            egui::ScrollArea::vertical().id_salt(("profile-friends", id)).max_height(ui.ctx().content_rect().height() - 260.0).auto_shrink([false, true]).show_rows(ui, row_h, people.len(), |ui, range| {
                for (fid, p) in &people[range] {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                    ui.painter().rect_filled(r.shrink2(vec2(0.0, 3.0)), t.corner_small(), w::mix(t.surface, t.surface_hover, hover));
                    let av = egui::Rect::from_min_size(r.min + vec2(10.0, 9.0), vec2(38.0, 38.0));
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(av));
                    w::avatar(&mut c, t, &art::avatar(p.avatar_hash.as_deref()), 38.0, Some(crate::shell::status_color(t, p)));
                    let name = if p.name.is_empty() { fid.to_string() } else { p.name.clone() };
                    ui.painter().text(av.right_center() + vec2(12.0, -8.0), Align2::LEFT_CENTER, name, w::font_semibold(14.5), t.text);
                    let status = p.game_name.clone().map(|g| t!("Playing {game}", game = g).to_string()).unwrap_or_else(|| i18n::tr_dyn(p.state.label()).to_string());
                    ui.painter().text(av.right_center() + vec2(12.0, 10.0), Align2::LEFT_CENTER, status, w::font(12.5), t.text_dim);
                    if ctx.state.friends.contains_key(fid) {
                        ui.painter().text(r.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, t!("Your friend"), w::font(12.0), t.text_faint);
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        ctx.navigate("profile", Some(fid.to_string()));
                    }
                }
            });
        });
    }

    /// The latest reviews someone wrote.
    pub(super) fn reviews_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, name: &str, mine: bool) {
        if self.lists_requested.insert((id, "reviews")) {
            ctx.send(Command::LoadProfileReviews { steam_id: id });
        }
        Self::back(ui, t, ctx, id);
        panel(ui, t, |ui| {
            let title = if mine { t!("Your reviews").to_string() } else { t!("{name}'s reviews", name = name) };
            ui.label(w::text_bold(title, 24.0, t.text));
            let list = match ctx.state.profile_reviews.get(&id).cloned() {
                None => return Self::loading_rows(ui, t, 4, 120.0),
                Some(Err(e)) => return Self::hidden(ui, t, &e, &t!("These reviews are hidden")),
                Some(Ok(l)) => l,
            };
            ui.label(w::text(t!("Latest {n} reviews", n = list.len()), 13.5, t.text_dim));
            ui.add_space(10.0);
            if list.is_empty() {
                ui.label(w::text(t!("No reviews yet."), 13.5, t.text_faint));
            }
            for r in &list {
                w::card(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        let (img, resp) = ui.allocate_exact_size(vec2(184.0, 69.0), Sense::click());
                        w::image_or_skeleton(ui, t, img, &art::capsule(r.app_id), t.corner_small());
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            ctx.navigate("store", Some(format!("app:{}", r.app_id)));
                        }
                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            let game = ctx.state.app_name(r.app_id).map(String::from).unwrap_or_else(|| t!("App {id}", id = r.app_id).to_string());
                            ui.label(w::text_semibold(game, 15.0, t.text));
                            let (icon, label, color) = if r.voted_up { (icons::THUMBS_UP, t!("Recommended|review"), t.play) } else { (icons::THUMBS_DOWN, t!("Not recommended|review"), t.danger) };
                            ui.horizontal(|ui| {
                                ui.label(w::text(icon, 15.0, color));
                                ui.label(w::text_semibold(label, 13.0, color));
                                if !r.hours.is_empty() {
                                    ui.label(w::text(&r.hours, 12.5, t.text_faint));
                                }
                            });
                            let meta: Vec<&str> = [r.posted.as_str(), r.helpful.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                            if !meta.is_empty() {
                                ui.label(w::text(meta.join("  ·  "), 12.0, t.text_faint));
                            }
                        });
                    });
                    if !r.text.is_empty() {
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(w::text(&r.text, 13.5, t.text_dim)).wrap());
                    }
                });
                ui.add_space(8.0);
            }
        });
    }

    /// The groups someone is in.
    pub(super) fn groups_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, name: &str, mine: bool) {
        if self.lists_requested.insert((id, "groups")) {
            ctx.send(Command::LoadProfileGroups { steam_id: id });
        }
        Self::back(ui, t, ctx, id);
        panel(ui, t, |ui| {
            let title = if mine { t!("Your groups").to_string() } else { t!("{name}'s groups", name = name) };
            ui.label(w::text_bold(title, 24.0, t.text));
            let list = match ctx.state.profile_groups.get(&id).cloned() {
                None => return Self::loading_rows(ui, t, 6, 64.0),
                Some(Err(e)) => return Self::hidden(ui, t, &e, &t!("These groups are hidden")),
                Some(Ok(l)) => l,
            };
            ui.label(w::text(t!("{n} groups", n = list.len()), 13.5, t.text_dim));
            ui.add_space(10.0);
            for g in &list {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
                let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                ui.painter().rect_filled(r.shrink2(vec2(0.0, 3.0)), t.corner_small(), w::mix(t.surface, t.surface_hover, hover));
                let av = egui::Rect::from_min_size(r.min + vec2(10.0, 10.0), vec2(44.0, 44.0));
                w::image_or_skeleton(ui, t, av, &g.avatar, t.corner_small());
                let mut name = g.name.clone();
                if g.primary {
                    name = format!("{name}  ·  {}", t!("Primary group"));
                }
                ui.painter().text(av.right_center() + vec2(12.0, -9.0), Align2::LEFT_CENTER, name, w::font_semibold(14.5), t.text);
                let mut line = t!("{n} members", n = g.members).to_string();
                if g.online > 0 {
                    line = format!("{line} · {}", t!("{n} online", n = g.online));
                }
                if !g.headline.is_empty() && g.headline != g.name {
                    line = format!("{line} · {}", g.headline);
                }
                ui.painter().text(av.right_center() + vec2(12.0, 10.0), Align2::LEFT_CENTER, line, w::font(12.5), t.text_dim);
                ui.painter().text(r.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, icons::CARET_RIGHT, w::font(12.0), t.text_faint);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    ctx.navigate("group", Some(g.id.to_string()));
                }
            }
        });
    }
}

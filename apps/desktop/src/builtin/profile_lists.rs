//! Profile: someone's friends, reviews and groups as pages in the app
//! (they used to open Steam's website), in the profile's main column.

use super::{Profile, inset, loading_rows, pager, section};
use client::Command;
use egui::{Align2, Rect, Sense, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{SteamId, art};
use ui::{Theme, icons, widgets as w};

/// People or groups per page.
const PER_PAGE: usize = 20;

impl Profile {
    fn hidden(ui: &mut Ui, t: &Theme, e: &str, what: &str) {
        let body = if e == "private" { t!("Its owner has chosen not to show this.").to_string() } else { i18n::tr_dyn(e).to_string() };
        w::empty(ui, t, icons::LOCK_SIMPLE, what, &body);
    }

    /// Someone's friends list.
    pub(super) fn friends_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, mine: bool) {
        if self.lists_requested.insert((id, "friends")) {
            ctx.send(Command::LoadProfileFriends { steam_id: id });
        }
        let title = if mine { t!("Your friends") } else { t!("Friends") };
        let ids = match ctx.state.profile_friends.get(&id).cloned() {
            None => return section(ui, t, title, "", |ui| loading_rows(ui, t, 8, t.dense(56.0))),
            Some(Err(e)) => return section(ui, t, title, "", |ui| Self::hidden(ui, t, &e, t!("These friends are hidden"))),
            Some(Ok(ids)) => ids,
        };
        let common = ids.iter().filter(|f| ctx.state.friends.contains_key(f)).count();
        let mut note = t!("{n} friends", n = ids.len()).to_string();
        if !mine && common > 0 {
            note = format!("{note} · {}", t!("{n} in common with you", n = common));
        }
        let q = self.list_search.trim().to_lowercase();
        let mut people: Vec<(SteamId, model::Persona)> = ids.iter().map(|f| (*f, ctx.state.persona(*f))).filter(|(_, p)| q.is_empty() || p.name.to_lowercase().contains(&q)).collect();
        // Online first, then by name, like Steam.
        people.sort_by_key(|(_, p)| (p.state == model::PersonaState::Offline, p.name.to_lowercase()));
        let pages = people.len().div_ceil(PER_PAGE).max(1);
        self.list_page = self.list_page.min(pages - 1);
        let mut open = None;
        section(ui, t, title, &note, |ui| {
            let width = ui.available_width().min(320.0);
            if w::search(ui, t, &mut self.list_search, t!("Search friends"), width).changed() {
                self.list_page = 0;
            }
            for (fid, p) in people.iter().skip(self.list_page * PER_PAGE).take(PER_PAGE) {
                let name = if p.name.is_empty() { fid.to_string() } else { p.name.clone() };
                let status = p.game_name.clone().map(|g| t!("Playing {game}", game = g).to_string()).unwrap_or_else(|| i18n::tr_dyn(p.state.label()).to_string());
                let note = if ctx.state.friends.contains_key(fid) { t!("Your friend") } else { "" };
                if person_row(ui, t, &art::avatar(p.avatar_hash.as_deref()), &name, &status, crate::shell::status_color(t, p), note, true) {
                    open = Some(*fid);
                }
            }
            if let Some(p) = pager(ui, t, self.list_page, pages) {
                self.list_page = p;
            }
        });
        if let Some(f) = open {
            ctx.navigate("profile", Some(f.to_string()));
        }
    }

    /// The latest reviews someone wrote.
    pub(super) fn reviews_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, mine: bool) {
        if self.lists_requested.insert((id, "reviews")) {
            ctx.send(Command::LoadProfileReviews { steam_id: id });
        }
        let title = if mine { t!("Your reviews") } else { t!("Reviews") };
        let list = match ctx.state.profile_reviews.get(&id).cloned() {
            None => return section(ui, t, title, "", |ui| loading_rows(ui, t, 4, t.dense(120.0))),
            Some(Err(e)) => return section(ui, t, title, "", |ui| Self::hidden(ui, t, &e, t!("These reviews are hidden"))),
            Some(Ok(l)) => l,
        };
        let mut open = None;
        section(ui, t, title, &t!("Latest {n} reviews", n = list.len()), |ui| {
            if list.is_empty() {
                ui.label(w::body_m(t, t!("No reviews yet."), t.text_faint));
            }
            for r in &list {
                inset(ui, t, |ui| {
                    ui.horizontal_top(|ui| {
                        let (img, resp) = ui.allocate_exact_size(vec2(t.dense(184.0), t.dense(69.0)), Sense::click());
                        w::image_or_skeleton(ui, t, img, &art::capsule(r.app_id), t.corner_small());
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            open = Some(r.app_id);
                        }
                        ui.add_space(t.spacing.sm);
                        ui.vertical(|ui| {
                            let game = ctx.state.app_name(r.app_id).map(String::from).unwrap_or_else(|| t!("App {id}", id = r.app_id).to_string());
                            ui.label(w::body_l(t, game, t.text));
                            let (icon, label, color) = if r.voted_up { (icons::THUMBS_UP, t!("Recommended|review"), t.play) } else { (icons::THUMBS_DOWN, t!("Not recommended|review"), t.danger) };
                            ui.horizontal(|ui| {
                                ui.label(w::body_l(t, icon, color));
                                ui.label(w::body_m(t, label, color));
                                if !r.hours.is_empty() {
                                    ui.label(w::body_s(t, &r.hours, t.text_faint));
                                }
                            });
                            let meta: Vec<&str> = [r.posted.as_str(), r.helpful.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                            if !meta.is_empty() {
                                ui.label(w::body_s(t, meta.join("  ·  "), t.text_faint));
                            }
                        });
                    });
                    if !r.text.is_empty() {
                        ui.add_space(t.spacing.xs);
                        ui.add(egui::Label::new(w::body_m(t, &r.text, t.text_dim)).wrap());
                    }
                });
            }
        });
        if let Some(app) = open {
            ctx.navigate("store", Some(format!("app:{app}")));
        }
    }

    /// The groups someone is in.
    pub(super) fn groups_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, mine: bool) {
        if self.lists_requested.insert((id, "groups")) {
            ctx.send(Command::LoadProfileGroups { steam_id: id });
        }
        let title = if mine { t!("Your groups") } else { t!("Groups") };
        let list = match ctx.state.profile_groups.get(&id).cloned() {
            None => return section(ui, t, title, "", |ui| loading_rows(ui, t, 6, t.dense(64.0))),
            Some(Err(e)) => return section(ui, t, title, "", |ui| Self::hidden(ui, t, &e, t!("These groups are hidden"))),
            Some(Ok(l)) => l,
        };
        let pages = list.len().div_ceil(PER_PAGE).max(1);
        self.list_page = self.list_page.min(pages - 1);
        let mut open = None;
        section(ui, t, title, &t!("{n} groups", n = list.len()), |ui| {
            for g in list.iter().skip(self.list_page * PER_PAGE).take(PER_PAGE) {
                let mut name = g.name.clone();
                if g.primary {
                    name = format!("{name}  ·  {}", t!("Primary group"));
                }
                let mut line = t!("{n} members", n = g.members).to_string();
                if g.online > 0 {
                    line = format!("{line} · {}", t!("{n} online", n = g.online));
                }
                if !g.headline.is_empty() && g.headline != g.name {
                    line = format!("{line} · {}", g.headline);
                }
                if person_row(ui, t, &g.avatar, &name, &line, t.text_dim, icons::CARET_RIGHT, false) {
                    open = Some(g.id);
                }
            }
            if let Some(p) = pager(ui, t, self.list_page, pages) {
                self.list_page = p;
            }
        });
        if let Some(g) = open {
            ctx.navigate("group", Some(g.to_string()));
        }
    }
}

/// A person or group in a list (`profile_row`): picture, name, a line
/// under it and a note on the right; true when clicked. `ring` outlines
/// the picture and colors the line in `color` (a friend's status).
#[allow(clippy::too_many_arguments)]
fn person_row(ui: &mut Ui, t: &Theme, picture: &str, name: &str, line: &str, color: egui::Color32, note: &str, ring: bool) -> bool {
    let c = &t.components.profile_row;
    let pad = t.padding_of(c);
    let pic = t.dense(44.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), pic + pad.y * 2.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, w::interact(ui, &resp, false));
        w::paint_box(ui, t, rect, c, &look, 0.0);
        let av = Rect::from_min_size(rect.min + pad, vec2(pic, pic));
        w::image_or_skeleton(ui, t, av, picture, t.corner_small());
        if ring {
            ui.painter().rect_stroke(av, t.corner_small(), egui::Stroke::new(t.dense(2.0), color), egui::StrokeKind::Inside);
        }
        let x = av.right() + t.spacing.md;
        let right = rect.right() - pad.x;
        let note_w = if note.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(note.to_string(), t.typography.body_s.font(), look.icon).size().x + t.spacing.md };
        let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), t.font_of(c), look.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - note_w - x).max(20.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, av.top() + t.spacing.xs - 2.0), g, look.text);
        let line_color = if ring { color } else { look.icon };
        let mut job = egui::text::LayoutJob::simple_singleline(line.to_string(), t.typography.body_s.font(), line_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - note_w - x).max(20.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, av.bottom() - g.size().y - 2.0), g, line_color);
        if !note.is_empty() {
            ui.painter().text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, note, t.typography.body_s.font(), look.icon);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

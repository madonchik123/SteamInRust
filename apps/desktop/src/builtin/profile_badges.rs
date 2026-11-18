//! Badges in the app, like Steam's badge pages: every badge with its level,
//! XP, unlock date, playtime and card drops left; and a badge's own page with
//! its trading card set (which cards you have), crafting the next level, and
//! a way to find the missing cards on the Market.

use super::{Profile, loading_rows, section, unlocked_text};
use client::Command;
use egui::{Align2, Rect, Sense, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{Badge, SteamId};
use ui::{Theme, icons, widgets as w};

impl Profile {
    /// All of someone's badges.
    pub(super) fn badges_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, name: &str) {
        if let Some(url) = self.badge_open.clone() {
            return self.badge_page(ui, t, ctx, id, &url);
        }
        let _ = name;
        let Some(Ok(b)) = ctx.state.badges.get(&id).cloned() else {
            section(ui, t, t!("Badges"), "", |ui| loading_rows(ui, t, 5, t.dense(72.0)));
            return;
        };
        section(ui, t, t!("Badges"), &b.items.len().to_string(), |ui| {
            let mut open = None;
            for badge in &b.items {
                if badge_row(ui, t, badge) && !badge.url.is_empty() {
                    open = Some(badge.url.clone());
                }
            }
            if let Some(url) = open {
                self.badge_open = Some(url);
            }
        });
    }

    /// One badge: its level now, and for a game badge the card set.
    fn badge_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, url: &str) {
        let mine = id == ctx.state.steam_id;
        if self.badge_asked.insert(url.to_string()) {
            ctx.send(Command::LoadBadge { url: url.to_string() });
        }
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::ARROW_LEFT), t!("All badges"), w::Kind::Ghost, w::Size::Sm).clicked() {
                self.badge_open = None;
            }
        });
        ui.add_space(8.0);
        let d = match ctx.state.badge_pages.get(url) {
            None => {
                section(ui, t, "", "", |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 220.0), Sense::hover());
                    w::skeleton(ui, t, r, t.corner_small());
                });
                return;
            }
            Some(Err(e)) => {
                section(ui, t, "", "", |ui| w::empty(ui, t, icons::WARNING, t!("Couldn't load this badge"), &i18n::tr_dyn(e)));
                return;
            }
            Some(Ok(d)) => d.clone(),
        };
        section(ui, t, "", "", |ui| {
            ui.horizontal_top(|ui| {
                let (img, _) = ui.allocate_exact_size(vec2(t.dense(96.0), t.dense(96.0)), Sense::hover());
                match &d.current.icon {
                    Some(u) if !d.current.title.is_empty() => {
                        if !w::paint_contain(ui, img, u, Align2::CENTER_CENTER) {
                            w::skeleton(ui, t, img, t.corner());
                        }
                    }
                    _ => w::placeholder(ui, t, img, icons::MEDAL, t.corner()),
                }
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(w::heading_m(t, &d.title, t.text));
                    if d.current.title.is_empty() {
                        ui.label(w::body_m(t, t!("Not earned yet"), t.text_dim));
                    } else {
                        ui.label(w::body_l(t, &d.current.title, t.text));
                        ui.label(w::body_m(t, &d.current.detail, t.accent));
                        ui.label(w::body_s(t, unlocked_text(&d.current), t.text_faint));
                    }
                    let extra = [d.current.playtime.as_str(), d.current.drops.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                    if !extra.is_empty() {
                        ui.label(w::body_s(t, extra, t.text_dim));
                    }
                });
            });
            if d.cards.is_empty() {
                return;
            }
            ui.add_space(14.0);
            let have = d.cards.iter().filter(|c| c.owned > 0).count();
            ui.horizontal(|ui| {
                ui.label(w::heading_s(t, t!("Trading cards"), t.text));
                ui.label(w::body_m(t, t!("{have} of {all} collected", have = have, all = d.cards.len()), t.text_dim));
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                for c in &d.cards {
                    let (r, resp) = ui.allocate_exact_size(vec2(t.dense(130.0), t.dense(196.0)), Sense::hover());
                    let pic = Rect::from_min_size(r.min, vec2(t.dense(130.0), t.dense(152.0)));
                    if !w::paint_contain(ui, pic, &c.image, Align2::CENTER_CENTER) {
                        w::skeleton(ui, t, pic, t.corner_small());
                    }
                    let p = ui.painter();
                    if c.owned == 0 {
                        // Missing cards are dimmed, like Steam's.
                        p.rect_filled(pic, t.corner_small(), t.overlay_dark(2));
                    } else if c.owned > 1 {
                        let b = Rect::from_min_size(pic.right_top() + vec2(-34.0, 4.0), vec2(30.0, 20.0));
                        p.rect_filled(b, t.corner_small(), t.accent);
                        p.text(b.center(), Align2::CENTER_CENTER, format!("×{}", c.owned), t.typography.body_s.font(), t.on_accent);
                    }
                    let color = if c.owned > 0 { t.text } else { t.text_faint };
                    p.text(pos2(r.left(), pic.bottom() + 12.0), Align2::LEFT_CENTER, &c.name, t.typography.body_s.font(), color);
                    p.text(pos2(r.left(), pic.bottom() + 30.0), Align2::LEFT_CENTER, &c.position, t.typography.body_s.font(), t.text_faint);
                    resp.on_hover_text(&c.name);
                }
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if mine && have == d.cards.len() {
                    if self.crafting {
                        ui.add(egui::Spinner::new().size(18.0));
                    } else if w::button(ui, t, Some(icons::SPARKLE), t!("Craft badge"), w::Kind::Primary, w::Size::Md).clicked() {
                        self.craft_confirm = d.app_id;
                    }
                }
                if mine && have < d.cards.len() {
                    if let Some(app) = d.app_id {
                        if w::button(ui, t, Some(icons::STOREFRONT), t!("Find the missing cards on the Market"), w::Kind::Secondary, w::Size::Md).clicked() {
                            ctx.navigate("inventory", Some(format!("market:753:cards:{app}")));
                        }
                    }
                }
                if mine {
                    if let Some(app) = d.app_id {
                        if w::button(ui, t, Some(icons::BACKPACK), t!("Cards in your inventory"), w::Kind::Ghost, w::Size::Md).clicked() {
                            ctx.navigate("inventory", Some(format!("{id}/753")));
                            let _ = app;
                        }
                    }
                }
            });
        });
        self.craft_dialog(ui, t, ctx, url);
    }

    fn craft_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, url: &str) {
        let Some(app) = self.craft_confirm else { return };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("craft-badge")).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.label(w::heading_s(t, t!("Craft this badge?"), t.text));
            ui.add_space(4.0);
            ui.add(egui::Label::new(w::body_m(t, t!("One of each card in the set is used up. You get the next badge level, XP, and a random emoticon, background and coupon."), t.text_dim)).wrap());
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if w::button(ui, t, Some(icons::SPARKLE), t!("Craft badge"), w::Kind::Primary, w::Size::Md).clicked() {
                    self.crafting = true;
                    self.craft_url = Some(url.to_string());
                    ctx.send(Command::CraftBadge { app_id: app, foil: false });
                    close = true;
                }
                if w::button(ui, t, None, t!("Not now"), w::Kind::Ghost, w::Size::Md).clicked() {
                    close = true;
                }
            });
        });
        if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.craft_confirm = None;
        }
    }

    pub(super) fn badge_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        let client::Event::BadgeCrafted { result, .. } = event else { return };
        self.crafting = false;
        match result {
            Ok(name) if !name.is_empty() => ctx.toast(t!("Badge crafted: {name}", name = name)),
            Ok(_) => ctx.toast(t!("Badge crafted")),
            Err(e) => ctx.toast(i18n::tr_dyn(e)),
        }
        // Steam's new level and cards.
        if let Some(url) = self.craft_url.take() {
            self.badge_asked.remove(&url);
        }
        ctx.send(Command::LoadBadges { steam_id: ctx.state.steam_id });
    }
}

/// A badge in the list (`profile_row`); true when clicked.
fn badge_row(ui: &mut Ui, t: &Theme, badge: &Badge) -> bool {
    let c = &t.components.profile_row;
    let pad = t.padding_of(c);
    let pic = t.dense(60.0);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), pic + pad.y * 2.0), Sense::click());
    if !ui.is_rect_visible(r) {
        return resp.clicked();
    }
    let look = t.look(c, w::interact(ui, &resp, false));
    w::paint_box(ui, t, r, c, &look, 0.0);
    let img = Rect::from_min_size(r.min + pad, vec2(pic, pic));
    match &badge.icon {
        Some(u) => {
            if !w::paint_contain(ui, img, u, Align2::CENTER_CENTER) {
                w::skeleton(ui, t, img, t.corner_small());
            }
        }
        None => w::placeholder(ui, t, img, "", t.corner_small()),
    }
    let x = img.right() + t.spacing.md + 2.0;
    let p = ui.painter();
    let line = pic / 3.0;
    p.text(pos2(x, img.top() + line * 0.5), Align2::LEFT_CENTER, &badge.title, t.font_of(c), look.text);
    p.text(pos2(x, img.top() + line * 1.5), Align2::LEFT_CENTER, &badge.detail, t.typography.body_s.font(), t.accent);
    p.text(pos2(x, img.top() + line * 2.5), Align2::LEFT_CENTER, unlocked_text(badge), t.typography.body_s.font(), look.icon);
    // Game badges: playtime and card drops on the right, like Steam's.
    let right = [badge.playtime.as_str(), badge.drops.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>();
    let caret = t.typography.body_m.size + pad.x;
    for (i, s) in right.iter().enumerate() {
        p.text(pos2(r.right() - pad.x - caret, img.top() + line * (1.0 + i as f32)), Align2::RIGHT_CENTER, *s, t.typography.body_s.font(), if i == 1 { t.online } else { look.icon });
    }
    p.text(pos2(r.right() - pad.x, r.center().y), Align2::RIGHT_CENTER, icons::CARET_RIGHT, t.typography.body_m.font(), look.icon);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

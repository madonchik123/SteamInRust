//! Badges in the app, like Steam's badge pages: every badge with its level,
//! XP, unlock date, playtime and card drops left; and a badge's own page with
//! its trading card set (which cards you have), crafting the next level, and
//! a way to find the missing cards on the Market.

use super::{Profile, panel, unlocked_text};
use client::Command;
use egui::{Align, Align2, CornerRadius, Layout, Rect, Sense, Ui, pos2, vec2};
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
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Back to profile"), w::Kind::Ghost, w::Size::Sm).clicked() {
                ctx.navigate("profile", Some(id.to_string()));
            }
        });
        ui.add_space(8.0);
        let Some(Ok(b)) = ctx.state.badges.get(&id).cloned() else {
            panel(ui, t, |ui| {
                for _ in 0..5 {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
                    w::skeleton(ui, t, r.shrink2(vec2(0.0, 5.0)), t.corner_small());
                }
            });
            return;
        };
        panel(ui, t, |ui| {
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("{name}'s badges", name = name), 20.0, t.text));
                ui.label(w::text(b.items.len().to_string(), 15.0, t.text_dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::text_bold(t!("Level {n}", n = b.level), 15.0, t.text));
                        if !b.xp.is_empty() {
                            ui.label(w::text(&b.xp, 12.0, t.text_dim));
                        }
                    });
                });
            });
            if !b.next.is_empty() {
                ui.add_space(6.0);
                let width = ui.available_width();
                w::progress(ui, t, b.progress, width);
                ui.label(w::text(super::next_level_text(&b.next), 11.5, t.text_faint));
            }
            ui.add_space(10.0);
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
                panel(ui, t, |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 220.0), Sense::hover());
                    w::skeleton(ui, t, r, t.corner_small());
                });
                return;
            }
            Some(Err(e)) => {
                panel(ui, t, |ui| w::empty(ui, t, icons::WARNING, t!("Couldn't load this badge"), &i18n::tr_dyn(e)));
                return;
            }
            Some(Ok(d)) => d.clone(),
        };
        panel(ui, t, |ui| {
            ui.horizontal_top(|ui| {
                let (img, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
                match &d.current.icon {
                    Some(u) if !d.current.title.is_empty() => {
                        if !w::paint_contain(ui, img, u, Align2::CENTER_CENTER) {
                            w::skeleton(ui, t, img, CornerRadius::same(8));
                        }
                    }
                    _ => w::placeholder(ui, t, img, icons::MEDAL, CornerRadius::same(8)),
                }
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(w::text_bold(&d.title, 20.0, t.text));
                    if d.current.title.is_empty() {
                        ui.label(w::text(t!("Not earned yet"), 13.5, t.text_dim));
                    } else {
                        ui.label(w::text_semibold(&d.current.title, 15.0, t.text));
                        ui.label(w::text(&d.current.detail, 13.0, t.accent));
                        ui.label(w::text(unlocked_text(&d.current), 12.0, t.text_faint));
                    }
                    let extra = [d.current.playtime.as_str(), d.current.drops.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                    if !extra.is_empty() {
                        ui.label(w::text(extra, 12.0, t.text_dim));
                    }
                });
            });
            if d.cards.is_empty() {
                return;
            }
            ui.add_space(14.0);
            let have = d.cards.iter().filter(|c| c.owned > 0).count();
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("Trading cards"), 16.0, t.text));
                ui.label(w::text(t!("{have} of {all} collected", have = have, all = d.cards.len()), 13.0, t.text_dim));
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                for c in &d.cards {
                    let (r, resp) = ui.allocate_exact_size(vec2(130.0, 196.0), Sense::hover());
                    let pic = Rect::from_min_size(r.min, vec2(130.0, 152.0));
                    if !w::paint_contain(ui, pic, &c.image, Align2::CENTER_CENTER) {
                        w::skeleton(ui, t, pic, t.corner_small());
                    }
                    let p = ui.painter();
                    if c.owned == 0 {
                        // Missing cards are dimmed, like Steam's.
                        p.rect_filled(pic, t.corner_small(), egui::Color32::from_black_alpha(150));
                    } else if c.owned > 1 {
                        let b = Rect::from_min_size(pic.right_top() + vec2(-34.0, 4.0), vec2(30.0, 20.0));
                        p.rect_filled(b, CornerRadius::same(4), t.accent);
                        p.text(b.center(), Align2::CENTER_CENTER, format!("×{}", c.owned), w::font_bold(11.5), egui::Color32::WHITE);
                    }
                    let color = if c.owned > 0 { t.text } else { t.text_faint };
                    p.text(pos2(r.left(), pic.bottom() + 12.0), Align2::LEFT_CENTER, &c.name, w::font_semibold(12.5), color);
                    p.text(pos2(r.left(), pic.bottom() + 30.0), Align2::LEFT_CENTER, &c.position, w::font(11.0), t.text_faint);
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
            ui.label(w::text_bold(t!("Craft this badge?"), 18.0, t.text));
            ui.add_space(4.0);
            ui.add(egui::Label::new(w::text(t!("One of each card in the set is used up. You get the next badge level, XP, and a random emoticon, background and coupon."), 13.0, t.text_dim)).wrap());
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

/// A badge in the list; true when clicked.
fn badge_row(ui: &mut Ui, t: &Theme, badge: &Badge) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 76.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
    }
    let img = Rect::from_min_size(r.min + vec2(6.0, 8.0), vec2(60.0, 60.0));
    match &badge.icon {
        Some(u) => {
            if !w::paint_contain(ui, img, u, Align2::CENTER_CENTER) {
                w::skeleton(ui, t, img, CornerRadius::same(6));
            }
        }
        None => w::placeholder(ui, t, img, "", CornerRadius::same(6)),
    }
    let x = img.right() + 14.0;
    let p = ui.painter();
    p.text(pos2(x, r.top() + 20.0), Align2::LEFT_CENTER, &badge.title, w::font_semibold(14.0), t.text);
    p.text(pos2(x, r.top() + 39.0), Align2::LEFT_CENTER, &badge.detail, w::font(12.5), t.accent);
    p.text(pos2(x, r.top() + 57.0), Align2::LEFT_CENTER, unlocked_text(badge), w::font(11.5), t.text_faint);
    // Game badges: playtime and card drops on the right, like Steam's.
    let right = [badge.playtime.as_str(), badge.drops.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>();
    for (i, s) in right.iter().enumerate() {
        p.text(pos2(r.right() - 34.0, r.top() + 28.0 + i as f32 * 20.0), Align2::RIGHT_CENTER, *s, w::font(12.0), if i == 1 { t.online } else { t.text_dim });
    }
    p.text(pos2(r.right() - 12.0, r.center().y), Align2::CENTER_CENTER, icons::CARET_RIGHT, w::font(14.0), t.text_faint);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

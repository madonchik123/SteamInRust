//! Store: writing your own review on a game's page, like Steam's "Write a
//! review for …" box (shown for games you own). Posting, changing and
//! deleting all happen here, not on Steam's website.

use super::Store;
use client::Command;
use egui::{Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, MyReview};
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

/// What the review box is editing.
#[derive(Default)]
pub(super) struct ReviewBox {
    /// The game, and the review being written (a copy of yours, or new).
    draft: Option<(AppId, MyReview)>,
    asked: std::collections::HashSet<AppId>,
    /// Waiting for Steam to take a post, change or delete.
    saving: bool,
    /// The saved review is shown; "Edit" opens the editor.
    editing: bool,
    confirm_delete: bool,
}

impl ReviewBox {
    pub(super) fn on_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        if let client::Event::ReviewSaved { deleted, result, .. } = event {
            self.saving = false;
            match result {
                Ok(()) => {
                    self.editing = false;
                    self.draft = None;
                    ctx.toast(if *deleted { t!("Your review was deleted.") } else { t!("Your review was posted.") });
                }
                Err(e) => ctx.toast(t!("Couldn't save the review: {error}", error = e)),
            }
        }
    }
}

impl Store {
    pub(super) fn review_box(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, name: &str) {
        if self.review.asked.insert(id) {
            ctx.send(Command::LoadMyReview { app_id: id });
        }
        let Some(loaded) = ctx.state.my_reviews.get(&id) else {
            // Until Steam says whether there's a review, don't offer a blank one.
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().color(t.accent));
                    ui.label(w::text(t!("Loading your review…"), 13.0, t.text_dim));
                });
            });
            ui.add_space(20.0);
            return;
        };
        let mine = loaded.as_ref().ok().cloned().flatten();
        let b = &mut self.review;
        // A fresh draft when the game or the saved review changes.
        let saved_id = mine.as_ref().map(|r| r.id).unwrap_or(0);
        if b.draft.as_ref().is_none_or(|(a, d)| *a != id || (d.id != saved_id && !b.saving)) {
            let draft = mine.clone().unwrap_or_else(|| MyReview { voted_up: true, public: true, language: i18n::language().steam_name().to_string(), ..Default::default() });
            b.draft = Some((id, draft));
            b.editing = mine.is_none();
            b.confirm_delete = false;
        }
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match (&mine, b.editing) {
                // Your review, as posted.
                (Some(r), false) => {
                    ui.horizontal(|ui| {
                        let (icon, label, color) = if r.voted_up { (icons::THUMBS_UP, t!("Recommended|review"), t.play) } else { (icons::THUMBS_DOWN, t!("Not recommended|review"), t.danger) };
                        ui.label(w::text(icon, 22.0, color));
                        ui.vertical(|ui| {
                            ui.label(w::text_bold(t!("Your review of {name}", name = name), 16.0, t.text));
                            ui.label(w::text_semibold(label, 13.0, color));
                        });
                    });
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(w::text(&r.text, 13.5, t.text_dim)).wrap());
                    ui.add_space(6.0);
                    let mut facts = vec![if r.public { t!("Public").to_string() } else { t!("Friends only").to_string() }];
                    if r.playtime > 0 {
                        let h = |m: u32| format!("{:.1}", m as f64 / 60.0);
                        let mut hours = t!("{h} hrs on record", h = h(r.playtime)).to_string();
                        if r.playtime_at_review > 0 && r.playtime_at_review != r.playtime {
                            hours = format!("{hours} ({})", t!("{h} hrs at review time", h = h(r.playtime_at_review)));
                        }
                        facts.push(hours);
                    }
                    if r.updated > 0 {
                        facts.push(ui::ago(r.updated));
                    }
                    facts.push(t!("{n} found it helpful", n = r.votes_up).to_string());
                    if r.comments > 0 {
                        facts.push(t!("{n} comments", n = r.comments).to_string());
                    }
                    ui.label(w::text(facts.join("  ·  "), 12.0, t.text_faint));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if w::button(ui, t, Some(icons::NOTE), t!("Edit"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            b.editing = true;
                        }
                        if b.confirm_delete {
                            ui.label(w::text(t!("Delete your review?"), 12.5, t.text_dim));
                            if w::button(ui, t, Some(icons::TRASH), t!("Delete"), w::Kind::Danger, w::Size::Sm).clicked() && !b.saving {
                                b.saving = true;
                                ctx.send(Command::DeleteReview { app_id: id });
                            }
                            if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                b.confirm_delete = false;
                            }
                        } else if w::button(ui, t, Some(icons::TRASH), t!("Delete"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            b.confirm_delete = true;
                        }
                        if b.saving {
                            ui.add(egui::Spinner::new().color(t.accent));
                        }
                    });
                }
                // Writing or changing it.
                _ => {
                    let Some((_, d)) = b.draft.as_mut() else { return };
                    let title = if mine.is_some() { t!("Edit your review of {name}", name = name) } else { t!("Write a review for {name}", name = name) };
                    ui.label(w::text_bold(title, 16.0, t.text));
                    ui.label(w::text(t!("Say what you liked or didn't, so other players can decide. Be respectful and stick to the game."), 12.5, t.text_dim));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(w::text_semibold(t!("Do you recommend this game?"), 13.5, t.text));
                        ui.add_space(6.0);
                        let up = w::chip(ui, t, &format!("{}  {}", icons::THUMBS_UP, t!("Yes")), d.voted_up);
                        if up.clicked() {
                            d.voted_up = true;
                        }
                        if w::chip(ui, t, &format!("{}  {}", icons::THUMBS_DOWN, t!("No")), !d.voted_up).clicked() {
                            d.voted_up = false;
                        }
                    });
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::multiline(&mut d.text).hint_text(t!("Write your review…")).desired_rows(5).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if w::chip(ui, t, &format!("{}  {}", icons::GLOBE, t!("Public")), d.public).clicked() {
                            d.public = true;
                        }
                        if w::chip(ui, t, &format!("{}  {}", icons::USERS, t!("Friends only")), !d.public).clicked() {
                            d.public = false;
                        }
                        ui.add_space(8.0);
                        let mut allow = !d.comments_disabled;
                        if w::check(ui, t, &mut allow, w::text(t!("Allow comments"), 13.0, t.text)).changed() {
                            d.comments_disabled = !allow;
                        }
                        w::check(ui, t, &mut d.received_compensation, w::text(t!("I got this game for free"), 13.0, t.text));
                    });
                    ui.add_space(8.0);
                    let draft = d.clone();
                    let mut cancel = false;
                    ui.horizontal(|ui| {
                        let label = if mine.is_some() { t!("Save changes") } else { t!("Post review") };
                        let ready = !draft.text.trim().is_empty() && !b.saving;
                        if ui.add_enabled_ui(ready, |ui| w::button(ui, t, Some(icons::CHECK), label, w::Kind::Primary, w::Size::Md)).inner.clicked() {
                            b.saving = true;
                            ctx.send(Command::SaveReview { app_id: id, review: draft.clone() });
                        }
                        if mine.is_some() && w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                            cancel = true;
                        }
                        if b.saving {
                            ui.add(egui::Spinner::new().color(t.accent));
                        }
                    });
                    if cancel {
                        b.editing = false;
                        b.draft = None;
                    }
                }
            }
        });
        ui.add_space(20.0);
    }
}

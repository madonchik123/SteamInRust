//! Store: the discovery queue. Like Steam's, a dozen games picked for you,
//! each shown as its store page under a bar with "Not interested",
//! "Wishlist" and "Next in queue".

use super::{Store, View};
use client::Command;
use egui::{Align, Layout, Margin, Ui};
use extensions::Ctx;
use i18n::t;
use model::AppId;
use ui::{Theme, icons, widgets as w};

impl Store {
    /// The queue at `pos` (one past the end: the "you're done" card).
    pub(super) fn discovery_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, pos: usize) {
        if ctx.state.discovery.is_none() && !self.discovery_wait {
            self.discovery_wait = true;
            ctx.send(Command::LoadDiscoveryQueue { rebuild: false });
        }
        let state = if self.discovery_wait { None } else { ctx.state.discovery.clone() };
        let queue: Vec<AppId> = match state {
            Some(Ok(q)) => q,
            Some(Err(e)) => {
                egui::Frame::new().inner_margin(Margin::same(32)).show(ui, |ui| {
                    w::empty(ui, t, icons::WARNING, t!("Couldn't load your discovery queue"), &e);
                    ui.add_space(8.0);
                    if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Primary, w::Size::Md).clicked() {
                        self.discovery_wait = true;
                        ctx.send(Command::LoadDiscoveryQueue { rebuild: false });
                    }
                });
                return;
            }
            None => {
                w::loading(ui, t, t!("Loading your discovery queue…"));
                return;
            }
        };
        let Some(&id) = queue.get(pos) else {
            self.discovery_done(ui, t, ctx, queue.len());
            return;
        };

        // The queue bar, then the game's store page.
        egui::Frame::new().fill(t.surface).inner_margin(Margin::symmetric(28, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(w::text(icons::STACK, 18.0, t.accent));
                ui.vertical(|ui| {
                    ui.label(w::text_semibold(t!("Your discovery queue"), 15.0, t.text));
                    ui.label(w::text(t!("{n} of {total}", n = pos + 1, total = queue.len()), 12.5, t.text_dim));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let last = pos + 1 == queue.len();
                    let label = if last { t!("Finish queue") } else { t!("Next in queue") };
                    if w::button(ui, t, Some(icons::ARROW_RIGHT), label, w::Kind::Primary, w::Size::Md).clicked() {
                        ctx.send(Command::DiscoverySeen { app_id: id });
                        self.open(View::Discovery(pos + 1));
                    }
                    let wished = ctx.state.wishlist.as_ref().is_some_and(|l| l.iter().any(|(a, _)| *a == id));
                    let owned = self.owned.contains(&id);
                    if !owned {
                        let (icon, label) = if wished { (icons::CHECK, t!("On wishlist")) } else { (icons::HEART, t!("Add to wishlist")) };
                        if w::button(ui, t, Some(icon), label, w::Kind::Secondary, w::Size::Md).clicked() {
                            ctx.send(if wished { Command::WishlistRemove { app_id: id } } else { Command::WishlistAdd { app_id: id } });
                        }
                    }
                    let ignored = ctx.state.ignored.contains(&id);
                    let (icon, label) = if ignored { (icons::EYE, t!("Interested again")) } else { (icons::EYE_SLASH, t!("Not interested")) };
                    if w::button(ui, t, Some(icon), label, w::Kind::Ghost, w::Size::Md).clicked() {
                        ctx.send(Command::IgnoreApp { app_id: id, ignore: !ignored });
                        // Like Steam: not interested moves straight on.
                        if !ignored {
                            ctx.send(Command::DiscoverySeen { app_id: id });
                            self.open(View::Discovery(pos + 1));
                        }
                    }
                    if pos > 0 && w::button(ui, t, Some(icons::ARROW_LEFT), t!("Previous"), w::Kind::Ghost, w::Size::Md).clicked() {
                        self.open(View::Discovery(pos - 1));
                    }
                });
            });
        });
        if !self.allowed.contains(&id) && self.details_mature(id) {
            self.mature_gate(ui, t, id);
        } else {
            self.app_page(ui, t, ctx, id);
        }
    }

    /// End of the queue: start another, or back to the store.
    fn discovery_done(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, count: usize) {
        ui.add_space(60.0);
        w::centered(ui, 640.0, 28.0, |ui| {
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(w::text(icons::CHECK, 34.0, t.play));
                    ui.add_space(6.0);
                    let title = if count == 0 { t!("Your discovery queue is empty") } else { t!("You've been through your queue") };
                    ui.label(w::text_bold(title, 20.0, t.text));
                    ui.add_space(4.0);
                    ui.label(w::text(t!("Steam picks a new set of games for you each time."), 13.5, t.text_dim));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        // Centered pair of buttons.
                        ui.add_space((ui.available_width() - 360.0).max(0.0) / 2.0);
                        if w::button(ui, t, Some(icons::REFRESH), t!("Start another queue"), w::Kind::Primary, w::Size::Md).clicked() {
                            self.discovery_wait = true;
                            ctx.send(Command::LoadDiscoveryQueue { rebuild: true });
                            self.open(View::Discovery(0));
                        }
                        if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Back to the store"), w::Kind::Ghost, w::Size::Md).clicked() {
                            self.open(View::Home);
                        }
                    });
                    ui.add_space(10.0);
                });
            });
        });
    }
}

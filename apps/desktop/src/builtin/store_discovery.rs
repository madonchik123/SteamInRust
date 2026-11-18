//! Store: the discovery queue. Like Steam's, a dozen games picked for you:
//! as a feed of cards (Figma "discover" `306:2010`), or one at a time, each
//! shown as its store page under a bar with "Not interested", "Wishlist"
//! and "Next in queue".

use super::{Store, View, store_page::block};
use client::Command;
use egui::{Align, Layout, Margin, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::AppId;
use ui::{Theme, icons, widgets as w};

/// How long to wait for Steam to send a queue before saying so.
const QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

impl Store {
    /// Ask Steam for the queue; `rebuild` asks for a new set of games
    /// (Steam's "Start another queue").
    fn ask_queue(&mut self, ctx: &mut Ctx, rebuild: bool) {
        self.discovery_wait = true;
        self.discovery_late = false;
        self.discovery_asked = Some(std::time::Instant::now());
        ctx.send(Command::LoadDiscoveryQueue { rebuild });
    }

    /// The queue as it stands: asked for once, `None` while Steam answers,
    /// an error if it didn't answer in time.
    fn queue_state(&mut self, ctx: &mut Ctx) -> Option<Result<Vec<AppId>, String>> {
        if ctx.state.discovery.is_none() && !self.discovery_wait && !self.discovery_late {
            self.ask_queue(ctx, false);
        }
        if self.discovery_wait {
            let asked = self.discovery_asked.get_or_insert_with(std::time::Instant::now);
            if asked.elapsed() < QUEUE_WAIT {
                self.egui.request_repaint_after(std::time::Duration::from_secs(1));
                return None;
            }
            self.discovery_wait = false;
            self.discovery_asked = None;
            self.discovery_late = true;
        }
        // Stays until asked again (not the old queue behind it).
        if self.discovery_late {
            return Some(Err(t!("Steam didn't send a queue. Try again in a moment.").to_string()));
        }
        ctx.state.discovery.clone()
    }

    /// The queue, asked for once; `None` while it loads (or failed: shown).
    fn queue(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) -> Option<Vec<AppId>> {
        let state = self.queue_state(ctx);
        match state {
            Some(Ok(q)) => Some(q),
            Some(Err(e)) => {
                w::empty(ui, t, icons::WARNING, t!("Couldn't load your discovery queue"), &e);
                ui.add_space(t.spacing.sm);
                let again = ui.vertical_centered(|ui| w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Primary, w::Size::Md).clicked()).inner;
                if again {
                    self.ask_queue(ctx, false);
                }
                None
            }
            None => {
                w::loading(ui, t, t!("Loading your discovery queue…"));
                None
            }
        }
    }

    /// The discovery feed (Figma "discover" `306:2010`): every game in the
    /// queue as a card.
    pub(super) fn discover_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.persona(ctx.state.steam_id).name;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = t.spacing.sm;
                let title = if me.is_empty() { t!("Your discovery feed").to_string() } else { t!("{name}'s Discovery Feed", name = me) };
                ui.label(w::heading_l(t, title, t.text));
                ui.label(w::body_m(t, t!("Games Steam picked for you, from what you play and what you want."), t.text));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = t.dense(10.0);
                let waiting = self.discovery_wait;
                let r = ui.add_enabled_ui(!waiting, |ui| w::button(ui, t, Some(icons::REFRESH), t!("New picks"), w::Kind::Tertiary, w::Size::Md)).inner;
                if r.on_hover_text(t!("Marks these games as seen and asks Steam for a new set")).clicked() {
                    self.ask_queue(ctx, true);
                }
                if w::button(ui, t, Some(icons::STACK), t!("One at a time"), w::Kind::Secondary, w::Size::Md).on_hover_text(t!("Go through the queue game by game")).clicked() {
                    self.open(View::Discovery(0));
                }
            });
        });
        ui.add_space(t.dense(60.0));
        let Some(queue) = self.queue(ui, t, ctx) else { return };
        if queue.is_empty() {
            w::empty(ui, t, icons::STACK, t!("Your discovery queue is empty"), t!("Steam picks a new set of games for you each time."));
            return;
        }
        for id in queue {
            self.feed_card(ui, t, ctx, id);
            ui.add_space(t.dense(40.0));
        }
    }

    /// One game in the feed: a big picture and three more, then its name,
    /// tags, systems, release date and reviews, with Ignore, Wishlist,
    /// "Find more like this", the price and "Visit Store Page".
    fn feed_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId) {
        if self.safe(id) == Some(false) {
            return;
        }
        let d = self.details(id);
        let info = self.browse_info(id);
        block(t).inner_margin(Margin::same(t.dense(15.0) as i8)).show(ui, |ui| {
            let width = ui.available_width();
            ui.set_width(width);
            let gap = t.dense(8.0);
            let big_w = (width * 520.0 / 1531.0).round().max(t.dense(260.0));
            let h = (big_w * 0.5625).round();
            let small_h = ((h - 2.0 * gap) / 3.0).floor();
            let small_w = (small_h / 0.5625).round();
            let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
            let big = Rect::from_min_size(rect.min, vec2(big_w, h));
            let Some(d) = d else {
                w::skeleton(ui, t, rect, t.corner_small());
                return;
            };
            let open_big = ui.interact(big, ui.scope_id().with(("feed-art", id)), Sense::click());
            let shots = &d.screenshots;
            w::image_or_skeleton(ui, t, big, shots.first().map(|s| s.full.as_str()).unwrap_or(&d.header), t.corner_small());
            super::more::hover_wash(ui, t, big, &open_big);
            let mut x = big.right() + gap;
            if width > t.dense(900.0) {
                for (i, s) in shots.iter().skip(1).take(3).enumerate() {
                    let r = Rect::from_min_size(pos2(x, rect.top() + i as f32 * (small_h + gap)), vec2(small_w, small_h));
                    w::image_or_skeleton(ui, t, r, &s.thumb, t.corner_small());
                }
                x += small_w + gap;
            }
            let info_r = Rect::from_min_max(pos2(x + t.dense(9.0), rect.top()), rect.max);
            // Top right: Ignore and Wishlist.
            let wished = ctx.state.wishlist.as_ref().is_some_and(|l| l.iter().any(|(a, _)| *a == id));
            let ignored = ctx.state.ignored.contains(&id);
            let owned = self.owned.contains(&id);
            let mut top = ui.new_child(UiBuilder::new().max_rect(info_r).layout(Layout::right_to_left(Align::Min)));
            top.spacing_mut().item_spacing.x = t.dense(10.0);
            if !owned && w::button(&mut top, t, if wished { Some(icons::CHECK) } else { None }, t!("Wishlist"), w::Kind::Secondary, w::Size::Md).clicked() {
                ctx.send(if wished { Command::WishlistRemove { app_id: id } } else { Command::WishlistAdd { app_id: id } });
            }
            let label = if ignored { t!("Ignored") } else { t!("Ignore") };
            if w::button(&mut top, t, None, label, w::Kind::Secondary, w::Size::Md).on_hover_text(t!("Hide it from your recommendations")).clicked() {
                ctx.send(Command::IgnoreApp { app_id: id, ignore: !ignored });
            }
            let buttons_w = info_r.right() - top.min_rect().left() + t.dense(10.0);
            // Name, tags, systems and date, reviews.
            let text_r = Rect::from_min_max(info_r.min, pos2((info_r.right() - buttons_w).max(info_r.left() + 10.0), info_r.bottom() - t.dense(50.0)));
            let mut c = ui.new_child(UiBuilder::new().max_rect(text_r).layout(Layout::top_down(Align::Min)));
            c.set_clip_rect(text_r.intersect(ui.clip_rect()));
            c.spacing_mut().item_spacing.y = t.dense(10.0);
            c.add(egui::Label::new(w::heading_m(t, &d.name, t.text)).truncate().selectable(false));
            let tags = self.top_tags(id, &d.genres, 4);
            let mut picked = None;
            c.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.xs;
                for (tag, name) in &tags {
                    if w::tag(ui, t, name, None, w::TagState::Plain).clicked() {
                        picked = *tag;
                    }
                }
            });
            c.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.dense(10.0);
                let (win, mac, lin) = d.platforms;
                let systems: String = [(win, icons::WINDOWS), (mac, icons::APPLE), (lin, icons::LINUX)].iter().filter(|x| x.0).map(|x| x.1).collect::<Vec<_>>().join("  ");
                ui.label(w::body_l(t, systems, t.text_dim));
                ui.label(w::body_m(t, &d.release_date, t.text_dim));
            });
            if let Some(r) = info.as_ref().and_then(|i| i.reviews.clone()) {
                c.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = t.dense(10.0);
                    w::review_bar(ui, t, r.percent_positive.min(100) as u8, t.dense(138.0));
                    ui.label(w::body_s(t, t!("{count} user reviews", count = super::thousands(r.count as u64)), t.accent));
                });
            }
            // Bottom: "Find more like this" on the left; price and the store page on the right.
            let bottom = Rect::from_min_max(pos2(info_r.left(), info_r.bottom() - t.dense(44.0)), info_r.max);
            let mut b = ui.new_child(UiBuilder::new().max_rect(bottom).layout(Layout::left_to_right(Align::Center)));
            if let Some(tag) = tags.iter().find_map(|(id, _)| *id) {
                if w::button(&mut b, t, None, t!("Find more like this"), w::Kind::Secondary, w::Size::Md).clicked() {
                    picked = Some(tag);
                }
            }
            let mut open = open_big.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            b.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = t.dense(13.0);
                open |= w::button(ui, t, None, t!("Visit Store Page"), w::Kind::Secondary, w::Size::Md).clicked();
                if owned {
                    ui.label(w::body_l(t, t!("In library"), t.play));
                } else if let Some(p) = &d.price {
                    w::price(ui, t, &p.label(), p.original_label().as_deref(), p.discount);
                } else if d.is_free {
                    ui.label(w::styled(t!("Free"), t.typography.price, t.text_dim));
                }
            });
            if let Some(tag) = picked {
                self.open_tag(tag);
            }
            if open {
                self.open(View::App(id));
            }
        });
    }

    /// The queue at `pos` (one past the end: the "you're done" card).
    pub(super) fn discovery_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, pos: usize) {
        let state = self.queue_state(ctx);
        let queue: Vec<AppId> = match state {
            Some(Ok(q)) => q,
            Some(Err(e)) => {
                egui::Frame::new().inner_margin(Margin::same(32)).show(ui, |ui| {
                    w::empty(ui, t, icons::WARNING, t!("Couldn't load your discovery queue"), &e);
                    ui.add_space(8.0);
                    if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Primary, w::Size::Md).clicked() {
                        self.ask_queue(ctx, false);
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
        w::component_frame(t, &t.components.store_card).corner_radius(0).inner_margin(Margin::symmetric(28, 12)).show(ui, |ui| {
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
                    if w::button(ui, t, Some(icons::LIST), t!("See them all"), w::Kind::Ghost, w::Size::Md).on_hover_text(t!("The whole queue as a list")).clicked() {
                        self.open(View::Discover);
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
                            self.ask_queue(ctx, true);
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

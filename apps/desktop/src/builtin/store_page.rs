//! Store: a game's page (Figma "store page" `411:2833`): its name with
//! Ignore, Wishlist and Community Hub; the trailers and screenshots beside
//! the header, blurb, reviews and facts; then the buy boxes, the price
//! history, "About", the requirements, and on the right the features,
//! languages, Points Shop items and links; the players' reviews last.

use super::{Load, Store, View, blocks, thousands};
use client::Command;
use egui::{Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::AppId;
use store::Details;
use ui::{Theme, icons, widgets as w};

/// The page's widest (Figma: 1567).
pub(super) const MAX: f32 = 1567.0;

/// A content block on the store's pages (the `store_card` style, the
/// design's 25 px inside).
pub(super) fn block(t: &Theme) -> Frame {
    let pad = t.dense(25.0).round() as i8;
    w::component_frame(t, &t.components.store_card).inner_margin(Margin::same(pad))
}

/// A row inside a block (features, languages, links): the design's
/// `tag` rows, Bg / Highlight at 39%.
pub(super) fn row_fill(t: &Theme) -> Color32 {
    t.color(&"raised@0.39".into())
}

impl Store {
    pub(super) fn app_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId) {
        let state = self.details.get(&id).map(|s| match &*s.lock().unwrap() {
            Load::Loading => None,
            Load::Ready(d) => Some(Ok(d.clone())),
            Load::Failed(e) => Some(Err(e.clone())),
        });
        let d = match state.flatten() {
            Some(Ok(d)) => d,
            Some(Err(e)) => {
                egui::Frame::new().inner_margin(Margin::same(32)).show(ui, |ui| {
                    if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Store"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        self.open(View::Home);
                    }
                    w::empty(ui, t, icons::WARNING, t!("This game isn't available"), &e);
                });
                return;
            }
            None => {
                self.details(id);
                w::loading(ui, t, t!("Loading game…"));
                return;
            }
        };

        // The game's background art behind the top of the page, fading out.
        let full = ui.max_rect();
        let bg = Rect::from_min_size(pos2(full.min.x, ui.cursor().min.y), vec2(full.width(), t.dense(927.0)));
        paint_art(ui, t, bg, &d.background);

        let gap = t.dense(11.0);
        ui.add_space(t.dense(45.0));
        let pad = super::pad_margin(ui, MAX).min(28.0);
        w::centered(ui, MAX, pad, |ui| {
            ui.spacing_mut().item_spacing.y = gap;
            self.title_bar(ui, t, ctx, id, &d);
            crate::builtin::broadcast::store_strip(ui, t, ctx, id, &mut self.live_asked);
            self.media_block(ui, t, id, &d);
            let total = ui.available_width();
            let right_w = (total * 451.0 / 1567.0).clamp(300.0, 451.0);
            let left_w = total - right_w - gap;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(gap, t.dense(12.0));
                ui.vertical(|ui| {
                    ui.set_width(left_w);
                    self.buy_blocks(ui, t, ctx, id, &d);
                    if !d.is_free {
                        self.price_history(ui, t, id);
                    }
                    // Like Steam: games you own get a box to review them.
                    if self.owned.contains(&id) && ctx.state.steam_id != 0 {
                        self.review_box(ui, t, ctx, id, &d.name);
                    }
                    block(t).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(w::heading_s(t, t!("About {name}", name = d.name), t.text_dim));
                        ui.add_space(t.dense(18.0));
                        blocks(ui, t, &d.about, ui.available_width());
                    });
                    self.requirements(ui, t, &d, left_w);
                });
                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    self.relevance(ui, t, ctx, id, &d);
                    self.curators_for_app(ui, t, id);
                    self.app_offers(ui, t, id, d.package_id);
                    self.features(ui, t, &d);
                    self.languages_block(ui, t, id, &d);
                    self.point_items(ui, t, id);
                    self.links(ui, t, ctx, id, &d);
                });
            });
            ui.add_space(t.dense(60.0));
            self.review_list(ui, t, ctx, id);
        });
    }

    /// The name and the page's buttons (Figma `429:6013`).
    fn title_bar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, d: &Details) {
        let owned = self.owned.contains(&id);
        let signed_in = ctx.state.steam_id != 0;
        w::component_frame(t, &t.components.store_card).inner_margin(Margin::symmetric(t.dense(25.0) as i8, t.dense(15.0) as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(t.dense(53.0));
                ui.add(egui::Label::new(w::heading_l(t, &d.name, t.text)).truncate().selectable(false));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = t.dense(10.0);
                    if w::button(ui, t, None, t!("Community Hub"), w::Kind::Secondary, w::Size::Lg).clicked() {
                        ctx.navigate("hub", Some(id.to_string()));
                    }
                    if signed_in && !owned {
                        let wished = ctx.state.wishlist.as_ref().is_some_and(|l| l.iter().any(|x| x.0 == id));
                        let label = if wished { t!("On wishlist") } else { t!("Wishlist") };
                        if w::wishlist_button(ui, t, wished, Some(label)).clicked() {
                            ctx.send(if wished { Command::WishlistRemove { app_id: id } } else { Command::WishlistAdd { app_id: id } });
                        }
                    }
                    if signed_in {
                        let ignored = ctx.state.ignored.contains(&id);
                        let label = if ignored { t!("Ignored") } else { t!("Ignore") };
                        let tip = if ignored { t!("Show it in the store again") } else { t!("Hide it from your recommendations") };
                        if w::button(ui, t, None, label, w::Kind::Tertiary, w::Size::Lg).on_hover_text(tip).clicked() {
                            ctx.send(Command::IgnoreApp { app_id: id, ignore: !ignored });
                        }
                    }
                });
            });
        });
    }

    /// Trailers and screenshots, with the header picture and the facts
    /// beside them (Figma `415:2900`).
    fn media_block(&mut self, ui: &mut Ui, t: &Theme, id: AppId, d: &Details) {
        // See-through, over the game's art (the `shelf` style).
        w::component_frame(t, &t.components.shelf).inner_margin(Margin::same(t.dense(25.0) as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let total = ui.available_width();
            let gap = t.dense(23.0);
            let right_w = (total * 416.0 / 1517.0).clamp(280.0, 416.0);
            let left_w = total - right_w - gap;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.vertical(|ui| {
                    ui.set_width(left_w);
                    let trailers = self.browse_info(id).map(|b| b.trailers).unwrap_or_default();
                    self.media(ui, t, d, &trailers, left_w);
                });
                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    self.facts(ui, t, id, d, right_w);
                });
            });
        });
    }

    /// The header, blurb, review bars, release, developer, publisher and tags.
    fn facts(&mut self, ui: &mut Ui, t: &Theme, id: AppId, d: &Details, width: f32) {
        ui.spacing_mut().item_spacing.y = t.dense(18.0);
        let (r, _) = ui.allocate_exact_size(vec2(width, (width * 215.0 / 460.0).round()), Sense::hover());
        w::image_or_skeleton(ui, t, r, &d.header, t.corner_small());
        ui.add(egui::Label::new(w::body_m(t, &d.short_description, t.text)).wrap());
        let info = self.browse_info(id);
        let label_w = t.dense(105.0);
        // Labels in a column, values after them.
        let row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(label_w, t.typography.body_m.size + 3.0), Sense::hover());
                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, t.typography.body_m.font(), t.text_dim);
                add(ui);
            });
        };
        ui.spacing_mut().item_spacing.y = t.dense(14.0);
        if let Some(all) = info.as_ref().and_then(|i| i.reviews.clone()) {
            ui.label(w::body_s(t, t!("Reviews"), t.text_dim));
            // Bars instead of Steam's "Very Positive": the share of thumbs
            // up and down says more at a glance.
            if let Some((up, down)) = self.recent_votes(id).filter(|(u, d)| u + d > 0) {
                row(ui, t!("Past month"), &mut |ui| review_line(ui, t, (up * 100 / (up + down)) as u8, up + down));
            }
            row(ui, t!("All time"), &mut |ui| review_line(ui, t, all.percent_positive as u8, all.count as u64));
        } else if let Some(r) = d.recommendations {
            row(ui, t!("Reviews"), &mut |ui| {
                ui.label(w::body_m(t, format!("{} {}", icons::THUMBS_UP, thousands(r)), t.text));
            });
        }
        if let Some(n) = self.current_players(id) {
            row(ui, t!("Playing now"), &mut |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = t.spacing.sm;
                    let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, t.play);
                    ui.label(w::body_m(t, thousands(n as u64), t.text));
                })
                .response
                .on_hover_text(t!("People playing on Steam right now. Updates every minute."));
            });
        }
        if let Some(m) = d.metacritic {
            row(ui, "Metacritic", &mut |ui| {
                let color = if m >= 75 { t.play } else if m >= 50 { t.away } else { t.danger };
                ui.label(w::styled(m.to_string(), ui::theme::TextStyle::new(t.typography.body_m.size, ui::theme::Weight::Bold), color));
            });
        }
        row(ui, t!("Release date"), &mut |ui| {
            ui.label(w::body_m(t, &d.release_date, t.text));
        });
        for (label, kind, names) in [(t!("Developer"), "developer", &d.developers), (t!("Publisher"), "publisher", &d.publishers)] {
            if names.is_empty() {
                continue;
            }
            let mut open = None;
            row(ui, label, &mut |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (i, name) in names.iter().enumerate() {
                        if i > 0 {
                            ui.label(w::body_m(t, ", ", t.text_dim));
                        }
                        if super::link(ui, t, name, t.typography.body_m.size, t.accent).on_hover_text(t!("All games by {name}", name = name)).clicked() {
                            open = Some(name.clone());
                        }
                    }
                });
            });
            if let Some(name) = open {
                self.open_creator(kind, &name);
            }
        }
        // Popular tags (or the genres before tags load); each opens Browse
        // with it. As many as fit on two lines; "+" shows the rest.
        let tags: Vec<(Option<u32>, String)> = info.as_ref().map(|i| i.tags.iter().take(20).filter_map(|id| Some((Some(*id), self.tag_name(*id)?))).collect()).unwrap_or_default();
        let list: Vec<(Option<u32>, String)> = if tags.is_empty() { d.genres.iter().map(|g| (None, g.clone())).collect() } else { tags };
        if !list.is_empty() {
            ui.spacing_mut().item_spacing.y = t.dense(16.0);
            ui.label(w::body_l(t, t!("Popular tags"), t.text_dim));
            let mut picked = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
                let shown = list.len().min(6);
                for (id, name) in &list[..shown] {
                    if w::tag(ui, t, name, None, w::TagState::Plain).clicked() {
                        picked = Some((*id, name.clone()));
                    }
                }
                if list.len() > shown {
                    let more = w::tag(ui, t, "+", None, w::TagState::Plain).on_hover_text(t!("More tags"));
                    egui::Popup::from_toggle_button_response(&more).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(320.0).show(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
                            for (id, name) in &list[shown..] {
                                if w::tag(ui, t, name, None, w::TagState::Plain).clicked() {
                                    picked = Some((*id, name.clone()));
                                }
                            }
                        });
                    });
                }
            });
            if let Some((id, name)) = picked {
                if let Some(tag) = id.or_else(|| self.tag_by_name(&name)) {
                    self.open_tag(tag);
                }
            }
        }
    }

    /// The demo, then the buy box (Figma `419:3321`).
    fn buy_blocks(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, d: &Details) {
        let owned = ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&id)) || ctx.local.games.iter().any(|g| g.app_id == id);
        // Like Steam: "Download <game> Demo" above buying it.
        if let Some(demo) = d.demo.filter(|_| !owned) {
            let have = ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&demo));
            buy_row(ui, t, &t!("Download {name} Demo", name = d.name), Some(t!("Try it for free before you buy.")), None, |ui| {
                if have {
                    if w::button(ui, t, Some(icons::BOOKS), t!("In your library"), w::Kind::Play, w::Size::Md).clicked() {
                        ctx.navigate("library", Some(demo.to_string()));
                    }
                } else if self.free_pending == Some(demo) {
                    ui.add(egui::Spinner::new().color(t.accent));
                } else if w::button(ui, t, Some(icons::DOWNLOAD), t!("Download demo"), w::Kind::Play, w::Size::Md).clicked() {
                    self.free_pending = Some(demo);
                    ctx.send(Command::RequestFreeLicense { app_id: demo });
                }
            });
        }
        let title = if d.is_free { t!("Play {name}", name = d.name) } else { t!("Buy {name}", name = d.name) };
        let note = if owned {
            Some(t!("It's already in your library").to_string())
        } else if d.coming_soon {
            Some(t!("Coming soon").to_string())
        } else {
            d.price.as_ref().filter(|p| p.discount > 0).map(|_| t!("Special offer").to_string())
        };
        let price = d.price.as_ref().map(|p| (p.label(), p.original_label(), p.discount));
        if let Some(c) = d.price.as_ref().and_then(|p| p.final_) {
            if !owned {
                self.log_price(id, c);
            }
        }
        let package = d.package_id;
        let name = d.name.clone();
        let platforms = d.platforms;
        let (is_free, coming_soon, price_text) = (d.is_free, d.coming_soon, d.price_text.clone());
        buy_row(ui, t, &title, note.as_deref(), Some(platforms), |ui| {
            if owned {
                // Like Steam: you can still buy copies, as gifts.
                if let Some(pkg) = package.filter(|_| !is_free) {
                    self.purchase_actions(ui, t, ctx, id, Some(pkg), &name, price.clone(), true);
                    ui.add_space(t.spacing.sm);
                }
                if w::button(ui, t, Some(icons::BOOKS), t!("In your library"), w::Kind::Play, w::Size::Md).clicked() {
                    ctx.navigate("library", Some(id.to_string()));
                }
            } else if is_free {
                // Like Steam's "Play Game": the free license goes on the account.
                let pending = self.free_pending == Some(id);
                let label = if pending { t!("Adding…") } else { t!("Add to library") };
                let r = ui.add_enabled_ui(!pending, |ui| w::button(ui, t, Some(icons::PLUS), label, w::Kind::Play, w::Size::Md)).inner;
                if r.on_hover_text(t!("Free to play: adds it to your account, then you can install it")).clicked() {
                    self.free_pending = Some(id);
                    ctx.send(Command::RequestFreeLicense { app_id: id });
                }
            } else if coming_soon {
                ui.label(w::body_l(t, t!("Coming soon"), t.text_dim));
            } else if price.is_some() {
                self.purchase_actions(ui, t, ctx, id, package, &name, price.clone(), false);
            } else {
                ui.label(w::styled(price_text.clone().unwrap_or_default(), t.typography.price, t.text));
            }
        });
    }

    /// The features (Figma `417:2997`): a row per Steam feature.
    fn features(&mut self, ui: &mut Ui, t: &Theme, d: &Details) {
        if d.categories.is_empty() {
            return;
        }
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.dense(7.0);
            ui.label(w::body_m(t, t!("Features"), t.text_dim));
            for c in &d.categories {
                fill_row(ui, t, |ui| {
                    ui.label(w::body_l(t, feature_icon(c), t.accent));
                    ui.add_space(t.spacing.xs);
                    ui.label(w::body_s(t, c, t.accent));
                });
            }
        });
    }

    fn languages_block(&mut self, ui: &mut Ui, t: &Theme, id: AppId, d: &Details) {
        let languages = self.browse_info(id).map(|b| b.languages).unwrap_or_default();
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.dense(7.0);
            ui.label(w::body_l(t, t!("Languages"), t.text_dim));
            if languages.is_empty() {
                ui.add(egui::Label::new(w::body_s(t, d.languages.replace("*languages with full audio support", ""), t.text_dim)).wrap());
            } else {
                self.languages_table(ui, t, &languages);
            }
        });
    }

    /// Points Shop items from this game (Figma `Point Shop Items`): a few
    /// pictures and how many more; opens the Points Shop at this game.
    fn point_items(&mut self, ui: &mut Ui, t: &Theme, id: AppId) {
        let Some(page) = self.points_for_app(id) else { return };
        if page.items.is_empty() {
            return;
        }
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::body_l(t, t!("Points Shop items"), t.text_dim));
            ui.add_space(t.dense(4.0));
            let size = ((ui.available_width() - 4.0 * t.dense(7.0)) / 5.0).floor().min(t.dense(48.0));
            let mut open = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.dense(7.0);
                for it in page.items.iter().take(4) {
                    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                    if !w::paint_cover(ui, r, &it.image, t.corner_small()) {
                        w::skeleton(ui, t, r, t.corner_small());
                    }
                    super::more::hover_wash(ui, t, r, &resp);
                    open |= resp.on_hover_text(&it.name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                }
                let more = page.total.saturating_sub(4);
                if more > 0 {
                    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                    ui.painter().rect_filled(r, t.corner_small(), w::mix(row_fill(t), t.surface_hover, hover));
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("+{more}"), t.typography.body_l.font(), t.text);
                    open |= resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                }
            });
            if open {
                self.points_open_app(id);
            }
        });
    }

    /// Where else to go for the game (Figma "Links").
    fn links(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, d: &Details) {
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.dense(7.0);
            ui.label(w::body_l(t, t!("Links"), t.text_dim));
            let mut go: Option<(&str, String)> = None;
            for (label, page, arg) in [
                (t!("Community Hub"), "hub", id.to_string()),
                (t!("Discussions"), "discussions", id.to_string()),
                (t!("Guides"), "guides", id.to_string()),
                (t!("Workshop"), "hub", format!("{id}/workshop")),
            ] {
                if fill_row(ui, t, |ui| {
                    ui.label(w::body_s(t, label, t.text));
                })
                .clicked()
                {
                    go = Some((page, arg));
                }
            }
            if let Some(site) = &d.website {
                if fill_row(ui, t, |ui| {
                    ui.label(w::body_s(t, format!("{}  {}", t!("Official website"), icons::EXTERNAL), t.text));
                })
                .clicked()
                {
                    crate::links::open_http(site);
                }
            }
            if fill_row(ui, t, |ui| {
                ui.label(w::body_s(t, format!("{}  {}", icons::LINK, t!("Copy the store link")), t.text));
            })
            .clicked()
            {
                ui.ctx().copy_text(store::store_url(id));
                ctx.toast(t!("Link copied"));
            }
            if let Some((page, arg)) = go {
                ctx.navigate(page, Some(arg));
            }
        });
    }

    /// The big picture or trailer, then the strip of thumbnails with arrows
    /// at its ends and dots under it.
    fn media(&mut self, ui: &mut Ui, t: &Theme, d: &Details, trailers: &[store::browse::Trailer], width: f32) {
        // Trailers come first in the strip, like on Steam. `self.shot` counts
        // trailers first, then screenshots.
        let n_tr = trailers.len();
        let total = n_tr + d.screenshots.len();
        if total == 0 {
            let (r, _) = ui.allocate_exact_size(vec2(width, width * 0.5625), Sense::hover());
            w::image_or_skeleton(ui, t, r, &d.header, t.corner_small());
            return;
        }
        self.shot %= total;
        let (main, resp) = ui.allocate_exact_size(vec2(width, (width * 0.5625).round()), Sense::click());
        if self.shot >= n_tr {
            // Leaving the trailers stops playback.
            self.trailer = None;
        }
        if self.shot < n_tr {
            let tr = &trailers[self.shot];
            let source = tr.hls.clone().map(video::Source::Hls).or_else(|| tr.micro_mp4.clone().map(video::Source::Mp4));
            if let Some(src) = source {
                if self.trailer.as_ref().is_none_or(|(url, _)| url != src.url()) {
                    // A picked trailer plays right away (muted until the user
                    // turns sound on once), like Steam; full screen carries over.
                    let fullscreen = self.trailer.as_ref().is_some_and(|(_, p)| p.fullscreen);
                    let url = src.url().to_string();
                    let mut p = video::Player::new(&self.rt, ui.ctx(), src, video::widget::options(ui.ctx()));
                    p.fullscreen = fullscreen;
                    self.trailer = Some((url, p));
                }
                let (_, p) = self.trailer.as_mut().expect("just set");
                if video::widget::show(ui, t, main, p, &tr.thumbnail, &tr.name).ended {
                    // Like Steam: on to the next trailer or screenshot.
                    self.step_shot(1, total);
                }
            } else {
                w::image_or_skeleton(ui, t, main, &tr.thumbnail, t.corner_small());
                ui.painter().text(main.left_bottom() + vec2(16.0, -16.0), Align2::LEFT_BOTTOM, &tr.name, t.typography.body_l.font(), t.text);
            }
        } else {
            let s = &d.screenshots[self.shot - n_tr];
            if !w::paint_cover(ui, main, &s.full, t.corner_small()) {
                w::image_or_skeleton(ui, t, main, &s.thumb, t.corner_small());
            }
        }
        // A screenshot moves on to the next when clicked.
        if self.shot >= n_tr && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.step_shot(1, total);
        }
        ui.add_space(t.dense(20.0) - ui.spacing().item_spacing.y);
        self.strip(ui, t, d, trailers, width);
    }

    /// Move the highlight by one, wrapping around.
    pub(super) fn step_shot(&mut self, dir: i32, total: usize) {
        self.shot = (self.shot as i32 + dir).rem_euclid(total as i32) as usize;
    }

    /// The thumbnail strip (Figma `429:6017`): arrows at both ends; the
    /// picked thumbnail scrolls to the middle and has an outline.
    fn strip(&mut self, ui: &mut Ui, t: &Theme, d: &Details, trailers: &[store::browse::Trailer], width: f32) {
        let n_tr = trailers.len();
        let total = n_tr + d.screenshots.len();
        let arrow_w = t.dense(24.0);
        let gap = t.dense(16.0);
        let thumb_h = t.dense(111.0).min(((width - 2.0 * (arrow_w + gap)) / 5.0 - 7.0) * 9.0 / 16.0).max(40.0);
        let thumb_w = (thumb_h * 16.0 / 9.0).round();
        let center_now = self.shot_centered != Some(self.shot);
        let mut pick = None;
        let top = ui.cursor().top();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let arrow = |ui: &mut Ui, icon: &str| -> bool {
                let (r, b) = ui.allocate_exact_size(vec2(arrow_w, thumb_h), Sense::click());
                let hover = ui.ctx().animate_bool_with_time(b.id, b.hovered(), 0.1);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, w::font(arrow_w), w::mix(t.text, t.accent, hover));
                b.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
            };
            if arrow(ui, icons::CARET_LEFT) && total > 0 {
                pick = Some((self.shot + total - 1) % total);
            }
            let inner = (width - 2.0 * (arrow_w + gap)).max(100.0);
            ui.allocate_ui(vec2(inner, thumb_h), |ui| {
                egui::ScrollArea::horizontal().id_salt("shots").max_width(inner).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.dense(7.0);
                        for i in 0..total {
                            let (r, resp) = ui.allocate_exact_size(vec2(thumb_w, thumb_h), Sense::click());
                            if i < n_tr {
                                let tr = &trailers[i];
                                w::image_or_skeleton(ui, t, r, &tr.thumbnail, t.corner_small());
                                ui.painter().circle_filled(r.center(), 16.0, t.overlay_dark(2));
                                w::paint_icon(ui.painter(), r.center(), icons::PLAY, 14.0, t.text);
                            } else {
                                w::image_or_skeleton(ui, t, r, &d.screenshots[i - n_tr].thumb, t.corner_small());
                            }
                            if i == self.shot {
                                ui.painter().rect_stroke(r, t.corner_small(), Stroke::new(2.0, t.accent_2), egui::StrokeKind::Inside);
                                if center_now {
                                    resp.scroll_to_me(Some(Align::Center));
                                }
                            } else if !resp.hovered() {
                                ui.painter().rect_filled(r, t.corner_small(), t.overlay_dark(1));
                            }
                            let resp = if i < n_tr { resp.on_hover_text(&trailers[i].name) } else { resp };
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                pick = Some(i);
                            }
                        }
                    });
                });
            });
            if arrow(ui, icons::CARET_RIGHT) && total > 0 {
                pick = Some((self.shot + 1) % total);
            }
        });
        // The strip's full height, whatever the scroll area took.
        ui.add_space((top + thumb_h - ui.cursor().top()).max(0.0));
        if center_now {
            self.shot_centered = Some(self.shot);
        }
        // Dots: one per strip's width of thumbnails.
        let per = ((width - 2.0 * (arrow_w + gap)) / (thumb_w + t.dense(7.0))).floor().max(1.0) as usize;
        let pages = total.div_ceil(per);
        if pages > 1 {
            ui.add_space(t.dense(20.0) - ui.spacing().item_spacing.y);
            if let Some(p) = w::pager(ui, t, pages, self.shot / per, 1.0, t.dense(35.0)) {
                pick = Some((p * per).min(total - 1));
            }
        }
        if let Some(i) = pick {
            self.shot = i;
        }
    }

    /// "Is this game relevant to you?" — what Steam users check before buying.
    fn relevance(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: AppId, d: &Details) {
        let info = self.browse_info(id);
        let local = ctx.local.games.iter().find(|g| g.app_id == id);
        let owned = ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&id)) || local.is_some();
        let top_seller = matches!(self.front_data(), Some(Ok((_, ref s))) if s.iter().any(|s| s.id == "top_sellers" && s.items.iter().any(|i| i.app_id == id)));
        let playing: Vec<_> = ctx.state.friend_list().into_iter().filter(|p| p.game_app_id == Some(id)).collect();
        let mut rows: Vec<(&str, Color32, String)> = Vec::new();
        if let Some(m) = local.and_then(|g| g.playtime_minutes).filter(|m| *m > 0) {
            rows.push((icons::INFO, t.away, t!("You've played {time}", time = ui::playtime(m))));
        } else if owned {
            rows.push((icons::CHECK, t.play, t!("It's already in your library").to_string()));
        }
        if let Some(r) = info.as_ref().and_then(|i| i.reviews.as_ref()) {
            let (icon, color) = if r.score >= 7 { (icons::CHECK, t.play) } else if r.score <= 4 { (icons::X, t.danger) } else { (icons::INFO, t.away) };
            rows.push((icon, color, t!("User reviews: {label}", label = i18n::tr_dyn(&r.label))));
        }
        if top_seller {
            rows.push((icons::CHECK, t.play, t!("In the Top Sellers").to_string()));
        }
        if let Some(m) = d.metacritic.filter(|m| *m >= 75) {
            rows.push((icons::CHECK, t.play, t!("Metacritic score: {score}", score = m)));
        }
        if !playing.is_empty() {
            rows.push((icons::USERS, t.in_game, t!("Friends playing right now: {n}", n = playing.len())));
        }
        if d.coming_soon {
            rows.push((icons::CALENDAR, t.accent, t!("Not released yet").to_string()));
        }
        if rows.is_empty() {
            return;
        }
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.dense(7.0);
            ui.label(w::body_l(t, t!("Is this game relevant to you?"), t.text_dim));
            for (icon, color, text) in rows {
                fill_row(ui, t, |ui| {
                    ui.label(w::body_l(t, icon, color));
                    ui.add_space(t.spacing.xs);
                    ui.add(egui::Label::new(w::body_s(t, text, t.text)).truncate());
                });
            }
            if !playing.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for p in playing.iter().take(8) {
                        w::avatar(ui, t, &model::art::avatar(p.avatar_hash.as_deref()), 30.0, Some(t.in_game)).on_hover_text(&p.name);
                    }
                });
            }
        });
    }
}

/// The game's background art across the top of the page, washed darker and
/// fading out at the bottom into whatever the theme paints behind the page.
fn paint_art(ui: &Ui, t: &Theme, rect: Rect, url: &str) {
    if url.is_empty() {
        return;
    }
    let Ok(egui::load::TexturePoll::Ready { texture }) = ui.ctx().try_load_texture(url, egui::TextureOptions::LINEAR, egui::SizeHint::default()) else {
        return;
    };
    let tex = texture.size;
    // Cover the width; crop the top and bottom.
    let (ta, ra) = (tex.x / tex.y.max(1.0), rect.width() / rect.height());
    let uv = if ta > ra {
        let w = ra / ta;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, ta / ra))
    };
    // Full strength down to 45%, then fading to nothing.
    let stops = [(0.0, 1.0), (0.45, 1.0), (1.0, 0.0)];
    let wash = t.overlay_dark(2);
    let mut art = egui::Mesh::with_texture(texture.id);
    let mut dark = egui::Mesh::default();
    for (i, (y, a)) in stops.iter().enumerate() {
        let py = rect.top() + rect.height() * y;
        let v = uv.top() + uv.height() * y;
        let tint = Color32::WHITE.gamma_multiply(*a);
        art.vertices.push(egui::epaint::Vertex { pos: pos2(rect.left(), py), uv: pos2(uv.left(), v), color: tint });
        art.vertices.push(egui::epaint::Vertex { pos: pos2(rect.right(), py), uv: pos2(uv.right(), v), color: tint });
        dark.colored_vertex(pos2(rect.left(), py), wash.gamma_multiply(*a));
        dark.colored_vertex(pos2(rect.right(), py), wash.gamma_multiply(*a));
        if i > 0 {
            let b = (i as u32) * 2;
            for m in [&mut art, &mut dark] {
                m.add_triangle(b - 2, b - 1, b);
                m.add_triangle(b - 1, b + 1, b);
            }
        }
    }
    let p = ui.painter_at(rect);
    p.add(art);
    p.add(dark);
}

/// "90% 👍 … 10% 👎" and how many reviews.
fn review_line(ui: &mut Ui, t: &Theme, positive: u8, count: u64) {
    let label = t!("{count} reviews", count = thousands(count));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.dense(14.0);
        let need = ui.painter().layout_no_wrap(label.clone(), t.typography.body_s.font(), t.accent).size().x + ui.spacing().item_spacing.x;
        let bar = t.dense(138.0).min((ui.available_width() - need).max(t.dense(80.0)));
        let r = w::review_bar(ui, t, positive, bar);
        // No room for the count: it moves into the bar's tooltip.
        if ui.available_width() >= need {
            ui.label(w::body_s(t, &label, t.accent));
        } else {
            r.on_hover_text(&label);
        }
    });
}

/// One buy box: a title and a note on the left, the systems in the middle,
/// the buttons (added right to left) on the right.
fn buy_row(ui: &mut Ui, t: &Theme, title: &str, note: Option<&str>, platforms: Option<(bool, bool, bool)>, add: impl FnOnce(&mut Ui)) {
    w::component_frame(t, &t.components.store_card).inner_margin(Margin::symmetric(t.dense(28.0) as i8, t.dense(26.0) as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.set_min_height(t.dense(48.0));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = t.dense(6.0);
                ui.label(w::heading_s(t, title, t.text));
                if let Some(n) = note {
                    ui.label(w::body_m(t, n, t.accent));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = t.dense(13.0);
                add(ui);
                if let Some((win, mac, lin)) = platforms {
                    ui.add_space(t.dense(40.0));
                    let mut s = String::new();
                    for (on, i) in [(lin, icons::LINUX), (mac, icons::APPLE), (win, icons::WINDOWS)] {
                        if on {
                            s.push_str(i);
                            s.push_str("  ");
                        }
                    }
                    ui.label(w::styled(s.trim_end(), ui::theme::TextStyle::new(t.typography.heading_s.size, Default::default()), t.text_dim));
                }
            });
        });
    });
}

/// A full-width row with the design's light fill; the response is the row's.
pub(super) fn fill_row(ui: &mut Ui, t: &Theme, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let h = t.dense(34.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, w::mix(row_fill(t), t.surface_hover, hover * 0.5));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(t.dense(13.0), 0.0))).layout(Layout::left_to_right(Align::Center)));
    c.spacing_mut().item_spacing.x = t.spacing.xs;
    add(&mut c);
    resp
}

/// Icon for a store "feature" category (Co-op, Trading Cards, ...).
fn feature_icon(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("single") || n.contains("одиноч") {
        icons::USER
    } else if n.contains("multi") || n.contains("co-op") || n.contains("pvp") || n.contains("мульти") || n.contains("кооп") {
        icons::USERS
    } else if n.contains("achiev") || n.contains("достиж") {
        icons::TROPHY
    } else if n.contains("card") || n.contains("карточ") {
        icons::TAG
    } else if n.contains("cloud") || n.contains("облак") {
        icons::DOWNLOAD
    } else if n.contains("controller") || n.contains("контрол") {
        icons::CONTROLLER
    } else if n.contains("workshop") || n.contains("мастерск") {
        icons::SLIDERS
    } else if n.contains("anti-cheat") || n.contains("vac") || n.contains("античит") {
        icons::SHIELD
    } else if n.contains("purchase") || n.contains("покуп") {
        icons::CART
    } else {
        icons::CHECK
    }
}

//! Store: bundle and package pages: what's included, what you already own,
//! the bundle discount, "Complete your collection" and adding it to the
//! cart. Also the "Bundles and editions" box on game pages.

use super::more::{hover_wash, snapshot};
use super::store_page::{block, fill_row, row_fill};
use super::{Shared, Store, View};
use client::Command;
use egui::{Align, CursorIcon, Label, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::AppId;
use std::collections::{HashMap, HashSet};
use store::offers::{Offer, OfferKind, OfferLink, OfferPart, YourPrice};
use ui::{Theme, icons, widgets as w};

#[derive(Default)]
pub(super) struct Offers {
    pages: HashMap<(OfferKind, u32), Shared<Offer>>,
    links: HashMap<AppId, Shared<Vec<OfferLink>>>,
    /// `--demo`: what was "added". Nothing reaches a real cart.
    demo_cart: HashSet<(OfferKind, u32)>,
}

impl Offers {
    /// Prices are per country and language.
    pub(super) fn clear(&mut self) {
        self.pages.clear();
        self.links.clear();
    }
}

impl Store {
    fn offer(&mut self, kind: OfferKind, id: u32) -> Option<Result<Offer, String>> {
        if !self.offers.pages.contains_key(&(kind, id)) {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.offer(kind, id).await });
            self.offers.pages.insert((kind, id), slot);
        }
        snapshot(self.offers.pages.get(&(kind, id))?)
    }

    /// You have this part: Steam lists its package, or you have all its games.
    fn part_owned(&self, p: &OfferPart) -> bool {
        let user = self.curators.user();
        if p.package_id != 0 && user.is_some_and(|u| u.owned_packages.contains(&p.package_id)) {
            return true;
        }
        !p.apps.is_empty() && p.apps.iter().all(|a| self.owned.contains(a) || user.is_some_and(|u| u.owned_apps.contains(a)))
    }

    pub(super) fn offer_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, kind: OfferKind, id: u32) {
        let o = match self.offer(kind, id) {
            None => {
                w::loading(ui, t, t!("Loading…"));
                return;
            }
            Some(Err(e)) => {
                w::empty(ui, t, icons::WARNING, t!("This item isn't available"), &i18n::tr_dyn(&e));
                return;
            }
            Some(Ok(o)) => o,
        };
        ui.add(Label::new(w::heading_l(t, &o.name, t.text)).wrap());
        ui.label(w::body_m(t, if kind == OfferKind::Bundle { t!("Bundle") } else { t!("Package") }, t.text_dim));
        ui.add_space(t.dense(18.0));
        let gap = t.dense(14.0);
        let total = ui.available_width();
        let wide = total >= t.dense(820.0);
        let art_w = if wide { (total * 0.42).min(t.dense(616.0)).floor() } else { total };
        let art_h = (art_w * 353.0 / 616.0).floor();
        let art = |ui: &mut Ui| {
            let (r, _) = ui.allocate_exact_size(vec2(art_w, art_h), Sense::hover());
            if !w::paint_cover(ui, r, &o.art, t.corner_small()) {
                w::skeleton(ui, t, r, t.corner_small());
            }
        };
        if wide {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                art(ui);
                ui.vertical(|ui| {
                    ui.set_width(total - art_w - gap);
                    self.offer_buy(ui, t, ctx, &o);
                });
            });
        } else {
            art(ui);
            ui.add_space(gap);
            self.offer_buy(ui, t, ctx, &o);
        }
        ui.add_space(gap);
        self.offer_parts(ui, t, &o);
    }

    /// The price, what owning part of it means, and the button.
    fn offer_buy(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, o: &Offer) {
        let your = o.your_price(|p| self.part_owned(p));
        let have = o.parts.iter().filter(|p| self.part_owned(p)).count();
        let parts = o.parts.len();
        let key = (o.kind, o.id);
        let demo = crate::app::demo();
        let in_cart = self.offers.demo_cart.contains(&key)
            || ctx.state.cart.as_ref().is_some_and(|c| {
                c.iter().any(|l| match o.kind {
                    OfferKind::Bundle => l.bundle_id == o.id,
                    OfferKind::Package => l.bundle_id == 0 && l.package_id == o.id,
                })
            });
        let mut add = false;
        let mut to_cart = false;
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.dense(8.0);
            ui.add(Label::new(w::heading_s(t, t!("Buy {name}", name = o.name), t.text)).wrap());
            if o.bundle_pct > 0 {
                ui.label(w::body_m(t, t!("Bundle discount: {pct}%", pct = o.bundle_pct), t.accent));
            }
            let note = |ui: &mut Ui, text: &str| {
                ui.add(Label::new(w::body_m(t, text, t.text_dim)).wrap());
            };
            match &your {
                YourPrice::Full => {}
                YourPrice::OwnAll => {
                    ui.label(w::heading_s(t, format!("{}  {}", icons::CHECK, t!("You already own everything here")), t.play));
                }
                YourPrice::Complete { missing, text, .. } => {
                    ui.label(w::heading_s(t, t!("Complete your collection"), t.play));
                    note(ui, &t!("You own {have} of {total} items. Get the other {missing} for about {price}. Steam confirms the final price in your cart.", have = have, total = parts, missing = missing, price = text));
                }
                YourPrice::WholeSet { owned } => {
                    note(ui, &t!("You own {have} of {total} items. This is sold only as a complete set, so you'd pay for those too.", have = owned, total = parts));
                }
                YourPrice::InCart { .. } => {
                    note(ui, &t!("You own {have} of {total} items. Steam works out the price of the rest in your cart.", have = have, total = parts));
                }
            }
            if your == YourPrice::OwnAll {
                return;
            }
            ui.add_space(t.spacing.sm);
            if o.cents.is_none() {
                ui.label(w::body_l(t, t!("Not for sale right now"), t.text_dim));
                return;
            }
            let (label, original, discount) = match &your {
                YourPrice::Complete { text, .. } => (format!("~{text}"), None, 0),
                _ => (o.price.clone(), Some(o.before_text.clone()).filter(|b| !b.is_empty() && *b != o.price), o.discount_pct.min(100) as u8),
            };
            let button = if in_cart { t!("In cart").to_string() } else { t!("Add to cart").to_string() };
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::main_cta_rtl(ui, t, &label, original.as_deref(), discount, &button).clicked() {
                        if !in_cart {
                            add = true;
                        } else if !demo {
                            to_cart = true;
                        }
                    }
                });
            });
        });
        if to_cart {
            self.open(View::Cart);
        }
        if add {
            if demo {
                self.offers.demo_cart.insert(key);
                ctx.toast(t!("Demo account: nothing was added to a real cart."));
            } else {
                match o.kind {
                    OfferKind::Bundle => ctx.send(Command::CartAddBundle { bundle_id: o.id, gift: false }),
                    OfferKind::Package => ctx.send(Command::CartAdd { package_id: o.id, gift: false, gift_to: None }),
                }
            }
        }
    }

    fn offer_parts(&mut self, ui: &mut Ui, t: &Theme, o: &Offer) {
        if o.parts.is_empty() {
            return;
        }
        let marks: Vec<bool> = o.parts.iter().map(|p| self.part_owned(p)).collect();
        let mut open = None;
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.dense(7.0);
            ui.label(w::heading_s(t, t!("What's included ({n})", n = o.parts.len()), t.text_dim));
            ui.add_space(t.spacing.xs);
            for (p, owned) in o.parts.iter().zip(&marks) {
                if part_row(ui, t, p, *owned).clicked() {
                    open = p.apps.first().copied();
                }
            }
        });
        if let Some(app) = open {
            self.open(View::App(app));
        }
    }

    /// A game page's other ways to buy it.
    pub(super) fn app_offers(&mut self, ui: &mut Ui, t: &Theme, id: AppId, main: Option<u32>) {
        if !self.offers.links.contains_key(&id) {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.offers_for_app(id).await });
            self.offers.links.insert(id, slot);
        }
        let Some(Ok(links)) = self.offers.links.get(&id).and_then(snapshot) else { return };
        let links: Vec<OfferLink> = links.into_iter().filter(|l| !(l.kind == OfferKind::Package && Some(l.id) == main)).collect();
        if links.is_empty() {
            return;
        }
        let mut open = None;
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.dense(7.0);
            ui.label(w::body_l(t, t!("Bundles and editions"), t.text_dim));
            for l in &links {
                let r = fill_row(ui, t, |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(Label::new(w::body_s(t, &l.price, t.text)).selectable(false));
                        if l.discount_pct > 0 {
                            ui.add(Label::new(w::body_s(t, format!("-{}%", l.discount_pct), t.play)).selectable(false));
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(Label::new(w::body_s(t, &l.name, t.accent)).truncate().selectable(false));
                        });
                    });
                });
                if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    open = Some((l.kind, l.id));
                }
            }
        });
        if let Some((kind, id)) = open {
            self.open(View::Offer(kind, id));
        }
    }
}

/// One included item: its art and name, and its price or that you own it.
fn part_row(ui: &mut Ui, t: &Theme, p: &OfferPart, owned: bool) -> egui::Response {
    let h = t.dense(62.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    ui.painter().rect_filled(rect, t.corner_small(), row_fill(t));
    let art = Rect::from_min_size(rect.min, vec2((h * 460.0 / 215.0).floor(), h));
    if !w::paint_cover(ui, art, &p.art, t.corner_small()) {
        w::skeleton(ui, t, art, t.corner_small());
    }
    let pad = t.dense(14.0);
    let (right, color) = if owned { (format!("{}  {}", icons::CHECK, t!("In your library")), t.play) } else { (p.price.clone(), t.text) };
    let galley = ui.painter().layout_no_wrap(right, t.typography.body_m.font(), color);
    let right_x = rect.right() - pad - galley.size().x;
    ui.painter().galley(pos2(right_x, rect.center().y - galley.size().y / 2.0), galley, color);
    let name = Rect::from_min_max(pos2(art.right() + pad, rect.top()), pos2((right_x - pad).max(art.right() + pad), rect.bottom()));
    let mut c = ui.new_child(UiBuilder::new().max_rect(name).layout(Layout::left_to_right(Align::Center)));
    c.add(Label::new(w::body_l(t, &p.name, t.text)).truncate().selectable(false));
    hover_wash(ui, t, rect, &resp);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

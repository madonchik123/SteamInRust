//! Store: Curators: the ones you follow, the most followed, a curator's
//! page with its reviews, following one, and "Recommended by curators you
//! follow" on game pages.

use super::more::{hover_wash, snapshot};
use super::store_page::{block, fill_row, row_fill};
use super::{Shared, Store, View, thousands};
use client::store_user::StoreUser;
use client::{Command, Event};
use egui::{Align, CornerRadius, CursorIcon, Frame, Label, Layout, Margin, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, SteamId};
use std::collections::{HashMap, HashSet};
use store::curators::{Curator, Pick, Picks, Verdict};
use ui::{Theme, icons, widgets as w};

/// Curators and reviews per request.
const PAGE: u32 = 20;
/// Followed curators shown before "Show all".
const FOLLOWED: usize = 12;
/// `--demo`: real curators the made-up account "follows".
const DEMO_FOLLOWS: [u32; 3] = [1850, 6856269, 4771848];

#[derive(Default)]
pub(super) struct Curators {
    /// Whose data this is.
    account: SteamId,
    user: Option<Result<StoreUser, String>>,
    top: Vec<Shared<(Vec<Curator>, u64)>>,
    info: HashMap<u32, Shared<Curator>>,
    picks: HashMap<u32, Vec<Shared<Picks>>>,
    /// Follows on their way to Steam.
    busy: HashSet<u32>,
    show_all: bool,
}

impl Curators {
    /// Prices in reviews are per country, and text per language.
    pub(super) fn clear(&mut self) {
        self.top.clear();
        self.info.clear();
        self.picks.clear();
    }

    pub(super) fn user(&self) -> Option<&StoreUser> {
        self.user.as_ref()?.as_ref().ok()
    }

    fn follows(&self, id: u32) -> bool {
        self.user().is_some_and(|u| u.followed.contains(&id))
    }

    fn set_followed(&mut self, id: u32, on: bool) {
        if let Some(Ok(u)) = &mut self.user {
            u.followed.retain(|c| *c != id);
            if on {
                u.followed.push(id);
            }
        }
    }
}

impl Store {
    /// Asks once per account for who you follow and what you own.
    pub(super) fn curators_tick(&mut self, ctx: &Ctx) {
        let me = ctx.state.steam_id;
        if self.curators.account == me {
            return;
        }
        self.curators.account = me;
        self.curators.user = None;
        self.curators.busy.clear();
        if me == 0 {
            return;
        }
        if crate::app::demo() {
            self.curators.user = Some(Ok(StoreUser { followed: DEMO_FOLLOWS.to_vec(), ..Default::default() }));
        } else {
            ctx.send(Command::LoadStoreUser);
        }
    }

    pub(super) fn curators_event(&mut self, event: &Event, ctx: &mut Ctx) {
        match event {
            // A failed refresh keeps what's already known.
            Event::StoreUser(r) if r.is_ok() || self.curators.user().is_none() => self.curators.user = Some(r.clone()),
            Event::CuratorFollowed { clan, follow, result } => {
                self.curators.busy.remove(clan);
                match result {
                    Ok(()) => self.curators.set_followed(*clan, *follow),
                    Err(e) => ctx.toast(t!("Couldn't change who you follow: {error}", error = i18n::tr_dyn(e))),
                }
            }
            _ => {}
        }
    }

    fn curator_info(&mut self, id: u32) -> Option<Result<Curator, String>> {
        if !self.curators.info.contains_key(&id) {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.curator(id).await });
            self.curators.info.insert(id, slot);
        }
        snapshot(self.curators.info.get(&id)?)
    }

    fn first_picks(&mut self, id: u32) -> Option<Result<Picks, String>> {
        if !self.curators.picks.contains_key(&id) {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.curator_picks(id, 0, PAGE).await });
            self.curators.picks.insert(id, vec![slot]);
        }
        snapshot(self.curators.picks.get(&id)?.first()?)
    }

    pub(super) fn curators_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.label(w::heading_l(t, t!("Curators"), t.text));
        ui.add(Label::new(w::body_m(t, t!("People and groups who review games. Follow the ones whose taste you trust."), t.text_dim)).wrap());
        ui.add_space(t.dense(22.0));
        if ctx.state.steam_id != 0 {
            self.followed_section(ui, t, ctx);
            ui.add_space(t.dense(26.0));
        }
        ui.label(w::heading_m(t, t!("Popular curators"), t.text));
        ui.add_space(t.dense(10.0));
        if self.curators.top.is_empty() {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.top_curators(0, PAGE).await });
            self.curators.top.push(slot);
        }
        let pages: Vec<_> = self.curators.top.iter().map(snapshot).collect();
        let mut list: Vec<Curator> = Vec::new();
        let mut total = 0;
        let mut failed = None;
        let mut waiting = false;
        for p in pages {
            match p {
                Some(Ok((items, n))) => {
                    total = n;
                    for c in items {
                        if !list.iter().any(|x| x.id == c.id) {
                            list.push(c);
                        }
                    }
                }
                Some(Err(e)) => failed = Some(e),
                None => waiting = true,
            }
        }
        if let (Some(e), true) = (&failed, list.is_empty()) {
            w::empty(ui, t, icons::WARNING, t!("Couldn't load curators"), &i18n::tr_dyn(e));
            if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Md).clicked() {
                self.curators.top.clear();
            }
            return;
        }
        self.curator_grid(ui, t, &list);
        if waiting {
            w::loading(ui, t, t!("Loading…"));
        } else if (list.len() as u64) < total {
            ui.add_space(t.dense(10.0));
            if w::button(ui, t, None, t!("Show more"), w::Kind::Secondary, w::Size::Md).clicked() {
                let (api, start) = (self.api.clone(), self.curators.top.len() as u32 * PAGE);
                let slot = self.fetch(async move { api.top_curators(start, PAGE).await });
                self.curators.top.push(slot);
            }
        }
    }

    fn followed_section(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.label(w::heading_m(t, t!("Curators you follow"), t.text));
        ui.add_space(t.dense(10.0));
        let followed = match &self.curators.user {
            None => {
                w::loading(ui, t, t!("Loading…"));
                return;
            }
            Some(Err(e)) => {
                let e = e.clone();
                ui.add(Label::new(w::body_m(t, t!("Couldn't load who you follow: {error}", error = i18n::tr_dyn(&e)), t.danger)).wrap());
                if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Md).clicked() {
                    self.curators.user = None;
                    ctx.send(Command::LoadStoreUser);
                }
                return;
            }
            Some(Ok(u)) => u.followed.clone(),
        };
        if followed.is_empty() {
            ui.label(w::body_m(t, t!("You're not following any curators yet."), t.text_dim));
            return;
        }
        let shown = if self.curators.show_all { followed.len() } else { followed.len().min(FOLLOWED) };
        let list: Vec<Curator> = followed[..shown].iter().filter_map(|id| self.curator_info(*id)?.ok()).collect();
        self.curator_grid(ui, t, &list);
        if followed.len() > FOLLOWED && !self.curators.show_all {
            ui.add_space(t.dense(10.0));
            if w::button(ui, t, None, &t!("Show all ({n})", n = followed.len()), w::Kind::Secondary, w::Size::Md).clicked() {
                self.curators.show_all = true;
            }
        }
    }

    fn curator_grid(&mut self, ui: &mut Ui, t: &Theme, list: &[Curator]) {
        let gap = t.dense(12.0);
        let total = ui.available_width();
        let cols = (((total + gap) / (t.dense(340.0) + gap)).floor() as usize).max(1);
        let width = ((total - gap * (cols - 1) as f32) / cols as f32).floor();
        let mut open = None;
        ui.spacing_mut().item_spacing.y = gap;
        for row in list.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for c in row {
                    if curator_card(ui, t, c, width).clicked() {
                        open = Some(c.id);
                    }
                }
            });
        }
        if let Some(id) = open {
            self.open(View::Curator(id));
        }
    }

    pub(super) fn curator_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: u32) {
        let c = match self.curator_info(id) {
            None => {
                w::loading(ui, t, t!("Loading…"));
                return;
            }
            Some(Err(e)) => {
                w::empty(ui, t, icons::WARNING, t!("Couldn't load this curator"), &i18n::tr_dyn(&e));
                return;
            }
            Some(Ok(c)) => c,
        };
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = t.dense(18.0);
                let side = t.dense(92.0);
                let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                paint_avatar(ui, t, r, &c.avatar);
                let text_w = (ui.available_width() - t.dense(170.0)).max(t.dense(160.0));
                ui.vertical(|ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing.y = t.dense(6.0);
                    ui.add(Label::new(w::heading_l(t, &c.name, t.text)).wrap());
                    ui.label(w::body_m(t, format!("{}  ·  {}", t!("{n} followers", n = thousands(c.followers)), t!("{n} reviews", n = thousands(c.reviews))), t.text_dim));
                    if !c.description.is_empty() {
                        ui.add(Label::new(w::body_m(t, &c.description, t.text)).wrap());
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| self.follow_button(ui, t, ctx, id));
            });
        });
        ui.add_space(t.dense(18.0));
        ui.label(w::heading_m(t, t!("Reviews"), t.text));
        ui.add_space(t.dense(10.0));
        self.picks_list(ui, t, id);
    }

    fn follow_button(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: u32) {
        // Not until it's known whether you follow them already.
        if ctx.state.steam_id == 0 || self.curators.user().is_none() {
            return;
        }
        if self.curators.busy.contains(&id) {
            ui.add(egui::Spinner::new().color(t.accent));
            return;
        }
        let on = self.curators.follows(id);
        let (icon, label, kind) = if on { (icons::CHECK, t!("Following"), w::Kind::Secondary) } else { (icons::PLUS, t!("Follow"), w::Kind::Primary) };
        if w::button(ui, t, Some(icon), label, kind, w::Size::Md).clicked() {
            if crate::app::demo() {
                self.curators.set_followed(id, !on);
                ctx.toast(t!("Demo account: nothing changed on Steam."));
            } else {
                self.curators.busy.insert(id);
                ctx.send(Command::FollowCurator { clan: id, follow: !on });
            }
        }
    }

    fn picks_list(&mut self, ui: &mut Ui, t: &Theme, id: u32) {
        if let Some(Err(e)) = self.first_picks(id) {
            w::empty(ui, t, icons::WARNING, t!("Couldn't load this curator"), &i18n::tr_dyn(&e));
            return;
        }
        let pages: Vec<_> = self.curators.picks.get(&id).map(|p| p.iter().map(snapshot).collect()).unwrap_or_default();
        let mut items: Vec<Pick> = Vec::new();
        let mut total = 0;
        let mut waiting = false;
        for p in pages {
            match p {
                Some(Ok(page)) => {
                    total = page.total;
                    items.extend(page.items);
                }
                Some(Err(_)) => {}
                None => waiting = true,
            }
        }
        if items.is_empty() && !waiting {
            ui.label(w::body_m(t, t!("This curator hasn't reviewed anything yet."), t.text_dim));
            return;
        }
        let mut open = None;
        ui.spacing_mut().item_spacing.y = t.dense(8.0);
        for (i, p) in items.iter().enumerate() {
            if pick_row(ui, t, p, i).clicked() {
                open = Some(p.app_id);
            }
        }
        if let Some(app) = open {
            self.open(View::App(app));
        }
        if waiting {
            w::loading(ui, t, t!("Loading…"));
        } else if (items.len() as u64) < total {
            ui.add_space(t.dense(6.0));
            if w::button(ui, t, None, t!("Show more"), w::Kind::Secondary, w::Size::Md).clicked() {
                let start = self.curators.picks.get(&id).map_or(0, Vec::len) as u32 * PAGE;
                let api = self.api.clone();
                let slot = self.fetch(async move { api.curator_picks(id, start, PAGE).await });
                self.curators.picks.entry(id).or_default().push(slot);
            }
        }
    }

    /// Curators you follow who recommend this game.
    fn recommenders(&mut self, app: AppId) -> Vec<u32> {
        let Some(u) = self.curators.user() else { return Vec::new() };
        if !crate::app::demo() {
            return u.recommends.get(&app).map(|who| who.iter().copied().filter(|c| u.followed.contains(c)).collect()).unwrap_or_default();
        }
        // The demo account has no real follows: use these curators' real reviews.
        let followed = u.followed.clone();
        followed
            .into_iter()
            .filter(|c| matches!(self.first_picks(*c), Some(Ok(p)) if p.items.iter().any(|x| x.app_id == app && x.verdict == Verdict::Recommended)))
            .collect()
    }

    /// "Recommended by curators you follow", on a game's page.
    pub(super) fn curators_for_app(&mut self, ui: &mut Ui, t: &Theme, app: AppId) {
        let who = self.recommenders(app);
        if who.is_empty() {
            return;
        }
        let list: Vec<Curator> = who.iter().take(5).filter_map(|id| self.curator_info(*id)?.ok()).collect();
        if list.is_empty() {
            return;
        }
        let mut open = None;
        block(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = t.dense(7.0);
            ui.add(Label::new(w::body_l(t, t!("Recommended by curators you follow"), t.text_dim)).wrap());
            for c in &list {
                let r = fill_row(ui, t, |ui| {
                    let side = t.dense(24.0);
                    let (a, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                    paint_avatar(ui, t, a, &c.avatar);
                    ui.add_space(t.spacing.xs);
                    ui.add(Label::new(w::body_s(t, &c.name, t.accent)).truncate().selectable(false));
                });
                if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    open = Some(c.id);
                }
            }
        });
        if let Some(id) = open {
            self.open(View::Curator(id));
        }
    }
}

fn paint_avatar(ui: &Ui, t: &Theme, rect: Rect, url: &str) {
    let radius = CornerRadius::same((rect.width() / 2.0).min(255.0) as u8);
    if url.is_empty() || !w::paint_cover(ui, rect, url, radius) {
        w::skeleton(ui, t, rect, radius);
    }
}

fn curator_card(ui: &mut Ui, t: &Theme, c: &Curator, width: f32) -> egui::Response {
    let h = t.dense(96.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    ui.painter().rect_filled(rect, t.corner_small(), row_fill(t));
    let pad = t.dense(12.0);
    let side = h - 2.0 * pad;
    let avatar = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(side, side));
    paint_avatar(ui, t, avatar, &c.avatar);
    let text = Rect::from_min_max(pos2(avatar.right() + pad, rect.top() + pad), pos2(rect.right() - pad, rect.bottom() - pad));
    let mut col = ui.new_child(UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
    col.spacing_mut().item_spacing.y = t.dense(3.0);
    col.add(Label::new(w::body_l(t, &c.name, t.text)).truncate().selectable(false));
    col.add(Label::new(w::body_s(t, t!("{n} followers", n = thousands(c.followers)), t.text_dim)).truncate().selectable(false));
    if !c.description.is_empty() {
        col.add(Label::new(w::body_s(t, &c.description, t.text_dim)).truncate().selectable(false));
    }
    hover_wash(ui, t, rect, &resp);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn pick_row(ui: &mut Ui, t: &Theme, p: &Pick, index: usize) -> egui::Response {
    let art_w = t.dense(184.0);
    let art_h = (art_w * 215.0 / 460.0).floor();
    let inner = Frame::new().fill(row_fill(t)).corner_radius(t.corner_small()).inner_margin(Margin::same(t.dense(10.0) as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = t.dense(14.0);
            let (r, _) = ui.allocate_exact_size(vec2(art_w, art_h), Sense::hover());
            if !w::paint_cover(ui, r, &p.art, t.corner_small()) {
                w::skeleton(ui, t, r, t.corner_small());
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = t.dense(4.0);
                ui.add(Label::new(w::body_l(t, &p.name, t.text)).truncate().selectable(false));
                let (icon, word, color) = match p.verdict {
                    Verdict::Recommended => (icons::THUMBS_UP, t!("Recommended|curator"), t.play),
                    Verdict::NotRecommended => (icons::THUMBS_DOWN, t!("Not recommended|curator"), t.danger),
                    Verdict::Informational => (icons::INFO, t!("Informational"), t.accent),
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = t.spacing.sm;
                    ui.add(Label::new(w::body_m(t, format!("{icon}  {word}"), color)).selectable(false));
                    if !p.date.is_empty() {
                        ui.add(Label::new(w::body_s(t, &p.date, t.text_dim)).selectable(false));
                    }
                    if !p.price.is_empty() {
                        ui.add(Label::new(w::body_s(t, &p.price, t.text)).selectable(false));
                    }
                });
                if !p.blurb.is_empty() {
                    ui.add(Label::new(w::body_m(t, &p.blurb, t.text_dim)).wrap().selectable(false));
                }
            });
        });
    });
    let rect = inner.response.rect;
    let resp = ui.interact(rect, ui.scope_id().with(("curator-pick", index, p.app_id)), Sense::click());
    hover_wash(ui, t, rect, &resp);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

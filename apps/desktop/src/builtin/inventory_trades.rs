//! Trade offers in the app: the offers you got and sent, accepting,
//! declining and cancelling them, and making a new offer (or a counter
//! offer) by picking items from both inventories.

use super::{Inventory, Sort, View, item_tile_plain as item_tile, skeleton_grid, tag_filters, wallet_currency};
use client::{Command, Event};
use egui::{Align, Align2, CornerRadius, Layout, Margin, Rect, Sense, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, InventoryItem, SteamId, TradeAction, TradeAsset, TradeItem, TradeOffer, TradeState, art};
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Tab {
    #[default]
    Received,
    Sent,
}

#[derive(Default)]
pub(super) struct Trades {
    pub tab: Tab,
    /// Asked Steam for the list since the page was opened.
    pub loaded: bool,
    /// An answer waiting for "are you sure?": (offer, partner, action).
    pub confirm: Option<(u64, SteamId, TradeAction)>,
    /// Offers with an answer on its way.
    pub busy: std::collections::HashSet<u64>,
    pub builder: Option<Builder>,
    /// From a link: start an offer with these items.
    pub pending: Option<Prefill>,
    /// From a link: ask "Accept this trade?" for this offer once it's loaded.
    pub review: Option<u64>,
    /// People whose names were asked for.
    names: std::collections::HashSet<SteamId>,
}

/// A new offer asked for by a link (a plugin, the gift button...).
#[derive(Default, Debug, PartialEq)]
pub(crate) struct Prefill {
    pub partner: SteamId,
    /// (app, context, asset)
    pub give: Vec<(AppId, u64, u64)>,
    pub get: Vec<(AppId, u64, u64)>,
    pub message: String,
}

impl Prefill {
    /// `<partner>`, `<partner>:<app>:<context>:<asset>` (giving that item), or
    /// `<partner>?give=a_c_x,a_c_x&get=a_c_x&msg=<percent-encoded text>`.
    pub fn parse(s: &str) -> Option<Prefill> {
        let (head, query) = s.split_once('?').unwrap_or((s, ""));
        let n: Vec<u64> = head.split(':').filter_map(|x| x.parse().ok()).collect();
        let mut p = Prefill { partner: *n.first().filter(|x| **x > 0)?, ..Default::default() };
        if n.len() == 4 {
            p.give.push((n[1] as AppId, n[2], n[3]));
        }
        let items = |v: &str| -> Vec<(AppId, u64, u64)> {
            v.split(',')
                .filter_map(|x| {
                    let mut it = x.split('_').map(|n| n.parse::<u64>().ok());
                    Some((it.next()?? as AppId, it.next()??, it.next()??))
                })
                .take(256)
                .collect()
        };
        for kv in query.split('&') {
            match kv.split_once('=') {
                Some(("give", v)) => p.give.extend(items(v)),
                Some(("get", v)) => p.get.extend(items(v)),
                Some(("msg", v)) => p.message = decode(v),
                _ => {}
            }
        }
        Some(p)
    }

    pub fn link(&self) -> String {
        let list = |l: &[(AppId, u64, u64)]| l.iter().map(|(a, c, x)| format!("{a}_{c}_{x}")).collect::<Vec<_>>().join(",");
        format!("trade:{}?give={}&get={}&msg={}", self.partner, list(&self.give), list(&self.get), encode(&self.message))
    }
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A new offer being put together.
#[derive(Default)]
pub(super) struct Builder {
    pub partner: Option<SteamId>,
    /// From the partner's trade link (needed when they aren't your friend).
    pub token: Option<String>,
    /// The offer this replaces (a counter offer).
    pub counter: Option<u64>,
    pub give: Vec<(AppId, InventoryItem)>,
    pub get: Vec<(AppId, InventoryItem)>,
    pub message: String,
    mine: Side,
    theirs: Side,
    /// Look up market prices for the items shown.
    prices: bool,
    /// The partner picker's search, or a pasted trade link.
    find: String,
    review: bool,
    sending: bool,
    /// Items asked for by a link, added once their inventory is loaded:
    /// (yours, app, context, asset).
    waiting: Vec<(bool, AppId, u64, u64)>,
}

impl Builder {
    pub fn to(partner: SteamId) -> Builder {
        Builder { partner: Some(partner), prices: true, ..Default::default() }
    }
}

/// How one side's inventory is shown: game, search, sort and tag filters.
#[derive(Default)]
struct Side {
    app: Option<(AppId, u64)>,
    search: String,
    sort: Sort,
    tags: std::collections::BTreeMap<String, std::collections::HashSet<String>>,
}

/// An item's market price in cents, if it's been looked up.
fn price_of(ctx: &Ctx, app: AppId, i: &InventoryItem) -> Option<u64> {
    match ctx.state.market_prices.get(&(app, i.market_hash_name.clone())) {
        Some(Ok(p)) => p.lowest_sell.or_else(|| p.lowest.as_deref().and_then(model::parse_money)),
        _ => None,
    }
}

/// The total of the priced items, and whether every marketable one is priced.
fn value(ctx: &Ctx, items: impl Iterator<Item = (AppId, InventoryItem)>) -> (u64, bool) {
    let (mut total, mut all) = (0, true);
    for (a, i) in items {
        match price_of(ctx, a, &i) {
            Some(p) => total += p * i.amount.max(1),
            None if i.marketable => all = false,
            None => {}
        }
    }
    (total, all)
}

fn money(ctx: &Ctx, cents: u64, complete: bool) -> String {
    let m = model::format_money(cents as i64, wallet_currency(ctx));
    if complete { m } else { format!("≈ {m}+") }
}

fn assets(list: &[(AppId, InventoryItem)]) -> Vec<TradeAsset> {
    list.iter().map(|(a, i)| TradeAsset { app_id: *a, context_id: i.context_id, asset_id: i.asset_id, amount: i.amount.max(1) }).collect()
}

fn lang() -> String {
    i18n::language().steam_name().to_string()
}

impl Inventory {
    pub(super) fn trades_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.steam_id;
        if !self.trades.loaded {
            self.trades.loaded = true;
            ctx.send(Command::LoadTradeOffers { language: lang() });
        }
        if let Some(p) = self.trades.pending.take() {
            let mut b = Builder::to(p.partner);
            b.message = p.message;
            b.waiting = p.give.iter().map(|(a, c, x)| (true, *a, *c, *x)).chain(p.get.iter().map(|(a, c, x)| (false, *a, *c, *x))).collect();
            if let Some((a, c, _)) = p.give.first() {
                b.mine.app = Some((*a, *c));
            }
            if let Some((a, c, _)) = p.get.first() {
                b.theirs.app = Some((*a, *c));
            }
            self.trades.builder = Some(b);
        }
        // Add the linked items as their inventories arrive.
        if let Some(b) = self.trades.builder.as_mut().filter(|b| !b.waiting.is_empty()) {
            let partner = b.partner.unwrap_or(0);
            let mut load = Vec::new();
            b.waiting.retain(|(mine, a, c, x)| {
                let owner = if *mine { me } else { partner };
                let Some(inv) = ctx.state.inventories.get(&(owner, *a, *c)) else {
                    load.push((owner, *a, *c));
                    return true;
                };
                if let Some(i) = inv.items.iter().find(|i| i.asset_id == *x && i.tradable) {
                    let list = if *mine { &mut b.give } else { &mut b.get };
                    if !list.iter().any(|(la, li)| *la == *a && li.asset_id == *x) {
                        list.push((*a, i.clone()));
                    }
                }
                false
            });
            for (owner, a, c) in load {
                if self.requested.insert((owner, a, c)) {
                    ctx.send(Command::LoadInventory { steam_id: owner, app_id: a, context_id: c, language: lang() });
                }
            }
        }
        // "Accept this trade?" asked for by a link, once the offer is known.
        if let Some(id) = self.trades.review {
            if let Some(Ok(o)) = &ctx.state.trade_offers {
                self.trades.review = None;
                if let Some(x) = o.received.iter().find(|x| x.id == id && x.state == TradeState::Active) {
                    self.trades.tab = Tab::Received;
                    self.trades.confirm = Some((x.id, x.partner, TradeAction::Accept));
                }
            }
        }
        egui::Frame::new().inner_margin(Margin::symmetric(28, 22)).show(ui, |ui| {
            if self.trades.builder.is_some() {
                self.trade_builder(ui, t, ctx);
            } else {
                self.offer_list(ui, t, ctx);
            }
        });
        self.trade_confirm(ui, t, ctx);
    }

    fn offer_list(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let offers = ctx.state.trade_offers.clone();
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Inventory"), w::Kind::Ghost, w::Size::Sm).clicked() {
                self.view = View::Items;
            }
            ui.add_space(6.0);
            ui.label(w::text_bold(t!("Trade offers"), 22.0, t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(ui, t, Some(icons::PLUS), t!("New trade offer"), w::Kind::Primary, w::Size::Sm).clicked() {
                    self.trades.builder = Some(Builder { prices: true, ..Default::default() });
                }
                if w::button(ui, t, None, t!("Refresh"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    ctx.send(Command::LoadTradeOffers { language: lang() });
                }
            });
        });
        ui.add_space(12.0);
        let Some(offers) = offers else {
            skeleton_grid(ui, t);
            return;
        };
        let offers = match offers {
            Ok(o) => o,
            Err(e) => {
                w::empty(ui, t, icons::WARNING, t!("Couldn't load your trade offers"), &i18n::tr_dyn(&e));
                return;
            }
        };
        let waiting = |l: &[TradeOffer]| l.iter().filter(|o| o.state.open()).count();
        let (got, sent) = (waiting(&offers.received), waiting(&offers.sent));
        let incoming = if got > 0 { format!("{} ({got})", t!("Incoming")) } else { t!("Incoming").to_string() };
        let outgoing = if sent > 0 { format!("{} ({sent})", t!("Sent")) } else { t!("Sent").to_string() };
        w::segmented(ui, t, &mut self.trades.tab, &[(Tab::Received, &incoming), (Tab::Sent, &outgoing)]);
        ui.add_space(12.0);
        let list = if self.trades.tab == Tab::Received { &offers.received } else { &offers.sent };
        if list.is_empty() {
            let (title, body) = if self.trades.tab == Tab::Received {
                (t!("No trade offers"), t!("Offers people send you show up here."))
            } else {
                (t!("No sent offers"), t!("Offers you send show up here until they're answered."))
            };
            w::empty(ui, t, icons::SWAP, title, body);
            return;
        }
        egui::ScrollArea::vertical().id_salt("trade-offers").auto_shrink([false, false]).show(ui, |ui| {
            for o in list {
                self.offer_card(ui, t, ctx, o);
                ui.add_space(10.0);
            }
        });
    }

    fn offer_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, o: &TradeOffer) {
        let persona = ctx.state.persona(o.partner);
        let name = if persona.name.is_empty() { o.partner.to_string() } else { persona.name.clone() };
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                w::image_or_skeleton(ui, t, r, &art::avatar_full(persona.avatar_hash.as_deref()), CornerRadius::same(5));
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Profile")).clicked() {
                    ctx.navigate("profile", Some(o.partner.to_string()));
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let title = if o.ours { t!("Your offer to {name}", name = name) } else { t!("{name} offered you a trade", name = name) };
                    ui.label(w::text_semibold(title, 14.5, t.text));
                    let mut when = ui::ago(o.created.max(1));
                    if o.state.open() && o.expires > 0 {
                        when = format!("{when} · {}", t!("expires {date}", date = ui::date(o.expires)));
                    }
                    ui.label(w::text(when, 12.0, t.text_faint));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| state_pill(ui, t, o.state));
            });
            if !o.message.trim().is_empty() {
                ui.add_space(6.0);
                egui::Frame::new().fill(t.raised).corner_radius(t.corner_small()).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(w::text(format!("“{}”", o.message.trim()), 13.0, t.text_dim)).wrap());
                });
            }
            ui.add_space(8.0);
            let half = (ui.available_width() - 40.0) / 2.0;
            ui.horizontal_top(|ui| {
                item_column(ui, t, ctx, half, &t!("You get"), &o.get, &t!("Nothing — it's a gift from you"));
                ui.allocate_ui(vec2(28.0, 70.0), |ui| {
                    ui.add_space(30.0);
                    ui.label(w::text(icons::SWAP, 18.0, t.text_faint));
                });
                item_column(ui, t, ctx, half, &t!("You give"), &o.give, &t!("Nothing — it's a gift to you"));
            });
            if o.escrow_end > 0 {
                ui.add_space(4.0);
                ui.label(w::text(format!("{}  {}", icons::CLOCK, t!("Steam holds these items until {date}.", date = ui::date(o.escrow_end))), 12.5, t.away));
            }
            if o.state == TradeState::NeedsConfirmation {
                ui.add_space(4.0);
                ui.label(w::text(format!("{}  {}", icons::CLOCK, t!("Waiting for you to confirm it in the Steam Mobile app.")), 12.5, t.away));
            }
            if !o.state.open() && o.state != TradeState::NeedsConfirmation || o.state == TradeState::NeedsConfirmation && !o.ours {
                return;
            }
            ui.add_space(10.0);
            let busy = self.trades.busy.contains(&o.id);
            ui.horizontal(|ui| {
                if busy {
                    ui.add(egui::Spinner::new().size(16.0));
                    return;
                }
                if !o.ours && o.state == TradeState::Active {
                    if w::button(ui, t, Some(icons::CHECK), t!("Accept"), w::Kind::Primary, w::Size::Sm).clicked() {
                        self.trades.confirm = Some((o.id, o.partner, TradeAction::Accept));
                    }
                    if w::button(ui, t, Some(icons::X), t!("Decline"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        self.trades.confirm = Some((o.id, o.partner, TradeAction::Decline));
                    }
                    if w::button(ui, t, Some(icons::SWAP), t!("Counter offer"), w::Kind::Ghost, w::Size::Sm).on_hover_text(t!("Change the items and send it back")).clicked() {
                        let mut b = Builder::to(o.partner);
                        b.counter = Some(o.id);
                        b.give = o.give.iter().map(|x| (x.app_id, x.item.clone())).collect();
                        b.get = o.get.iter().map(|x| (x.app_id, x.item.clone())).collect();
                        self.trades.builder = Some(b);
                    }
                } else if o.ours && w::button(ui, t, Some(icons::X), t!("Cancel offer"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    self.trades.confirm = Some((o.id, o.partner, TradeAction::Cancel));
                }
                if w::button(ui, t, None, t!("Their inventory"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    ctx.navigate("inventory", Some(o.partner.to_string()));
                }
            });
        });
    }

    /// "Are you sure?" before answering an offer.
    fn trade_confirm(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some((id, partner, action)) = self.trades.confirm else { return };
        let offer = match &ctx.state.trade_offers {
            Some(Ok(o)) => o.received.iter().chain(o.sent.iter()).find(|o| o.id == id).cloned(),
            _ => None,
        };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("trade-confirm")).show(ui.ctx(), |ui| {
            ui.set_width(400.0);
            let (title, body, label, kind) = match action {
                TradeAction::Accept => {
                    let (give, get) = offer.as_ref().map(|o| (o.give.len(), o.get.len())).unwrap_or_default();
                    let body = if give > 0 {
                        t!("You'll give {give} items and get {get}. Steam then asks you to confirm in the Steam Mobile app.", give = give, get = get)
                    } else {
                        t!("You'll get {get} items and give nothing.", get = get)
                    };
                    (t!("Accept this trade?"), body, t!("Accept trade"), w::Kind::Primary)
                }
                TradeAction::Decline => (t!("Decline this trade?"), t!("The offer goes away and nothing is traded.").to_string(), t!("Decline"), w::Kind::Danger),
                TradeAction::Cancel => (t!("Cancel this offer?"), t!("They won't be able to accept it any more.").to_string(), t!("Cancel offer"), w::Kind::Danger),
            };
            ui.label(w::text_bold(title, 18.0, t.text));
            ui.add_space(4.0);
            ui.add(egui::Label::new(w::text(body, 13.0, t.text_dim)).wrap());
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if w::button(ui, t, None, label, kind, w::Size::Md).clicked() {
                    self.trades.busy.insert(id);
                    ctx.send(Command::RespondTradeOffer { id, partner, action });
                    close = true;
                }
                if w::button(ui, t, None, t!("Not now"), w::Kind::Ghost, w::Size::Md).clicked() {
                    close = true;
                }
            });
        });
        if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.trades.confirm = None;
        }
    }

    fn trade_builder(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.steam_id;
        let Some(b) = self.trades.builder.as_mut() else { return };
        let mut close = false;
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Trade offers"), w::Kind::Ghost, w::Size::Sm).clicked() {
                close = true;
            }
            ui.add_space(6.0);
            ui.label(w::text_bold(if b.counter.is_some() { t!("Counter offer") } else { t!("New trade offer") }, 22.0, t.text));
        });
        ui.add_space(12.0);
        let Some(partner) = b.partner else {
            partner_picker(ui, t, ctx, b);
            if close {
                self.trades.builder = None;
            }
            return;
        };
        if close {
            self.trades.builder = None;
            return;
        }
        let persona = ctx.state.persona(partner);
        if persona.name.is_empty() && self.trades.names.insert(partner) {
            ctx.send(Command::LoadPersonas { ids: vec![partner] });
        }
        let Some(b) = self.trades.builder.as_mut() else { return };
        let name = if persona.name.is_empty() { partner.to_string() } else { persona.name.clone() };
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
            w::image_or_skeleton(ui, t, r, &art::avatar(persona.avatar_hash.as_deref()), CornerRadius::same(4));
            ui.label(w::text_semibold(t!("Trading with {name}", name = name), 15.0, t.text));
            if b.counter.is_none() && ui.link(w::text(t!("Change"), 12.5, t.accent)).clicked() {
                b.partner = None;
                b.get.clear();
                b.theirs = Side::default();
            }
        });
        ui.add_space(8.0);
        // Both inventories, side by side.
        let lang = lang();
        for owner in [me, partner] {
            if self.apps_requested.insert(owner) {
                ctx.send(Command::LoadInventoryApps { steam_id: owner });
            }
        }
        let bottom = 104.0;
        let height = (ui.available_height() - bottom).max(220.0);
        let half = (ui.available_width() - 18.0) / 2.0;
        let mut sends = Vec::new();
        let mut wants = Vec::new();
        let Some(b) = self.trades.builder.as_mut() else { return };
        let prices = b.prices;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(half, height), Layout::top_down(Align::Min), |ui| {
                ui.set_width(half);
                trade_side(ui, t, ctx, me, &t!("You give"), &mut b.give, &mut b.mine, prices, &self.requested, &mut sends, &mut wants);
            });
            ui.add_space(18.0);
            ui.allocate_ui_with_layout(vec2(half, height), Layout::top_down(Align::Min), |ui| {
                ui.set_width(half);
                trade_side(ui, t, ctx, partner, &t!("You get"), &mut b.get, &mut b.theirs, prices, &self.requested, &mut sends, &mut wants);
            });
        });
        for (owner, app, cx) in sends {
            if self.requested.insert((owner, app, cx)) {
                ctx.send(Command::LoadInventory { steam_id: owner, app_id: app, context_id: cx, language: lang.clone() });
            }
        }
        self.want_prices(ui, ctx, wants);
        let Some(b) = self.trades.builder.as_mut() else { return };
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut b.message).hint_text(t!("Add a message (optional)")).desired_width((ui.available_width() - 200.0).max(160.0)).margin(vec2(10.0, 8.0)).char_limit(128))
                .with_edit_menu();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let empty = b.give.is_empty() && b.get.is_empty();
                if b.sending {
                    ui.add(egui::Spinner::new().size(18.0));
                } else if ui.add_enabled_ui(!empty, |ui| w::button(ui, t, Some(icons::SWAP), t!("Send offer"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                    b.review = true;
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(w::text(t!("Giving items away needs your confirmation in the Steam Mobile app, like on Steam."), 12.0, t.text_faint));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                w::toggle_chip(ui, t, &format!("{}  {}", icons::COINS, t!("Show prices")), &mut b.prices)
                    .on_hover_text(t!("Look up each item's market price (a few per second, so Steam doesn't block it)"));
            });
        });
        if b.review {
            let mut close = false;
            egui::Modal::new(egui::Id::unique("trade-review")).show(ui.ctx(), |ui| {
                ui.set_width(420.0);
                ui.label(w::text_bold(t!("Send this offer to {name}?", name = name), 18.0, t.text));
                ui.add_space(4.0);
                ui.label(w::text(t!("You give {give} items and get {get}.", give = b.give.len(), get = b.get.len()), 13.0, t.text_dim));
                let (give, all_give) = value(ctx, b.give.iter().cloned());
                let (get, all_get) = value(ctx, b.get.iter().cloned());
                if give > 0 || get > 0 {
                    ui.label(w::text(t!("Market value: you give {give}, you get {get}.", give = money(ctx, give, all_give), get = money(ctx, get, all_get)), 13.0, t.text_dim));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if w::button(ui, t, Some(icons::SWAP), t!("Send offer"), w::Kind::Primary, w::Size::Md).clicked() {
                        b.sending = true;
                        ctx.send(Command::SendTradeOffer { partner, give: assets(&b.give), get: assets(&b.get), message: b.message.trim().to_string(), counter: b.counter, token: b.token.clone() });
                        close = true;
                    }
                    if w::button(ui, t, None, t!("Not now"), w::Kind::Ghost, w::Size::Md).clicked() {
                        close = true;
                    }
                });
            });
            if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                b.review = false;
            }
        }
    }

    pub(super) fn trade_event(&mut self, event: &Event, ctx: &mut Ctx) {
        match event {
            Event::TradeOfferAnswered { id, action, result } => {
                self.trades.busy.remove(id);
                match result {
                    Ok(true) => ctx.toast(t!("Almost done: confirm the trade in the Steam Mobile app.")),
                    Ok(false) => ctx.toast(match action {
                        TradeAction::Accept => t!("Trade accepted"),
                        TradeAction::Decline => t!("Trade declined"),
                        TradeAction::Cancel => t!("Offer cancelled"),
                    }),
                    Err(e) => ctx.toast(t!("Steam didn't take your answer: {error}", error = i18n::tr_dyn(e))),
                }
                ctx.send(Command::LoadTradeOffers { language: lang() });
            }
            Event::TradeOfferSent { result, .. } => {
                if let Some(b) = self.trades.builder.as_mut() {
                    b.sending = false;
                }
                match result {
                    Ok((_, confirm)) => {
                        ctx.toast(if *confirm { t!("Offer made. Confirm it in the Steam Mobile app to send it.") } else { t!("Trade offer sent") });
                        self.trades.builder = None;
                        self.trades.tab = Tab::Sent;
                        ctx.send(Command::LoadTradeOffers { language: lang() });
                    }
                    Err(e) => ctx.toast(t!("Couldn't send the offer: {error}", error = i18n::tr_dyn(e))),
                }
            }
            _ => {}
        }
    }
}

fn state_pill(ui: &mut Ui, t: &Theme, s: TradeState) {
    let (label, color) = match s {
        TradeState::Active => (t!("Waiting"), t.accent),
        TradeState::Accepted => (t!("Accepted"), t.online),
        TradeState::InEscrow => (t!("On hold"), t.away),
        TradeState::NeedsConfirmation => (t!("Needs confirming"), t.away),
        TradeState::Countered => (t!("Countered"), t.text_dim),
        TradeState::Expired => (t!("Expired"), t.text_faint),
        TradeState::Cancelled => (t!("Cancelled"), t.text_faint),
        TradeState::Declined => (t!("Declined"), t.text_faint),
        TradeState::InvalidItems => (t!("Items gone"), t.danger),
        TradeState::Other => (t!("Closed"), t.text_faint),
    };
    w::pill(ui, label, color, color.gamma_multiply(0.15));
}

/// One side of an offer: a title and its items' pictures.
fn item_column(ui: &mut Ui, t: &Theme, ctx: &Ctx, width: f32, title: &str, items: &[TradeItem], none: &str) {
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        let (total, all) = value(ctx, items.iter().map(|x| (x.app_id, x.item.clone())));
        let worth = if total > 0 { format!(" · {}", money(ctx, total, all)) } else { String::new() };
        ui.label(w::text_semibold(format!("{title} ({}){worth}", items.len()), 12.5, t.text_dim));
        ui.add_space(4.0);
        if items.is_empty() {
            ui.label(w::text(none, 12.5, t.text_faint));
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for x in items {
                let price = price_of(ctx, x.app_id, &x.item).map(|p| model::format_money(p as i64, wallet_currency(ctx)));
                let r = item_tile(ui, t, &x.item, x.item.amount, price, 64.0, false);
                if x.missing {
                    ui.painter().rect_filled(r.rect, t.corner_small(), egui::Color32::from_black_alpha(140));
                }
                r.on_hover_ui(|ui| item_hover(ui, t, &x.item));
            }
        });
    });
}

fn item_hover(ui: &mut Ui, t: &Theme, i: &InventoryItem) {
    ui.set_max_width(260.0);
    ui.label(w::text_semibold(&i.name, 13.5, super::hex(i.name_color.as_deref()).unwrap_or(t.text)));
    if !i.kind.is_empty() {
        ui.label(w::text(&i.kind, 12.0, t.text_dim));
    }
}

/// Pick who to trade with: a friend, or anyone by their trade link.
fn partner_picker(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, b: &mut Builder) {
    ui.label(w::text(t!("Who do you want to trade with? Pick a friend, or paste someone's trade link."), 13.5, t.text_dim));
    ui.add_space(8.0);
    w::search(ui, t, &mut b.find, &t!("Search friends or paste a trade link"), 420.0);
    ui.add_space(8.0);
    if let Some((id, token)) = client::trade::parse_link(b.find.trim()) {
        if w::button(ui, t, Some(icons::LINK), t!("Trade with this link"), w::Kind::Primary, w::Size::Sm).clicked() {
            b.partner = Some(id);
            b.token = token;
            b.find.clear();
        }
        return;
    }
    let q = b.find.trim().to_lowercase();
    let friends: Vec<_> = ctx.state.friend_list().into_iter().filter(|f| q.is_empty() || f.name.to_lowercase().contains(&q)).collect();
    if friends.is_empty() {
        ui.label(w::text(t!("No friends match."), 13.0, t.text_faint));
        return;
    }
    egui::ScrollArea::vertical().id_salt("trade-partner").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(520.0);
        for f in friends {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(520.0), 44.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
            }
            let av = Rect::from_min_size(r.min + vec2(6.0, 6.0), vec2(32.0, 32.0));
            w::image_or_skeleton(ui, t, av, &art::avatar(f.avatar_hash.as_deref()), CornerRadius::same(4));
            ui.painter().text(pos2(av.right() + 10.0, r.center().y), Align2::LEFT_CENTER, &f.name, w::font_medium(14.0), t.text);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                b.partner = Some(f.steam_id);
                b.find.clear();
            }
        }
    });
}

/// One side of a new offer: what's in it, then the owner's tradable items
/// to add, by game, with search, tag filters, sorting and prices.
#[allow(clippy::too_many_arguments)]
fn trade_side(
    ui: &mut Ui,
    t: &Theme,
    ctx: &Ctx,
    owner: SteamId,
    title: &str,
    chosen: &mut Vec<(AppId, InventoryItem)>,
    side: &mut Side,
    prices: bool,
    requested: &std::collections::HashSet<(SteamId, AppId, u64)>,
    load: &mut Vec<(SteamId, AppId, u64)>,
    wants: &mut Vec<(AppId, String)>,
) {
    let currency = wallet_currency(ctx);
    let tag = |a: AppId, i: &InventoryItem| if prices { price_of(ctx, a, i).map(|p| model::format_money(p as i64, currency)) } else { None };
    // What's in the offer from this side; click one to take it out.
    egui::Frame::new().fill(t.raised).corner_radius(t.corner_small()).inner_margin(Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(w::text_semibold(format!("{title} ({})", chosen.len()), 13.5, t.text));
            let (total, all) = value(ctx, chosen.iter().cloned());
            if total > 0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(w::text_semibold(money(ctx, total, all), 13.5, t.online)).on_hover_text(t!("Market value of these items"));
                });
            }
        });
        ui.add_space(4.0);
        if chosen.is_empty() {
            ui.label(w::text(t!("Click items below to add them."), 12.5, t.text_faint));
        } else {
            let mut remove = None;
            egui::ScrollArea::vertical().id_salt(("trade-chosen", owner)).max_height(124.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for (k, (a, i)) in chosen.iter().enumerate() {
                        let r = item_tile(ui, t, i, i.amount, tag(*a, i), 56.0, false);
                        if r.on_hover_ui(|ui| {
                            item_hover(ui, t, i);
                            ui.label(w::text(t!("Click to take it out"), 11.5, t.text_faint));
                        })
                        .clicked()
                        {
                            remove = Some(k);
                        }
                    }
                });
            });
            if let Some(k) = remove {
                chosen.remove(k);
            }
        }
    });
    if prices {
        for (a, i) in chosen.iter() {
            if i.marketable && !i.market_hash_name.is_empty() {
                wants.push((*a, i.market_hash_name.clone()));
            }
        }
    }
    ui.add_space(8.0);
    // Games with items, with Steam's icon for each.
    let listed = ctx.state.inventory_apps.get(&owner);
    let tabs: Vec<(AppId, u64, String, Option<String>, u32)> = match listed {
        Some(Ok(apps)) => apps
            .iter()
            .flat_map(|a| {
                let parts: Vec<_> = a.contexts.iter().filter(|c| c.2 > 0).collect();
                let multi = parts.len() > 1;
                parts.into_iter().map(move |c| (a.app_id, c.0, if multi && !c.1.is_empty() { format!("{} · {}", a.name, c.1) } else { a.name.clone() }, a.icon.clone(), c.2))
            })
            .collect(),
        Some(Err(_)) => super::KNOWN.iter().take(4).map(|(a, c, n)| (*a, *c, n.to_string(), None, 0)).collect(),
        None => {
            skeleton_grid(ui, t);
            return;
        }
    };
    if tabs.is_empty() {
        ui.label(w::text(t!("No items to trade."), 13.0, t.text_faint));
        return;
    }
    if side.app.is_none_or(|a| !tabs.iter().any(|x| (x.0, x.1) == a)) {
        side.app = tabs.first().map(|x| (x.0, x.1));
    }
    let (a, cx) = side.app.unwrap_or((753, 6));
    if !requested.contains(&(owner, a, cx)) {
        load.push((owner, a, cx));
    }
    let inv = ctx.state.inventories.get(&(owner, a, cx));
    // One toolbar row: game, search, filters, sort.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        game_picker(ui, t, owner, &tabs, side);
        let rest = ui.available_width() - 8.0 - 104.0 - 8.0 - 132.0;
        w::search(ui, t, &mut side.search, &t!("Search items"), rest.max(120.0));
        let active: usize = side.tags.values().map(|v| v.len()).sum();
        let label = if active > 0 { format!("{}  {} ({active})", icons::FUNNEL, t!("Filters")) } else { format!("{}  {}", icons::FUNNEL, t!("Filters")) };
        let r = w::chip(ui, t, &label, active > 0).on_hover_text(t!("Filter by this game's own item tags"));
        let tradable: Vec<InventoryItem> = inv.map(|i| i.items.iter().filter(|i| i.tradable).cloned().collect()).unwrap_or_default();
        egui::Popup::from_toggle_button_response(&r)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(460.0)
            .show(|ui| tag_filters(ui, t, &mut side.tags, &tradable));
        sort_menu(ui, t, owner, &mut side.sort, prices);
    });
    if !prices && side.sort == Sort::Price {
        side.sort = Sort::Newest;
    }
    ui.add_space(8.0);
    let Some(inv) = inv else {
        skeleton_grid(ui, t);
        return;
    };
    if let Some(e) = &inv.error {
        let msg = if e == "private" { t!("This inventory is private.").to_string() } else { i18n::tr_dyn(e).to_string() };
        ui.label(w::text(msg, 13.0, t.text_faint));
        return;
    }
    let q = side.search.trim().to_lowercase();
    let mut items: Vec<&InventoryItem> = inv
        .items
        .iter()
        .filter(|i| i.tradable)
        .filter(|i| !chosen.iter().any(|(ca, c)| *ca == a && c.asset_id == i.asset_id))
        .filter(|i| q.is_empty() || i.name.to_lowercase().contains(&q) || i.kind.to_lowercase().contains(&q))
        .filter(|i| side.tags.iter().all(|(cat, vals)| vals.is_empty() || i.tags.iter().any(|(c, v, _)| c == cat && vals.contains(v))))
        .collect();
    match side.sort {
        Sort::Newest => {}
        Sort::Name => items.sort_by(|x, y| x.name.to_lowercase().cmp(&y.name.to_lowercase())),
        Sort::Kind => items.sort_by(|x, y| x.kind.cmp(&y.kind).then_with(|| x.name.cmp(&y.name))),
        Sort::Price => items.sort_by_key(|i| std::cmp::Reverse(price_of(ctx, a, i).map(|p| p * i.amount.max(1)))),
    }
    if items.is_empty() {
        ui.label(w::text(t!("No tradable items here."), 13.0, t.text_faint));
        return;
    }
    let want = |i: &InventoryItem, wants: &mut Vec<(AppId, String)>| {
        if prices && i.marketable && !i.market_hash_name.is_empty() {
            wants.push((a, i.market_hash_name.clone()));
        }
    };
    if side.sort == Sort::Price {
        for i in &items {
            want(i, wants);
        }
    }
    let tile = 64.0;
    let gap = 6.0;
    let cols = (((ui.available_width() + gap) / (tile + gap)).floor() as usize).max(1);
    let rows = items.len().div_ceil(cols);
    let mut add = None;
    egui::ScrollArea::vertical().id_salt(("trade-items", owner, a, cx)).auto_shrink([false, false]).show_rows(ui, tile + gap, rows, |ui, range| {
        // Price what's on screen, and a row either side for scrolling.
        let near = range.start.saturating_sub(1)..(range.end + 1).min(rows);
        for row in near {
            for i in items.iter().skip(row * cols).take(cols) {
                want(i, wants);
            }
        }
        for row in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for i in items.iter().skip(row * cols).take(cols) {
                    if item_tile(ui, t, i, i.amount, tag(a, i), tile, false).on_hover_ui(|ui| item_hover(ui, t, i)).clicked() {
                        add = Some((*i).clone());
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
    });
    if let Some(i) = add {
        chosen.push((a, i));
    }
}

/// The game whose items are shown: a button with its icon, name and item
/// count that opens a searchable list (like the redesign's "Filter by game").
fn game_picker(ui: &mut Ui, t: &Theme, owner: SteamId, tabs: &[(AppId, u64, String, Option<String>, u32)], side: &mut Side) {
    let cur = side.app.and_then(|(a, c)| tabs.iter().find(|x| (x.0, x.1) == (a, c))).or(tabs.first());
    let Some(cur) = cur else { return };
    let icon = |x: &(AppId, u64, String, Option<String>, u32)| x.3.clone().unwrap_or_else(|| art::header(x.0));
    let name = if cur.2.chars().count() > 18 { format!("{}…", cur.2.chars().take(17).collect::<String>()) } else { cur.2.clone() };
    let label = if cur.4 > 0 { format!("{name}  {}  {}", cur.4, icons::CARET_DOWN) } else { format!("{name}  {}", icons::CARET_DOWN) };
    let r = w::image_chip(ui, t, Some(&icon(cur)), &label, false).on_hover_text(&cur.2);
    let find_id = egui::Id::unique(("trade-game-find", owner));
    egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(300.0).show(|ui| {
        let mut find: String = ui.data_mut(|d| d.get_temp(find_id).unwrap_or_default());
        if tabs.len() > 6 {
            w::search(ui, t, &mut find, &t!("Search for a game"), 284.0);
            ui.add_space(6.0);
        }
        let q = find.trim().to_lowercase();
        egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
            for x in tabs.iter().filter(|x| q.is_empty() || x.2.to_lowercase().contains(&q)) {
                let on = (x.0, x.1) == (cur.0, cur.1);
                let (row, resp) = ui.allocate_exact_size(vec2(284.0, 32.0), Sense::click());
                if on || resp.hovered() {
                    ui.painter().rect_filled(row, t.corner_small(), if on { t.accent.gamma_multiply(0.15) } else { t.surface_hover });
                }
                let pic = Rect::from_min_size(row.min + vec2(6.0, 6.0), vec2(20.0, 20.0));
                w::image_or_skeleton(ui, t, pic, &icon(x), CornerRadius::same(3));
                let p = ui.painter();
                p.text(pos2(pic.right() + 8.0, row.center().y), Align2::LEFT_CENTER, &x.2, w::font_medium(13.0), if on { t.accent } else { t.text });
                if x.4 > 0 {
                    p.text(pos2(row.right() - 8.0, row.center().y), Align2::RIGHT_CENTER, x.4.to_string(), w::font(12.0), t.text_faint);
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    if !on {
                        side.app = Some((x.0, x.1));
                        side.tags.clear();
                    }
                    find.clear();
                    ui.close();
                }
            }
        });
        ui.data_mut(|d| d.insert_temp(find_id, find));
    });
}

/// Sort order as a small menu (takes less room than a row of tabs).
fn sort_menu(ui: &mut Ui, t: &Theme, owner: SteamId, sort: &mut Sort, prices: bool) {
    let opts = [(Sort::Newest, t!("Newest")), (Sort::Name, t!("Name")), (Sort::Kind, t!("Type")), (Sort::Price, t!("Price"))];
    let cur = opts.iter().find(|o| o.0 == *sort).map(|o| o.1.to_string()).unwrap_or_default();
    let r = w::chip(ui, t, &format!("{}  {cur}  {}", icons::SORT, icons::CARET_DOWN), false).on_hover_text(t!("Sort"));
    let _ = owner;
    egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(160.0).show(|ui| {
        for (o, label) in &opts {
            if *o == Sort::Price && !prices {
                continue;
            }
            if w::menu_item(ui, t, if *o == *sort { icons::CHECK } else { "" }, label).clicked() {
                *sort = *o;
                ui.close();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::Prefill;

    #[test]
    fn trade_links_round_trip() {
        let p = Prefill { partner: 76561197960265770, give: vec![(730, 2, 123)], get: vec![(753, 6, 55), (440, 2, 9)], message: "hi & thanks: 100%!".into() };
        assert_eq!(Prefill::parse(p.link().strip_prefix("trade:").unwrap()), Some(p));
        let one = Prefill::parse("76561197960265770:730:2:5").unwrap();
        assert_eq!(one.give, vec![(730, 2, 5)]);
        assert!(Prefill::parse("nope").is_none());
    }
}

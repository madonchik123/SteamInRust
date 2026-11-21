//! Inventory: buying on the Market, buy orders, gems, booster packs and
//! pending gifts. Whatever spends wallet money or gems, or destroys an item,
//! goes through a dialog that shows exactly what happens and the total.

use super::{Inventory, View};
use client::Command;
use client::economy::{Reply, Request, exchange_out};
use egui::{Align, Align2, Layout, Margin, Sense, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::economy::*;
use model::{AppId, InventoryItem, art, format_money};
use ui::{Theme, icons, widgets as w};

const SUBSCRIBER_AGREEMENT: &str = "https://store.steampowered.com/subscriber_agreement/";
const ADD_FUNDS: &str = "https://store.steampowered.com/steamaccount/addfunds";
const BOOSTER_PAGE: &str = "https://steamcommunity.com/tradingcards/boostercreator/";

/// An item a dialog is about.
#[derive(Clone)]
struct Subject {
    context: u64,
    asset: u64,
    name: String,
    /// A full picture link.
    image: String,
}

enum Dialog {
    Buy { app: AppId, hash: String, name: String, image: String },
    Order { app: AppId, hash: String, name: String, image: String, price: String, quantity: u64 },
    Grind { game: AppId, item: Subject },
    /// Pack gems into sacks (`from_size` 1) or open sacks (`from_size` 1000).
    Exchange { game: AppId, item: Subject, have: u64, from_size: u64, count: u64 },
    MakeBooster { game: BoosterGame, preference: u8 },
    OpenBooster { game: AppId, item: Subject },
    Cards(Vec<BoosterCard>),
    AcceptGift { gift: PendingGift, to_library: bool },
    DeclineGift { gift: PendingGift, note: String },
}

#[derive(Default)]
pub(super) struct Economy {
    dialog: Option<Dialog>,
    /// A request that spends something is out: its button stays off, so one
    /// click can't become two purchases.
    busy: bool,
    waiting_for_phone: bool,
    error: Option<String>,
    /// Steam gave no clear answer, so it may have gone through: no second
    /// try from this dialog.
    uncertain: bool,
    agreed: bool,
    /// The price and wallet were loaded for the dialog that's open now.
    fresh_listing: bool,
    fresh_info: bool,
    boosters_asked: bool,
    gifts_asked: bool,
    steam_items_asked: bool,
    booster_search: String,
}

/// Send a request. The demo answers with sample data and never reaches Steam.
fn submit(ctx: &Ctx, r: Request) {
    if crate::app::demo() {
        super::economy_demo::reply(r, ctx.state);
    } else {
        ctx.send(Command::Economy(r));
    }
}

fn lang() -> String {
    i18n::language().steam_name().to_string()
}

fn money(cents: u64, currency: i32) -> String {
    format_money(cents as i64, currency)
}

fn gems(n: u64) -> String {
    t!("{n} gems", n = crate::builtin::hub::post::number(n as u32)).to_string()
}

impl Inventory {
    /// "Buy" and "Place buy order" for an item on the Market.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn buy_buttons(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, hash: &str, name: &str, image: &str, price: Option<&model::MarketPrice>) {
        if ctx.state.steam_id == 0 {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
            if w::button(ui, t, Some(icons::COINS), t!("Buy…"), w::Kind::Primary, w::Size::Sm).on_hover_text(t!("Buy the cheapest one for sale")).clicked() {
                let e = &mut self.economy;
                (e.fresh_listing, e.fresh_info, e.agreed, e.error) = (false, false, false, None);
                e.dialog = Some(Dialog::Buy { app, hash: hash.to_string(), name: name.to_string(), image: image.to_string() });
                submit(ctx, Request::LoadPurchaseInfo);
                submit(ctx, Request::LoadListing { app_id: app, hash_name: hash.to_string(), name: name.to_string() });
            }
            if w::button(ui, t, Some(icons::TAG), t!("Place buy order…"), w::Kind::Secondary, w::Size::Sm).on_hover_text(t!("Name your price and let Steam buy it when someone sells at that price")).clicked() {
                let e = &mut self.economy;
                (e.fresh_info, e.agreed, e.error) = (false, false, None);
                let start = price.and_then(|p| p.highest_buy.map(|b| b + 1).or(p.lowest_sell)).unwrap_or(0);
                let price = if start > 0 { format!("{:.2}", start as f64 / 100.0) } else { String::new() };
                e.dialog = Some(Dialog::Order { app, hash: hash.to_string(), name: name.to_string(), image: image.to_string(), price, quantity: 1 });
                submit(ctx, Request::LoadPurchaseInfo);
            }
        });
    }

    /// What you can do with one of your own Steam items.
    pub(super) fn item_economy(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, item: &InventoryItem) {
        let actions = item_actions(&item.owner_actions);
        if actions.is_empty() {
            return;
        }
        let subject = Subject { context: item.context_id, asset: item.asset_id, name: item.name.clone(), image: art::econ(&item.icon, 192) };
        ui.add_space(t.spacing.md);
        w::caption(ui, t, t!("Gems and booster packs"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
            for a in actions {
                self.action_button(ui, t, ctx, a, &subject, item.amount.max(1));
            }
        });
    }

    fn action_button(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, action: ItemAction, item: &Subject, amount: u64) {
        let (icon, label) = match action {
            ItemAction::Grind { .. } => (icons::SPARKLE, t!("Turn into gems…")),
            ItemAction::OpenBooster { .. } => (icons::PACKAGE, t!("Open booster pack…")),
            ItemAction::PackGems { .. } => (icons::PACKAGE, t!("Pack into sacks…")),
            ItemAction::UnpackSack { .. } => (icons::STACK, t!("Open sacks…")),
        };
        if !w::button(ui, t, Some(icon), label, w::Kind::Secondary, w::Size::Sm).clicked() {
            return;
        }
        let e = &mut self.economy;
        e.error = None;
        e.dialog = Some(match action {
            ItemAction::Grind { game } => {
                submit(ctx, Request::LoadGemValue { game, context_id: item.context, asset_id: item.asset });
                Dialog::Grind { game, item: item.clone() }
            }
            ItemAction::OpenBooster { game } => Dialog::OpenBooster { game, item: item.clone() },
            ItemAction::PackGems { game } => Dialog::Exchange { game, item: item.clone(), have: amount, from_size: 1, count: (amount / GEMS_PER_SACK).max(1) },
            ItemAction::UnpackSack { game } => Dialog::Exchange { game, item: item.clone(), have: amount, from_size: GEMS_PER_SACK, count: 1 },
        });
    }

    /// A Steam answer for these features.
    pub(super) fn economy_event(&mut self, r: &Reply, ctx: &mut Ctx) {
        let e = &mut self.economy;
        let mut finished: Option<Result<String, String>> = None;
        let mut reload = false;
        match r {
            Reply::PurchaseInfo(_) => e.fresh_info = true,
            Reply::Listing { .. } => e.fresh_listing = true,
            Reply::WaitingForPhone => e.waiting_for_phone = true,
            Reply::Bought { name, result } => {
                finished = Some(result.as_ref().map(|_| t!("Bought {name}. It's in your inventory.", name = name).to_string()).map_err(Clone::clone));
                reload = true;
            }
            Reply::OrderPlaced(result) => {
                finished = Some(result.as_ref().map(|filled| if *filled { t!("Buy order placed, and an item was bought right away.").to_string() } else { t!("Buy order placed").to_string() }).map_err(Clone::clone));
                reload = true;
            }
            Reply::Ground { result, .. } => {
                finished = Some(result.as_ref().map(|n| t!("You got {gems}", gems = gems(*n)).to_string()).map_err(Clone::clone));
                reload = true;
            }
            Reply::Exchanged { from_size, amount_out, result, .. } => {
                let done = if *from_size == 1 { t!("You got {n} sacks of gems", n = amount_out).to_string() } else { t!("You got {gems}", gems = gems(*amount_out)).to_string() };
                finished = Some(result.as_ref().map(|_| done).map_err(Clone::clone));
                reload = true;
            }
            Reply::BoosterMade { result, .. } => {
                finished = Some(result.as_ref().map(|_| t!("Booster pack made. It's in your inventory.").to_string()).map_err(Clone::clone));
                reload = true;
            }
            Reply::BoosterOpened { result, .. } => match result {
                Ok(cards) => {
                    (e.busy, e.waiting_for_phone, e.error) = (false, false, None);
                    e.dialog = Some(Dialog::Cards(cards.clone()));
                    self.reload_after_change(ctx);
                }
                Err(m) => finished = Some(Err(m.clone())),
            },
            Reply::GiftAnswered { accepted, to_library, result, .. } => {
                let done = match (accepted, to_library) {
                    (true, true) => t!("Gift added to your library"),
                    (true, false) => t!("Gift moved to your inventory"),
                    _ => t!("Gift declined"),
                };
                finished = Some(result.as_ref().map(|_| done.to_string()).map_err(Clone::clone));
                reload = *accepted;
            }
            _ => {}
        }
        let Some(outcome) = finished else { return };
        let e = &mut self.economy;
        (e.busy, e.waiting_for_phone) = (false, false);
        match outcome {
            Ok(text) => {
                e.dialog = None;
                e.error = None;
                ctx.toast(text);
                if reload {
                    self.reload_after_change(ctx);
                }
            }
            // The dialog stays open with Steam's reason. If it was closed
            // while waiting for your phone, the reason shows as a message.
            Err(m) => {
                e.agreed = false;
                if e.dialog.is_none() {
                    ctx.toast(i18n::tr_dyn(&m));
                    return;
                }
                e.uncertain = m == client::economy::UNKNOWN_RESULT;
                if !e.uncertain {
                    // Probably sold: show the price that's there now.
                    if let Some(Dialog::Buy { app, hash, name, .. }) = &e.dialog {
                        e.fresh_listing = false;
                        submit(ctx, Request::LoadListing { app_id: *app, hash_name: hash.clone(), name: name.clone() });
                    }
                }
                e.error = Some(m);
            }
        }
    }

    /// Items and orders changed on Steam: show them as they are now.
    fn reload_after_change(&mut self, ctx: &mut Ctx) {
        if crate::app::demo() {
            return;
        }
        let me = ctx.state.steam_id;
        ctx.send(Command::LoadMyListings);
        // Every game's list loads again when it's next shown.
        self.requested.retain(|(owner, _, _)| *owner != me);
        ctx.send(Command::LoadInventory { steam_id: me, app_id: 753, context_id: 6, language: lang() });
        ctx.send(Command::LoadInventoryApps { steam_id: me });
    }

    /// The open dialog, whichever page is showing.
    pub(super) fn economy_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(mut dialog) = self.economy.dialog.take() else { return };
        if self.economy.busy {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
        let mut close = false;
        egui::Modal::new(egui::Id::unique("economy-dialog")).show(ui.ctx(), |ui| {
            ui.set_width(t.dense(440.0));
            let e = &mut self.economy;
            close = match &mut dialog {
                Dialog::Buy { app, hash, name, image } => e.buy(ui, t, ctx, *app, hash, name, image),
                Dialog::Order { app, hash, name, image, price, quantity } => e.order(ui, t, ctx, *app, hash, name, image, price, quantity),
                Dialog::Grind { game, item } => e.grind(ui, t, ctx, *game, item),
                Dialog::Exchange { game, item, have, from_size, count } => e.exchange(ui, t, ctx, *game, item, *have, *from_size, count),
                Dialog::MakeBooster { game, preference } => e.make_booster(ui, t, ctx, game, *preference),
                Dialog::OpenBooster { game, item } => e.open_booster(ui, t, ctx, *game, item),
                Dialog::Cards(cards) => cards_body(ui, t, cards),
                Dialog::AcceptGift { gift, to_library } => e.accept_gift(ui, t, ctx, gift, *to_library),
                Dialog::DeclineGift { gift, note } => e.decline_gift(ui, t, ctx, gift, note),
            };
        });
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        // Nothing closes while a request that spends something is out,
        // except while it waits for your phone (the answer then shows as a
        // message, and new purchases stay off until it arrives).
        let locked = self.economy.busy && !self.economy.waiting_for_phone;
        if locked || !(close || escape) {
            // An answer may have replaced it; that one wins.
            if self.economy.dialog.is_none() {
                self.economy.dialog = Some(dialog);
            }
        } else {
            self.economy.error = None;
            self.economy.uncertain = false;
        }
    }

    /// Gems and booster packs, or pending gifts.
    pub(super) fn economy_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        egui::Frame::new().inner_margin(Margin::symmetric(t.spacing.xl as i8, t.spacing.xl as i8)).show(ui, |ui| {
            let gifts = self.view == View::Gifts;
            ui.horizontal(|ui| {
                if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Inventory"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    self.view = View::Items;
                }
                ui.add_space(t.spacing.sm);
                ui.label(w::heading_s(t, if gifts { t!("Pending gifts") } else { t!("Gems and booster packs") }, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::button(ui, t, Some(icons::REFRESH), t!("Refresh"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        if gifts {
                            self.economy.gifts_asked = false;
                        } else {
                            self.economy.boosters_asked = false;
                            self.economy.steam_items_asked = false;
                        }
                    }
                });
            });
            ui.add_space(t.spacing.lg);
            if gifts {
                self.gifts_page(ui, t, ctx);
            } else {
                self.gems_page(ui, t, ctx);
            }
        });
    }

    fn gems_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if !std::mem::replace(&mut self.economy.boosters_asked, true) {
            submit(ctx, Request::LoadBoosters);
        }
        let me = ctx.state.steam_id;
        if !std::mem::replace(&mut self.economy.steam_items_asked, true) && !crate::app::demo() {
            ctx.send(Command::LoadInventory { steam_id: me, app_id: 753, context_id: 6, language: lang() });
        }
        let catalog = match ctx.state.economy.boosters.clone() {
            None => {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
                w::skeleton(ui, t, r, t.corner());
                return;
            }
            Some(Err(e)) => {
                w::empty(ui, t, icons::WARNING, t!("Couldn't load your booster packs"), &i18n::tr_dyn(&e));
                return;
            }
            Some(Ok(c)) => c,
        };
        // Your gems, and the stacks you can pack or open.
        w::post_card(t).inner_margin(Margin::same(t.spacing.lg as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.label(w::caption_text(t, t!("Your gems")));
                    ui.label(w::heading_m(t, crate::builtin::hub::post::number(catalog.gems as u32), t.text));
                    if catalog.untradable_gems > 0 {
                        ui.label(w::body_s(t, t!("{tradable} tradable, {other} not tradable", tradable = catalog.tradable_gems, other = catalog.untradable_gems), t.text_dim));
                    }
                });
                ui.add_space(t.spacing.xl);
                let stacks: Vec<InventoryItem> = ctx.state.inventories.get(&(me, 753, 6)).map(|i| i.items.iter().filter(|x| item_actions(&x.owner_actions).iter().any(|a| matches!(a, ItemAction::PackGems { .. } | ItemAction::UnpackSack { .. }))).cloned().collect()).unwrap_or_default();
                for item in stacks {
                    let subject = Subject { context: item.context_id, asset: item.asset_id, name: item.name.clone(), image: art::econ(&item.icon, 192) };
                    ui.vertical(|ui| {
                        ui.label(w::body_s(t, format!("{} × {}", item.name, crate::builtin::hub::post::number(item.amount as u32)), t.text_dim));
                        ui.horizontal(|ui| {
                            for a in item_actions(&item.owner_actions) {
                                self.action_button(ui, t, ctx, a, &subject, item.amount.max(1));
                            }
                        });
                    });
                    ui.add_space(t.spacing.lg);
                }
            });
        });
        ui.add_space(t.spacing.lg);
        ui.horizontal(|ui| {
            ui.label(w::heading_s(t, t!("Booster packs you can make"), t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                w::search(ui, t, &mut self.economy.booster_search, &t!("Search games"), 240.0);
            });
        });
        ui.label(w::body_s(t, t!("You can make one pack per game each day, for games you own that have trading cards."), t.text_dim));
        ui.add_space(t.spacing.sm);
        let q = self.economy.booster_search.trim().to_lowercase();
        let shown: Vec<&BoosterGame> = catalog.games.iter().filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q)).collect();
        if shown.is_empty() {
            ui.label(w::body_m(t, t!("No booster packs to show."), t.text_faint));
            return;
        }
        egui::ScrollArea::vertical().id_salt("booster-list").auto_shrink([false, false]).show_rows(ui, t.dense(62.0), shown.len(), |ui, range| {
            for g in &shown[range] {
                let choice = catalog.choice(g);
                let sub = match choice {
                    BoosterChoice::Unavailable => match &g.available_at {
                        Some(when) => t!("Available again {when}", when = when).to_string(),
                        None => t!("Available again tomorrow").to_string(),
                    },
                    BoosterChoice::NotEnoughGems => t!("Not enough gems").to_string(),
                    BoosterChoice::UseSteam => t!("Needs gems that can't be traded. Make this one on Steam.").to_string(),
                    BoosterChoice::Make { .. } => t!("Ready to make").to_string(),
                };
                w::post_card(t).inner_margin(Margin::symmetric(t.spacing.md as i8, t.spacing.sm as i8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(t.dense(84.0), t.dense(40.0)), Sense::hover());
                        w::image_or_skeleton(ui, t, r, &art::header(g.app_id), t.corner_small());
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(w::body_m(t, &g.name, t.text)).truncate());
                            ui.label(w::body_s(t, sub, t.text_dim));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match choice {
                            BoosterChoice::Make { preference } => {
                                if w::button(ui, t, Some(icons::PLUS), t!("Make…"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                    self.economy.error = None;
                                    self.economy.dialog = Some(Dialog::MakeBooster { game: (*g).clone(), preference });
                                }
                                ui.label(w::body_m(t, gems(g.gems), t.text));
                            }
                            BoosterChoice::UseSteam => {
                                if w::button(ui, t, Some(icons::EXTERNAL), t!("Open on Steam"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                    crate::links::open_http(BOOSTER_PAGE);
                                }
                                ui.label(w::body_m(t, gems(g.gems), t.text_dim));
                            }
                            _ => {
                                ui.label(w::body_m(t, gems(g.gems), t.text_faint));
                            }
                        });
                    });
                });
                ui.add_space(t.spacing.xs);
            }
        });
    }

    fn gifts_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if !std::mem::replace(&mut self.economy.gifts_asked, true) {
            submit(ctx, Request::LoadGifts);
        }
        let list = match ctx.state.economy.gifts.clone() {
            None => {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
                w::skeleton(ui, t, r, t.corner());
                return;
            }
            Some(Err(e)) => {
                w::empty(ui, t, icons::WARNING, t!("Couldn't load your gifts"), &i18n::tr_dyn(&e));
                return;
            }
            Some(Ok(l)) => l,
        };
        if list.is_empty() {
            w::empty(ui, t, icons::GIFT, t!("No gifts waiting"), t!("Gifts friends send you show up here."));
            return;
        }
        egui::ScrollArea::vertical().id_salt("pending-gifts").auto_shrink([false, false]).show(ui, |ui| {
            for g in &list {
                w::post_card(t).inner_margin(Margin::same(t.spacing.md as i8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(t.dense(120.0), t.dense(56.0)), Sense::hover());
                        ui.painter().rect_filled(r, t.corner_small(), t.raised);
                        if g.image.is_empty() {
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::GIFT, t.typography.body_l.font(), t.text_dim);
                        } else {
                            w::paint_contain(ui, r, &g.image, Align2::CENTER_CENTER);
                        }
                        ui.vertical(|ui| {
                            ui.label(w::body_m(t, gift_name(g), t.text));
                            ui.label(w::body_s(t, t!("From {name}", name = sender(ctx, g)), t.text_dim));
                            if !g.message.is_empty() {
                                ui.add(egui::Label::new(w::body_s(t, format!("“{}”", g.message), t.text_dim)).wrap());
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if w::button(ui, t, Some(icons::X), t!("Decline…"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                self.economy.error = None;
                                self.economy.dialog = Some(Dialog::DeclineGift { gift: g.clone(), note: String::new() });
                            }
                            if g.can_keep_in_inventory && w::button(ui, t, Some(icons::BACKPACK), t!("Keep in inventory…"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                self.economy.error = None;
                                self.economy.dialog = Some(Dialog::AcceptGift { gift: g.clone(), to_library: false });
                            }
                            if g.can_add_to_library && w::button(ui, t, Some(icons::CHECK), t!("Add to library…"), w::Kind::Primary, w::Size::Sm).clicked() {
                                self.economy.error = None;
                                self.economy.dialog = Some(Dialog::AcceptGift { gift: g.clone(), to_library: true });
                            }
                            if !g.can_add_to_library && !g.can_keep_in_inventory && w::button(ui, t, Some(icons::EXTERNAL), t!("Open on Steam"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                crate::links::open_http(&format!("https://steamcommunity.com/profiles/{}/inventory/#pending_gifts", ctx.state.steam_id));
                            }
                        });
                    });
                });
                ui.add_space(t.spacing.sm);
            }
        });
    }
}

fn gift_name(g: &PendingGift) -> String {
    if g.name.is_empty() { t!("A gift").to_string() } else { g.name.clone() }
}

fn sender(ctx: &Ctx, g: &PendingGift) -> String {
    if !g.sender_name.is_empty() {
        return g.sender_name.clone();
    }
    let p = ctx.state.persona(g.sender);
    if p.name.is_empty() { t!("a friend").to_string() } else { p.name.clone() }
}

/// Picture and name at the top of a dialog.
fn head(ui: &mut Ui, t: &Theme, title: &str, name: &str, image: &str) {
    ui.horizontal(|ui| {
        let side = t.dense(64.0);
        let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
        ui.painter().rect_filled(r, t.corner_small(), t.raised);
        if !image.is_empty() {
            w::paint_contain(ui, r.shrink(t.spacing.xs), image, Align2::CENTER_CENTER);
        }
        ui.vertical(|ui| {
            ui.label(w::heading_s(t, title, t.text));
            ui.add(egui::Label::new(w::body_m(t, name, t.text_dim)).wrap());
        });
    });
    ui.add_space(t.spacing.md);
}

/// The box of sums.
fn sums(ui: &mut Ui, t: &Theme, rows: &[(String, String, bool)]) {
    egui::Frame::new().fill(t.raised).corner_radius(t.corner_small()).inner_margin(Margin::same(t.spacing.md as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for (label, value, strong) in rows {
            ui.horizontal(|ui| {
                ui.label(w::body_s(t, label, t.text_dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(if *strong { w::heading_s(t, value, t.text) } else { w::body_s(t, value, t.text) });
                });
            });
        }
    });
    ui.add_space(t.spacing.sm);
}

fn note(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add(egui::Label::new(w::body_s(t, text, t.text_dim)).wrap());
}

fn warning(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add(egui::Label::new(w::body_s(t, text, t.danger)).wrap());
}

fn loading(ui: &mut Ui, t: &Theme) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.dense(96.0)), Sense::hover());
    w::skeleton(ui, t, r, t.corner_small());
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
}

fn close_only(ui: &mut Ui, t: &Theme) -> bool {
    ui.add_space(t.spacing.md);
    w::button(ui, t, None, t!("Close"), w::Kind::Ghost, w::Size::Md).clicked()
}

impl Economy {
    /// Steam's reason, the confirm button and Cancel. (confirmed, closed)
    fn footer(&mut self, ui: &mut Ui, t: &Theme, label: &str, enabled: bool) -> (bool, bool) {
        if let Some(e) = &self.error {
            warning(ui, t, &i18n::tr_dyn(e));
        }
        if self.waiting_for_phone {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(t.accent));
                ui.add(egui::Label::new(w::body_s(t, t!("Approve this in the Steam Mobile app to finish."), t.text)).wrap());
            });
        }
        ui.add_space(t.spacing.md);
        let (mut go, mut close) = (false, false);
        ui.horizontal(|ui| {
            let enabled = enabled && !self.busy && !self.uncertain;
            ui.add_enabled_ui(enabled, |ui| {
                go = w::button(ui, t, Some(icons::CHECK), label, w::Kind::Primary, w::Size::Md).clicked() && enabled;
            });
            if self.busy {
                ui.add(egui::Spinner::new().color(t.accent));
                if self.waiting_for_phone {
                    close = w::button(ui, t, None, t!("Close"), w::Kind::Ghost, w::Size::Md).clicked();
                }
            } else {
                close = w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked();
            }
        });
        (go, close)
    }

    fn agreement(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal_wrapped(|ui| {
            w::checkbox(ui, t, &mut self.agreed, t.dense(18.0));
            ui.label(w::body_s(t, t!("I agree to the terms of the"), t.text_dim));
            if ui.link(w::body_s(t, t!("Steam Subscriber Agreement"), t.accent)).clicked() {
                crate::links::open_http(SUBSCRIBER_AGREEMENT);
            }
        });
    }

    /// Your wallet details for a purchase, or a reason you can't buy here.
    fn wallet(&self, ui: &mut Ui, t: &Theme, ctx: &Ctx, app: AppId, hash: &str) -> Option<PurchaseInfo> {
        if !self.fresh_info {
            loading(ui, t);
            return None;
        }
        let steam_link = |ui: &mut Ui| {
            if ui.link(w::body_s(t, t!("Open the market page"), t.accent)).clicked() {
                crate::links::open_http(&client::market::listing_url(app, hash));
            }
        };
        match &ctx.state.economy.purchase_info {
            Some(Ok(i)) if i.requires_billing => {
                warning(ui, t, t!("Steam needs your billing address before your first Market purchase. Finish this one on Steam's website."));
                steam_link(ui);
                None
            }
            Some(Ok(i)) => Some(i.clone()),
            Some(Err(e)) => {
                warning(ui, t, &i18n::tr_dyn(e));
                steam_link(ui);
                None
            }
            None => {
                loading(ui, t);
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn buy(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, hash: &str, name: &str, image: &str) -> bool {
        head(ui, t, t!("Buy this item"), name, image);
        let Some(info) = self.wallet(ui, t, ctx, app, hash) else { return close_only(ui, t) };
        if !self.fresh_listing {
            if let Some(e) = &self.error {
                warning(ui, t, &i18n::tr_dyn(e));
            }
            loading(ui, t);
            return close_only(ui, t);
        }
        let listing = match ctx.state.economy.listings.get(&(app, hash.to_string())) {
            Some(Ok(l)) => l.clone(),
            Some(Err(e)) => {
                warning(ui, t, &i18n::tr_dyn(e));
                return close_only(ui, t);
            }
            None => {
                loading(ui, t);
                return close_only(ui, t);
            }
        };
        if listing.currency != info.wallet_currency {
            warning(ui, t, t!("This item's price isn't in your wallet currency, so it can't be bought here."));
            if ui.link(w::body_s(t, t!("Open the market page"), t.accent)).clicked() {
                crate::links::open_http(&client::market::listing_url(app, hash));
            }
            return close_only(ui, t);
        }
        let c = listing.cost(info.tax_percent);
        let cur = info.wallet_currency;
        let mut rows = vec![(t!("Seller receives").to_string(), money(c.subtotal, cur), false)];
        if c.fee > 0 {
            rows.push((t!("Steam fee").to_string(), money(listing.steam_fee, cur), false));
            rows.push((t!("Game fee").to_string(), money(listing.game_fee, cur), false));
        }
        if c.tax > 0 {
            rows.push((t!("Tax").to_string(), money(c.tax, cur), false));
        }
        rows.push((t!("You pay").to_string(), money(c.total, cur), true));
        sums(ui, t, &rows);
        let enough = info.wallet_balance >= c.total as i64;
        if enough {
            note(ui, t, &t!("Your wallet goes from {before} to {after}.", before = format_money(info.wallet_balance, cur), after = format_money(info.wallet_balance - c.total as i64, cur)));
        } else {
            warning(ui, t, &t!("You have {balance} in your wallet, so you need {more} more.", balance = format_money(info.wallet_balance, cur), more = format_money(c.total as i64 - info.wallet_balance, cur)));
            if ui.link(w::body_s(t, t!("Add funds on Steam"), t.accent)).clicked() {
                crate::links::open_http(ADD_FUNDS);
            }
        }
        note(ui, t, t!("This buys one item from the cheapest listing. Purchases on the Market can't be refunded."));
        self.agreement(ui, t);
        let (go, close) = self.footer(ui, t, &t!("Buy for {price}", price = money(c.total, cur)), enough && self.agreed);
        if go {
            (self.busy, self.error) = (true, None);
            submit(ctx, Request::BuyListing { listing, info });
        }
        close
    }

    #[allow(clippy::too_many_arguments)]
    fn order(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, hash: &str, name: &str, image: &str, price: &mut String, quantity: &mut u64) -> bool {
        head(ui, t, t!("Place a buy order"), name, image);
        let Some(info) = self.wallet(ui, t, ctx, app, hash) else { return close_only(ui, t) };
        let cur = info.wallet_currency;
        if let Some(Ok(l)) = &ctx.state.my_listings {
            if l.buy_orders.iter().any(|o| o.app_id == app && o.market_hash_name == hash) {
                warning(ui, t, t!("You already have a buy order for this item. Cancel it first to place a new one."));
                return close_only(ui, t);
            }
        }
        ui.horizontal(|ui| {
            ui.label(w::body_s(t, t!("Price per item"), t.text_dim));
            ui.add(egui::TextEdit::singleline(price).desired_width(t.dense(110.0)).margin(vec2(t.spacing.sm, t.spacing.xs)));
            ui.label(w::body_s(t, model::currency(cur).0, t.text_faint));
            ui.add_space(t.spacing.md);
            ui.label(w::body_s(t, t!("Quantity"), t.text_dim));
            ui.add(egui::DragValue::new(quantity).range(1..=5000));
        });
        ui.add_space(t.spacing.sm);
        let each = super::market::parse_amount(price).unwrap_or(0);
        let tax = order_tax(each, *quantity, info.tax_percent);
        let total = order_total(each, *quantity, info.tax_percent);
        let mut rows = vec![(t!("{n} × {price}", n = *quantity, price = money(each, cur)).to_string(), money(each * *quantity, cur), false)];
        if tax > 0 {
            rows.push((t!("Tax").to_string(), money(tax, cur), false));
        }
        rows.push((t!("The most you can pay").to_string(), money(total, cur), true));
        sums(ui, t, &rows);
        let high_enough = each >= info.min_order_price();
        let enough = info.wallet_balance >= total as i64;
        if each > 0 && !high_enough {
            warning(ui, t, &t!("The lowest price Steam allows is {price}.", price = money(info.min_order_price(), cur)));
        } else if each > 0 && !enough {
            warning(ui, t, &t!("You have {balance} in your wallet, so you need {more} more.", balance = format_money(info.wallet_balance, cur), more = format_money(total as i64 - info.wallet_balance, cur)));
            if ui.link(w::body_s(t, t!("Add funds on Steam"), t.accent)).clicked() {
                crate::links::open_http(ADD_FUNDS);
            }
        }
        note(ui, t, t!("Steam buys for you whenever someone sells at or below your price, until the quantity is reached or you cancel the order. Fees are included in your price."));
        self.agreement(ui, t);
        let label = t!("Place order, up to {price}", price = money(total, cur));
        let (go, close) = self.footer(ui, t, &label, each > 0 && high_enough && enough && self.agreed);
        if go {
            (self.busy, self.error) = (true, None);
            submit(ctx, Request::PlaceBuyOrder { app_id: app, hash_name: hash.to_string(), price: each, quantity: *quantity, info });
        }
        close
    }

    fn grind(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, game: AppId, item: &Subject) -> bool {
        head(ui, t, t!("Turn into gems"), &item.name, &item.image);
        let value = match ctx.state.economy.gem_values.get(&item.asset) {
            Some(Ok(v)) => *v,
            Some(Err(e)) => {
                warning(ui, t, &i18n::tr_dyn(e));
                return close_only(ui, t);
            }
            None => {
                loading(ui, t);
                return close_only(ui, t);
            }
        };
        sums(ui, t, &[(t!("You lose").to_string(), item.name.clone(), false), (t!("You get").to_string(), gems(value), true)]);
        warning(ui, t, t!("The item is destroyed. This can't be undone."));
        let (go, close) = self.footer(ui, t, &t!("Destroy it for {gems}", gems = gems(value)), true);
        if go {
            (self.busy, self.error) = (true, None);
            submit(ctx, Request::Grind { game, context_id: item.context, asset_id: item.asset, expected: value });
        }
        close
    }

    #[allow(clippy::too_many_arguments)]
    fn exchange(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, game: AppId, item: &Subject, have: u64, from_size: u64, count: &mut u64) -> bool {
        let packing = from_size == 1;
        head(ui, t, if packing { t!("Pack gems into sacks") } else { t!("Open sacks of gems") }, &item.name, &item.image);
        let max = if packing { have / GEMS_PER_SACK } else { have };
        if max == 0 {
            warning(ui, t, &t!("You need at least {gems} to fill a sack.", gems = gems(GEMS_PER_SACK)));
            return close_only(ui, t);
        }
        *count = (*count).clamp(1, max);
        ui.horizontal(|ui| {
            ui.label(w::body_s(t, t!("Sacks"), t.text_dim));
            ui.add(egui::DragValue::new(count).range(1..=max));
            ui.label(w::body_s(t, t!("of {n}", n = max), t.text_faint));
        });
        ui.add_space(t.spacing.sm);
        let sacks = *count;
        let all_gems = sacks * GEMS_PER_SACK;
        let rows = if packing {
            [(t!("You use").to_string(), gems(all_gems), false), (t!("You get").to_string(), t!("{n} sacks of gems", n = sacks).to_string(), true)]
        } else {
            [(t!("You use").to_string(), t!("{n} sacks of gems", n = sacks).to_string(), false), (t!("You get").to_string(), gems(all_gems), true)]
        };
        sums(ui, t, &rows);
        note(ui, t, t!("One sack holds 1,000 gems. You can pack or open sacks again later."));
        let label = if packing { t!("Pack {n} sacks", n = sacks) } else { t!("Open {n} sacks", n = sacks) };
        let (to_size, amount_in) = if packing { (GEMS_PER_SACK, all_gems) } else { (1, sacks) };
        let valid = exchange_out(amount_in, from_size, to_size).is_some();
        let (go, close) = self.footer(ui, t, &label, valid);
        if go {
            (self.busy, self.error) = (true, None);
            submit(ctx, Request::ExchangeGems { game, asset_id: item.asset, from_size, to_size, amount_in });
        }
        close
    }

    fn make_booster(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, game: &BoosterGame, preference: u8) -> bool {
        head(ui, t, t!("Make a booster pack"), &game.name, &art::header(game.app_id));
        let left = match &ctx.state.economy.boosters {
            Some(Ok(c)) => c.gems.saturating_sub(game.gems),
            _ => 0,
        };
        sums(ui, t, &[(t!("You spend").to_string(), gems(game.gems), true), (t!("Gems left").to_string(), crate::builtin::hub::post::number(left as u32), false)]);
        if preference == 1 {
            note(ui, t, t!("Only your tradable gems are used, so the pack can be traded."));
        }
        note(ui, t, t!("You get one booster pack of three cards. You can make one pack for this game each day. Gems spent can't be returned."));
        let (go, close) = self.footer(ui, t, &t!("Spend {gems}", gems = gems(game.gems)), true);
        if go {
            (self.busy, self.error) = (true, None);
            submit(ctx, Request::MakeBooster { app_id: game.app_id, series: game.series, preference });
        }
        close
    }

    fn open_booster(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, game: AppId, item: &Subject) -> bool {
        head(ui, t, t!("Open booster pack"), &item.name, &item.image);
        warning(ui, t, t!("The pack is used up and you get three trading cards. This can't be undone."));
        let (go, close) = self.footer(ui, t, t!("Open pack"), true);
        if go {
            (self.busy, self.error) = (true, None);
            submit(ctx, Request::OpenBooster { game, asset_id: item.asset });
        }
        close
    }

    fn accept_gift(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, gift: &PendingGift, to_library: bool) -> bool {
        head(ui, t, if to_library { t!("Add gift to your library") } else { t!("Keep gift in your inventory") }, &gift_name(gift), &gift.image);
        ui.label(w::body_s(t, t!("From {name}", name = sender(ctx, gift)), t.text_dim));
        ui.add_space(t.spacing.sm);
        note(ui, t, if to_library { t!("The gift becomes part of your account for good. It can't be traded or given to anyone else afterwards.") } else { t!("The gift moves to your inventory. You can add it to your library later.") });
        let (go, close) = self.footer(ui, t, if to_library { t!("Add to library") } else { t!("Keep in inventory") }, true);
        if go {
            (self.busy, self.error) = (true, None);
            submit(ctx, Request::AcceptGift { id: gift.id, to_library });
        }
        close
    }

    fn decline_gift(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, gift: &PendingGift, message: &mut String) -> bool {
        head(ui, t, t!("Decline this gift"), &gift_name(gift), &gift.image);
        let who = sender(ctx, gift);
        warning(ui, t, &t!("{name} gets the gift back, and you can't accept it afterwards.", name = who));
        ui.add_space(t.spacing.sm);
        ui.label(w::body_s(t, t!("A note to them (optional)"), t.text_dim));
        ui.add(egui::TextEdit::multiline(message).desired_rows(2).desired_width(f32::INFINITY));
        let (go, close) = self.footer(ui, t, t!("Decline gift"), true);
        if go {
            (self.busy, self.error) = (true, None);
            submit(ctx, Request::DeclineGift { id: gift.id, sender: gift.sender, note: message.trim().to_string() });
        }
        close
    }
}

/// What came out of a booster pack.
fn cards_body(ui: &mut Ui, t: &Theme, cards: &[BoosterCard]) -> bool {
    ui.label(w::heading_s(t, t!("Your new cards"), t.text));
    ui.add_space(t.spacing.md);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.spacing.md, t.spacing.md);
        for c in cards {
            ui.vertical(|ui| {
                ui.set_width(t.dense(128.0));
                let (r, _) = ui.allocate_exact_size(vec2(t.dense(128.0), t.dense(150.0)), Sense::hover());
                ui.painter().rect_filled(r, t.corner_small(), t.raised);
                if !c.image.is_empty() {
                    w::paint_contain(ui, r.shrink(t.spacing.xs), &c.image, Align2::CENTER_CENTER);
                }
                ui.add(egui::Label::new(w::body_s(t, &c.name, t.text)).wrap());
                if c.foil {
                    w::pill(ui, t!("Foil"), t.accent, t.raised);
                }
            });
        }
    });
    close_only(ui, t)
}

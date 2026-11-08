//! The item panel's market side: prices, order book, price history, selling,
//! gifting to a friend, and your own listings. Selling goes through Steam's
//! market with your signed-in session; Steam then asks you to confirm each
//! listing in the Steam Mobile app, the same as on its website.

use super::{Inventory, hex, wallet_currency};
use client::Command;
use egui::{Align, Align2, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, InventoryItem, ListingStatus, MarketPrice, art, fees, format_money};
use ui::{Theme, icons, widgets as w};

/// "Put up for sale" dialog.
pub struct SellDialog {
    app: AppId,
    context: u64,
    asset: u64,
    name: String,
    icon: String,
    name_color: Option<String>,
    max_amount: u64,
    amount: u64,
    /// What the buyer pays, as typed.
    pays: String,
    game_fee: f64,
}

fn money(cents: u64, currency: i32) -> String {
    format_money(cents as i64, currency)
}

/// "12,50" / "12.5" / "12" -> cents.
fn parse_amount(s: &str) -> Option<u64> {
    let s = s.trim().replace(' ', "").replace(',', ".");
    let s: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let v: f64 = s.parse().ok()?;
    (v > 0.0).then(|| (v * 100.0).round() as u64)
}

impl Inventory {
    /// Right-hand panel for the selected item.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn detail(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        ctx: &mut Ctx,
        item: &InventoryItem,
        count: u64,
        app: AppId,
        mine: bool,
    ) {
        egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(t.corner()).inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let wdt = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(wdt, wdt * 0.7), Sense::hover());
            let bg = hex(item.background.as_deref()).map(|c| w::mix(t.raised, c, 0.3)).unwrap_or(t.raised);
            ui.painter().rect_filled(r, t.corner_small(), bg);
            let icon = item.icon_large.as_deref().unwrap_or(&item.icon);
            // The full picture, scaled up to fill the panel.
            if !w::paint_contain(ui, r.shrink(8.0), &art::econ(icon, 512), Align2::CENTER_CENTER) {
                w::skeleton(ui, t, r.shrink(8.0), t.corner_small());
            }
            ui.add_space(10.0);
            ui.add(egui::Label::new(w::text_bold(&item.name, 18.0, hex(item.name_color.as_deref()).unwrap_or(t.text))).wrap());
            if !item.kind.is_empty() {
                ui.label(w::text(&item.kind, 13.0, t.text_dim));
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let (ok, no) = (t.online, t.text_faint);
                w::pill(ui, if item.tradable { t!("Tradable") } else { t!("Not tradable") }, if item.tradable { ok } else { no }, t.raised);
                w::pill(ui, if item.marketable { t!("Marketable") } else { t!("Not marketable") }, if item.marketable { ok } else { no }, t.raised);
                if count > 1 {
                    w::pill(ui, &t!("You have {n}", n = count), t.text, t.raised);
                }
            });
            // Holds and owner-only notes ("Tradable After ...").
            for note in &item.owner_notes {
                w::markup_label(ui, note.trim(), 12.5, t.away);
            }
            if !item.tradable && item.trade_hold_days > 0 && item.owner_notes.is_empty() {
                ui.label(w::text(t!("New items of this kind can't be traded for {n} days.", n = item.trade_hold_days), 12.5, t.text_faint));
            }

            if item.marketable && !item.market_hash_name.is_empty() {
                ui.add_space(14.0);
                self.market_section(ui, t, ctx, item, count, app, mine);
            } else if mine {
                ui.add_space(12.0);
                let why = if item.market_hold_days > 0 && !item.marketable {
                    t!("This item can be sold on the Market once its {n}-day hold ends.", n = item.market_hold_days).to_string()
                } else {
                    t!("This item can't be sold on the Community Market.").to_string()
                };
                ui.label(w::text(why, 12.5, t.text_faint));
            }

            if mine && item.tradable {
                ui.add_space(12.0);
                w::caption(ui, t, t!("Trade or gift"));
                ui.add(egui::Label::new(w::text(t!("Send it to a friend as a gift or part of a trade. Trade offers open on Steam's website for now."), 12.5, t.text_dim)).wrap());
                ui.add_space(4.0);
                if w::button(ui, t, Some(icons::GIFT), t!("Gift to a friend…"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    self.gift = Some(item.asset_id);
                }
            }

            if !item.description.is_empty() {
                ui.add_space(12.0);
                w::caption(ui, t, t!("Description"));
                // Steam descriptions mix BBCode ([color=…]) and HTML.
                for (line, color) in &item.description {
                    if !line.trim().is_empty() {
                        w::markup_label(ui, line.trim(), 12.5, hex(color.as_deref()).unwrap_or(t.text_dim));
                    }
                }
            }
            if !item.tags.is_empty() {
                ui.add_space(12.0);
                w::caption(ui, t, t!("Tags"));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for (cat, tag, color) in &item.tags {
                        let on = self.tags.get(cat).is_some_and(|s| s.contains(tag));
                        let r = w::tinted_chip(ui, t, tag, hex(color.as_deref()), on).on_hover_text(t!("{category} — click to show only these", category = cat));
                        if r.clicked() {
                            self.tags.entry(cat.clone()).or_default().insert(tag.clone());
                        }
                    }
                });
            }
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                for (label, link) in &item.actions {
                    if !label.is_empty() && !link.is_empty() && w::button(ui, t, Some(icons::EXTERNAL), label, w::Kind::Ghost, w::Size::Sm).clicked() {
                        let _ = open::that_detached(link);
                    }
                }
                if w::button(ui, t, Some(icons::COPY), t!("Copy item ID"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    ui.ctx().copy_text(item.asset_id.to_string());
                    ctx.toast(t!("Item ID copied"));
                }
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn market_section(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        ctx: &mut Ctx,
        item: &InventoryItem,
        count: u64,
        app: AppId,
        mine: bool,
    ) {
        let currency = wallet_currency(ctx);
        let key = (app, item.market_hash_name.clone());
        if self.full_asked.insert(key.clone()) {
            ctx.send(Command::MarketPrice {
                app_id: app,
                hash_name: item.market_hash_name.clone(),
                currency,
                country: ctx.state.country.clone(),
                full: true,
            });
        }
        w::caption(ui, t, t!("Community Market"));
        let price = match ctx.state.market_prices.get(&key) {
            None => {
                for wd in [160.0, 220.0, 120.0] {
                    let (r, _) = ui.allocate_exact_size(vec2(wd, 14.0), Sense::hover());
                    w::skeleton(ui, t, r, CornerRadius::same(4));
                }
                return;
            }
            Some(Err(e)) => {
                ui.label(w::text(i18n::tr_dyn(e), 12.5, t.text_faint));
                if ui.link(w::text(t!("Try again"), 12.5, t.accent)).clicked() {
                    self.full_asked.remove(&key);
                }
                market_link(ui, t, app, &item.market_hash_name);
                return;
            }
            Some(Ok(p)) => p.clone(),
        };
        let lowest = price
            .lowest_sell
            .map(|c| money(c, currency))
            .or(price.lowest.clone());
        ui.horizontal(|ui| {
            ui.label(w::text(t!("Starting at"), 13.0, t.text_dim));
            ui.label(w::text_bold(
                lowest.clone().unwrap_or_else(|| "—".into()),
                20.0,
                t.text,
            ));
        });
        let mut facts = Vec::new();
        if let Some(m) = &price.median {
            facts.push(t!("average {price}", price = m).to_string());
        }
        if let Some(v) = price.volume {
            facts.push(t!("{n} sold in 24 hours", n = v).to_string());
        }
        if let Some(n) = price.sell_count {
            facts.push(t!("{n} for sale", n = n).to_string());
        }
        if !facts.is_empty() {
            ui.label(w::text(facts.join(" · "), 12.5, t.text_dim));
        }
        match (price.highest_buy, price.buy_count) {
            (Some(b), n) => {
                ui.label(w::text(
                    t!(
                        "Highest buy order: {price} ({n} requests)",
                        price = money(b, currency),
                        n = n.unwrap_or(0)
                    ),
                    12.5,
                    t.text_dim,
                ));
            }
            (None, Some(_)) => {
                ui.label(w::text(t!("No buy orders right now"), 12.5, t.text_faint));
            }
            _ => {}
        }
        if !price.history.is_empty() {
            ui.add_space(8.0);
            chart(ui, t, &price, currency);
        }
        if !price.sell_ladder.is_empty() || !price.buy_ladder.is_empty() {
            ui.add_space(6.0);
            ladders(ui, t, &price, currency);
        }

        let rules = price.fee_rules.unwrap_or(ctx.state.fee_rules);
        let game_fee = item.market_fee.unwrap_or(rules.game_default);
        if mine {
            ui.add_space(10.0);
            let min_pay = fees::buyer_pays(1, game_fee, &rules);
            // Steam extensions' two shortcuts: undercut the cheapest listing by
            // one unit, or sell straight to the highest buy order.
            let quick = price
                .lowest_sell
                .or_else(|| price.lowest.as_deref().and_then(model::parse_money))
                .map(|l| l.saturating_sub(1).max(min_pay));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if let Some(q) = quick {
                    let r = w::button(
                        ui,
                        t,
                        Some(icons::LIGHTNING),
                        &t!("Quick sell · {price}", price = money(q, currency)),
                        w::Kind::Primary,
                        w::Size::Sm,
                    )
                    .on_hover_text(t!(
                        "List it just below the cheapest listing, so it sells first."
                    ));
                    if r.clicked() {
                        self.open_sell(item, count, app, game_fee, q, currency);
                    }
                }
                if let Some(b) = price.highest_buy.filter(|b| *b >= min_pay) {
                    let r = w::button(
                        ui,
                        t,
                        Some(icons::COINS),
                        &t!("Sell now · {price}", price = money(b, currency)),
                        w::Kind::Secondary,
                        w::Size::Sm,
                    )
                    .on_hover_text(t!(
                        "List it at the highest buy order, so it sells right away."
                    ));
                    if r.clicked() {
                        self.open_sell(item, count, app, game_fee, b, currency);
                    }
                }
                if w::button(
                    ui,
                    t,
                    Some(icons::TAG),
                    t!("Sell…"),
                    w::Kind::Secondary,
                    w::Size::Sm,
                )
                .clicked()
                {
                    self.open_sell(
                        item,
                        count,
                        app,
                        game_fee,
                        quick.unwrap_or(min_pay),
                        currency,
                    );
                }
            });
        }
        ui.add_space(4.0);
        market_link(ui, t, app, &item.market_hash_name);
    }

    fn open_sell(
        &mut self,
        item: &InventoryItem,
        count: u64,
        app: AppId,
        game_fee: f64,
        pays: u64,
        currency: i32,
    ) {
        let _ = currency;
        self.sell = Some(SellDialog {
            app,
            context: item.context_id,
            asset: item.asset_id,
            name: item.name.clone(),
            icon: item.icon.clone(),
            name_color: item.name_color.clone(),
            // Grouped tiles stand for several assets; sell from this stack only.
            max_amount: item.amount.max(1).min(count.max(1)),
            amount: 1,
            pays: format!("{:.2}", pays as f64 / 100.0),
            game_fee,
        });
    }

    pub(super) fn sell_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(d) = &mut self.sell else { return };
        let currency = wallet_currency(ctx);
        let rules = ctx.state.fee_rules;
        let mut close = false;
        let mut send = None;
        egui::Modal::new(egui::Id::unique("sell-item")).show(ui.ctx(), |ui| {
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().rect_filled(r, t.corner_small(), t.raised);
                w::paint_contain(ui, r.shrink(4.0), &art::econ(&d.icon, 192), Align2::CENTER_CENTER);
                ui.vertical(|ui| {
                    ui.label(w::text_bold(t!("Put up for sale"), 18.0, t.text));
                    ui.label(w::text_semibold(&d.name, 14.0, hex(d.name_color.as_deref()).unwrap_or(t.text)));
                });
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(w::text(t!("Buyer pays"), 13.5, t.text_dim));
                ui.add(egui::TextEdit::singleline(&mut d.pays).desired_width(120.0).margin(vec2(10.0, 6.0)));
                ui.label(w::text(model::currency(currency).0, 12.5, t.text_faint));
            });
            if d.max_amount > 1 {
                ui.horizontal(|ui| {
                    ui.label(w::text(t!("Quantity"), 13.5, t.text_dim));
                    ui.add(egui::DragValue::new(&mut d.amount).range(1..=d.max_amount));
                    ui.label(w::text(t!("of {n}", n = d.max_amount), 12.5, t.text_faint));
                });
            }
            let pays = parse_amount(&d.pays).unwrap_or(0);
            let receive = fees::you_receive(pays, d.game_fee, &rules);
            let (steam_fee, game_fee) = fees::split(receive, d.game_fee, &rules);
            ui.add_space(8.0);
            egui::Frame::new().fill(t.raised).corner_radius(t.corner_small()).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let row = |ui: &mut Ui, k: &str, v: String, strong: bool| {
                    ui.horizontal(|ui| {
                        ui.label(w::text(k, 13.0, t.text_dim));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(if strong { w::text_bold(v, 15.0, t.text) } else { w::text(v, 13.0, t.text) });
                        });
                    });
                };
                row(ui, t!("Steam fee"), money(steam_fee, currency), false);
                row(ui, t!("Game fee"), money(game_fee, currency), false);
                let each = if d.amount > 1 { format!("  ×{}", d.amount) } else { String::new() };
                row(ui, t!("You receive"), format!("{}{each}", money(receive, currency)), true);
            });
            let valid = receive > 0;
            if !valid && pays > 0 {
                ui.label(w::text(t!("That's below the lowest price Steam allows."), 12.5, t.danger));
            }
            ui.add_space(6.0);
            ui.add(egui::Label::new(w::text(t!("Steam will ask you to confirm the listing in the Steam Mobile app. Money from sales goes to your Steam Wallet."), 12.0, t.text_faint)).wrap());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(valid, |ui| {
                    if w::button(ui, t, Some(icons::TAG), t!("Put up for sale"), w::Kind::Primary, w::Size::Md).clicked() {
                        send = Some(Command::SellItem { app_id: d.app, context_id: d.context, asset_id: d.asset, amount: d.amount, receive });
                    }
                });
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                    close = true;
                }
            });
        });
        if let Some(cmd) = send {
            ctx.send(cmd);
            ctx.toast(t!("Listing your item…"));
            close = true;
        }
        if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.sell = None;
        }
    }

    pub(super) fn gift_picker(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if self.gift.is_none() {
            return;
        }
        let mut close = false;
        let friends = ctx.state.friend_list();
        egui::Modal::new(egui::Id::unique("gift-item")).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.label(w::text_bold(t!("Gift to a friend"), 18.0, t.text));
            ui.add(egui::Label::new(w::text(t!("Pick a friend. A trade offer to them opens on Steam's website; add this item and send it."), 12.5, t.text_dim)).wrap());
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                if friends.is_empty() {
                    ui.label(w::text(t!("Your friends list is empty."), 13.0, t.text_faint));
                }
                for f in &friends {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
                    }
                    let av = Rect::from_min_size(r.min + vec2(6.0, 6.0), vec2(28.0, 28.0));
                    w::image_or_skeleton(ui, t, av, &art::avatar(f.avatar_hash.as_deref()), CornerRadius::same(4));
                    ui.painter().text(pos2(av.right() + 10.0, r.center().y), Align2::LEFT_CENTER, &f.name, w::font_medium(13.5), t.text);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        let account = f.steam_id & 0xFFFF_FFFF;
                        let _ = open::that_detached(format!("https://steamcommunity.com/tradeoffer/new/?partner={account}"));
                        close = true;
                    }
                }
            });
            ui.add_space(8.0);
            if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                close = true;
            }
        });
        if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.gift = None;
        }
    }

    /// Your listings (active, waiting for confirmation, on hold) and buy orders.
    pub(super) fn listings_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let data = match &ctx.state.my_listings {
            None => {
                for _ in 0..5 {
                    let (r, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
                    w::skeleton(ui, t, r.shrink2(vec2(0.0, 4.0)), t.corner_small());
                }
                return;
            }
            Some(Err(e)) => {
                w::empty(
                    ui,
                    t,
                    icons::WARNING,
                    t!("Couldn't load your listings"),
                    &i18n::tr_dyn(e),
                );
                return;
            }
            Some(Ok(d)) => d.clone(),
        };
        ui.horizontal(|ui| {
            let total: u64 = data
                .listings
                .iter()
                .filter(|l| l.status == ListingStatus::Active)
                .map(|l| l.you_receive * l.amount)
                .sum();
            let currency = data
                .listings
                .first()
                .map(|l| l.currency)
                .unwrap_or_else(|| wallet_currency(ctx));
            ui.label(w::text(
                t!(
                    "You'll receive {total} if everything sells",
                    total = money(total, currency)
                ),
                13.0,
                t.text_dim,
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(
                    ui,
                    t,
                    Some(icons::REFRESH),
                    t!("Refresh"),
                    w::Kind::Ghost,
                    w::Size::Sm,
                )
                .clicked()
                {
                    ctx.send(Command::LoadMyListings);
                }
                if w::button(
                    ui,
                    t,
                    Some(icons::EXTERNAL),
                    t!("Market history"),
                    w::Kind::Ghost,
                    w::Size::Sm,
                )
                .on_hover_text(t!("Opens in your browser"))
                .clicked()
                {
                    let _ = open::that_detached("https://steamcommunity.com/market/#myhistory");
                }
            });
        });
        ui.add_space(8.0);
        if data.listings.is_empty() && data.buy_orders.is_empty() {
            w::empty(
                ui,
                t,
                icons::STOREFRONT,
                t!("Nothing on the market"),
                t!("Items you put up for sale show up here."),
            );
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("my-listings")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let groups = [
                    (
                        ListingStatus::NeedsConfirmation,
                        t!("Waiting for confirmation"),
                        t!("Confirm these in the Steam Mobile app to put them on sale."),
                    ),
                    (ListingStatus::Active, t!("On sale"), ""),
                    (
                        ListingStatus::OnHold,
                        t!("On hold"),
                        t!("Steam is holding these for a few days before they go on sale."),
                    ),
                ];
                for (status, title, hint) in groups {
                    let rows: Vec<_> = data
                        .listings
                        .iter()
                        .filter(|l| l.status == status)
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.label(w::text_bold(
                        format!("{title} ({})", rows.len()),
                        15.0,
                        t.text,
                    ));
                    if !hint.is_empty() {
                        ui.label(w::text(hint, 12.5, t.text_faint));
                    }
                    ui.add_space(4.0);
                    for l in rows {
                        let sub = if l.created > 0 {
                            format!("{} · {}", l.game, ui::ago(l.created))
                        } else {
                            l.game.clone()
                        };
                        row(
                            ui,
                            t,
                            &l.icon,
                            &l.name,
                            l.name_color.as_deref(),
                            &sub,
                            |ui| {
                                if w::button(
                                    ui,
                                    t,
                                    Some(icons::X),
                                    t!("Remove"),
                                    w::Kind::Ghost,
                                    w::Size::Sm,
                                )
                                .clicked()
                                {
                                    ctx.send(Command::CancelListing {
                                        listing_id: l.listing_id,
                                    });
                                }
                                ui.vertical(|ui| {
                                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                                        ui.label(w::text_bold(
                                            money(l.buyer_pays, l.currency),
                                            14.0,
                                            t.text,
                                        ));
                                        ui.label(w::text(
                                            t!(
                                                "you get {price}",
                                                price = money(l.you_receive, l.currency)
                                            ),
                                            12.0,
                                            t.text_dim,
                                        ));
                                    });
                                });
                            },
                        );
                    }
                    ui.add_space(14.0);
                }
                if !data.buy_orders.is_empty() {
                    ui.label(w::text_bold(
                        format!("{} ({})", t!("Buy orders"), data.buy_orders.len()),
                        15.0,
                        t.text,
                    ));
                    ui.add_space(4.0);
                    for o in &data.buy_orders {
                        let sub = t!("{left} of {n} left", left = o.remaining, n = o.quantity);
                        row(ui, t, &o.icon, &o.name, None, &sub, |ui| {
                            if w::button(
                                ui,
                                t,
                                Some(icons::X),
                                t!("Cancel"),
                                w::Kind::Ghost,
                                w::Size::Sm,
                            )
                            .clicked()
                            {
                                ctx.send(Command::CancelBuyOrder {
                                    order_id: o.order_id,
                                });
                            }
                            ui.label(w::text_bold(money(o.price, o.currency), 14.0, t.text));
                        });
                    }
                }
            });
    }
}

fn row(
    ui: &mut Ui,
    t: &Theme,
    icon: &str,
    name: &str,
    color: Option<&str>,
    sub: &str,
    right: impl FnOnce(&mut Ui),
) {
    egui::Frame::new()
        .fill(t.surface)
        .corner_radius(t.corner_small())
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(4), t.raised);
                if !icon.is_empty() {
                    w::paint_contain(
                        ui,
                        r.shrink(3.0),
                        &art::econ(icon, 192),
                        Align2::CENTER_CENTER,
                    );
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(
                        egui::Label::new(w::text_semibold(
                            name,
                            14.0,
                            hex(color).unwrap_or(t.text),
                        ))
                        .truncate(),
                    );
                    ui.label(w::text(sub, 12.0, t.text_dim));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), right);
            });
        });
    ui.add_space(6.0);
}

fn market_link(ui: &mut Ui, t: &Theme, app: AppId, hash: &str) {
    if ui
        .link(w::text(
            format!("{}  {}", icons::EXTERNAL, t!("Open the market page")),
            12.5,
            t.accent,
        ))
        .on_hover_text(t!("Opens in your browser"))
        .clicked()
    {
        let _ = open::that_detached(client::market::listing_url(app, hash));
    }
}

/// Average sale price over time, with a range switch. Steam reports recent
/// sales hour by hour; they're grouped (per day, or per 6 hours for a week)
/// so the line shows the trend instead of every single sale.
fn chart(ui: &mut Ui, t: &Theme, p: &MarketPrice, currency: i32) {
    let id = ui.scope_id().with("price-range");
    let mut days: u64 = ui.data(|d| d.get_temp(id)).unwrap_or(30);
    ui.label(w::text_semibold(t!("Average sale price"), 12.5, t.text_dim));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (d, label) in [
            (7u64, "1W"),
            (30, "1M"),
            (90, "3M"),
            (365, "1Y"),
            (0, t!("All")),
        ] {
            if w::chip(ui, t, label, days == d).clicked() {
                days = d;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(id, days));

    let last = p.history.last().map(|h| h.0).unwrap_or(0);
    let bucket: u64 = if days > 0 && days <= 7 {
        6 * 3600
    } else {
        86_400
    };
    // (bucket start, weighted average price, units sold)
    let mut points: Vec<(u64, f64, u32)> = Vec::new();
    for h in p
        .history
        .iter()
        .filter(|h| days == 0 || h.0 + days * 86_400 >= last)
    {
        let key = h.0 / bucket * bucket;
        let weight = h.2.max(1) as f64;
        match points.last_mut() {
            Some(pt) if pt.0 == key => {
                let total = pt.2.max(1) as f64;
                pt.1 = (pt.1 * total + h.1 * weight) / (total + weight);
                pt.2 += h.2.max(1);
            }
            _ => points.push((key, h.1, h.2.max(1))),
        }
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, t.corner_small(), t.raised);
    if points.len() < 2 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            t!("Not enough sales yet"),
            w::font(12.0),
            t.text_faint,
        );
        return;
    }
    // Scale with headroom; ignore the top 3% so one odd sale can't flatten it.
    let mut sorted: Vec<f64> = points.iter().map(|h| h.1).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let top = sorted[((sorted.len() as f64 * 0.97) as usize).min(sorted.len() - 1)];
    let bottom = sorted[0];
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let span = (top - bottom).max(mean * 0.12).max(0.01);
    let (lo, hi) = ((bottom - span * 0.15).max(0.0), top + span * 0.15);
    let inner = Rect::from_min_max(rect.min + vec2(8.0, 20.0), rect.max - vec2(8.0, 18.0));
    let (t0, t1) = (points[0].0 as f64, points[points.len() - 1].0 as f64);
    let at = |h: &(u64, f64, u32)| {
        let x = inner.left() + ((h.0 as f64 - t0) / (t1 - t0).max(1.0)) as f32 * inner.width();
        let y = inner.bottom() - (((h.1.min(hi) - lo) / (hi - lo)) as f32) * inner.height();
        pos2(x, y)
    };
    let line: Vec<_> = points.iter().map(|h| at(h)).collect();
    // Soft fill under the line.
    let mesh_color = t.accent.gamma_multiply(0.12);
    for pair in line.windows(2) {
        let quad = vec![
            pair[0],
            pair[1],
            pos2(pair[1].x, inner.bottom()),
            pos2(pair[0].x, inner.bottom()),
        ];
        painter.add(egui::Shape::convex_polygon(quad, mesh_color, Stroke::NONE));
    }
    painter.add(egui::Shape::line(line.clone(), Stroke::new(2.0, t.accent)));
    let cents = |v: f64| money((v * 100.0).round() as u64, currency);
    painter.text(
        rect.left_top() + vec2(8.0, 4.0),
        Align2::LEFT_TOP,
        cents(hi),
        w::font(10.5),
        t.text_faint,
    );
    painter.text(
        rect.left_bottom() + vec2(8.0, -4.0),
        Align2::LEFT_BOTTOM,
        cents(lo),
        w::font(10.5),
        t.text_faint,
    );
    if let Some(pos) = resp.hover_pos() {
        let i = line
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1.x - pos.x)
                    .abs()
                    .partial_cmp(&(b.1.x - pos.x).abs())
                    .unwrap()
            })
            .map(|x| x.0)
            .unwrap_or(0);
        let h = points[i];
        painter.vline(
            line[i].x,
            inner.y_range(),
            Stroke::new(1.0, t.text_faint.gamma_multiply(0.5)),
        );
        painter.circle_filled(line[i], 4.0, t.accent);
        let label = format!(
            "{} · {} · {}",
            ui::date(h.0),
            cents(h.1),
            t!("{n} sold", n = h.2)
        );
        painter.text(
            pos2(rect.center().x, rect.top() + 4.0),
            Align2::CENTER_TOP,
            label,
            w::font_semibold(11.0),
            t.text,
        );
    }
}

/// Cheapest listings next to the highest buy orders, as a compact table.
fn ladders(ui: &mut Ui, t: &Theme, p: &MarketPrice, currency: i32) {
    let rows = p.sell_ladder.len().max(p.buy_ladder.len()).min(5);
    let line = 18.0;
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), line * (rows as f32 + 1.0) + 4.0),
        Sense::hover(),
    );
    let painter = ui.painter();
    let half = rect.width() / 2.0;
    let sides = [
        (t!("For sale"), &p.sell_ladder, t.text),
        (t!("Buy orders"), &p.buy_ladder, t.online),
    ];
    for (k, (title, list, color)) in sides.into_iter().enumerate() {
        let left = rect.left() + k as f32 * half;
        let right = left + half - 12.0;
        painter.text(
            pos2(left, rect.top() + line / 2.0),
            Align2::LEFT_CENTER,
            title,
            w::font_semibold(11.5),
            t.text_dim,
        );
        if list.is_empty() {
            painter.text(
                pos2(left, rect.top() + line * 1.5),
                Align2::LEFT_CENTER,
                "—",
                w::font(12.0),
                t.text_faint,
            );
        }
        for (i, (price, n)) in list.iter().take(rows).enumerate() {
            let y = rect.top() + line * (i as f32 + 1.5);
            painter.text(
                pos2(left, y),
                Align2::LEFT_CENTER,
                money(*price, currency),
                w::font(12.0),
                color,
            );
            painter.text(
                pos2(right, y),
                Align2::RIGHT_CENTER,
                n.to_string(),
                w::font(12.0),
                t.text_faint,
            );
        }
    }
}

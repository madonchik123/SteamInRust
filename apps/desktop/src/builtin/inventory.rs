//! Steam inventory, native (Econ.GetInventoryItemsWithDescriptions over the
//! Steam connection). Works for your own inventory and for public ones.
//! Open with `ctx.navigate("inventory", Some("<steamid>"))` or
//! `"<steamid>/<appid>"` to jump to one game.

use client::{Command, Event};
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2,
};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, InventoryItem, Relationship, SteamId, art};
use std::collections::{BTreeMap, HashSet, VecDeque};
use ui::{Theme, icons, widgets as w};

#[path = "inventory_market.rs"]
mod market;

/// Games with community inventories: (app, context, name). 753/6 is Steam's
/// own inventory (trading cards, backgrounds, emoticons, boosters).
const KNOWN: &[(AppId, u64, &str)] = &[
    (753, 6, "Steam"),
    (730, 2, "Counter-Strike 2"),
    (570, 2, "Dota 2"),
    (440, 2, "Team Fortress 2"),
    (252490, 2, "Rust"),
    (578080, 2, "PUBG: BATTLEGROUNDS"),
    (218620, 2, "PAYDAY 2"),
    (304930, 2, "Unturned"),
    (232090, 2, "Killing Floor 2"),
    (322330, 1, "Don't Starve Together"),
    (2923300, 2, "Banana"),
];

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Sort {
    #[default]
    Newest,
    Name,
    Kind,
    Price,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Items,
    Listings,
}

#[derive(Default)]
pub struct Inventory {
    owner: Option<SteamId>,
    app: Option<(AppId, u64)>,
    selected: Option<u64>,
    search: String,
    tradable_only: bool,
    marketable_only: bool,
    sort: Sort,
    requested: HashSet<(SteamId, AppId, u64)>,
    games_requested: HashSet<SteamId>,
    apps_requested: HashSet<SteamId>,
    /// Game asked for by a link, resolved once the list of games is known.
    want_app: Option<AppId>,
    view: View,
    /// Tag filters: category -> chosen values (any value in a category,
    /// every category must match), like Steam's advanced filters.
    tags: BTreeMap<String, HashSet<String>>,
    group: bool,
    show_prices: bool,
    price_queue: VecDeque<(AppId, String)>,
    price_asked: HashSet<(AppId, String)>,
    full_asked: HashSet<(AppId, String)>,
    next_price_at: f64,
    sell: Option<market::SellDialog>,
    gift: Option<u64>,
    listings_requested: bool,
}

impl Extension for Inventory {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "inventory",
            name: "Inventory",
            author: "SteamInRust",
            description: "Steam inventory",
            core: true,
            ..Manifest::DEFAULT
        }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![
            PageSpec::new("inventory", "Inventory", icons::BACKPACK)
                .hidden()
                .order(960),
        ]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        if arg == Some("listings") {
            self.owner = None;
            self.view = View::Listings;
            return;
        }
        let mut parts = arg.unwrap_or_default().split('/');
        let owner = parts.next().and_then(|s| s.parse().ok());
        let app = parts.next().and_then(|s| s.parse::<AppId>().ok());
        if owner != self.owner {
            self.owner = owner;
            self.app = None;
            self.selected = None;
            self.search.clear();
            self.tags.clear();
            self.view = View::Items;
        }
        if let Some(app) = app {
            self.want_app = Some(app);
            self.app = None;
            self.selected = None;
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        let owner = self.owner.filter(|o| *o != 0).unwrap_or(ctx.state.steam_id);
        let mine = owner == ctx.state.steam_id;
        if owner == 0 {
            w::empty(
                ui,
                &t,
                icons::BACKPACK,
                t!("Sign in to see your inventory"),
                "",
            );
            return;
        }
        let lang = i18n::language().steam_name().to_string();
        if !mine && self.games_requested.insert(owner) {
            ctx.send(Command::LoadPlayerGames {
                steam_id: owner,
                language: lang.clone(),
            });
        }

        if self.apps_requested.insert(owner) {
            ctx.send(Command::LoadInventoryApps { steam_id: owner });
        }
        // One tab per inventory section, from Steam's own list of games with
        // items. Only if that list can't be read, probe well-known games.
        let listed = ctx.state.inventory_apps.get(&owner);
        let fallback = matches!(listed, Some(Err(_)));
        let tabs: Vec<(AppId, u64, String, Option<u32>)> = match listed {
            None => {
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(28, 22))
                    .show(ui, |ui| {
                        self.header(ui, &t, ctx, owner, mine, &[]);
                        skeleton_grid(ui, &t);
                    });
                return;
            }
            Some(Ok(apps)) => apps
                .iter()
                .flat_map(|a| {
                    let sections: Vec<_> = a.contexts.iter().filter(|c| c.2 > 0).collect();
                    let sections = if sections.is_empty() {
                        a.contexts.iter().take(1).collect()
                    } else {
                        sections
                    };
                    let multi = sections.len() > 1;
                    sections.into_iter().map(move |c| {
                        let title = if multi && !c.1.is_empty() {
                            format!("{} · {}", a.name, c.1)
                        } else {
                            a.name.clone()
                        };
                        (a.app_id, c.0, title, Some(c.2))
                    })
                })
                .collect(),
            Some(Err(_)) => KNOWN
                .iter()
                .filter(|(app, _, _)| {
                    *app == 753
                        || if mine {
                            ctx.state.apps.as_ref().is_none_or(|a| a.contains_key(app))
                        } else {
                            match ctx.state.player_games.get(&owner) {
                                Some(Ok(g)) => g.iter().any(|g| g.app_id == *app),
                                _ => true,
                            }
                        }
                })
                .map(|(app, cx, name)| {
                    (
                        *app,
                        *cx,
                        ctx.state
                            .app_name(*app)
                            .map(String::from)
                            .unwrap_or_else(|| name.to_string()),
                        None,
                    )
                })
                .collect(),
        };
        if fallback {
            for (app, cx, _, _) in &tabs {
                if self.requested.insert((owner, *app, *cx)) {
                    ctx.send(Command::LoadInventory {
                        steam_id: owner,
                        app_id: *app,
                        context_id: *cx,
                        language: lang.clone(),
                    });
                }
            }
        }
        // A game asked for by link, then the biggest inventory first.
        if let Some(want) = self.want_app.take() {
            if let Some(tab) = tabs.iter().find(|x| x.0 == want) {
                self.app = Some((tab.0, tab.1));
            } else {
                self.app = Some((
                    want,
                    KNOWN.iter().find(|k| k.0 == want).map(|k| k.1).unwrap_or(2),
                ));
            }
        }
        if self.app.is_none() {
            if !fallback {
                self.app = tabs.first().map(|x| (x.0, x.1));
            } else if tabs
                .iter()
                .all(|(a, c, _, _)| ctx.state.inventories.contains_key(&(owner, *a, *c)))
            {
                let best = tabs
                    .iter()
                    .filter_map(|(a, c, _, _)| {
                        ctx.state
                            .inventories
                            .get(&(owner, *a, *c))
                            .map(|i| (*a, *c, i.items.len()))
                    })
                    .filter(|x| x.2 > 0)
                    .max_by_key(|x| x.2);
                self.app = Some(best.map(|b| (b.0, b.1)).unwrap_or((753, 6)));
            }
        }
        if tabs.is_empty() && !fallback {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(28, 22))
                .show(ui, |ui| {
                    self.header(ui, &t, ctx, owner, mine, &tabs);
                    w::empty(
                        ui,
                        &t,
                        icons::BACKPACK,
                        t!("No items here"),
                        t!("This inventory is empty."),
                    );
                });
            return;
        }
        let (app, cx) = self.app.unwrap_or((753, 6));
        if self.requested.insert((owner, app, cx)) {
            ctx.send(Command::LoadInventory {
                steam_id: owner,
                app_id: app,
                context_id: cx,
                language: lang.clone(),
            });
        }

        egui::Frame::new().inner_margin(Margin::symmetric(28, 22)).show(ui, |ui| {
            self.header(ui, &t, ctx, owner, mine, &tabs);
            // Game tabs with counts.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for (a, c, title, listed) in &tabs {
                    let inv = ctx.state.inventories.get(&(owner, *a, *c));
                    let count = match (listed, inv) {
                        (Some(n), _) => n.to_string(),
                        (None, Some(i)) if i.error.is_some() => "—".to_string(),
                        (None, Some(i)) => i.total.to_string(),
                        (None, None) => "…".to_string(),
                    };
                    // Steam's own icon for the game, else its header art.
                    let icon = match ctx.state.inventory_apps.get(&owner) {
                        Some(Ok(list)) => list.iter().find(|x| x.app_id == *a).and_then(|x| x.icon.clone()),
                        _ => None,
                    }
                    .unwrap_or_else(|| art::header(*a));
                    if w::image_chip(ui, &t, Some(&icon), &format!("{title}  {count}"), self.view == View::Items && (*a, *c) == (app, cx)).clicked() {
                        if (*a, *c) != (app, cx) {
                            self.tags.clear();
                        }
                        self.app = Some((*a, *c));
                        self.selected = None;
                        self.view = View::Items;
                    }
                }
                if mine {
                    let n = match &ctx.state.my_listings {
                        Some(Ok(l)) => format!("  {}", l.listings.len() + l.buy_orders.len()),
                        _ => String::new(),
                    };
                    if w::chip(ui, &t, &format!("{}  {}{n}", icons::STOREFRONT, t!("My listings")), self.view == View::Listings).clicked() {
                        self.view = View::Listings;
                        ctx.send(Command::LoadMyListings);
                    }
                }
            });
            if mine && !self.listings_requested {
                self.listings_requested = true;
                ctx.send(Command::LoadMyListings);
            }
            if self.view == View::Listings {
                ui.add_space(12.0);
                self.listings_page(ui, &t, ctx);
                return;
            }
            ui.add_space(12.0);

            let Some(inv) = ctx.state.inventories.get(&(owner, app, cx)).cloned() else {
                skeleton_grid(ui, &t);
                return;
            };
            if let Some(err) = &inv.error {
                if err == "private" {
                    w::empty(ui, &t, icons::LOCK_SIMPLE, t!("This inventory is private"), t!("Its owner has chosen not to show it."));
                } else {
                    w::empty(ui, &t, icons::WARNING, t!("Couldn't load this inventory"), err);
                }
                return;
            }
            if inv.items.is_empty() {
                w::empty(ui, &t, icons::BACKPACK, t!("No items here"), t!("Items from this game will show up here."));
                return;
            }

            // Toolbar: search, quick filters, game filters, prices, sort.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
                w::search(ui, &t, &mut self.search, &t!("Search items"), 240.0);
                ui.add_space(6.0);
                w::toggle_chip(ui, &t, t!("Tradable"), &mut self.tradable_only);
                w::toggle_chip(ui, &t, t!("Marketable"), &mut self.marketable_only);
                w::toggle_chip(ui, &t, t!("Group duplicates"), &mut self.group);
                let active: usize = self.tags.values().map(|v| v.len()).sum();
                let label = if active > 0 { format!("{}  {} ({active})", icons::FUNNEL, t!("Filters")) } else { format!("{}  {}", icons::FUNNEL, t!("Filters")) };
                let r = w::chip(ui, &t, &label, active > 0).on_hover_text(t!("Filter by this game's own item tags"));
                egui::Popup::from_toggle_button_response(&r)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .width(520.0)
                    .show(|ui| self.tag_filters(ui, &t, &inv.items));
                if w::chip(ui, &t, &format!("{}  {}", icons::COINS, t!("Show prices")), self.show_prices).on_hover_text(t!("Look up each item's market price (a few per second, so Steam doesn't block it)")).clicked() {
                    self.show_prices = !self.show_prices;
                }
                w::segmented(ui, &t, &mut self.sort, &[(Sort::Newest, &t!("Newest")), (Sort::Name, &t!("Name")), (Sort::Kind, &t!("Type")), (Sort::Price, &t!("Price"))]);
            });
            if self.sort == Sort::Price {
                self.show_prices = true;
            }
            ui.add_space(12.0);

            let q = self.search.trim().to_lowercase();
            let prices = &ctx.state.market_prices;
            let price_of = |i: &InventoryItem| -> Option<u64> {
                match prices.get(&(app, i.market_hash_name.clone())) {
                    Some(Ok(p)) => p.lowest_sell.or_else(|| p.lowest.as_deref().and_then(model::parse_money)),
                    _ => None,
                }
            };
            let filtered: Vec<&InventoryItem> = inv
                .items
                .iter()
                .filter(|i| !self.tradable_only || i.tradable)
                .filter(|i| !self.marketable_only || i.marketable)
                .filter(|i| q.is_empty() || i.name.to_lowercase().contains(&q) || i.kind.to_lowercase().contains(&q))
                .filter(|i| self.tags.iter().all(|(cat, vals)| vals.is_empty() || i.tags.iter().any(|(c, v, _)| c == cat && vals.contains(v))))
                .collect();
            // Optionally stack identical items into one tile with a count.
            let mut items: Vec<(&InventoryItem, u64)> = Vec::new();
            if self.group {
                let mut index: std::collections::HashMap<(u64, &str), usize> = Default::default();
                for i in filtered {
                    match index.get(&(i.class_id, i.name.as_str())) {
                        Some(&k) => items[k].1 += i.amount,
                        None => {
                            index.insert((i.class_id, i.name.as_str()), items.len());
                            items.push((i, i.amount));
                        }
                    }
                }
            } else {
                items = filtered.into_iter().map(|i| (i, i.amount)).collect();
            }
            match self.sort {
                Sort::Newest => {}
                Sort::Name => items.sort_by(|a, b| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase())),
                Sort::Kind => items.sort_by(|a, b| a.0.kind.cmp(&b.0.kind).then_with(|| a.0.name.cmp(&b.0.name))),
                Sort::Price => items.sort_by_key(|(i, n)| std::cmp::Reverse(price_of(i).map(|p| p * n))),
            }
            if self.selected.is_none_or(|s| !items.iter().any(|(i, _)| i.asset_id == s)) {
                self.selected = items.first().map(|(i, _)| i.asset_id);
            }

            // Price lookups, throttled: Steam blocks fast repeated requests.
            if self.show_prices {
                for (i, _) in &items {
                    let key = (app, i.market_hash_name.clone());
                    if i.marketable && !i.market_hash_name.is_empty() && !prices.contains_key(&key) && self.price_asked.insert(key.clone()) {
                        self.price_queue.push_back(key);
                    }
                }
                let now = ui.input(|i| i.time);
                if now >= self.next_price_at {
                    if let Some((a, h)) = self.price_queue.pop_front() {
                        ctx.send(Command::MarketPrice { app_id: a, hash_name: h, currency: wallet_currency(ctx), country: ctx.state.country.clone(), full: false });
                        self.next_price_at = now + 3.2;
                    }
                }
                if !self.price_queue.is_empty() {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                }
                let (mut total, mut priced, mut marketable) = (0u64, 0usize, 0usize);
                for (i, n) in &items {
                    if i.marketable {
                        marketable += 1;
                        if let Some(p) = price_of(i) {
                            total += p * n;
                            priced += 1;
                        }
                    }
                }
                ui.label(w::text(
                    t!("Estimated value: {value} (priced {done} of {all})", value = model::format_money(total as i64, wallet_currency(ctx)), done = priced, all = marketable),
                    13.0,
                    t.text_dim,
                ));
                ui.add_space(6.0);
            }

            let side = 360.0_f32.min(ui.available_width() * 0.42);
            let grid_w = ui.available_width() - side - 18.0;
            let height = ui.available_height();
            let currency = wallet_currency(ctx);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(grid_w, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(grid_w);
                    if items.is_empty() {
                        ui.label(w::text(t!("No items match your filters."), 13.5, t.text_faint));
                        return;
                    }
                    let tile = 96.0;
                    let gap = 8.0;
                    let cols = (((grid_w + gap) / (tile + gap)).floor() as usize).max(1);
                    let rows = items.len().div_ceil(cols);
                    egui::ScrollArea::vertical().id_salt(("inv", owner, app)).auto_shrink([false, false]).show_rows(ui, tile + gap, rows, |ui, range| {
                        for row in range {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                for (item, count) in items.iter().skip(row * cols).take(cols) {
                                    let price = if self.show_prices { price_of(item).map(|p| model::format_money(p as i64, currency)) } else { None };
                                    if item_tile(ui, &t, item, *count, price, tile, self.selected == Some(item.asset_id)).clicked() {
                                        self.selected = Some(item.asset_id);
                                    }
                                }
                            });
                            ui.add_space(gap - ui.spacing().item_spacing.y);
                        }
                    });
                });
                ui.add_space(18.0);
                ui.allocate_ui_with_layout(vec2(side, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(side);
                    if let Some((item, count)) = items.iter().find(|(i, _)| Some(i.asset_id) == self.selected) {
                        let (item, count) = ((*item).clone(), *count);
                        egui::ScrollArea::vertical().id_salt("inv-detail").auto_shrink([false, false]).show(ui, |ui| self.detail(ui, &t, ctx, &item, count, app, mine));
                    }
                });
            });
            self.sell_dialog(ui, &t, ctx);
            self.gift_picker(ui, &t, ctx);
        });
    }

    fn on_event(&mut self, event: &Event, ctx: &mut Ctx) {
        match event {
            Event::ItemListed { result, .. } => {
                match result {
                    Ok(true) => ctx.toast(t!(
                        "Listed. Confirm the sale in the Steam Mobile app to put it on the market."
                    )),
                    Ok(false) => ctx.toast(t!("Your item is on the Community Market")),
                    Err(e) => ctx.toast(t!(
                        "Couldn't list the item: {error}",
                        error = i18n::tr_dyn(e)
                    )),
                }
                ctx.send(Command::LoadMyListings);
            }
            Event::ListingCancelled { result, .. } => match result {
                Ok(()) => ctx.toast(t!("Listing removed. The item is back in your inventory.")),
                Err(e) => ctx.toast(t!(
                    "Couldn't remove the listing: {error}",
                    error = i18n::tr_dyn(e)
                )),
            },
            Event::BuyOrderCancelled { result, .. } => match result {
                Ok(()) => ctx.toast(t!("Buy order cancelled")),
                Err(e) => ctx.toast(t!(
                    "Couldn't cancel the buy order: {error}",
                    error = i18n::tr_dyn(e)
                )),
            },
            _ => {}
        }
    }
}

pub(crate) fn wallet_currency(ctx: &Ctx) -> i32 {
    ctx.state
        .wallet
        .map(|w| w.1)
        .filter(|c| *c > 0)
        .unwrap_or(1)
}

impl Inventory {
    /// Filters built from the tags this game puts on its items (hero, rarity,
    /// exterior, card set, ...), so every game gets its own.
    fn tag_filters(&mut self, ui: &mut Ui, t: &Theme, items: &[InventoryItem]) {
        let mut cats: BTreeMap<&str, BTreeMap<&str, (usize, Option<&str>)>> = BTreeMap::new();
        for i in items {
            for (c, v, color) in &i.tags {
                if c.is_empty() || v.is_empty() {
                    continue;
                }
                let e = cats
                    .entry(c.as_str())
                    .or_default()
                    .entry(v.as_str())
                    .or_insert((0, color.as_deref()));
                e.0 += 1;
            }
        }
        egui::Frame::new()
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(w::text_bold(t!("Filters for this game"), 14.0, t.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.tags.is_empty()
                            && ui.link(w::text(t!("Clear all"), 12.5, t.accent)).clicked()
                        {
                            self.tags.clear();
                        }
                    });
                });
                if cats.is_empty() {
                    ui.label(w::text(
                        t!("This game doesn't tag its items."),
                        13.0,
                        t.text_faint,
                    ));
                    return;
                }
                // Categories with a single value don't filter anything.
                for (cat, values) in cats.iter().filter(|(_, v)| v.len() > 1) {
                    let chosen = self.tags.get(*cat).map(|s| s.len()).unwrap_or(0);
                    let title = if chosen > 0 {
                        format!("{cat} ({chosen})")
                    } else {
                        cat.to_string()
                    };
                    egui::CollapsingHeader::new(w::text_semibold(title, 13.0, t.text_dim))
                        .id_salt(("tagcat", *cat))
                        .default_open(values.len() <= 12)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                                let mut list: Vec<_> = values.iter().collect();
                                list.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(b.0)));
                                for (value, (n, color)) in list {
                                    let on =
                                        self.tags.get(*cat).is_some_and(|s| s.contains(*value));
                                    // Steam colors some tags (rarity, quality, card border); keep them.
                                    if w::tinted_chip(
                                        ui,
                                        t,
                                        &format!("{value}  {n}"),
                                        hex(*color),
                                        on,
                                    )
                                    .clicked()
                                    {
                                        let set = self.tags.entry(cat.to_string()).or_default();
                                        if on {
                                            set.remove(*value);
                                        } else {
                                            set.insert(value.to_string());
                                        }
                                        if set.is_empty() {
                                            self.tags.remove(*cat);
                                        }
                                    }
                                }
                            });
                        });
                }
            });
    }

    fn header(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        ctx: &mut Ctx,
        owner: SteamId,
        mine: bool,
        tabs: &[(AppId, u64, String, Option<u32>)],
    ) {
        let t = t.clone();
        let persona = ctx.state.persona(owner);
        let name = if mine && !ctx.state.persona_name.is_empty() {
            ctx.state.persona_name.clone()
        } else {
            persona.name.clone()
        };
        ui.horizontal(|ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
            if !w::paint_cover(
                ui,
                r,
                &art::avatar_full(persona.avatar_hash.as_deref()),
                CornerRadius::same(6),
            ) {
                w::skeleton(ui, &t, r, CornerRadius::same(6));
            }
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                ctx.navigate("profile", Some(owner.to_string()));
            }
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(w::text_bold(
                    if mine {
                        t!("Your inventory").to_string()
                    } else {
                        t!("{name}'s inventory", name = name)
                    },
                    22.0,
                    t.text,
                ));
                let total: u64 = tabs
                    .iter()
                    .map(|(a, c, _, n)| {
                        n.map(u64::from)
                            .or_else(|| {
                                ctx.state
                                    .inventories
                                    .get(&(owner, *a, *c))
                                    .map(|i| i.total as u64)
                            })
                            .unwrap_or(0)
                    })
                    .sum();
                let games = tabs.iter().map(|x| x.0).collect::<HashSet<_>>().len();
                ui.label(w::text(
                    t!("{n} items in {games} games", n = total, games = games),
                    13.0,
                    t.text_dim,
                ));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(
                    ui,
                    &t,
                    Some(icons::STOREFRONT),
                    t!("Community Market"),
                    w::Kind::Secondary,
                    w::Size::Sm,
                )
                .on_hover_text(t!("Opens in your browser"))
                .clicked()
                {
                    let _ = open::that_detached("https://steamcommunity.com/market/");
                }
                if mine {
                    if w::button(
                        ui,
                        &t,
                        Some(icons::SWAP),
                        t!("Trade offers"),
                        w::Kind::Secondary,
                        w::Size::Sm,
                    )
                    .on_hover_text(t!("Opens in your browser"))
                    .clicked()
                    {
                        let _ = open::that_detached(format!(
                            "https://steamcommunity.com/profiles/{owner}/tradeoffers/"
                        ));
                    }
                } else if ctx.state.friends.get(&owner) == Some(&Relationship::Friend)
                    && w::button(
                        ui,
                        &t,
                        Some(icons::SWAP),
                        t!("Offer a trade"),
                        w::Kind::Secondary,
                        w::Size::Sm,
                    )
                    .on_hover_text(t!("Opens in your browser"))
                    .clicked()
                {
                    let account = owner & 0xFFFF_FFFF;
                    let _ = open::that_detached(format!(
                        "https://steamcommunity.com/tradeoffer/new/?partner={account}"
                    ));
                }
                if !mine
                    && w::button(
                        ui,
                        &t,
                        Some(icons::USER_CIRCLE),
                        t!("Profile"),
                        w::Kind::Ghost,
                        w::Size::Sm,
                    )
                    .clicked()
                {
                    ctx.navigate("profile", Some(owner.to_string()));
                }
            });
        });
        ui.add_space(14.0);
    }
}

/// The item's rarity color, like Steam's colored tile edges: the most vivid
/// of its name color and tag colors (games put rarity in different places).
pub(crate) fn rarity_color(item: &InventoryItem) -> Option<Color32> {
    let vivid = |c: &Color32| {
        let (r, g, b) = (c.r() as i32, c.g() as i32, c.b() as i32);
        r.max(g).max(b) - r.min(g).min(b)
    };
    std::iter::once(item.name_color.as_deref())
        .chain(item.tags.iter().map(|t| t.2.as_deref()))
        .filter_map(hex)
        .max_by_key(vivid)
        .filter(|c| vivid(c) > 40)
        .or_else(|| hex(item.name_color.as_deref()))
}

pub(crate) fn hex(c: Option<&str>) -> Option<Color32> {
    let c = c?;
    let v = u32::from_str_radix(c.get(..6)?, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn item_tile(
    ui: &mut Ui,
    t: &Theme,
    item: &InventoryItem,
    count: u64,
    price: Option<String>,
    size: f32,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let p = ui.painter();
    let base = hex(item.background.as_deref())
        .map(|c| w::mix(t.surface, c, 0.35))
        .unwrap_or(t.surface);
    let base = if selected {
        w::mix(base, t.accent, 0.12)
    } else {
        base
    };
    p.rect_filled(
        rect,
        t.corner_small(),
        w::mix(base, t.surface_hover, hover * 0.6),
    );
    // The picture fills the tile (above the rarity bar), sharp on high-DPI screens.
    let img = Rect::from_min_max(rect.min + vec2(5.0, 5.0), rect.max - vec2(5.0, 10.0));
    if !w::paint_contain(ui, img, &art::econ(&item.icon, 192), Align2::CENTER_CENTER) {
        w::skeleton(ui, t, img, CornerRadius::same(4));
    }
    let p = ui.painter();
    if let Some(c) = rarity_color(item) {
        // Inside the border, so the selection outline stays unbroken.
        let bar = Rect::from_min_max(
            pos2(rect.left() + 8.0, rect.bottom() - 6.0),
            pos2(rect.right() - 8.0, rect.bottom() - 3.5),
        );
        p.rect_filled(bar, CornerRadius::same(2), c);
    }
    let badge = |text: String, top: bool| {
        let g = p.layout_no_wrap(text, w::font_bold(10.5), t.text);
        let pos = if top {
            pos2(rect.right() - g.size().x - 10.0, rect.top() + 4.0)
        } else {
            pos2(rect.left() + 4.0, rect.bottom() - g.size().y - 8.0)
        };
        let r = Rect::from_min_size(pos, g.size() + vec2(6.0, 2.0));
        p.rect_filled(r, CornerRadius::same(3), Color32::from_black_alpha(170));
        p.galley(r.min + vec2(3.0, 1.0), g, t.text);
    };
    if count > 1 {
        badge(format!("×{count}"), true);
    }
    if let Some(price) = price {
        badge(price, false);
    }
    if !item.tradable {
        p.text(
            pos2(rect.left() + 8.0, rect.top() + 10.0),
            Align2::LEFT_CENTER,
            icons::LOCK_SIMPLE,
            w::font(11.0),
            t.text_faint,
        );
    }
    let stroke = if selected {
        Stroke::new(2.0, t.accent)
    } else {
        Stroke::new(1.0, w::mix(t.border, t.text_faint, hover))
    };
    p.rect_stroke(rect, t.corner_small(), stroke, egui::StrokeKind::Inside);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&item.name)
}

fn skeleton_grid(ui: &mut Ui, t: &Theme) {
    let tile = 96.0;
    let cols = ((ui.available_width() + 8.0) / (tile + 8.0))
        .floor()
        .max(1.0) as usize;
    for _ in 0..3 {
        ui.horizontal(|ui| {
            for _ in 0..cols.min(10) {
                let (r, _) = ui.allocate_exact_size(vec2(tile, tile), Sense::hover());
                w::skeleton(ui, t, r, t.corner_small());
            }
        });
        ui.add_space(4.0);
    }
}

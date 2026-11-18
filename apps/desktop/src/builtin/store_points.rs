//! Store: the Points Shop (Figma "point shop" `317:1001`). Steam's catalog
//! of profile items, chat stickers and the like, by kind, from any game or
//! from yours. Browsing needs no sign-in. Your points balance and getting an
//! item need a signed-in call SteamInRust doesn't make yet, so those open
//! Steam's Points Shop in the browser.

use super::{Load, Shared, Store, View, store_page::block, thousands};
use egui::{Align, Align2, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::AppId;
use std::collections::HashMap;
use store::points::{RewardItem, RewardPage, class};
use ui::{Theme, icons, widgets as w};

/// Items per page.
const PAGE: u32 = 24;

/// A list in the Points Shop's sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(super) enum Shelf {
    /// Everything, newest first.
    #[default]
    Featured,
    /// Items from games you own.
    YourGames,
    /// Items from one game (from its store page).
    Game(AppId),
    /// One kind of item.
    Kind(u32),
}

/// The sidebar: (group, [(list, name)]).
fn groups() -> Vec<(&'static str, Vec<(Shelf, &'static str)>)> {
    vec![
        (t!("Featured"), vec![(Shelf::Featured, t!("All items")), (Shelf::YourGames, t!("From your games"))]),
        (
            t!("Profile"),
            vec![
                (Shelf::Kind(class::AVATAR), t!("Animated avatars")),
                (Shelf::Kind(class::AVATAR_FRAME), t!("Avatar frames")),
                (Shelf::Kind(class::BACKGROUND), t!("Backgrounds")),
                (Shelf::Kind(class::MINI_BACKGROUND), t!("Mini-profile backgrounds")),
                (Shelf::Kind(class::PROFILE), t!("Game profiles")),
            ],
        ),
        (t!("Chat"), vec![(Shelf::Kind(class::STICKER), t!("Animated stickers")), (Shelf::Kind(class::CHAT_EFFECT), t!("Chat effects")), (Shelf::Kind(class::EMOTICON), t!("Emoticons"))]),
        (t!("Interface"), vec![(Shelf::Kind(class::KEYBOARD), t!("Keyboards")), (Shelf::Kind(class::STARTUP_MOVIE), t!("Startup movies"))]),
    ]
}

/// The name of a kind of item, for its card.
fn kind_name(c: u32) -> &'static str {
    match c {
        class::AVATAR => t!("Animated avatar"),
        class::AVATAR_FRAME => t!("Avatar frame"),
        class::BACKGROUND => t!("Profile background"),
        class::MINI_BACKGROUND => t!("Mini-profile background"),
        class::PROFILE => t!("Game profile"),
        class::STICKER => t!("Animated sticker"),
        class::CHAT_EFFECT => t!("Chat effect"),
        class::EMOTICON => t!("Emoticon"),
        class::KEYBOARD => t!("Keyboard"),
        class::STARTUP_MOVIE => t!("Startup movie"),
        _ => t!("Item"),
    }
}

#[derive(Default)]
pub(super) struct Points {
    pub shelf: Shelf,
    /// Pages loaded for the open list, in order.
    pages: Vec<Shared<RewardPage>>,
    /// The open list's key (what was asked for), to start over when it changes.
    key: Option<(Shelf, Vec<AppId>)>,
    /// A few items per game, for game pages.
    per_app: HashMap<AppId, Shared<RewardPage>>,
}

impl Store {
    /// A game's first few items and how many there are (for its store page).
    pub(super) fn points_for_app(&mut self, id: AppId) -> Option<RewardPage> {
        if !self.points.per_app.contains_key(&id) {
            let api = self.api.clone();
            let slot = self.fetch(async move { api.reward_items(&[], &[id], None, 4).await });
            self.points.per_app.insert(id, slot);
        }
        match &*self.points.per_app[&id].lock().unwrap() {
            Load::Ready(p) => Some(p.clone()),
            _ => None,
        }
    }

    /// The Points Shop at one game's items.
    pub(super) fn points_open_app(&mut self, id: AppId) {
        self.points.shelf = Shelf::Game(id);
        self.open(View::Points);
    }

    pub(super) fn points_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        // What to ask Steam for.
        let shelf = self.points.shelf;
        let apps: Vec<AppId> = match shelf {
            Shelf::YourGames => {
                let mut v: Vec<AppId> = self.owned.iter().copied().collect();
                v.sort_unstable();
                v.truncate(200);
                v
            }
            Shelf::Game(id) => vec![id],
            _ => Vec::new(),
        };
        let key = (shelf, apps.clone());
        if self.points.key.as_ref() != Some(&key) {
            self.points.key = Some(key);
            self.points.pages.clear();
            if !(shelf == Shelf::YourGames && apps.is_empty()) {
                self.points_more(None);
            }
        }

        self.points_header(ui, t, ctx);
        ui.add_space(t.dense(70.0));
        let gap = t.dense(16.0);
        let side_w = t.dense(250.0).min(ui.available_width() * 0.3);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.vertical(|ui| {
                ui.set_width(side_w);
                self.points_sidebar(ui, t);
            });
            ui.vertical(|ui| {
                let width = ui.available_width();
                ui.set_width(width);
                self.points_items(ui, t, &apps);
            });
        });
    }

    fn points_more(&mut self, cursor: Option<String>) {
        let Some((shelf, apps)) = self.points.key.clone() else { return };
        let classes: Vec<u32> = match shelf {
            Shelf::Kind(c) => vec![c],
            _ => Vec::new(),
        };
        let api = self.api.clone();
        let slot = self.fetch(async move { api.reward_items(&classes, &apps, cursor.as_deref(), PAGE).await });
        self.points.pages.push(slot);
    }

    /// "Buy Games, Earn Points", with How it works, History, My Profile and
    /// the balance on the right (Figma `317:1001`, top).
    fn points_header(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = t.spacing.sm;
                ui.label(w::heading_l(t, t!("Buy Games, Earn Points"), t.text));
                ui.label(w::body_m(t, t!("Make your profile and chats your own with Points Shop items."), t.text));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = t.dense(10.0);
                // The balance: Steam only tells a signed-in session, which
                // SteamInRust doesn't ask yet; Steam's own page shows it.
                let r = w::button(ui, t, Some(icons::COINS), t!("Your points"), w::Kind::Secondary, w::Size::Lg)
                    .on_hover_text(t!("Your points balance is on Steam's Points Shop page (opens in your browser)."));
                if r.clicked() {
                    crate::links::open_http("https://store.steampowered.com/points/shop");
                }
                if ctx.state.steam_id != 0 && w::button(ui, t, None, t!("My Profile"), w::Kind::Tertiary, w::Size::Lg).clicked() {
                    ctx.navigate("profile", None);
                }
                if w::button(ui, t, None, t!("History"), w::Kind::Tertiary, w::Size::Lg).on_hover_text(t!("Opens in your browser")).clicked() {
                    crate::links::open_http("https://store.steampowered.com/points/history");
                }
                if w::button(ui, t, None, t!("How it works"), w::Kind::Tertiary, w::Size::Lg).on_hover_text(t!("Opens in your browser")).clicked() {
                    crate::links::open_http("https://store.steampowered.com/points/howitworks");
                }
            });
        });
    }

    /// The lists, grouped (Figma `317:1001`, left).
    fn points_sidebar(&mut self, ui: &mut Ui, t: &Theme) {
        block(t).inner_margin(egui::Margin::symmetric(t.dense(14.0) as i8, t.dense(20.0) as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let game = match self.points.shelf {
                Shelf::Game(id) => Some(id),
                _ => None,
            };
            for (gi, (group, lists)) in groups().into_iter().enumerate() {
                if gi > 0 {
                    ui.add_space(t.dense(6.0));
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
                    ui.add_space(t.dense(6.0));
                }
                ui.label(w::body_l(t, group, t.text));
                ui.add_space(t.dense(2.0));
                let mut lists = lists;
                if gi == 0 {
                    if let Some(id) = game {
                        lists.push((Shelf::Game(id), t!("This game")));
                    }
                }
                for (shelf, label) in lists {
                    if list_row(ui, t, label, self.points.shelf == shelf).clicked() {
                        self.points.shelf = shelf;
                    }
                }
            }
        });
    }

    /// The open list's items, a grid of cards (Figma `317:1001`, right).
    fn points_items(&mut self, ui: &mut Ui, t: &Theme, apps: &[AppId]) {
        let shelf = self.points.shelf;
        let title = match shelf {
            Shelf::Featured => t!("All items").to_string(),
            Shelf::YourGames => t!("From your games").to_string(),
            Shelf::Game(id) => self.details(id).map(|d| d.name).unwrap_or_else(|| t!("This game").to_string()),
            Shelf::Kind(c) => groups().into_iter().flat_map(|g| g.1).find(|(s, _)| *s == Shelf::Kind(c)).map(|(_, n)| n.to_string()).unwrap_or_default(),
        };
        let pages: Vec<Option<Result<RewardPage, String>>> = self
            .points
            .pages
            .iter()
            .map(|s| match &*s.lock().unwrap() {
                Load::Loading => None,
                Load::Ready(p) => Some(Ok(p.clone())),
                Load::Failed(e) => Some(Err(e.clone())),
            })
            .collect();
        let items: Vec<RewardItem> = pages.iter().flatten().flatten().flat_map(|p| p.items.clone()).collect();
        let total = pages.iter().flatten().flatten().next().map(|p| p.total);
        let c = &t.components.shelf;
        w::component_frame(t, c).inner_margin(egui::Margin::symmetric(t.dense(24.0) as i8, t.dense(24.0) as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(w::heading_m(t, &title, t.text));
                if let Some(n) = total {
                    ui.label(w::body_m(t, t!("{n} items", n = thousands(n)), t.text_dim));
                }
            });
            ui.add_space(t.dense(26.0));
            if shelf == Shelf::YourGames && apps.is_empty() {
                w::empty(ui, t, icons::GIFT, t!("No games yet"), t!("Items from games in your library show up here."));
                return;
            }
            if let Some(Some(Err(e))) = pages.first() {
                w::empty(ui, t, icons::WARNING, t!("The Points Shop didn't load"), e);
                if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Md).clicked() {
                    self.points.key = None;
                }
                return;
            }
            if items.is_empty() {
                if pages.iter().any(|p| p.is_none()) {
                    w::loading(ui, t, t!("Loading…"));
                } else {
                    w::empty(ui, t, icons::GIFT, t!("Nothing here yet"), t!("Steam has no items of this kind right now."));
                }
                return;
            }
            let gap = t.dense(31.0);
            let width = ui.available_width();
            let per = (((width + gap) / (t.dense(240.0) + gap)).floor() as usize).max(1);
            let cw = ((width - gap * (per as f32 - 1.0)) / per as f32).floor();
            for row in items.chunks(per) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for it in row {
                        if reward_card(ui, t, it, cw).clicked() {
                            // Getting an item needs your points: on Steam's page.
                            crate::links::open_http(&format!("https://store.steampowered.com/points/shop/app/{}", it.app_id));
                        }
                    }
                });
                ui.add_space(gap * 0.6);
            }
            match pages.last() {
                Some(None) => w::loading(ui, t, t!("Loading…")),
                Some(Some(Ok(p))) if p.next.is_some() => {
                    let next = p.next.clone();
                    ui.vertical_centered(|ui| {
                        if w::button(ui, t, Some(icons::CARET_DOWN), t!("Show more"), w::Kind::Secondary, w::Size::Md).clicked() {
                            self.points_more(next);
                        }
                    });
                }
                _ => {}
            }
        });
    }
}

/// A list in the sidebar: the `tag` style's rows (Figma's 25 px rows).
fn list_row(ui: &mut Ui, t: &Theme, label: &str, selected: bool) -> egui::Response {
    let c = &t.components.tag;
    let h = t.dense(25.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let look = t.look(c, w::interact(ui, &resp, selected));
    if selected || resp.hovered() {
        w::paint_box(ui, t, rect, c, &look, 0.0);
    }
    let color = if selected { look.text } else { t.text };
    ui.painter().text(pos2(rect.left() + t.dense(9.0), rect.center().y), Align2::LEFT_CENTER, label, t.typography.body_s.font(), color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One item (Figma `317:1001`'s cards): its picture, name, kind and price.
fn reward_card(ui: &mut Ui, t: &Theme, it: &RewardItem, width: f32) -> egui::Response {
    let c = &t.components.store_card;
    let pad = t.dense(11.0);
    let img = width - pad * 2.0;
    let h = pad * 2.0 + img + t.dense(96.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let look = t.look(c, w::interact(ui, &resp, false));
    w::paint_box(ui, t, rect, c, &look, 0.0);
    let pic = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(img, img));
    if it.image.is_empty() || !w::paint_cover(ui, pic, &it.image, egui::CornerRadius::ZERO) {
        if it.image.is_empty() {
            w::placeholder(ui, t, pic, &it.name, egui::CornerRadius::ZERO);
        } else {
            w::skeleton(ui, t, pic, egui::CornerRadius::ZERO);
        }
    }
    let body = Rect::from_min_max(pos2(pic.left() + 2.0, pic.bottom() + t.dense(12.0)), pos2(pic.right(), rect.bottom() - pad));
    let mut b = ui.new_child(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)));
    b.spacing_mut().item_spacing.y = t.dense(9.0);
    b.add(egui::Label::new(w::body_l(t, &it.name, t.accent)).truncate().selectable(false));
    b.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.sm;
        ui.label(w::body_l(t, icons::USER_CIRCLE, t.text_dim));
        ui.add(egui::Label::new(w::body_m(t, kind_name(it.class), t.text_dim)).truncate().selectable(false));
    });
    b.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.sm;
        ui.label(w::heading_s(t, icons::COINS, t.accent));
        ui.label(w::heading_s(t, thousands(it.cost), t.text));
    });
    let resp = if it.description.is_empty() { resp } else { resp.on_hover_text(&it.description) };
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

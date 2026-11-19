//! More of a profile shown in its middle column, so the right column's
//! links never leave the profile: your wishlist, and anyone's inventory.
//! Each has a button to the full page (prices and sorting, trading and
//! selling).

use super::{Profile, pager, section};
use client::Command;
use egui::{Align, Align2, Layout, Rect, Sense, Stroke, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::{AppId, SteamId, art};
use ui::{Theme, icons, widgets as w};

/// Items a page of the inventory shows.
const ITEMS_PER_PAGE: usize = 48;
/// Games a page of the wishlist shows.
const WISHES_PER_PAGE: usize = 12;

impl Profile {
    /// Your wishlist, in Steam's order, as the profile's main column.
    pub(super) fn wishlist_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if ctx.state.wishlist.is_none() && self.lists_requested.insert((ctx.state.steam_id, "wishlist")) {
            ctx.send(Command::LoadWishlist);
        }
        let list = ctx.state.wishlist.clone();
        // Names for games that aren't in your library.
        if let Some(list) = &list {
            let unknown: Vec<AppId> = list.iter().map(|(a, _)| *a).filter(|a| ctx.state.app_name(*a).is_none() && self.names_asked.insert(*a)).collect();
            if !unknown.is_empty() {
                ctx.send(Command::LoadAppNames { ids: unknown });
            }
        }
        let note = list.as_ref().map(|l| t!("{n} games", n = l.len()).to_string()).unwrap_or_default();
        let mut open = None;
        let mut full = false;
        section(ui, t, t!("Wishlist"), &note, |ui| {
            let Some(list) = &list else { return super::loading_rows(ui, t, 4, t.dense(64.0)) };
            if list.is_empty() {
                w::empty(ui, t, icons::HEART, t!("Your wishlist is empty"), t!("Add games from their store pages and you'll be told when they go on sale."));
                return;
            }
            let pages = list.len().div_ceil(WISHES_PER_PAGE).max(1);
            self.list_page = self.list_page.min(pages - 1);
            let gap = t.spacing.md;
            let cols = if ui.available_width() > t.dense(560.0) { 3 } else { 2 };
            let cw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
            let shown: Vec<&(AppId, u64)> = list.iter().skip(self.list_page * WISHES_PER_PAGE).take(WISHES_PER_PAGE).collect();
            for row in shown.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (app, added) in row {
                        let name = ctx.state.app_name(*app).map(str::to_string).unwrap_or_else(|| t!("App {id}", id = app).to_string());
                        let when = if *added > 0 { t!("Added {date}", date = ui::date(*added)).to_string() } else { String::new() };
                        if game_tile(ui, t, *app, &name, &when, cw) {
                            open = Some(*app);
                        }
                    }
                });
            }
            if let Some(p) = pager(ui, t, self.list_page, pages) {
                self.list_page = p;
            }
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                full = w::button(ui, t, Some(icons::STOREFRONT), t!("Open the wishlist in the store"), w::Kind::Secondary, w::Size::Md).clicked();
            });
        });
        if let Some(app) = open {
            ctx.act(crate::shell::game_page(ctx.state, app));
        }
        if full {
            ctx.navigate("store", Some("wishlist".into()));
        }
    }

    /// Someone's inventory: a game picker, then that game's items.
    pub(super) fn inventory_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId) {
        if self.lists_requested.insert((id, "inventory")) {
            ctx.send(Command::LoadInventoryApps { steam_id: id });
        }
        let apps = ctx.state.inventory_apps.get(&id).cloned();
        let mut full = false;
        section(ui, t, t!("Inventory"), "", |ui| {
            let apps = match apps {
                None => return super::loading_rows(ui, t, 3, t.dense(64.0)),
                Some(Err(e)) => {
                    let private = e == "private";
                    let detail = if private { String::new() } else { i18n::tr_dyn(&e).to_string() };
                    w::empty(ui, t, icons::LOCK_SIMPLE, if private { t!("This inventory is private") } else { t!("Couldn't load the inventory") }, &detail);
                    return;
                }
                Some(Ok(a)) if a.is_empty() => {
                    w::empty(ui, t, icons::BACKPACK, t!("Nothing in this inventory"), "");
                    return;
                }
                Some(Ok(a)) => a,
            };
            // Which game (and section): the one picked, else the first.
            let chosen = self.inv_app.filter(|(a, c)| apps.iter().any(|x| x.app_id == *a && x.contexts.iter().any(|k| k.0 == *c)));
            let (app, context) = chosen.unwrap_or_else(|| (apps[0].app_id, apps[0].contexts.first().map(|c| c.0).unwrap_or(2)));
            let options: Vec<((AppId, u64), String)> = apps
                .iter()
                .flat_map(|a| {
                    a.contexts.iter().map(move |(c, cname, n)| {
                        let label = if a.contexts.len() > 1 { format!("{} · {} ({n})", a.name, cname) } else { format!("{} ({n})", a.name) };
                        ((a.app_id, *c), label)
                    })
                })
                .collect();
            let mut pick = (app, context);
            ui.horizontal(|ui| {
                let width = ui.available_width().min(t.dense(360.0));
                if w::select(ui, t, &mut pick, &options, width) {
                    self.inv_app = Some(pick);
                    self.list_page = 0;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    full = w::button(ui, t, Some(icons::BACKPACK), t!("Open the full inventory"), w::Kind::Secondary, w::Size::Md).clicked();
                });
            });
            let key = (id, app, context);
            let Some(inv) = ctx.state.inventories.get(&key) else {
                if self.inv_asked.insert(key) {
                    ctx.send(Command::LoadInventory { steam_id: id, app_id: app, context_id: context, language: i18n::language().steam_name().into() });
                }
                return super::loading_rows(ui, t, 3, t.dense(64.0));
            };
            if let Some(e) = &inv.error {
                ui.label(w::body_m(t, i18n::tr_dyn(e), t.text_dim));
                return;
            }
            let pages = inv.items.len().div_ceil(ITEMS_PER_PAGE).max(1);
            self.list_page = self.list_page.min(pages - 1);
            let tile = t.dense(84.0);
            let gap = t.spacing.sm;
            let cols = (((ui.available_width() + gap) / (tile + gap)).floor() as usize).max(1);
            let shown: Vec<&model::InventoryItem> = inv.items.iter().skip(self.list_page * ITEMS_PER_PAGE).take(ITEMS_PER_PAGE).collect();
            for row in shown.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for item in row {
                        item_tile(ui, t, item, tile);
                    }
                });
            }
            if let Some(p) = pager(ui, t, self.list_page, pages) {
                self.list_page = p;
            }
        });
        if full {
            let arg = match self.inv_app {
                Some((app, _)) => format!("{id}/{app}"),
                None => id.to_string(),
            };
            ctx.navigate("inventory", Some(arg));
        }
    }
}

/// A wished-for game: its header picture, name and when it was added.
/// True when clicked.
fn game_tile(ui: &mut Ui, t: &Theme, app: AppId, name: &str, sub: &str, width: f32) -> bool {
    let c = &t.components.profile_inset;
    let pad = t.dense(8.0);
    let img_h = (width - pad * 2.0) * 215.0 / 460.0;
    let line = t.typography.body_l.size + t.spacing.xs;
    let small = t.typography.body_s.size + t.spacing.xs;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, img_h + pad * 3.0 + line + small), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp.clicked();
    }
    let look = t.look(c, w::interact(ui, &resp, false));
    w::paint_box(ui, t, rect, c, &look, 0.0);
    let img = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(width - pad * 2.0, img_h));
    w::image_or_skeleton(ui, t, img, &art::header(app), t.corner_small());
    let text = |s: &str, font: egui::FontId, color, y: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(s.to_string(), font, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(img.width());
        let g = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(img.left(), y), g, color);
    };
    text(name, t.typography.body_l.font(), t.text, img.bottom() + pad);
    if !sub.is_empty() {
        text(sub, t.typography.body_s.font(), t.text_dim, img.bottom() + pad + line);
    }
    resp.on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// An item: its picture on its color, the count, and its name and type on
/// hover.
fn item_tile(ui: &mut Ui, t: &Theme, item: &model::InventoryItem, size: f32) {
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if !ui.is_rect_visible(r) {
        return;
    }
    ui.painter().rect_filled(r, t.corner_small(), t.overlay_dark(1));
    if !w::paint_contain(ui, r.shrink(t.spacing.sm), &art::econ(&item.icon, 96), Align2::CENTER_CENTER) {
        w::skeleton(ui, t, r.shrink(t.spacing.sm), t.corner_small());
    }
    let edge = crate::builtin::inventory::rarity_color(item).unwrap_or(t.surface_hover);
    ui.painter().rect_stroke(r, t.corner_small(), Stroke::new(if resp.hovered() { t.dense(2.0) } else { 1.0 }, edge), egui::StrokeKind::Inside);
    if item.amount > 1 {
        let at = r.right_bottom() - vec2(t.spacing.xs, t.spacing.xs);
        ui.painter().text(at, Align2::RIGHT_BOTTOM, item.amount.to_string(), t.typography.body_s.font(), t.text);
    }
    let tip = [item.name.clone(), item.kind.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    resp.on_hover_text(tip);
}

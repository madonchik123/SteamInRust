//! Main window chrome: top navigation, friends drawer, status bar, toasts.

use ui::widgets::EditMenu;
use i18n::t;
use crate::app::SteamApp;
use client::Command;
use egui::{Align, Align2, Layout, Margin, Rect, Sense, Stroke, Ui, vec2};
use extensions::{Action, Env, Placement};
use model::{Persona, PersonaState, art};
use ui::theme::{NavPosition, SearchPlace, Side};
use ui::{Theme, icons, widgets as w};

/// The main window, laid out by the theme ([`ui::theme::Layout`]): the top
/// bar (sections, friends, notifications, account, window buttons), the
/// page's own bar, an optional left navigation column and status bar, the
/// friends list on either side, and the page. `docs/themes.md` lists the
/// options; `docs/redesign.md` describes the default layout.
pub fn show(app: &mut SteamApp, ui: &mut Ui, actions: &mut Vec<Action>) {
    let t = app.prefs.theme.clone();
    let lay = t.layout.clone();

    egui::Panel::top("topbar")
        .exact_size(lay.top_bar_height)
        .resizable(false)
        .frame(egui::Frame::new())
        .show(ui, |ui| {
            bar_background(ui, &t, &t.components.top_bar, false);
            top_bar(app, ui, &t, actions)
        });

    if lay.status_bar {
        let c = &t.components.status_bar;
        egui::Panel::bottom("statusbar")
            .exact_size(lay.status_bar_height)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(t.padding_of(c).x as i8, 0)))
            .show(ui, |ui| {
                bar_background(ui, &t, c, true);
                status_bar(app, ui, &t)
            });
    }

    if lay.nav == NavPosition::Left {
        egui::Panel::left("nav")
            .exact_size(lay.nav_width)
            .resizable(false)
            .frame(w::component_frame(&t, &t.components.sidebar).corner_radius(egui::CornerRadius::ZERO))
            .show(ui, |ui| nav_column(app, ui, &t, actions));
    }

    if lay.page_bar {
        let c = &t.components.page_bar;
        egui::Panel::top("pagebar")
            .exact_size(lay.page_bar_height)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(t.padding_of(c).x as i8, 0)))
            .show(ui, |ui| {
                bar_background(ui, &t, c, false);
                page_bar(app, ui, &t, actions)
            });
    }

    if app.prefs.friends_open && app.prefs.page != "community" {
        let frame = w::component_frame(&t, &t.components.sidebar).corner_radius(egui::CornerRadius::ZERO).inner_margin(Margin { left: 14, right: 10, top: 16, bottom: 8 });
        let panel = match lay.friends_side {
            Side::Left => egui::Panel::left("friends"),
            Side::Right => egui::Panel::right("friends"),
        };
        panel.exact_size(lay.friends_width).resizable(false).frame(frame).show(ui, |ui| friends_drawer(app, ui, &t, actions));
    }

    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        let page = app.prefs.page.clone();
        // The area's background from the theme; pages draw on top.
        w::paint_background(ui, &t, ui.max_rect(), section_of(&page));
        // Offline: pages that need Steam say so instead.
        if crate::offline_ui::page(app, ui, &page) {
            return;
        }
        if page == "downloads" {
            crate::offline_ui::downloads_notice(app, ui, &t);
        }
        let env = Env {
            state: &app.state,
            local: &app.local,
            theme: &app.prefs.theme,
            zoom: app.prefs.zoom,
            client: &app.client,
            actions: &mut *actions,
        };
        if !app.registry.show_page(&page, ui, env) {
            w::empty(ui, &t, icons::WARNING, "Page not found", &page);
        }
    });
}

/// A bar's fill from its component style, with its `border` color as a
/// line along the edge that meets the page (`top` for a bar at the bottom).
fn bar_background(ui: &Ui, t: &Theme, c: &ui::theme::Component, top: bool) {
    let rect = ui.max_rect().expand2(vec2(t.padding_of(c).x, 0.0));
    let look = t.look(c, Default::default());
    w::paint_fill(ui, t, rect, egui::CornerRadius::ZERO, &look.fill);
    if look.border.a() > 0 {
        let y = if top { rect.top() + 0.5 } else { rect.bottom() - 0.5 };
        ui.painter().hline(rect.x_range(), y, Stroke::new(1.0, look.border));
    }
}

/// Font sizes in the shell follow the theme's body text size.
fn sz(t: &Theme, px: f32) -> f32 {
    (px * t.typography.body_m.size / 14.0 * 2.0).round() / 2.0
}

/// The main sections, in the design's order: Store, Community, you,
/// Library, Downloads; then pages extensions add as main pages.
/// (id, label, icon)
fn nav_items(app: &SteamApp) -> Vec<(String, String, String)> {
    let pages = app.registry.pages();
    let name = if app.state.persona_name.is_empty() { app.state.account.clone() } else { app.state.persona_name.clone() };
    let mut tabs = Vec::new();
    let order = ["store", "community", "profile", "library"];
    for id in order {
        if let Some(p) = pages.iter().find(|p| p.id == id && p.placement == Placement::Main) {
            let label = if id == "profile" && !name.is_empty() { name.clone() } else { i18n::tr_dyn(&p.title) };
            tabs.push((p.id.clone(), label, p.icon.clone()));
        }
    }
    if pages.iter().any(|p| p.id == "downloads") {
        tabs.push(("downloads".into(), t!("Downloads").to_string(), icons::DOWNLOAD.to_string()));
    }
    for p in pages.iter().filter(|p| p.placement == Placement::Main && !order.contains(&p.id.as_str())) {
        tabs.push((p.id.clone(), i18n::tr_dyn(&p.title), p.icon.clone()));
    }
    tabs
}

/// The left navigation column (`nav = "left"`): the sections as rows with
/// icons, then extension pages.
fn nav_column(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let current = section_of(&app.prefs.page).to_string();
    let c = &t.components.sidebar_item;
    let fid = t.font_of(c);
    let pad = t.padding_of(c).x;
    let row = |ui: &mut Ui, icon: &str, label: &str, selected: bool| -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), t.height_of(c, 40.0)), Sense::click());
        let look = t.look(c, w::interact(ui, &resp, selected));
        w::paint_box(ui, t, rect, c, &look, if selected { 1.0 } else { 0.0 });
        w::icon_centered(ui.painter(), egui::pos2(rect.left() + pad + 9.0, rect.center().y), icon, fid.size + 3.0, look.icon);
        ui.painter().text(egui::pos2(rect.left() + pad + 30.0, rect.center().y), Align2::LEFT_CENTER, label, fid.clone(), look.text);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    };
    ui.spacing_mut().item_spacing.y = t.spacing.xs;
    for (id, label, icon) in nav_items(app) {
        let r = row(ui, &icon, &label, current == id);
        if id == "downloads" {
            if let Some((_, fraction, _)) = active_download(app) {
                w::progress_ring(ui.painter(), t, egui::pos2(r.rect.right() - pad - 8.0, r.rect.center().y), 7.5, fraction);
            }
        }
        if r.clicked() {
            actions.push(Action::Navigate { page: id, arg: None });
        }
    }
    let plugin_pages: Vec<_> = app.registry.pages().into_iter().filter(|p| p.placement == Placement::Apps).collect();
    if !plugin_pages.is_empty() {
        ui.add_space(t.spacing.md);
        w::caption(ui, t, t!("Plugins"));
        for p in plugin_pages {
            if row(ui, &p.icon, &p.title, app.prefs.page == p.id).clicked() {
                actions.push(Action::Navigate { page: p.id.clone(), arg: None });
            }
        }
    }
}

/// The app-wide search box (your games, your friends, the store), in the
/// page bar or the top bar as the theme says.
fn global_search(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>, width: f32, tall: bool) {
    let r = if tall { w::bar_search(ui, t, &mut app.search, t!("Search"), width) } else { w::search(ui, t, &mut app.search, t!("Search"), width) };
    if r.has_focus() {
        app.search_open = true;
    }
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if enter && !app.search.trim().is_empty() {
        actions.push(Action::Navigate { page: "store".into(), arg: Some(format!("search:{}", app.search.trim())) });
        app.search_open = false;
    }
    if app.search_open && !app.search.trim().is_empty() {
        search_results(app, ui.ctx(), r.rect, t, actions);
    }
}

/// Which top-bar section a page belongs to, so the right one lights up
/// (a game's community hub is part of Community, inventory of your profile).
pub fn section_of(page: &str) -> &str {
    match page {
        "store" => "store",
        "library" => "library",
        "downloads" => "downloads",
        "profile" | "inventory" => "profile",
        "community" | "workshop" => "community",
        p if crate::builtin::hub::PAGES.contains(&p) => "community",
        p => p,
    }
}

/// The top bar (Figma `333:2433` "client", the first 59 px): back and
/// forward, the logo (the app menu), the main sections; on the right the
/// friends and notifications buttons, the account, Help and View menus and
/// the window buttons. The right side is laid out first so the sections
/// always get what's left, spacing out as the window widens.
fn top_bar(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let full = ui.max_rect();
    // Empty parts of the bar move the window (we draw our own title bar).
    crate::chrome::drag_area(ui, full, true);
    let wide = full.width() >= 1400.0;

    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(full).layout(Layout::right_to_left(Align::Center)));
    {
        let ui = &mut right;
        ui.spacing_mut().item_spacing.x = 10.0;
        crate::chrome::window_buttons(ui, t, t.layout.top_bar_height, true);
        if !crate::chrome::SYSTEM_FRAME {
            ui.add_space(4.0);
        }
        view_menu(app, ui, t, actions);
        help_menu(app, ui, t, actions);
        ui.add_space(8.0);
        account_menu(app, ui, t, actions, wide);
        notifications_button(app, ui, t, actions);
        let unread = app.state.total_unread();
        let fr = w::bar_button(ui, t, icons::USERS, t!("Friends"), app.prefs.friends_open, unread);
        if fr.clicked() {
            if app.prefs.page == "community" {
                // The side panel doesn't show here: go to the friends page
                // (out of an open chat).
                actions.push(Action::Navigate { page: "community".into(), arg: None });
            } else {
                app.prefs.friends_open = !app.prefs.friends_open;
            }
        }
        friends_hover(app, ui, t, &fr, actions);
        crate::offline_ui::badge(app, ui, t);
        if t.layout.search == SearchPlace::TopBar {
            ui.add_space(t.spacing.sm);
            let room = ui.available_width() - 420.0;
            if room >= 160.0 {
                global_search(app, ui, t, actions, room.min(300.0), false);
            } else if w::icon_button(ui, t, icons::SEARCH, t!("Search"), false, 0).clicked() {
                actions.push(Action::Navigate { page: "store".into(), arg: None });
            }
        }
    }
    let right_edge = right.min_rect().left() - 16.0;

    let left_rect = Rect::from_min_max(full.min, egui::pos2(right_edge.max(full.left()), full.bottom()));
    // The window is briefly small while it grows after sign-in: nothing
    // fits on the left then (and laying it out would go negative).
    if left_rect.width() < 160.0 {
        return;
    }
    let mut left = ui.new_child(egui::UiBuilder::new().max_rect(left_rect).layout(Layout::left_to_right(Align::Center)));
    let ui = &mut left;
    ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
    ui.spacing_mut().item_spacing.x = 0.0;
    ui.add_space(t.padding_of(&t.components.top_bar).x.max(14.0));
    crate::chrome::traffic_light_space(ui);
    let back = !app.back.is_empty();
    if nav_arrow(ui, t, icons::CARET_LEFT, t!("Back"), back).clicked() && back {
        app.go_back();
    }
    let fwd = !app.forward.is_empty();
    if nav_arrow(ui, t, icons::CARET_RIGHT, t!("Forward"), fwd).clicked() && fwd {
        app.go_forward();
    }
    ui.add_space(14.0);
    app_menu(app, ui, t, actions);

    // The sections across the bar, unless the theme puts them in a column
    // on the left.
    if t.layout.nav == NavPosition::Left {
        return;
    }
    let pages = app.registry.pages();
    let current = section_of(&app.prefs.page).to_string();
    let mut tabs: Vec<(String, String)> = nav_items(app).into_iter().map(|(id, label, _)| (id, label)).collect();
    let plugin_pages: Vec<_> = pages.iter().filter(|p| p.placement == Placement::Apps).collect();
    let plugins_label = format!("{} {}", t!("Plugins"), icons::CARET_DOWN);

    // Spread the sections out: 66 px apart in the design, closer (and a
    // size smaller) when the window is narrow.
    let room = ui.available_width() - 8.0;
    let section = &t.components.section_tab;
    let family = t.font_of(section).family;
    let pad = t.padding_of(section).x * 2.0;
    let measure = |tabs: &[(String, String)], size: f32| -> f32 {
        let caps = |l: &str| if section.uppercase { l.to_uppercase() } else { l.to_string() };
        let width = |l: &str| ui.painter().layout_no_wrap(caps(l), egui::FontId::new(size, family.clone()), t.text).size().x + pad;
        let mut sum: f32 = tabs.iter().map(|(_, l)| width(l)).sum();
        if tabs.iter().any(|(id, _)| id == "downloads") {
            sum += downloads_extra(app);
        }
        if !plugin_pages.is_empty() {
            sum += width(&plugins_label);
        }
        sum
    };
    let count = |tabs: &[(String, String)]| (tabs.len() + usize::from(!plugin_pages.is_empty())) as f32;
    let full_size = t.font_of(section).size;
    let mut size = full_size;
    let mut used = measure(&tabs, size);
    if room - used < count(&tabs) * 22.0 {
        size = (full_size * 0.82).round();
        used = measure(&tabs, size);
    }
    // Still too narrow: your profile and Downloads are also in the account
    // and View menus, so they go first (the open one stays).
    for drop in ["profile", "downloads"] {
        if room - used >= count(&tabs) * 14.0 {
            break;
        }
        if current != drop {
            tabs.retain(|(id, _)| id != drop);
            used = measure(&tabs, size);
        }
    }
    let count = count(&tabs) as usize;
    let gap = ((room - used) / count.max(1) as f32).clamp(14.0, 66.0);

    for (id, label) in &tabs {
        if ui.available_width() < gap + 40.0 {
            break;
        }
        ui.add_space(gap);
        let r = w::section_tab_sized(ui, t, label, current == *id, t.layout.top_bar_height, size);
        let r = if id == "downloads" { downloads_status(app, ui, t, r) } else { r };
        if r.clicked() {
            actions.push(Action::Navigate { page: id.clone(), arg: None });
        }
        section_menu(app, ui, t, id, &r, actions);
    }
    if !plugin_pages.is_empty() && ui.available_width() >= gap + 40.0 {
        ui.add_space(gap);
        let active = plugin_pages.iter().any(|p| p.id == app.prefs.page);
        let resp = w::section_tab_sized(ui, t, &plugins_label, active, t.layout.top_bar_height, size);
        egui::Popup::menu(&resp).width(220.0).show(|ui| {
            for p in &plugin_pages {
                if w::menu_item(ui, t, &p.icon, &p.title).clicked() {
                    actions.push(Action::Navigate { page: p.id.clone(), arg: None });
                }
            }
        });
    }
}

/// The menu that drops down while the pointer is on a top-bar section
/// (Figma `18:295`, `18:798`, `18:847`, `18:894`): shortcuts into it.
fn section_menu(app: &mut SteamApp, ui: &mut Ui, t: &Theme, section: &str, tab: &egui::Response, actions: &mut Vec<Action>) {
    // (icon, label, page, argument)
    let items: Vec<(&str, String, &str, Option<&str>)> = match section {
        "store" => vec![
            (icons::HOUSE, t!("Home").into(), "store", None),
            (icons::SLIDERS, t!("Browse|store").into(), "store", Some("browse")),
            (icons::STACK, t!("Discovery queue").into(), "store", Some("discovery")),
            (icons::HEART, t!("Wishlist").into(), "store", Some("wishlist")),
            (icons::CART, t!("Cart").into(), "store", Some("cart")),
        ],
        "community" => vec![
            (icons::USERS, t!("Friends").into(), "community", None),
            (icons::CHATS, t!("Group chats").into(), "community", Some("groups")),
            (icons::CUBE, t!("Workshop").into(), "workshop", None),
            (icons::STOREFRONT, t!("My market listings").into(), "inventory", Some("listings")),
        ],
        "profile" => vec![
            (icons::USER_CIRCLE, t!("Profile").into(), "profile", None),
            (icons::BACKPACK, t!("Inventory").into(), "inventory", None),
            (icons::MEDAL, t!("Badges").into(), "profile", Some("badges")),
            (icons::USERS, t!("Friends").into(), "profile", Some("friends")),
            (icons::PAINT, t!("Edit profile").into(), "profile", Some("edit")),
        ],
        "library" => vec![
            (icons::HOUSE, t!("Home").into(), "library", None),
            (icons::DOWNLOAD, t!("Downloads").into(), "downloads", None),
            (icons::KEY, t!("Activate a product…").into(), "", None),
        ],
        _ => return,
    };
    let id = egui::Id::unique(("section-menu", section));
    let Some(area) = hover_popup(ui, id, tab, |ui| {
        let mut picked = None;
        for (icon, label, page, arg) in &items {
            if w::menu_item(ui, t, icon, label).clicked() {
                picked = Some((page.to_string(), arg.map(String::from)));
            }
        }
        picked
    }) else {
        return;
    };
    if let Some((page, arg)) = area {
        if page.is_empty() {
            app.activate = Some(Default::default());
        } else {
            actions.push(Action::Navigate { page, arg });
        }
        // Closed until the pointer leaves the tab and the menu.
        ui.data_mut(|d| d.insert_temp(id.with("dismissed"), true));
    }
}

/// A menu shown while the pointer is on `anchor` or on the menu itself,
/// opening after a short pause (so passing over doesn't flash it), right
/// under the anchor. `None` while closed; else what `add` returned.
fn hover_popup<R>(ui: &mut Ui, id: egui::Id, anchor: &egui::Response, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let now = ui.input(|i| i.time);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let popup_rect: Option<Rect> = ui.data(|d| d.get_temp(id.with("rect")));
    let over_popup = pointer.zip(popup_rect).is_some_and(|(p, r)| r.expand(4.0).contains(p));
    let over_anchor = pointer.is_some_and(|p| anchor.rect.contains(p)) && anchor.hovered();
    let dismissed: bool = ui.data(|d| d.get_temp(id.with("dismissed"))).unwrap_or(false);
    if !over_anchor && !over_popup {
        ui.data_mut(|d| {
            d.remove::<f64>(id);
            d.remove::<Rect>(id.with("rect"));
            d.remove::<bool>(id.with("dismissed"));
        });
        return None;
    }
    // Another menu (a click-open popup) takes precedence.
    if dismissed || egui::Popup::is_any_open(ui.ctx()) {
        return None;
    }
    let since: f64 = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    if now - since < 0.15 && popup_rect.is_none() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        return None;
    }
    let t = ui.visuals().clone();
    let pos = egui::pos2(anchor.rect.left() - 4.0, anchor.rect.bottom() - 8.0);
    let area = egui::Area::new(id.with("area")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::menu(ui.style()).fill(t.window_fill).show(ui, |ui| {
            ui.set_min_width(200.0);
            add(ui)
        })
        .inner
    });
    ui.data_mut(|d| d.insert_temp(id.with("rect"), area.response.rect));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    Some(area.inner)
}

/// The back and forward arrows at the far left of the top bar.
fn nav_arrow(ui: &mut Ui, t: &Theme, icon: &str, tip: &str, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 34.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() && enabled, 0.1);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, t.corner_small(), t.raised.gamma_multiply(hover));
    }
    let color = if enabled { w::mix(t.text_dim, t.text, 0.4 + 0.6 * hover) } else { t.text_faint.gamma_multiply(0.7) };
    w::icon_centered(ui.painter(), rect.center(), icon, 17.0, color);
    let resp = resp.on_hover_text(tip);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// The current download, if any: (app, fraction done, paused).
fn active_download(app: &SteamApp) -> Option<(model::AppId, f32, bool)> {
    app.state
        .downloads
        .values()
        .find(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed))
        .map(|d| (d.app_id, d.fraction(), d.paused))
}

/// Room the download ring and percentage take after "DOWNLOADS".
fn downloads_extra(app: &SteamApp) -> f32 {
    if active_download(app).is_some() { 56.0 } else { 0.0 }
}

/// The ring and percentage after "DOWNLOADS" while something downloads
/// (Figma `383:10133`), with what's going on as the tab's tooltip.
fn downloads_status(app: &SteamApp, ui: &mut Ui, t: &Theme, tab: egui::Response) -> egui::Response {
    let waiting = app.state.download_queue.len();
    let Some((id, fraction, paused)) = active_download(app) else {
        return if waiting > 0 { tab.on_hover_text(t!("{n} queued", n = waiting)) } else { tab };
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(56.0, t.layout.top_bar_height), Sense::click());
    let c = egui::pos2(rect.left() + 4.0 + 7.5, rect.center().y);
    w::progress_ring(ui.painter(), t, c, 7.5, fraction);
    let pct = format!("{:.0}%", fraction * 100.0);
    ui.painter().text(egui::pos2(c.x + 7.5 + 4.0, c.y), Align2::LEFT_CENTER, pct, w::font_bold(sz(&t, 13.0)), if paused { t.text_dim } else { t.accent });
    let name = app.state.app_name(id).unwrap_or("").to_string();
    let pct = format!("{:.0}", fraction * 100.0);
    let mut tip = if paused { t!("{game}: paused at {pct}%", game = name, pct = pct) } else { t!("{game}: {pct}%", game = name, pct = pct) };
    if waiting > 1 {
        tip = format!("{tip}\n{}", t!("+{n} queued", n = waiting - 1));
    }
    if !app.state.online() {
        tip = format!("{tip} {}", t!("· paused while offline"));
    }
    let both = tab.union(resp.on_hover_cursor(egui::CursorIcon::PointingHand));
    both.on_hover_text(tip)
}

/// The bar under the top bar: the page's own (tabs, search, view buttons)
/// when it has one, else the app-wide search.
fn page_bar(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let page = app.prefs.page.clone();
    let rect = ui.max_rect();
    let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    let env = Env {
        state: &app.state,
        local: &app.local,
        theme: &app.prefs.theme,
        zoom: app.prefs.zoom,
        client: &app.client,
        actions: &mut *actions,
    };
    if app.registry.show_page_bar(&page, &mut bar, env) {
        return;
    }
    // One search box for everything: your games, your friends, the store.
    if t.layout.search != SearchPlace::PageBar {
        return;
    }
    let ui = &mut bar;
    let width = (ui.available_width() - 10.0).min(374.0);
    if width >= 120.0 {
        global_search(app, ui, t, actions, width, true);
    }
}

/// A menu in the top bar shown as dim text with a caret ("View ▾").
fn text_menu_button(ui: &mut Ui, t: &Theme, label: &str, caret: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), w::font(sz(&t, 15.0)), t.text_dim);
    let caret_w = if caret { 18.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + caret_w + 8.0, 32.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let color = w::mix(t.text_dim, t.text, hover);
    ui.painter().galley(egui::pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0), galley, color);
    if caret {
        ui.painter().text(egui::pos2(rect.right() - 12.0, rect.center().y + 1.0), Align2::CENTER_CENTER, icons::CARET_DOWN, w::font(sz(&t, 12.0)), color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "View": other windows and places, like Steam's View menu.
fn view_menu(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let r = text_menu_button(ui, t, t!("View|menu"), true);
    egui::Popup::menu(&r).width(240.0).show(|ui| {
        let item = |ui: &mut Ui, icon: &str, label: &str| w::menu_item(ui, t, icon, label).clicked();
        if item(ui, icons::MONITOR, t!("Big Picture mode")) {
            app.bigpicture.enter(ui.ctx());
        }
        ui.separator();
        for (icon, label, page) in [
            (icons::STORE, t!("Store"), "store"),
            (icons::BOOKS, t!("Library"), "library"),
            (icons::USERS, t!("Community"), "community"),
            (icons::DOWNLOAD, t!("Downloads"), "downloads"),
            (icons::BACKPACK, t!("Inventory"), "inventory"),
        ] {
            if item(ui, icon, &label) {
                actions.push(Action::Navigate { page: page.into(), arg: None });
            }
        }
        if item(ui, icons::CHATS, t!("Friends & Chat")) {
            app.friends_window.open = true;
        }
        ui.separator();
        if item(ui, icons::GEAR, t!("Settings")) {
            actions.push(Action::Navigate { page: "settings".into(), arg: None });
        }
    });
}

/// "Help": support and updates.
fn help_menu(_app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let r = text_menu_button(ui, t, t!("Help|menu"), false);
    egui::Popup::menu(&r).width(280.0).show(|ui| {
        if w::menu_item(ui, t, icons::INFO, t!("Steam Support")).clicked() {
            actions.push(Action::Navigate { page: "support".into(), arg: None });
        }
        // Like Steam's "Check for Steam Client Updates…".
        if w::menu_item(ui, t, icons::REFRESH, t!("Check for SteamInRust updates…")).clicked() {
            crate::updater::check(ui.ctx().clone(), true);
        }
    });
}

/// Hovering the friends button shows a quick friends list, like Steam's,
/// with a way into the undocked Friends & Chat window.
fn friends_hover(app: &mut SteamApp, ui: &mut Ui, t: &Theme, button: &egui::Response, actions: &mut Vec<Action>) {
    let id = egui::Id::unique("friends-hover");
    let now = ui.input(|i| i.time);
    let mut since: Option<f64> = ui.data(|d| d.get_temp(id));
    let popup_rect: Option<Rect> = ui.data(|d| d.get_temp(id.with("rect")));
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let over_popup = pointer.zip(popup_rect).is_some_and(|(p, r)| r.expand(6.0).contains(p));
    if button.hovered() || over_popup {
        since.get_or_insert(now);
        ui.data_mut(|d| d.insert_temp(id.with("left"), now));
    } else {
        let left: f64 = ui.data(|d| d.get_temp(id.with("left"))).unwrap_or(0.0);
        if now - left > 0.25 {
            since = None;
        }
    }
    ui.data_mut(|d| {
        match since {
            Some(s) => {
                d.insert_temp(id, s);
            }
            None => d.remove::<f64>(id),
        }
    });
    // Short delay so passing over the button doesn't open it.
    let Some(start) = since else {
        ui.data_mut(|d| d.remove::<Rect>(id.with("rect")));
        return;
    };
    if now - start < 0.35 {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        return;
    }
    let pos = egui::pos2(button.rect.right() - 300.0, button.rect.bottom() + 6.0);
    let area = egui::Area::new(id.with("area")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        w::component_frame(t, &t.components.menu)
            .show(ui, |ui| {
                ui.set_width(300.0);
                let friends = app.state.friend_list();
                let online: Vec<_> = friends.iter().filter(|p| p.state != PersonaState::Offline).cloned().collect();
                ui.horizontal(|ui| {
                    ui.label(w::text_bold(t!("Friends"), sz(&t, 15.0), t.text));
                    ui.label(w::text(t!("{n} online", n = online.len()), 12.5, t.online));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::icon_button(ui, t, icons::EXTERNAL, t!("Open Friends & Chat"), false, 0).clicked() {
                            app.friends_window.open = true;
                        }
                    });
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    if online.is_empty() {
                        ui.label(w::text(t!("No friends online"), sz(&t, 13.0), t.text_faint));
                    }
                    for p in &online {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
                        }
                        let av = Rect::from_min_size(r.min + vec2(4.0, 5.0), vec2(32.0, 32.0));
                        if !w::paint_cover(ui, av, &art::avatar(p.avatar_hash.as_deref()), egui::CornerRadius::same(4)) {
                            w::skeleton(ui, t, av, egui::CornerRadius::same(4));
                        }
                        let color = status_color(t, p);
                        ui.painter().circle_filled(av.right_bottom() - vec2(1.0, 1.0), 4.5, color);
                        ui.painter().text(egui::pos2(av.right() + 10.0, r.top() + 14.0), Align2::LEFT_CENTER, &p.name, w::font_semibold(sz(&t, 13.0)), if p.in_game() { t.in_game } else { t.text });
                        ui.painter().text(egui::pos2(av.right() + 10.0, r.top() + 30.0), Align2::LEFT_CENTER, status_text(&app.state, p), w::font(sz(&t, 11.5)), color);
                        let unread = app.state.unread.get(&p.steam_id).copied().unwrap_or(0);
                        if unread > 0 {
                            let c = egui::pos2(r.right() - 12.0, r.center().y);
                            ui.painter().circle_filled(c, 8.0, t.accent);
                            ui.painter().text(c, Align2::CENTER_CENTER, unread.to_string(), w::font_bold(sz(&t, 10.0)), egui::Color32::WHITE);
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Send a message")).clicked() {
                            // Chat in the undocked window, like Steam.
                            app.friends_window.open = true;
                            app.friends_window.chat = Some(p.steam_id);
                            actions.push(Action::MarkRead(p.steam_id));
                        }
                    }
                });
                ui.separator();
                if ui.add(egui::Button::new(w::text_medium(format!("{}   {}", icons::USERS, t!("All friends")), 13.0, t.accent)).frame(false)).clicked() {
                    actions.push(Action::Navigate { page: "community".into(), arg: None });
                }
            });
    });
    ui.data_mut(|d| d.insert_temp(id.with("rect"), area.response.rect));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
}

/// Icon and plain-language text for a notification. Covers every type
/// Steam defines (ESteamNotificationType).
pub fn notification_text(state: &client::State, n: &model::Notification) -> (&'static str, String) {
    let who = || n.from.map(|id| state.persona(id).name).filter(|s| !s.is_empty()).unwrap_or_else(|| t!("Someone").to_string());
    let game_of = |id: Option<model::AppId>| id.filter(|a| *a != 0 && *a != 753).and_then(|a| state.app_name(a).map(String::from)).or_else(|| n.text.clone());
    let game = |id: Option<model::AppId>| game_of(id).unwrap_or_else(|| t!("a game").to_string());
    let s = |x: &str| x.to_string();
    let (icon, text) = match n.kind {
        1 => (icons::BELL, t!("Test notification").to_string()),
        2 => match game_of(n.app_id) {
            Some(g) if n.from.is_some() => (icons::GIFT, t!("{name} sent you {game} as a gift", name = who(), game = g)),
            Some(g) => (icons::GIFT, t!("You received {game} as a gift", game = g)),
            None if n.from.is_some() => (icons::GIFT, t!("{name} sent you a gift", name = who())),
            None => (icons::GIFT, s(t!("You received a gift"))),
        },
        3 if n.from == Some(state.steam_id) || n.from.is_none() => (icons::CHAT, s(t!("New comment on your profile"))),
        3 => (icons::CHAT, t!("{name} replied in a discussion you follow", name = who())),
        4 if n.app_id == Some(753) && n.source_app.is_some_and(|a| a != 0) => (icons::BACKPACK, t!("New item from {game}", game = game(n.source_app))),
        4 if n.app_id == Some(753) => (icons::BACKPACK, s(t!("New item in your Steam inventory"))),
        4 => (icons::BACKPACK, t!("New item in your {game} inventory", game = game(n.app_id))),
        5 => (icons::USER_PLUS, t!("{name} sent you a friend request", name = who())),
        6 => (icons::TAG, s(t!("A big Steam sale has started"))),
        7 => (icons::DOWNLOAD, t!("{game} is ready to preload", game = game(n.app_id))),
        8 => (icons::HEART, t!("{game} from your wishlist is on sale", game = game(n.app_id))),
        9 => (icons::SWAP, t!("{name} sent you a trade offer", name = who())),
        10 => (icons::BELL, n.text.clone().unwrap_or_else(|| s(t!("New Steam notification")))),
        11 => (icons::INFO, s(t!("Steam Support replied to your request"))),
        12 => (icons::CONTROLLER, t!("It's your turn in {game}", game = game(n.app_id))),
        13 => (icons::CHATS, t!("New message from {name}", name = who())),
        14 => (icons::SHIELD, s(t!("You have a message from Steam moderators"))),
        15 => (icons::USERS_THREE, t!("{name} asked to use a feature", name = who())),
        16 => (icons::USERS_THREE, t!("{name} invited you to their Steam Family", name = who())),
        17 => (icons::CART, t!("{name} asked you to buy {game}", name = who(), game = game(n.app_id))),
        18 => (icons::CLOCK, t!("{name} asked for more playtime", name = who())),
        19 => (icons::CART, t!("Your request to buy {game} was answered", game = game(n.app_id))),
        20 => (icons::USERS_THREE, s(t!("Your feature request was answered"))),
        21 => (icons::CLOCK, s(t!("Your playtime request was answered"))),
        22 => (icons::BOOKS, t!("{game} was added to your library", game = game(n.app_id))),
        23 => (icons::SEND, s(t!("Sent to your phone"))),
        24 => (icons::CAMERA, t!("Your {game} clip is ready", game = game(n.app_id))),
        25 => (icons::KEY, s(t!("Someone is signing in to your account. Check the Steam Mobile app."))),
        26 => (icons::SHIELD, s(t!("Something is waiting for you to confirm in the Steam Mobile app"))),
        27 => (icons::CALENDAR, t!("{game} has a new event", game = game(n.app_id))),
        28 => (icons::CONTROLLER, t!("You're invited to the {game} playtest", game = game(n.app_id))),
        29 => (icons::WARNING, s(t!("A trade was reversed"))),
        30 => (icons::SHIELD, s(t!("Steam took action on content you reported"))),
        _ => (icons::BELL, n.text.clone().unwrap_or_else(|| s(t!("New Steam notification")))),
    };
    // "{game} ..." can open with the lowercase fallback "a game".
    let mut chars = text.chars();
    let text = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or(text);
    (icon, text)
}

/// Where clicking a notification takes you.
fn open_notification(state: &client::State, n: &model::Notification, actions: &mut Vec<Action>) {
    let nav = |page: &str, arg: Option<String>| Action::Navigate { page: page.into(), arg };
    let web = |url: String| {
        crate::links::open_http(&url);
    };
    let me = state.steam_id;
    let app = n.app_id.filter(|a| *a != 0 && *a != 753);
    match n.kind {
        2 => web(format!("https://steamcommunity.com/profiles/{me}/inventory/#pending_gifts")),
        3 => actions.push(nav("profile", Some(me.to_string()))),
        4 => actions.push(nav("inventory", Some(format!("{me}/{}", n.app_id.unwrap_or(753))))),
        5 => actions.push(match n.from {
            Some(id) => nav("profile", Some(id.to_string())),
            None => nav("community", None),
        }),
        13 => {
            if let Some(id) = n.from {
                actions.push(Action::OpenChat(id));
            }
        }
        6 => actions.push(nav("store", None)),
        7 | 12 | 22 | 24 => actions.push(nav("library", app.map(|a| a.to_string()))),
        8 | 27 | 28 => actions.push(nav("store", app.map(|a| format!("app:{a}")))),
        9 | 29 => actions.push(nav("inventory", Some("trades".into()))),
        11 => actions.push(nav("support", n.ticket.as_ref().map(|t| format!("ticket:{t}")))),
        15..=21 => web("https://store.steampowered.com/account/familymanagement".into()),
        _ => {}
    }
}

/// The picture for a notification: which game it's about or who sent it,
/// with the kind of notification as a small badge, like Steam's.
fn notification_picture(ui: &Ui, t: &Theme, state: &client::State, n: &model::Notification, icon: &str, rect: Rect) {
    let radius = egui::CornerRadius::same(6);
    let game = match n.kind {
        4 if n.app_id == Some(753) => n.source_app,
        _ => n.app_id,
    }
    .filter(|a| *a != 0 && *a != 753);
    let person = n.from.filter(|id| *id != state.steam_id);
    // People first where the person is the point; the game otherwise.
    let person_first = matches!(n.kind, 3 | 5 | 9 | 13 | 15 | 16 | 18);
    let draw_person = |id: u64| w::paint_cover(ui, rect, &art::avatar(state.persona(id).avatar_hash.as_deref()), radius);
    // Portrait library art crops well to a square; fall back to the header.
    let draw_game = |app: model::AppId| w::paint_cover(ui, rect, &art::capsule(app), radius) || w::paint_cover(ui, rect, &art::header(app), radius);
    let drawn = match (person, game) {
        (Some(p), _) if person_first => draw_person(p),
        (_, Some(g)) => draw_game(g),
        (Some(p), None) => draw_person(p),
        (None, None) => false,
    };
    let p = ui.painter();
    if !drawn {
        p.rect_filled(rect, radius, t.raised);
        p.text(rect.center(), Align2::CENTER_CENTER, icon, w::font(sz(&t, 20.0)), if n.read { t.text_dim } else { t.accent });
        return;
    }
    // Kind badge in the corner.
    let c = rect.right_bottom() - vec2(3.0, 3.0);
    p.circle_filled(c, 10.0, t.chrome);
    p.circle_filled(c, 8.5, if n.read { t.raised } else { t.accent });
    p.text(c, Align2::CENTER_CENTER, icon, w::font(sz(&t, 10.5)), if n.read { t.text_dim } else { t.on_accent });
}

/// The bell: Steam notifications with an unread badge.
fn notifications_button(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let Some(mut data) = app.state.notifications.clone() else { return };
    data.items.sort_by_key(|n| std::cmp::Reverse(n.time));
    let waiting = data.pending_gifts + data.pending_friends + data.confirmations;
    let badge = (data.unread + waiting) as usize;
    let r = w::bar_button(ui, t, icons::BELL, t!("Notifications"), false, badge);
    egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(380.0).show(|ui| {
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("Notifications"), sz(&t, 16.0), t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if data.unread > 0 && ui.link(w::text(t!("Mark all as read"), sz(&t, 12.5), t.accent)).clicked() {
                        app.client.send(Command::MarkNotificationsRead { ids: vec![] });
                    }
                });
            });
            // Things waiting for you, like the summary rows in Steam's bell.
            let mut summary = Vec::new();
            if data.pending_gifts > 0 {
                summary.push((icons::GIFT, t!("{n} gifts waiting for you", n = data.pending_gifts), 2));
            }
            if data.pending_friends > 0 {
                summary.push((icons::USER_PLUS, t!("{n} friend requests", n = data.pending_friends), 5));
            }
            if data.confirmations > 0 {
                summary.push((icons::SHIELD, t!("{n} confirmations waiting in the Steam Mobile app", n = data.confirmations), 26));
            }
            for (icon, text, kind) in summary {
                if ui.add(egui::Button::new(w::text_semibold(format!("{icon}  {text}"), sz(&t, 13.0), t.accent)).frame(false)).clicked() {
                    let row = model::Notification { kind, ..Default::default() };
                    open_notification(&app.state, &row, actions);
                }
            }
            ui.separator();
            if data.items.is_empty() {
                ui.label(w::text(t!("You're all caught up."), sz(&t, 13.0), t.text_faint));
                return;
            }
            egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
                for n in data.items.iter().take(40) {
                    let (icon, text) = notification_text(&app.state, n);
                    // Up to two lines of text, then the time.
                    let mut job = egui::text::LayoutJob::simple(text.clone(), w::font_medium(sz(&t, 13.0)), if n.read { t.text_dim } else { t.text }, ui.available_width() - 86.0);
                    job.wrap.max_rows = 2;
                    job.wrap.overflow_character = Some('…');
                    let galley = ui.painter().layout_job(job);
                    let h = (galley.size().y + 34.0).max(58.0);
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
                    }
                    let color = if n.read { t.text_dim } else { t.text };
                    notification_picture(ui, t, &app.state, n, icon, Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(44.0, 44.0)));
                    let p = ui.painter();
                    let text_h = galley.size().y;
                    p.galley(egui::pos2(rect.left() + 60.0, rect.top() + 9.0), galley, color);
                    p.text(egui::pos2(rect.left() + 60.0, rect.top() + 14.0 + text_h), Align2::LEFT_TOP, ui::ago(n.time), w::font(sz(&t, 11.5)), t.text_faint);
                    if !n.read {
                        p.circle_filled(egui::pos2(rect.right() - 12.0, rect.center().y), 4.0, t.accent);
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&text).clicked() {
                        if !n.read {
                            app.client.send(Command::MarkNotificationsRead { ids: vec![n.id] });
                        }
                        open_notification(&app.state, n, actions);
                        egui::Popup::close_all(ui.ctx());
                    }
                }
            });
        });
    });
}

/// The wallet part of the account menu: balance, money on its way, and
/// wallet actions.
fn wallet_section(ui: &mut Ui, t: &Theme, balance: i64, currency: i32, pending: i64, actions: &mut Vec<Action>) {
    // Menus justify their text; the notes read better left-aligned.
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        ui.add_space(2.0);
        ui.label(w::caption_text(t, t!("Steam Wallet")));
        ui.horizontal(|ui| {
            ui.label(w::text_bold(model::format_money(balance, currency), sz(&t, 20.0), t.text)).on_hover_text(t!("Available to spend"));
            if pending > 0 {
                // What "pending" means, on hover: the menu stays short.
                let note = t!("Money from recent Market sales or refunds that Steam hasn't released yet. It's added to your balance automatically.");
                ui.label(w::text_semibold(format!("{}  {}", icons::CLOCK, t!("Pending: {amount}", amount = model::format_money(pending, currency))), 13.0, t.away)).on_hover_text(note);
            }
        });
        ui.add_space(4.0);
    });
    let item = |ui: &mut Ui, icon: &str, label: &str| w::menu_item(ui, t, icon, label).clicked();
    if item(ui, icons::PLUS, t!("Add funds…")) {
        crate::links::open_http("https://store.steampowered.com/steamaccount/addfunds");
    }
    if item(ui, icons::TAG, t!("Redeem a Steam Wallet code…")) {
        crate::links::open_http("https://store.steampowered.com/account/redeemwalletcode");
    }
    if item(ui, icons::STOREFRONT, t!("My market listings")) {
        actions.push(Action::Navigate { page: "inventory".into(), arg: Some("listings".into()) });
    }
    if item(ui, icons::SCROLL, t!("Purchase history")) {
        actions.push(Action::Navigate { page: "settings".into(), arg: Some("history".into()) });
    }
}

/// The logo is the app's menu, like Steam's "Steam" menu. Only the logo,
/// with no name beside it.
fn app_menu(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter().rect_filled(rect, t.corner_small(), t.raised.gamma_multiply(hover));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(rect.center(), vec2(26.0, 26.0))));
    w::logo(&mut c, t, 26.0);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("SteamInRust");
    egui::Popup::menu(&resp).width(300.0).show(|ui| {
        let item = |ui: &mut Ui, icon: &str, label: &str| w::menu_item(ui, t, icon, label).clicked();
        if item(ui, icons::GEAR, t!("Settings")) {
            actions.push(Action::Navigate { page: "settings".into(), arg: None });
        }
        if item(ui, icons::KEY, t!("Activate a product…")) {
            app.activate = Some(Default::default());
        }
        if item(ui, icons::TAG, t!("Redeem a Steam Wallet code…")) {
            crate::links::open_http("https://store.steampowered.com/account/redeemwalletcode");
        }
        if item(ui, icons::PLUGS, t!("Plugins")) {
            actions.push(Action::Navigate { page: "settings".into(), arg: Some("plugins".into()) });
        }
        ui.separator();
        if item(ui, icons::USERS, t!("Switch account…")) {
            app.client.send(Command::LogOut);
        }
        if item(ui, icons::POWER, t!("Exit")) {
            app.request_exit(ui.ctx());
        }
    });
}

/// One search box for everything: your games, your friends, and the store.
fn search_results(app: &mut SteamApp, ctx: &egui::Context, anchor: Rect, t: &Theme, actions: &mut Vec<Action>) {
    let q = app.search.trim().to_lowercase();
    let games: Vec<(model::AppId, String)> = crate::builtin::entries(&app.state, &app.local)
        .into_iter()
        .filter(|e| e.name.to_lowercase().contains(&q))
        .take(5)
        .map(|e| (e.app_id, e.name.to_string()))
        .collect();
    let friends: Vec<Persona> = app.state.friend_list().into_iter().filter(|p| p.name.to_lowercase().contains(&q)).take(4).collect();
    let mut close = false;
    let area = egui::Area::new(egui::Id::unique("global-search"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom() + vec2(0.0, 6.0))
        .show(ctx, |ui| {
            w::component_frame(t, &t.components.menu)
                .show(ui, |ui| {
                    ui.set_width(anchor.width().max(320.0));
                    let row = |ui: &mut Ui, icon: &str, text: &str| {
                        ui.add(egui::Button::new(w::text_medium(format!("{icon}   {text}"), sz(&t, 13.5), t.text)).frame(false).min_size(vec2(ui.available_width(), 32.0))).clicked()
                    };
                    if !games.is_empty() {
                        w::caption(ui, t, t!("Your games"));
                        for (id, name) in &games {
                            if row(ui, icons::CONTROLLER, name) {
                                actions.push(Action::Navigate { page: "library".into(), arg: Some(id.to_string()) });
                                close = true;
                            }
                        }
                    }
                    if !friends.is_empty() {
                        w::caption(ui, t, t!("Friends"));
                        for p in &friends {
                            if row(ui, icons::USER, &p.name) {
                                actions.push(Action::OpenChat(p.steam_id));
                                close = true;
                            }
                        }
                    }
                    w::caption(ui, t, t!("Store"));
                    if row(ui, icons::STORE, &t!("Search the store for “{q}”", q = app.search.trim())) {
                        actions.push(Action::Navigate { page: "store".into(), arg: Some(format!("search:{}", app.search.trim())) });
                        close = true;
                    }
                });
        });
    // Close when clicking anywhere else.
    let clicked_outside = ctx.input(|i| i.pointer.any_click()) && !area.response.hovered() && !ctx.pointer_hover_pos().is_some_and(|p| anchor.contains(p));
    if close || clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.search_open = false;
        if close {
            app.search.clear();
        }
    }
}

/// "Activate a product" dialog (product keys).
pub fn activate_dialog(app: &mut SteamApp, ctx: &egui::Context) {
    let t = app.prefs.theme.clone();
    let Some(dlg) = &mut app.activate else { return };
    let mut close = false;
    egui::Modal::new(egui::Id::unique("activate")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(w::text(icons::KEY, sz(&t, 28.0), t.accent));
        ui.label(w::text_bold(t!("Activate a product on Steam"), sz(&t, 20.0), t.text));
        ui.add_space(4.0);
        ui.label(w::text(t!("Enter a product key from a retail box, a bundle or a gift email."), sz(&t, 13.5), t.text_dim));
        ui.add_space(10.0);
        ui.add_enabled(
            !dlg.busy,
            egui::TextEdit::singleline(&mut dlg.key).hint_text("XXXXX-XXXXX-XXXXX").font(w::font_semibold(sz(&t, 18.0))).desired_width(f32::INFINITY).margin(vec2(12.0, 10.0)),
        ).with_edit_menu();
        match &dlg.result {
            Some(Ok(items)) => {
                ui.label(w::text(format!("{}  {}", icons::CHECK, t!("Activated!")), 14.0, t.play));
                for i in items {
                    ui.label(w::text(format!("•  {i}"), sz(&t, 13.5), t.text));
                }
            }
            Some(Err(e)) => {
                ui.label(w::text(format!("{}  {}", icons::WARNING, i18n::tr_dyn(e)), 13.5, t.danger));
            }
            None => {}
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ready = dlg.key.trim().len() >= 5 && !dlg.busy;
            if ui.add_enabled_ui(ready, |ui| w::button(ui, &t, None, t!("Activate"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                dlg.busy = true;
                dlg.result = None;
                app.client.send(Command::ActivateKey { key: dlg.key.trim().to_string() });
            }
            if dlg.busy {
                ui.add(egui::Spinner::new().color(t.accent));
            }
            if w::button(ui, &t, None, t!("Close"), w::Kind::Secondary, w::Size::Md).clicked() {
                close = true;
            }
        });
    });
    if close {
        app.activate = None;
    }
}

/// The account button (Figma `18:1220`): avatar, name and wallet balance
/// on Bg / Highlight. Its menu has your status, profile, settings, wallet
/// and sign-out. `wide` shows the balance (it's in the menu either way).
fn account_menu(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>, wide: bool) {
    let me = app.state.me().cloned().unwrap_or_default();
    let name = if app.state.persona_name.is_empty() { &app.state.account } else { &app.state.persona_name };
    let short: String = if name.chars().count() > 16 { name.chars().take(15).chain(std::iter::once('…')).collect() } else { name.clone() };
    let color = status_color(t, &me);
    let wallet = app.state.wallet.map(|(b, c)| model::format_money(b, c)).filter(|_| wide);
    // Painted as one fixed-size widget so it behaves inside right-to-left layouts.
    let name_g = ui.painter().layout_no_wrap(short, w::font(sz(&t, 14.0)), t.text);
    let wallet_g = wallet.map(|s| ui.painter().layout_no_wrap(s, w::font(sz(&t, 14.0)), t.text_dim));
    let wallet_w = wallet_g.as_ref().map(|g| g.size().x + 8.0).unwrap_or(0.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(3.0 + 31.0 + 8.0 + name_g.size().x + 8.0 + wallet_w + 24.0, 39.0), Sense::click());
    // The bar buttons' style (Bg / Highlight in the default theme).
    let style = &t.components.bar_button;
    let look = t.look(style, w::interact(ui, &resp, false));
    w::paint_box(ui, t, rect, style, &look, 0.0);
    let av = Rect::from_min_size(rect.min + vec2(3.0, 4.0), vec2(31.0, 31.0));
    if !w::paint_cover(ui, av, &art::avatar(me.avatar_hash.as_deref()), egui::CornerRadius::same(2)) {
        ui.painter().rect_filled(av, egui::CornerRadius::same(2), t.surface_hover);
    }
    // Your status as a small dot on the picture's corner.
    let dot = av.right_bottom() - vec2(2.0, 2.0);
    ui.painter().circle_filled(dot, 5.0, t.raised);
    status_mark(ui.painter(), dot, 3.5, me.state, color);
    let mut x = av.right() + 8.0;
    let p = ui.painter();
    let name_w = name_g.size().x;
    p.galley(egui::pos2(x, rect.center().y - name_g.size().y / 2.0), name_g, t.text);
    x += name_w + 8.0;
    if let Some(g) = wallet_g {
        p.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, t.text_dim);
    }
    p.text(rect.right_center() - vec2(13.0, 0.0), Align2::CENTER_CENTER, icons::CARET_DOWN, w::font(sz(&t, 13.0)), look.icon);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&resp).width(280.0).show(|ui| {
        crate::offline_ui::account_items(app, ui, t);
        ui.label(w::caption_text(t, t!("Status")));
        for s in PersonaState::SELECTABLE {
            // A colored dot per status, like Steam's.
            let on = me.state == s;
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
            if on || resp.hovered() {
                ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
            }
            let probe = model::Persona { state: s, ..Default::default() };
            let dot = egui::pos2(r.left() + 20.0, r.center().y);
            status_mark(ui.painter(), dot, 5.0, s, status_color(t, &probe));
            ui.painter().text(egui::pos2(r.left() + 40.0, r.center().y), Align2::LEFT_CENTER, i18n::tr_dyn(s.label()), w::font(sz(&t, 14.5)), t.text);
            if on {
                ui.painter().text(egui::pos2(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, icons::CHECK, w::font(sz(&t, 13.0)), t.accent);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.client.send(Command::SetPersonaState(s));
            }
        }
        ui.separator();
        for (icon, label, page) in [(icons::USER_CIRCLE, t!("View profile"), "profile"), (icons::GEAR, t!("Settings"), "settings")] {
            if w::menu_item(ui, t, icon, &label).clicked() {
                actions.push(Action::Navigate { page: page.into(), arg: None });
            }
        }
        if let Some((balance, currency)) = app.state.wallet {
            ui.separator();
            wallet_section(ui, t, balance, currency, app.state.wallet_pending, actions);
        }
        ui.separator();
        if w::menu_item(ui, t, icons::SIGN_OUT, t!("Sign out of account")).clicked() {
            app.client.send(Command::LogOut);
        }
        if w::menu_item_tinted(ui, t, icons::POWER, t!("Exit SteamInRust"), Some(t.danger)).clicked() {
            app.request_exit(ui.ctx());
        }
    });
}

/// The bottom bar (`layout.status_bar`): what's downloading, what you're
/// playing, and Friends & Chat.
fn status_bar(app: &mut SteamApp, ui: &mut Ui, t: &Theme) {
    ui.horizontal_centered(|ui| {
        let dl = ui.add(egui::Label::new(w::text(format!("{}  {}", icons::DOWNLOAD, t!("Downloads")), 12.5, t.text_dim)).sense(Sense::click()));
        if dl.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.navigate("downloads".into(), None);
        }
        // Same state as the Downloads page: what's downloading and what's waiting.
        let active = app.state.downloads.values().find(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).cloned();
        let waiting = app.state.download_queue.iter().filter(|id| active.as_ref().is_none_or(|d| d.app_id != **id)).count();
        match &active {
            Some(d) => {
                let name = app.state.app_name(d.app_id).unwrap_or("").to_string();
                // The same measure as the Downloads page (what this download fetches).
                let pct = d.fraction() as f64 * 100.0;
                let text = if d.paused { t!("{game}: paused at {pct}%", game = name, pct = format!("{pct:.0}")) } else { t!("{game}: {pct}%", game = name, pct = format!("{pct:.0}")) };
                ui.label(w::text(text, sz(&t, 12.5), t.accent));
                w::progress(ui, t, (pct / 100.0) as f32, 90.0);
                if waiting > 0 {
                    ui.label(w::text(t!("+{n} queued", n = waiting), sz(&t, 12.5), t.text_faint));
                }
            }
            None if waiting > 0 => {
                ui.label(w::text(t!("{n} queued", n = waiting), sz(&t, 12.5), t.text_faint));
            }
            None => {
                ui.label(w::text(t!("Nothing in the queue"), sz(&t, 12.5), t.text_faint));
            }
        }
        if !app.state.online() {
            ui.label(w::text(t!("· paused while offline"), sz(&t, 12.5), t.away));
        }
        if let Some(id) = app.state.playing {
            ui.add_space(16.0);
            let name = app.state.app_name(id).unwrap_or("").to_string();
            ui.label(w::text(format!("{}  {}", icons::CONTROLLER, t!("Playing {game}", game = name)), 12.5, t.in_game));
            ui.label(w::text(t!("· Shift+Tab opens the overlay"), sz(&t, 12.5), t.text_faint));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let online = app.state.friend_list().iter().filter(|p| p.state != PersonaState::Offline).count();
            let label = format!("{}  {}  ·  {}", icons::CHATS, t!("Friends & Chat"), t!("{n} online", n = online));
            let r = ui.add(egui::Label::new(w::text_medium(label, sz(&t, 12.5), t.text_dim)).sense(Sense::click()));
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.friends_window.open = !app.friends_window.open;
            }
        });
    });
}

/// The mark for a status in pickers: a dot, with a bar for Do Not Disturb
/// and an empty ring for Invisible, so they differ by more than color.
pub fn status_mark(p: &egui::Painter, at: egui::Pos2, r: f32, state: PersonaState, color: egui::Color32) {
    match state {
        PersonaState::Invisible | PersonaState::Offline => {
            p.circle_stroke(at, r - 0.75, egui::Stroke::new(1.5, color));
        }
        PersonaState::Busy => {
            p.circle_filled(at, r, color);
            p.line_segment([at - vec2(r * 0.55, 0.0), at + vec2(r * 0.55, 0.0)], egui::Stroke::new(1.6, egui::Color32::WHITE));
        }
        _ => {
            p.circle_filled(at, r, color);
        }
    }
}

pub fn status_color(t: &Theme, p: &Persona) -> egui::Color32 {
    if p.in_game() {
        return t.in_game;
    }
    match p.state {
        PersonaState::Offline | PersonaState::Invisible => t.text_faint,
        // Do not disturb in red, like the tray menu; away and snooze in amber.
        PersonaState::Busy => t.danger,
        PersonaState::Away | PersonaState::Snooze => t.away,
        _ => t.online,
    }
}

/// What a friend can be invited to while you play, like Steam's friend menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Invite {
    /// The game's matchmaking lobby.
    Lobby(model::AppId, u64),
    /// The game itself, with its join string (rich presence `connect`).
    Game(model::AppId, String),
    /// Just start the game (it says nothing about how to join).
    Play(model::AppId),
}

/// The invites that make sense right now, best first, with their labels.
pub fn invite_options(state: &client::State) -> Vec<(Invite, String)> {
    let Some(app) = state.playing else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(lobby) = state.game_lobby {
        out.push((Invite::Lobby(app, lobby), t!("Invite to lobby").to_string()));
    }
    if let Some(connect) = state.game_connect.clone() {
        out.push((Invite::Game(app, connect), t!("Invite to game").to_string()));
    }
    if out.is_empty() {
        let game = state.app_name(app).unwrap_or_default().to_string();
        out.push((Invite::Play(app), t!("Invite to play {game}", game = game).to_string()));
    }
    out
}

/// Send an invite. Lobby invites are Steam's own (every client shows them).
/// The others are a `steam://run` link with the join arguments: Steam
/// strips invite markup that doesn't come from its own client, and any Steam
/// client opens the link. Ours shows it as an invite card.
pub fn send_invite(app: &mut SteamApp, to: &Persona, invite: Invite, actions: &mut Vec<Action>) {
    let friend = to.steam_id;
    let game = |id| app.state.app_name(id).unwrap_or_default().to_string();
    match invite {
        Invite::Lobby(app_id, lobby) => app.client.send(Command::InviteToLobby { app_id, lobby, friend }),
        Invite::Game(app_id, connect) => {
            let link = model::GameInvite { app_id, connect }.link();
            let text = t!("Join me in {game}! {link}", game = game(app_id), link = link).to_string();
            app.client.send(Command::SendMessage { to: friend, text });
        }
        Invite::Play(app_id) => {
            let link = model::GameInvite { app_id, connect: String::new() }.link();
            let text = t!("Play {game} with me! {link}", game = game(app_id), link = link).to_string();
            app.client.send(Command::SendMessage { to: friend, text });
        }
    }
    let offline = to.state == PersonaState::Offline;
    actions.push(Action::Toast(if offline { t!("Invite sent. {name} will see it when they come online.", name = to.name) } else { t!("Invite sent to {name}", name = to.name) }));
}

/// Where a game opens: your library if you own it, else its store page.
pub fn game_page(state: &client::State, app: model::AppId) -> Action {
    let owned = state.apps.as_ref().is_some_and(|a| a.contains_key(&app));
    if owned {
        Action::Navigate { page: "library".into(), arg: Some(app.to_string()) }
    } else {
        Action::Navigate { page: "store".into(), arg: Some(format!("app:{app}")) }
    }
}

pub fn status_text(state: &client::State, p: &Persona) -> String {
    let game = p.game_app_id.and_then(|id| state.app_name(id).map(String::from)).or_else(|| p.game_name.clone());
    if p.in_game() {
        let mut line = t!("Playing {game}", game = game.unwrap_or_else(|| t!("a game").to_string()));
        if let Some(rich) = &p.rich_status {
            line = format!("{line} · {rich}");
        }
        return line;
    }
    match (p.state, p.last_logoff) {
        (PersonaState::Offline, Some(t)) if t > 0 => t!("Last online {when}", when = ui::ago(t)),
        (s, _) => i18n::tr_dyn(s.label()),
    }
}

fn friends_drawer(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let list = app.state.friend_list();
    let online = list.iter().filter(|p| p.state != PersonaState::Offline).count();
    ui.horizontal(|ui| {
        ui.label(w::text_bold(t!("Friends"), sz(&t, 17.0), t.text));
        ui.label(w::text(t!("{n} online", n = online), sz(&t, 13.0), t.text_dim));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked() {
                app.prefs.friends_open = false;
            }
        });
    });
    ui.add_space(6.0);
    let width = ui.available_width() - 4.0;
    w::search(ui, t, &mut app.friend_filter, t!("Search friends"), width);
    ui.add_space(10.0);
    crate::offline_ui::friends_notice(app, ui, t);
    friend_list(app, ui, t, actions, &list, None);
}

/// Grouped friend list, shared by the drawer and the Community page.
/// Returns the friend that was clicked.
pub fn friend_list(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>, list: &[Persona], selected: Option<u64>) -> Option<u64> {
    let filter = app.friend_filter.to_lowercase();
    let ok = |p: &&Persona| filter.is_empty() || p.name.to_lowercase().contains(&filter) || p.nickname.as_ref().is_some_and(|n| n.to_lowercase().contains(&filter));
    let favs = app.prefs.favorite_friends.clone();
    let fav = |p: &&Persona| favs.contains(&p.steam_id);
    let favorites: Vec<&Persona> = list.iter().filter(ok).filter(fav).collect();
    let in_game: Vec<&Persona> = list.iter().filter(ok).filter(|p| !fav(p) && p.in_game()).collect();
    let online: Vec<&Persona> = list.iter().filter(ok).filter(|p| !fav(p) && !p.in_game() && p.state != PersonaState::Offline).collect();
    let offline: Vec<&Persona> = list.iter().filter(ok).filter(|p| !fav(p) && p.state == PersonaState::Offline).collect();
    let mut clicked = None;

    egui::ScrollArea::vertical().id_salt("friend-list").auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            ui.label(w::text(t!("Loading friends…"), sz(&t, 13.0), t.text_faint));
        }
        for (title, group) in [(t!("Favorites"), &favorites), (t!("In game"), &in_game), (t!("Online"), &online), (t!("Offline"), &offline)] {
            if group.is_empty() {
                continue;
            }
            ui.add_space(4.0);
            w::caption(ui, t, &format!("{title} — {}", group.len()));
            ui.add_space(2.0);
            for p in group.iter() {
                if friend_row(app, ui, t, p, actions, selected == Some(p.steam_id)) {
                    clicked = Some(p.steam_id);
                }
            }
            ui.add_space(8.0);
        }
    });
    if let Some(id) = clicked {
        if selected.is_none() {
            actions.push(Action::OpenChat(id));
        }
    }
    clicked
}

fn friend_row(app: &mut SteamApp, ui: &mut Ui, t: &Theme, p: &Persona, actions: &mut Vec<Action>, selected: bool) -> bool {
    let offline = p.state == PersonaState::Offline;
    let color = status_color(t, p);
    let name = p.display_name();
    let status = if app.state.is_typing(p.steam_id) { t!("typing…").to_string() } else { status_text(&app.state, p) };
    let unread = app.state.unread.get(&p.steam_id).copied().unwrap_or(0);

    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    // The whole row lights up wherever the pointer is in it (also over the
    // game link inside it), and clearly enough to see on the dark list.
    let pointer_in = ui.rect_contains_pointer(rect) || resp.context_menu_opened();
    let hover = ui.ctx().animate_bool_with_time(resp.id, pointer_in, 0.1);
    if selected {
        ui.painter().rect_filled(rect, t.corner_small(), t.accent.gamma_multiply(0.18));
    } else if hover > 0.0 {
        ui.painter().rect_filled(rect, t.corner_small(), w::mix(egui::Color32::TRANSPARENT, t.surface_hover, hover));
    }
    // Leave room on the right for the unread badge, so text never runs under it.
    let inner = rect.shrink2(vec2(6.0, 6.0));
    let inner = if unread > 0 { Rect::from_min_max(inner.min, egui::pos2(inner.right() - 26.0, inner.bottom())) } else { inner };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    w::avatar(&mut child, t, &art::avatar(p.avatar_hash.as_deref()), 36.0, Some(color));
    child.add_space(4.0);
    child.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add_space(1.0);
        let nc = if offline { t.text_faint } else if p.in_game() { t.in_game } else { t.text };
        ui.add(egui::Label::new(w::text_medium(&name, 14.0, nc)).truncate());
        // The game is a link to its page.
        let r = ui.add(egui::Label::new(w::text(&status, 12.0, if offline { t.text_faint } else if p.in_game() { t.in_game } else { t.text_dim })).truncate().sense(Sense::click()));
        if let Some(target) = p.game_app_id.filter(|_| p.in_game()) {
            // The game is a link of its own: underlined while the pointer is
            // on it, so it's clear a click opens the game, not the chat.
            if r.hovered() {
                ui.painter().hline(r.rect.x_range(), r.rect.bottom() - 1.0, Stroke::new(1.0, t.in_game));
            }
            let r = r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Open the game's page"));
            if r.clicked() {
                actions.push(game_page(&app.state, target));
            }
        }
    });
    if unread > 0 {
        let c = rect.right_center() - vec2(16.0, 0.0);
        ui.painter().circle_filled(c, 9.0, t.accent);
        ui.painter().text(c, Align2::CENTER_CENTER, unread.min(99).to_string(), w::font_bold(sz(&t, 10.0)), t.on_accent);
    }

    let persona = p.clone();
    let vp = ui.ctx().viewport_id();
    resp.context_menu(|ui| {
        ui.label(w::text_semibold(&name, sz(&t, 14.0), t.text));
        if persona.nickname.is_some() && !persona.name.is_empty() {
            ui.label(w::text(&persona.name, sz(&t, 12.0), t.text_dim));
        }
        ui.separator();
        if ui.button(format!("{}  {}", icons::CHAT, t!("Send a message"))).clicked() {
            actions.push(Action::OpenChat(persona.steam_id));
            ui.close();
        }
        // In a game: join them, see their server, watch their broadcast, like Steam.
        if let Some(game) = persona.game_app_id.filter(|_| persona.in_game()) {
            let game_name = app.state.app_name(game).map(String::from).unwrap_or_else(|| t!("the game").to_string());
            ui.separator();
            if let Some(args) = persona.join_args() {
                if ui.button(format!("{}  {}", icons::PLAY, t!("Join game"))).on_hover_text(&args).clicked() {
                    match app.local.games.iter().find(|g| g.app_id == game) {
                        Some(g) => {
                            app.client.send(Command::Launch { app_id: game, install_dir: g.install_dir.clone(), args: client::LaunchArgs::connect(args) });
                            actions.push(Action::Toast(t!("Joining {name} in {game}…", name = persona.display_name(), game = game_name)));
                        }
                        None => actions.push(Action::Toast(t!("Install {game} to join {name}.", game = game_name, name = persona.display_name()))),
                    }
                    ui.close();
                }
            }
            if let Some(addr) = persona.server.clone() {
                if ui.button(format!("{}  {}", icons::INFO, t!("View server info"))).clicked() {
                    app.client.send(Command::QueryServer { addr: addr.clone() });
                    app.friend_dialog = Some((vp, FriendDialog::Server { name: persona.display_name(), game, addr }));
                    ui.close();
                }
            }
            if persona.broadcasting && ui.button(format!("{}  {}", icons::EYE, t!("Watch game"))).on_hover_text(t!("Opens their Steam broadcast")).clicked() {
                crate::links::open_http(&format!("https://steamcommunity.com/broadcast/watch/{}", persona.steam_id));
                ui.close();
            }
            if ui.button(format!("{}  {}", icons::CONTROLLER, t!("View game info"))).clicked() {
                actions.push(game_page(&app.state, game));
                ui.close();
            }
        }
        // While you're in a game: invite them to it, its lobby or its match.
        for (invite, label) in invite_options(&app.state) {
            if ui.button(format!("{}  {}", icons::USER_PLUS, label)).clicked() {
                send_invite(app, &persona, invite, actions);
                ui.close();
            }
        }
        ui.separator();
        if ui.button(format!("{}  {}", icons::USER_CIRCLE, t!("View profile"))).clicked() {
            actions.push(Action::Navigate { page: "profile".into(), arg: Some(persona.steam_id.to_string()) });
            ui.close();
        }
        if ui.button(format!("{}  {}", icons::BACKPACK, t!("Inventory"))).clicked() {
            actions.push(Action::Navigate { page: "inventory".into(), arg: Some(persona.steam_id.to_string()) });
            ui.close();
        }
        ui.menu_button(format!("{}  {}", icons::GEAR, t!("Manage")), |ui| {
            if ui.button(t!("Add nickname…")).clicked() {
                app.friend_dialog = Some((vp, FriendDialog::Nickname { id: persona.steam_id, text: persona.nickname.clone().unwrap_or_default() }));
                ui.close();
            }
            if persona.nickname.is_some() && ui.button(t!("Remove nickname")).clicked() {
                app.client.send(Command::SetNickname { steam_id: persona.steam_id, nickname: None });
                ui.close();
            }
            let is_fav = app.prefs.favorite_friends.contains(&persona.steam_id);
            if ui.button(if is_fav { t!("Remove from Favorites") } else { t!("Add to Favorites") }).clicked() {
                if is_fav {
                    app.prefs.favorite_friends.retain(|f| *f != persona.steam_id);
                } else {
                    app.prefs.favorite_friends.push(persona.steam_id);
                }
                ui.close();
            }
            ui.menu_button(t!("Copy SteamID"), |ui| {
                crate::builtin::steam_id_menu(ui, &t, persona.steam_id);
            });
            ui.separator();
            if ui.button(w::text(t!("Remove friend…"), sz(&t, 13.5), t.danger)).clicked() {
                app.friend_dialog = Some((vp, FriendDialog::Remove { id: persona.steam_id, name: persona.display_name() }));
                ui.close();
            }
            if ui.button(w::text(t!("Block all communication…"), sz(&t, 13.5), t.danger)).clicked() {
                app.friend_dialog = Some((vp, FriendDialog::Block { id: persona.steam_id, name: persona.display_name() }));
                ui.close();
            }
        });
        let env = Env {
            state: &app.state,
            local: &app.local,
            theme: &app.prefs.theme,
            zoom: app.prefs.zoom,
            client: &app.client,
            actions: &mut *actions,
        };
        app.registry.friend_menu(&persona, ui, env);
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

pub fn toasts(app: &mut SteamApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    app.toasts.retain(|t| t.until > now);
    if app.toasts.is_empty() {
        return;
    }
    let t = &app.prefs.theme;
    let above = if t.layout.status_bar { t.layout.status_bar_height + 14.0 } else { 20.0 };
    egui::Area::new(egui::Id::unique("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -above))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for toast in app.toasts.iter().rev().take(4) {
                let card = w::component_frame(t, &t.components.toast)
                    .show(ui, |ui| {
                        ui.set_width(320.0);
                        ui.label(w::text_semibold(&toast.title, sz(&t, 14.0), t.text));
                        if !toast.body.is_empty() {
                            ui.add(egui::Label::new(w::text(&toast.body, sz(&t, 13.0), t.text_dim)).wrap());
                        }
                    });
                // Accent bar inside the card, sized to it once the text is laid out.
                let r = card.response.rect;
                let bar = Rect::from_min_max(egui::pos2(r.left() + 9.0, r.top() + 10.0), egui::pos2(r.left() + 12.0, r.bottom() - 10.0));
                let mark = t.look(&t.components.toast, ui::theme::Interact { selected: true, ..Default::default() });
                if let ui::theme::ResolvedFill::Color(c) = mark.fill {
                    ui.painter().rect_filled(bar, egui::CornerRadius::same(2), c);
                }
                ui.add_space(8.0);
            }
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(200));
}

/// Dialogs opened from a friend's menu.
#[derive(Clone)]
pub enum FriendDialog {
    Nickname { id: model::SteamId, text: String },
    Server { name: String, game: model::AppId, addr: String },
    Remove { id: model::SteamId, name: String },
    Block { id: model::SteamId, name: String },
}

/// Draw the open friend dialog, in the window it was opened from.
pub fn friend_dialogs(app: &mut SteamApp, ctx: &egui::Context) {
    let Some((vp, dialog)) = app.friend_dialog.clone() else { return };
    if vp != ctx.viewport_id() {
        return;
    }
    let t = app.prefs.theme.clone();
    let mut close = false;
    let mut keep = dialog.clone();
    // Fits narrow windows too (the Friends window can be ~300 px wide).
    let width = (ctx.content_rect().width() - 64.0).clamp(220.0, 400.0);
    egui::Modal::new(egui::Id::unique("friend-dialog")).show(ctx, |ui| {
        ui.set_width(width);
        match &mut keep {
            FriendDialog::Nickname { id, text } => {
                ui.label(w::text_bold(t!("Add a nickname"), sz(&t, 17.0), t.text));
                ui.label(w::text(t!("Only you see it. Steam keeps it on all your devices."), sz(&t, 12.5), t.text_dim));
                ui.add_space(8.0);
                let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).hint_text(t!("Nickname"))).with_edit_menu();
                r.request_focus();
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let save = w::button(ui, &t, None, t!("Save"), w::Kind::Primary, w::Size::Sm).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if save {
                        let nick = text.trim().to_string();
                        app.client.send(Command::SetNickname { steam_id: *id, nickname: (!nick.is_empty()).then_some(nick) });
                        close = true;
                    }
                    if w::button(ui, &t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        close = true;
                    }
                });
            }
            FriendDialog::Server { name, game, addr } => {
                let game_name = app.state.app_name(*game).unwrap_or_default().to_string();
                ui.add(egui::Label::new(w::text_bold(t!("{name}'s server", name = name), sz(&t, 17.0), t.text)).wrap());
                ui.label(w::text(&game_name, sz(&t, 12.5), t.text_dim));
                ui.add_space(8.0);
                match app.state.servers.get(addr.as_str()) {
                    None => {
                        ui.label(w::text(t!("Asking the server…"), sz(&t, 13.0), t.text_dim));
                    }
                    Some(Err(e)) => {
                        ui.label(w::text(e, sz(&t, 13.0), t.text_dim));
                    }
                    Some(Ok(s)) => {
                        egui::Grid::new("server-info").num_columns(2).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
                            for (k, v) in [
                                (t!("Name"), s.name.clone()),
                                (t!("Map"), s.map.clone()),
                                (t!("Players"), format!("{} / {}{}", s.players, s.max_players, if s.bots > 0 { format!(" (+{} bots)", s.bots) } else { String::new() })),
                                (t!("Ping"), format!("{} ms", s.ping_ms)),
                                (t!("Anti-cheat"), if s.vac { "VAC".to_string() } else { "—".to_string() }),
                                (t!("Password"), if s.password { t!("Yes").to_string() } else { t!("No").to_string() }),
                            ] {
                                ui.label(w::text_bold(k.to_uppercase(), sz(&t, 10.5), t.text_faint));
                                ui.add(egui::Label::new(w::text(v, sz(&t, 13.0), t.text)).truncate());
                                ui.end_row();
                            }
                        });
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(w::text(addr.as_str(), sz(&t, 12.5), t.text_dim));
                    if ui.small_button(t!("Copy")).clicked() {
                        ui.ctx().copy_text(addr.clone());
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if let Some(g) = app.local.games.iter().find(|g| g.app_id == *game) {
                        if w::button(ui, &t, Some(icons::PLAY), t!("Join"), w::Kind::Primary, w::Size::Sm).clicked() {
                            app.client.send(Command::Launch { app_id: *game, install_dir: g.install_dir.clone(), args: client::LaunchArgs::connect(format!("+connect {addr}")) });
                            close = true;
                        }
                    }
                    if w::button(ui, &t, Some(icons::REFRESH), t!("Refresh"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        app.client.send(Command::QueryServer { addr: addr.clone() });
                    }
                    if w::button(ui, &t, None, t!("Close"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        close = true;
                    }
                });
            }
            FriendDialog::Remove { id, name } => {
                ui.add(egui::Label::new(w::text_bold(t!("Remove {name} from your friends?", name = name), sz(&t, 17.0), t.text)).wrap());
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if w::button(ui, &t, None, t!("Remove friend"), w::Kind::Danger, w::Size::Sm).clicked() {
                        app.client.send(Command::RemoveFriend { steam_id: *id });
                        close = true;
                    }
                    if w::button(ui, &t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        close = true;
                    }
                });
            }
            FriendDialog::Block { id, name } => {
                ui.add(egui::Label::new(w::text_bold(t!("Block all communication with {name}?", name = name), sz(&t, 17.0), t.text)).wrap());
                ui.add(egui::Label::new(w::text(t!("They'll be removed from your friends and can't message you or invite you."), sz(&t, 12.5), t.text_dim)).wrap());
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if w::button(ui, &t, None, t!("Block"), w::Kind::Danger, w::Size::Sm).clicked() {
                        app.client.send(Command::BlockFriend { steam_id: *id, block: true });
                        close = true;
                    }
                    if w::button(ui, &t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        close = true;
                    }
                });
            }
        }
    });
    if close {
        app.friend_dialog = None;
    } else if let Some((_, d)) = &mut app.friend_dialog {
        *d = keep;
    }
}

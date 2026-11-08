//! Main window chrome: top navigation, friends drawer, status bar, toasts.

use i18n::t;
use crate::app::SteamApp;
use client::Command;
use egui::{Align, Align2, Layout, Margin, Rect, Sense, Stroke, Ui, vec2};
use extensions::{Action, Env, Placement};
use model::{Persona, PersonaState, art};
use ui::{Theme, icons, widgets as w};

pub fn show(app: &mut SteamApp, ui: &mut Ui, actions: &mut Vec<Action>) {
    let t = app.prefs.theme.clone();

    egui::Panel::top("topbar")
        .exact_size(60.0)
        .resizable(false)
        .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin { left: 16, right: 0, top: 0, bottom: 0 }))
        .show(ui, |ui| top_bar(app, ui, &t, actions));

    egui::Panel::bottom("statusbar")
        .exact_size(34.0)
        .resizable(false)
        .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(16, 0)))
        .show(ui, |ui| status_bar(app, ui, &t));

    if app.prefs.friends_open && app.prefs.page != "community" {
        egui::Panel::right("friends")
            .exact_size(290.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin { left: 14, right: 10, top: 16, bottom: 8 }))
            .show(ui, |ui| friends_drawer(app, ui, &t, actions));
    }

    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        let page = app.prefs.page.clone();
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

fn top_bar(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let rect = ui.max_rect();
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, t.border));
    // Empty parts of the bar move the window (we draw our own title bar).
    crate::chrome::drag_area(ui, rect.expand2(vec2(16.0, 0.0)), true);
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_enabled_ui(!app.back.is_empty(), |ui| {
            if w::icon_button(ui, t, icons::ARROW_LEFT, t!("Back"), false, 0).clicked() {
                app.go_back();
            }
        });
        ui.add_enabled_ui(!app.forward.is_empty(), |ui| {
            if w::icon_button(ui, t, icons::ARROW_RIGHT, t!("Forward"), false, 0).clicked() {
                app.go_forward();
            }
        });
        ui.add_space(10.0);
        app_menu(app, ui, t, actions);
        ui.add_space(16.0);

        let pages = app.registry.pages();
        let current = app.prefs.page.clone();
        for p in pages.iter().filter(|p| p.placement == Placement::Main) {
            if w::nav_tab(ui, t, &i18n::tr_dyn(&p.title), current == p.id, 60.0).clicked() {
                actions.push(Action::Navigate { page: p.id.clone(), arg: None });
            }
        }
        let plugin_pages: Vec<_> = pages.iter().filter(|p| p.placement == Placement::Apps).collect();
        if !plugin_pages.is_empty() {
            let active = plugin_pages.iter().any(|p| p.id == current);
            let resp = w::nav_tab(ui, t, &format!("{} {}", t!("Plugins"), icons::CARET_DOWN), active, 60.0);
            egui::Popup::menu(&resp).width(220.0).show(|ui| {
                for p in &plugin_pages {
                    let label = format!("{}   {}", p.icon, p.title);
                    if ui.add(egui::Button::selectable(current == p.id, w::text_medium(label, 14.0, t.text)).min_size(vec2(200.0, 34.0))).clicked() {
                        actions.push(Action::Navigate { page: p.id.clone(), arg: None });
                    }
                }
            });
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            crate::chrome::window_buttons(ui, t, 60.0, true);
            ui.add_space(6.0);
            account_menu(app, ui, t, actions);
            crate::offline_ui::badge(app, ui, t);
            if let Some((balance, currency)) = app.state.wallet {
                wallet_button(ui, t, balance, currency, app.state.wallet_pending, actions);
            }
            let unread = app.state.total_unread();
            notifications_button(app, ui, t, actions);
            let fr = w::icon_button(ui, t, icons::USERS, t!("Friends"), app.prefs.friends_open, unread);
            if fr.clicked() {
                app.prefs.friends_open = !app.prefs.friends_open;
            }
            friends_hover(app, ui, t, &fr, actions);
            ui.add_space(8.0);
            // Shrinks, then collapses to an icon, so it never covers the tabs.
            let room = ui.available_width() - 16.0;
            if room >= 180.0 {
                let r = w::search(ui, t, &mut app.search, t!("Search"), room.min(300.0));
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
            } else if room >= 40.0 && w::icon_button(ui, t, icons::SEARCH, t!("Search"), false, 0).clicked() {
                actions.push(Action::Navigate { page: "store".into(), arg: None });
            }
        });
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
        egui::Frame::new()
            .fill(t.chrome)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(t.corner())
            .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: egui::Color32::from_black_alpha(140) })
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(300.0);
                let friends = app.state.friend_list();
                let online: Vec<_> = friends.iter().filter(|p| p.state != PersonaState::Offline).cloned().collect();
                ui.horizontal(|ui| {
                    ui.label(w::text_bold(t!("Friends"), 15.0, t.text));
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
                        ui.label(w::text(t!("No friends online"), 13.0, t.text_faint));
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
                        ui.painter().text(egui::pos2(av.right() + 10.0, r.top() + 14.0), Align2::LEFT_CENTER, &p.name, w::font_semibold(13.0), if p.in_game() { t.in_game } else { t.text });
                        ui.painter().text(egui::pos2(av.right() + 10.0, r.top() + 30.0), Align2::LEFT_CENTER, status_text(&app.state, p), w::font(11.5), color);
                        let unread = app.state.unread.get(&p.steam_id).copied().unwrap_or(0);
                        if unread > 0 {
                            let c = egui::pos2(r.right() - 12.0, r.center().y);
                            ui.painter().circle_filled(c, 8.0, t.accent);
                            ui.painter().text(c, Align2::CENTER_CENTER, unread.to_string(), w::font_bold(10.0), egui::Color32::WHITE);
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
    match n.kind {
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
    }
}

/// Where clicking a notification takes you.
fn open_notification(state: &client::State, n: &model::Notification, actions: &mut Vec<Action>) {
    let nav = |page: &str, arg: Option<String>| Action::Navigate { page: page.into(), arg };
    let web = |url: String| {
        let _ = open::that_detached(url);
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
        9 | 29 => web(format!("https://steamcommunity.com/profiles/{me}/tradeoffers/")),
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
        p.text(rect.center(), Align2::CENTER_CENTER, icon, w::font(20.0), if n.read { t.text_dim } else { t.accent });
        return;
    }
    // Kind badge in the corner.
    let c = rect.right_bottom() - vec2(3.0, 3.0);
    p.circle_filled(c, 10.0, t.chrome);
    p.circle_filled(c, 8.5, if n.read { t.raised } else { t.accent });
    p.text(c, Align2::CENTER_CENTER, icon, w::font(10.5), if n.read { t.text_dim } else { t.on_accent });
}

/// The bell: Steam notifications with an unread badge.
fn notifications_button(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let Some(mut data) = app.state.notifications.clone() else { return };
    data.items.sort_by_key(|n| std::cmp::Reverse(n.time));
    let waiting = data.pending_gifts + data.pending_friends + data.confirmations;
    let badge = (data.unread + waiting) as usize;
    let r = w::icon_button(ui, t, icons::BELL, t!("Notifications"), false, badge);
    egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(380.0).show(|ui| {
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("Notifications"), 16.0, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if data.unread > 0 && ui.link(w::text(t!("Mark all as read"), 12.5, t.accent)).clicked() {
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
                if ui.add(egui::Button::new(w::text_semibold(format!("{icon}  {text}"), 13.0, t.accent)).frame(false)).clicked() {
                    let row = model::Notification { kind, ..Default::default() };
                    open_notification(&app.state, &row, actions);
                }
            }
            ui.separator();
            if data.items.is_empty() {
                ui.label(w::text(t!("You're all caught up."), 13.0, t.text_faint));
                return;
            }
            egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
                for n in data.items.iter().take(40) {
                    let (icon, text) = notification_text(&app.state, n);
                    // Up to two lines of text, then the time.
                    let mut job = egui::text::LayoutJob::simple(text.clone(), w::font_medium(13.0), if n.read { t.text_dim } else { t.text }, ui.available_width() - 86.0);
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
                    p.text(egui::pos2(rect.left() + 60.0, rect.top() + 14.0 + text_h), Align2::LEFT_TOP, ui::ago(n.time), w::font(11.5), t.text_faint);
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

/// Wallet balance in the top bar; click for pending funds and wallet actions.
fn wallet_button(ui: &mut Ui, t: &Theme, balance: i64, currency: i32, pending: i64, actions: &mut Vec<Action>) {
    let mut label = model::format_money(balance, currency);
    if pending > 0 {
        label = format!("{label}  ·  +{}", model::format_money(pending, currency));
    }
    let r = ui.add(egui::Button::new(w::text_semibold(label, 13.0, t.text)).fill(t.raised).corner_radius(t.corner_small()));
    let r = r.on_hover_text(t!("Steam Wallet"));
    egui::Popup::menu(&r).show(|ui| {
        ui.set_width(300.0);
        // Menus justify their text; the notes read better left-aligned.
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
        ui.label(w::caption_text(t, t!("Steam Wallet")));
        ui.label(w::text_bold(model::format_money(balance, currency), 24.0, t.text));
        ui.label(w::text(t!("Available to spend"), 12.5, t.text_dim));
        if pending > 0 {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(w::text(icons::CLOCK, 14.0, t.away));
                ui.label(w::text_semibold(t!("Pending: {amount}", amount = model::format_money(pending, currency)), 13.5, t.away));
            });
            ui.add(egui::Label::new(w::text(t!("Money from recent Market sales or refunds that Steam hasn't released yet. It's added to your balance automatically."), 12.0, t.text_dim)).wrap());
        }
        ui.add_space(6.0);
        ui.add(egui::Label::new(w::text(t!("Wallet funds can be spent in the Store and on the Community Market. They can't be withdrawn."), 12.0, t.text_faint)).wrap());
        });
        ui.separator();
        let item = |ui: &mut Ui, icon: &str, label: &str| ui.add(egui::Button::new(w::text_medium(format!("{icon}   {label}"), 13.5, t.text)).frame(false).min_size(vec2(280.0, 30.0))).clicked();
        if item(ui, icons::PLUS, t!("Add funds…")) {
            let _ = open::that_detached("https://store.steampowered.com/steamaccount/addfunds");
        }
        if item(ui, icons::TAG, t!("Redeem a Steam Wallet code…")) {
            let _ = open::that_detached("https://store.steampowered.com/account/redeemwalletcode");
        }
        if item(ui, icons::STOREFRONT, t!("My market listings")) {
            actions.push(Action::Navigate { page: "inventory".into(), arg: Some("listings".into()) });
        }
        if item(ui, icons::SCROLL, t!("Purchase history")) {
            let _ = open::that_detached("https://store.steampowered.com/account/history/");
        }
    });
}

/// The logo is a menu, like Steam's "Steam" menu.
fn app_menu(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let galley = ui.painter().layout_no_wrap("SteamInRust".into(), w::font_bold(15.0), t.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0 + 8.0 + galley.size().x + 22.0, 40.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter().rect_filled(rect, t.corner_small(), w::mix(egui::Color32::TRANSPARENT, t.surface_hover, hover));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.min + vec2(4.0, 8.0), vec2(24.0, 24.0))));
    w::logo(&mut c, t, 24.0);
    ui.painter().galley(egui::pos2(rect.left() + 34.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
    ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::CENTER_CENTER, icons::CARET_DOWN, w::font(12.0), t.text_faint);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&resp).width(250.0).show(|ui| {
        let item = |ui: &mut Ui, icon: &str, label: &str| ui.add(egui::Button::new(w::text_medium(format!("{icon}   {label}"), 13.5, t.text)).frame(false).min_size(vec2(230.0, 32.0))).clicked();
        if item(ui, icons::GEAR, t!("Settings")) {
            actions.push(Action::Navigate { page: "settings".into(), arg: None });
        }
        if item(ui, icons::DOWNLOAD, t!("Downloads")) {
            actions.push(Action::Navigate { page: "downloads".into(), arg: None });
        }
        if item(ui, icons::BACKPACK, t!("Inventory")) {
            actions.push(Action::Navigate { page: "inventory".into(), arg: None });
        }
        if item(ui, icons::INFO, t!("Steam Support")) {
            actions.push(Action::Navigate { page: "support".into(), arg: None });
        }
        ui.separator();
        if item(ui, icons::KEY, t!("Activate a product…")) {
            app.activate = Some(Default::default());
        }
        if item(ui, icons::TAG, t!("Redeem a Steam Wallet code…")) {
            let _ = open::that_detached("https://store.steampowered.com/account/redeemwalletcode");
        }
        if item(ui, icons::PLUGS, t!("Plugins")) {
            actions.push(Action::Navigate { page: "settings".into(), arg: Some("plugins".into()) });
        }
        // Like Steam's "Check for Steam Client Updates…".
        if item(ui, icons::REFRESH, t!("Check for SteamInRust updates…")) {
            crate::updater::check(ui.ctx().clone(), true);
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
            egui::Frame::new()
                .fill(t.raised)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(t.corner())
                .shadow(egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: egui::Color32::from_black_alpha(140) })
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(anchor.width().max(320.0));
                    let row = |ui: &mut Ui, icon: &str, text: &str| {
                        ui.add(egui::Button::new(w::text_medium(format!("{icon}   {text}"), 13.5, t.text)).frame(false).min_size(vec2(ui.available_width(), 32.0))).clicked()
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
        ui.label(w::text(icons::KEY, 28.0, t.accent));
        ui.label(w::text_bold(t!("Activate a product on Steam"), 20.0, t.text));
        ui.add_space(4.0);
        ui.label(w::text(t!("Enter a product key from a retail box, a bundle or a gift email."), 13.5, t.text_dim));
        ui.add_space(10.0);
        ui.add_enabled(
            !dlg.busy,
            egui::TextEdit::singleline(&mut dlg.key).hint_text("XXXXX-XXXXX-XXXXX").font(w::font_semibold(18.0)).desired_width(f32::INFINITY).margin(vec2(12.0, 10.0)),
        );
        match &dlg.result {
            Some(Ok(items)) => {
                ui.label(w::text(format!("{}  {}", icons::CHECK, t!("Activated!")), 14.0, t.play));
                for i in items {
                    ui.label(w::text(format!("•  {i}"), 13.5, t.text));
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

fn account_menu(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let me = app.state.me().cloned().unwrap_or_default();
    let name = if app.state.persona_name.is_empty() { &app.state.account } else { &app.state.persona_name };
    let short: String = if name.chars().count() > 16 { name.chars().take(15).chain(std::iter::once('…')).collect() } else { name.clone() };
    let color = status_color(t, &me);
    // Painted as one fixed-size widget so it behaves inside right-to-left layouts.
    let name_g = ui.painter().layout_no_wrap(short, w::font_semibold(13.5), t.text);
    let status_g = ui.painter().layout_no_wrap(i18n::tr_dyn(me.state.label()), w::font(11.5), color);
    let text_w = name_g.size().x.max(status_g.size().x);
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0 + 10.0 + text_w + 22.0, 44.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter().rect_filled(rect, t.corner_small(), w::mix(egui::Color32::TRANSPARENT, t.surface_hover, hover));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.min + vec2(4.0, 5.0), vec2(34.0, 34.0))));
    w::avatar(&mut child, t, &art::avatar(me.avatar_hash.as_deref()), 34.0, Some(color));
    let x = rect.left() + 46.0;
    let p = ui.painter();
    p.galley(egui::pos2(x, rect.center().y - name_g.size().y + 1.0), name_g, t.text);
    p.galley(egui::pos2(x, rect.center().y + 1.0), status_g, color);
    p.text(rect.right_center() - vec2(10.0, 0.0), Align2::CENTER_CENTER, icons::CARET_DOWN, w::font(12.0), t.text_faint);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&resp).width(230.0).show(|ui| {
        crate::offline_ui::account_items(app, ui, t);
        ui.label(w::caption_text(t, t!("Status")));
        for s in PersonaState::SELECTABLE {
            // A colored dot per status, like Steam's.
            let on = me.state == s;
            let (r, resp) = ui.allocate_exact_size(vec2(210.0, 32.0), Sense::click());
            if on || resp.hovered() {
                ui.painter().rect_filled(r, t.corner_small(), if on { t.accent.gamma_multiply(0.18) } else { t.surface_hover });
            }
            let probe = model::Persona { state: s, ..Default::default() };
            let dot = egui::pos2(r.left() + 16.0, r.center().y);
            ui.painter().circle_filled(dot, 5.0, status_color(t, &probe));
            ui.painter().text(egui::pos2(r.left() + 32.0, r.center().y), Align2::LEFT_CENTER, i18n::tr_dyn(s.label()), w::font_medium(13.5), t.text);
            if on {
                ui.painter().text(egui::pos2(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, icons::CHECK, w::font(13.0), t.accent);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.client.send(Command::SetPersonaState(s));
            }
        }
        ui.separator();
        for (icon, label, page) in [(icons::USER_CIRCLE, t!("View profile"), "profile"), (icons::GEAR, t!("Settings"), "settings")] {
            if ui.add(egui::Button::new(w::text_medium(format!("{icon}   {label}"), 13.5, t.text)).frame(false).min_size(vec2(210.0, 32.0))).clicked() {
                actions.push(Action::Navigate { page: page.into(), arg: None });
            }
        }
        ui.separator();
        if ui.add(egui::Button::new(w::text_medium(format!("{}   {}", icons::SIGN_OUT, t!("Sign out of account")), 13.5, t.text)).frame(false).min_size(vec2(210.0, 32.0))).clicked() {
            app.client.send(Command::LogOut);
        }
        if ui.add(egui::Button::new(w::text_medium(format!("{}   {}", icons::POWER, t!("Exit SteamInRust")), 13.5, t.danger)).frame(false).min_size(vec2(210.0, 32.0))).clicked() {
            app.request_exit(ui.ctx());
        }
    });
}

fn status_bar(app: &mut SteamApp, ui: &mut Ui, t: &Theme) {
    let rect = ui.max_rect();
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, t.border));
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
                let pct = d.disk_done as f64 * 100.0 / d.disk_total.max(1) as f64;
                let text = if d.paused { t!("{game}: paused at {pct}%", game = name, pct = format!("{pct:.0}")) } else { t!("{game}: {pct}%", game = name, pct = format!("{pct:.0}")) };
                ui.label(w::text(text, 12.5, t.accent));
                w::progress(ui, t, (pct / 100.0) as f32, 90.0);
                if waiting > 0 {
                    ui.label(w::text(t!("+{n} queued", n = waiting), 12.5, t.text_faint));
                }
            }
            None if waiting > 0 => {
                ui.label(w::text(t!("{n} queued", n = waiting), 12.5, t.text_faint));
            }
            None => {
                ui.label(w::text(t!("Nothing in the queue"), 12.5, t.text_faint));
            }
        }
        crate::offline_ui::status_note(app, ui, t);
        if let Some(id) = app.state.playing {
            ui.add_space(16.0);
            let name = app.state.app_name(id).unwrap_or("").to_string();
            ui.label(w::text(format!("{}  {}", icons::CONTROLLER, t!("Playing {game}", game = name)), 12.5, t.in_game));
            ui.label(w::text(t!("· Shift+Tab opens the overlay"), 12.5, t.text_faint));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let online = app.state.friend_list().iter().filter(|p| p.state != PersonaState::Offline).count();
            let label = format!("{}  {}  ·  {}", icons::CHATS, t!("Friends & Chat"), t!("{n} online", n = online));
            let r = ui.add(egui::Label::new(w::text_medium(label, 12.5, t.text_dim)).sense(Sense::click()));
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.friends_window.open = !app.friends_window.open;
            }
        });
    });
}

pub fn status_color(t: &Theme, p: &Persona) -> egui::Color32 {
    if p.in_game() {
        return t.in_game;
    }
    match p.state {
        PersonaState::Offline | PersonaState::Invisible => t.text_faint,
        PersonaState::Away | PersonaState::Snooze | PersonaState::Busy => t.away,
        _ => t.online,
    }
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
        ui.label(w::text_bold(t!("Friends"), 17.0, t.text));
        ui.label(w::text(t!("{n} online", n = online), 13.0, t.text_dim));
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
    let ok = |p: &&Persona| filter.is_empty() || p.name.to_lowercase().contains(&filter);
    let in_game: Vec<&Persona> = list.iter().filter(ok).filter(|p| p.in_game()).collect();
    let online: Vec<&Persona> = list.iter().filter(ok).filter(|p| !p.in_game() && p.state != PersonaState::Offline).collect();
    let offline: Vec<&Persona> = list.iter().filter(ok).filter(|p| p.state == PersonaState::Offline).collect();
    let mut clicked = None;

    egui::ScrollArea::vertical().id_salt("friend-list").auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            ui.label(w::text(t!("Loading friends…"), 13.0, t.text_faint));
        }
        for (title, group) in [(t!("In game"), &in_game), (t!("Online"), &online), (t!("Offline"), &offline)] {
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
    let name = if p.name.is_empty() { p.steam_id.to_string() } else { p.name.clone() };
    let status = if app.state.is_typing(p.steam_id) { t!("typing…").to_string() } else { status_text(&app.state, p) };
    let unread = app.state.unread.get(&p.steam_id).copied().unwrap_or(0);

    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    if selected {
        ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
    } else if hover > 0.0 {
        ui.painter().rect_filled(rect, t.corner_small(), w::mix(egui::Color32::TRANSPARENT, t.surface, hover));
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 6.0))).layout(Layout::left_to_right(Align::Center)));
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
            let r = r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Open the game's page"));
            if r.clicked() {
                actions.push(game_page(&app.state, target));
            }
        }
    });
    if unread > 0 {
        let c = rect.right_center() - vec2(16.0, 0.0);
        ui.painter().circle_filled(c, 9.0, t.accent);
        ui.painter().text(c, Align2::CENTER_CENTER, unread.min(99).to_string(), w::font_bold(10.0), t.on_accent);
    }

    let persona = p.clone();
    resp.context_menu(|ui| {
        ui.label(w::text_semibold(&name, 14.0, t.text));
        ui.separator();
        if ui.button(format!("{}  {}", icons::CHAT, t!("Send a message"))).clicked() {
            actions.push(Action::OpenChat(persona.steam_id));
            ui.close();
        }
        if ui.button(format!("{}  {}", icons::USER_CIRCLE, t!("View profile"))).clicked() {
            actions.push(Action::Navigate { page: "profile".into(), arg: Some(persona.steam_id.to_string()) });
            ui.close();
        }
        if let Some(game) = persona.game_app_id.filter(|_| persona.in_game()) {
            let name = app.state.app_name(game).map(String::from).unwrap_or_else(|| t!("the game").to_string());
            if ui.button(format!("{}  {}", icons::CONTROLLER, t!("Open {game} page", game = name))).clicked() {
                actions.push(game_page(&app.state, game));
                ui.close();
            }
        }
        // While you're in a game: invite them to it (a link that starts it for them).
        if let Some(mine) = app.state.playing {
            let game = app.state.app_name(mine).unwrap_or_default().to_string();
            if ui.button(format!("{}  {}", icons::USER_PLUS, t!("Invite to play {game}", game = game))).clicked() {
                app.client.send(Command::SendMessage { to: persona.steam_id, text: t!("Play {game} with me! steam://run/{id}", game = game, id = mine) });
                actions.push(Action::Toast(t!("Invite sent to {name}", name = persona.name)));
                ui.close();
            }
        }
        if ui.button(format!("{}  {}", icons::BACKPACK, t!("Inventory"))).clicked() {
            actions.push(Action::Navigate { page: "inventory".into(), arg: Some(persona.steam_id.to_string()) });
            ui.close();
        }
        if ui.button(format!("{}  {}", icons::COPY, t!("Copy SteamID"))).clicked() {
            ui.ctx().copy_text(persona.steam_id.to_string());
            ui.close();
        }
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
    egui::Area::new(egui::Id::unique("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -48.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for toast in app.toasts.iter().rev().take(4) {
                let card = egui::Frame::new()
                    .fill(t.raised)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(t.corner())
                    .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(120) })
                    .inner_margin(Margin { left: 22, right: 16, top: 12, bottom: 12 })
                    .show(ui, |ui| {
                        ui.set_width(320.0);
                        ui.label(w::text_semibold(&toast.title, 14.0, t.text));
                        if !toast.body.is_empty() {
                            ui.add(egui::Label::new(w::text(&toast.body, 13.0, t.text_dim)).wrap());
                        }
                    });
                // Accent bar inside the card, sized to it once the text is laid out.
                let r = card.response.rect;
                let bar = Rect::from_min_max(egui::pos2(r.left() + 9.0, r.top() + 10.0), egui::pos2(r.left() + 12.0, r.bottom() - 10.0));
                ui.painter().rect_filled(bar, egui::CornerRadius::same(2), t.accent);
                ui.add_space(8.0);
            }
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(200));
}

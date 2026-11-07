//! Steam Support: help topics, per-game help with fixes we can do right here
//! (verify files, update, open the folder), and your support requests with
//! their full conversations. Steam has no API for creating or answering
//! requests, so those steps open the matching page on help.steampowered.com.

use client::Command;
use egui::{Align, Align2, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, art};
use ui::{Theme, icons, widgets as w};

const HELP: &str = "https://help.steampowered.com/en/wizard/";

#[derive(Default)]
pub struct Support {
    game: Option<AppId>,
    search: String,
    open_ticket: Option<String>,
    requested: bool,
    asked: std::collections::HashSet<String>,
}

impl Extension for Support {
    fn manifest(&self) -> Manifest {
        Manifest { id: "support", name: "Support", author: "SteamInRust", description: "Steam Support", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("support", "Support", icons::INFO).hidden().order(970)]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        // "game:570" opens help for one game.
        self.game = arg.and_then(|a| a.strip_prefix("game:")).and_then(|a| a.parse().ok());
        // "ticket:HTGDW5X4C6B4HT" (from a notification) opens that request.
        self.open_ticket = arg.and_then(|a| a.strip_prefix("ticket:")).map(String::from);
        self.requested = false;
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        if !self.requested && ctx.state.steam_id != 0 {
            self.requested = true;
            ctx.send(Command::LoadSupportTickets);
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            w::centered(ui, 1100.0, 28.0, |ui| {
                ui.add_space(24.0);
                ui.label(w::text_bold(t!("Steam Support"), 28.0, t.text));
                ui.label(w::text(t!("Fix common problems right here, or get in touch with Steam Support."), 14.0, t.text_dim));
                ui.add_space(18.0);
                if let Some(id) = self.open_ticket.clone() {
                    self.conversation(ui, &t, ctx, &id);
                    return;
                }
                let side = 380.0;
                let main = ui.available_width() - side - 20.0;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(vec2(main, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(main);
                        match self.game {
                            Some(app) => self.game_help(ui, &t, ctx, app),
                            None => self.topics(ui, &t, ctx),
                        }
                    });
                    ui.add_space(20.0);
                    ui.allocate_ui_with_layout(vec2(side, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(side);
                        self.tickets(ui, &t, ctx);
                    });
                });
            });
            ui.add_space(30.0);
        });
    }
}

fn card(ui: &mut Ui, t: &Theme, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(t.corner()).inner_margin(Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// A clickable row with an icon, title and one line of detail.
fn option(ui: &mut Ui, t: &Theme, icon: &str, title: &str, sub: &str, external: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let p = ui.painter();
    p.rect_filled(r, t.corner_small(), w::mix(t.surface, t.surface_hover, hover));
    p.text(pos2(r.left() + 18.0, r.center().y), Align2::CENTER_CENTER, icon, w::font(18.0), t.accent);
    p.text(pos2(r.left() + 40.0, r.top() + 18.0), Align2::LEFT_CENTER, title, w::font_semibold(14.0), t.text);
    p.text(pos2(r.left() + 40.0, r.top() + 37.0), Align2::LEFT_CENTER, sub, w::font(12.0), t.text_dim);
    let mark = if external { icons::EXTERNAL } else { icons::CARET_RIGHT };
    p.text(pos2(r.right() - 14.0, r.center().y), Align2::RIGHT_CENTER, mark, w::font(13.0), t.text_faint);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = if external { resp.on_hover_text(t!("Opens in your browser")) } else { resp };
    resp.clicked()
}

impl Support {
    fn topics(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        card(ui, t, |ui| {
            ui.label(w::text_bold(t!("Help with a game"), 17.0, t.text));
            ui.add_space(6.0);
            w::search(ui, t, &mut self.search, &t!("Find a game you own"), ui.available_width().min(360.0));
            ui.add_space(8.0);
            let q = self.search.trim().to_lowercase();
            let mut games = crate::builtin::entries(ctx.state, ctx.local);
            games.retain(|e| q.is_empty() || e.name.to_lowercase().contains(&q));
            // Most recently played first.
            games.sort_by_key(|e| std::cmp::Reverse(e.installed.and_then(|g| g.last_played).unwrap_or(0)));
            let shown: Vec<(AppId, String, Option<u64>)> = games.iter().take(8).map(|e| (e.app_id, e.name.to_string(), e.installed.and_then(|g| g.last_played))).collect();
            if shown.is_empty() {
                ui.label(w::text(t!("No games match."), 13.0, t.text_faint));
            }
            for (app, name, last) in shown {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
                }
                let img = Rect::from_min_size(r.min + vec2(6.0, 5.0), vec2(86.0, 40.0));
                w::image_or_skeleton(ui, t, img, &art::header(app), CornerRadius::same(4));
                ui.painter().text(pos2(img.right() + 12.0, r.top() + 17.0), Align2::LEFT_CENTER, &name, w::font_semibold(14.0), t.text);
                let sub = last.map(|l| t!("last played {when}", when = ui::ago(l))).unwrap_or_default();
                ui.painter().text(pos2(img.right() + 12.0, r.top() + 35.0), Align2::LEFT_CENTER, sub, w::font(12.0), t.text_dim);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.game = Some(app);
                }
            }
        });
        ui.add_space(14.0);
        card(ui, t, |ui| {
            ui.label(w::text_bold(t!("Other topics"), 17.0, t.text));
            ui.add_space(6.0);
            let topics = [
                (icons::CART, t!("Purchases"), t!("Charges, refunds, gifts and pending transactions"), "HelpWithPurchase"),
                (icons::SHIELD, t!("My account"), t!("Account details, Steam Guard, stolen accounts"), "HelpWithAccount"),
                (icons::KEY, t!("I can't sign in"), t!("Forgotten password, lost Steam Guard codes"), "HelpWithLogin"),
                (icons::SWAP, t!("Trading, gifting and the Market"), t!("Trade holds, items, Community Market listings"), "HelpWithTrading"),
                (icons::DESKTOP, t!("Steam Deck and hardware"), t!("Steam Deck, controllers, Steam Link"), "HelpWithHardware"),
            ];
            for (icon, title, sub, path) in topics {
                if option(ui, t, icon, title, sub, true) {
                    let _ = open::that_detached(format!("{HELP}{path}"));
                }
                ui.add_space(4.0);
            }
            ui.add_space(4.0);
            if option(ui, t, icons::STOREFRONT, t!("My market listings"), t!("See and remove items you're selling"), false) {
                ctx.navigate("inventory", Some("listings".into()));
            }
        });
    }

    fn game_help(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId) {
        let name = ctx.state.app_name(app).map(String::from).or_else(|| ctx.local.games.iter().find(|g| g.app_id == app).map(|g| g.name.clone())).unwrap_or_else(|| app.to_string());
        let installed = ctx.local.games.iter().find(|g| g.app_id == app).cloned();
        if w::button(ui, t, Some(icons::ARROW_LEFT), t!("All topics"), w::Kind::Ghost, w::Size::Sm).clicked() {
            self.game = None;
        }
        ui.add_space(8.0);
        card(ui, t, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(184.0, 69.0), Sense::hover());
                w::image_or_skeleton(ui, t, r, &art::header(app), t.corner_small());
                ui.vertical(|ui| {
                    ui.label(w::text_bold(&name, 20.0, t.text));
                    match &installed {
                        Some(g) => ui.label(w::text(t!("Installed · {time} played", time = ui::playtime(g.playtime_minutes.unwrap_or(0))), 13.0, t.text_dim)),
                        None => ui.label(w::text(t!("Not installed"), 13.0, t.text_dim)),
                    };
                });
            });
        });
        ui.add_space(14.0);
        card(ui, t, |ui| {
            ui.label(w::text_bold(t!("Try these first"), 17.0, t.text));
            ui.label(w::text(t!("Most problems with a game are fixed by one of these."), 12.5, t.text_dim));
            ui.add_space(8.0);
            if installed.is_some() {
                if option(ui, t, icons::CHECK, t!("Verify game files"), t!("Find and repair missing or damaged files"), false) {
                    ctx.navigate("library", Some(format!("verify:{app}")));
                }
                ui.add_space(4.0);
                if option(ui, t, icons::REFRESH, t!("Check for updates"), t!("Download the latest version of the game"), false) {
                    ctx.navigate("library", Some(format!("update:{app}")));
                }
                ui.add_space(4.0);
                if option(ui, t, icons::FOLDER, t!("Open the game's folder"), t!("See the installed files"), false) {
                    if let Some(g) = &installed {
                        local::open_folder(&g.install_dir);
                    }
                }
                ui.add_space(4.0);
                if option(ui, t, icons::GEAR, t!("Reset launch options"), t!("Clear custom launch options that may stop it starting"), false) {
                    ctx.navigate("library", Some(format!("resetargs:{app}")));
                }
                ui.add_space(4.0);
                if option(ui, t, icons::TRASH, t!("Reinstall the game"), t!("Uninstall it, then install it again"), false) {
                    ctx.navigate("library", Some(format!("uninstall:{app}")));
                }
            } else if option(ui, t, icons::DOWNLOAD, t!("Install the game"), t!("Download it to this PC"), false) {
                ctx.navigate("library", Some(format!("install:{app}")));
            }
            ui.add_space(4.0);
            if option(ui, t, icons::CHATS, t!("Ask the community"), t!("The game's discussion forums"), false) {
                ctx.navigate("discussions", Some(app.to_string()));
            }
        });
        ui.add_space(14.0);
        card(ui, t, |ui| {
            ui.label(w::text_bold(t!("Contact Steam Support"), 17.0, t.text));
            ui.label(w::text(t!("Refunds, purchase problems and anything else about this game."), 12.5, t.text_dim));
            ui.add_space(8.0);
            if option(ui, t, icons::COINS, t!("Request a refund"), t!("Usually possible within 14 days of purchase and under 2 hours played"), true) {
                let _ = open::that_detached(format!("{HELP}HelpWithGame/?appid={app}"));
            }
            ui.add_space(4.0);
            if option(ui, t, icons::INFO, t!("Other problems with this game"), t!("Steam Support's help for this game"), true) {
                let _ = open::that_detached(format!("{HELP}HelpWithGame/?appid={app}"));
            }
        });
    }

    fn tickets(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        card(ui, t, |ui| {
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("Your support requests"), 17.0, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::REFRESH, t!("Refresh"), false, 0).clicked() {
                        ctx.send(Command::LoadSupportTickets);
                    }
                });
            });
            ui.add_space(6.0);
            match &ctx.state.support_tickets {
                None => {
                    for _ in 0..3 {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
                        w::skeleton(ui, t, r.shrink2(vec2(0.0, 4.0)), t.corner_small());
                    }
                }
                Some(Err(e)) => {
                    ui.label(w::text(t!("Couldn't load your requests: {error}", error = i18n::tr_dyn(e)), 12.5, t.text_faint));
                }
                Some(Ok(list)) if list.is_empty() => {
                    ui.label(w::text(t!("You haven't contacted Steam Support yet."), 13.0, t.text_faint));
                }
                Some(Ok(list)) => {
                    let list = list.clone();
                    for tk in list {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
                        }
                        let img = Rect::from_min_size(r.min + vec2(6.0, 10.0), vec2(36.0, 36.0));
                        match &tk.icon {
                            Some(u) => w::image_or_skeleton(ui, t, img, u, CornerRadius::same(4)),
                            None => w::placeholder(ui, t, img, "", CornerRadius::same(4)),
                        }
                        let x = img.right() + 10.0;
                        let width = r.right() - x - 70.0;
                        let p = ui.painter();
                        let title = p.layout(tk.title.clone(), w::font_semibold(13.0), t.text, width);
                        let first = title.rows.first().map(|row| row.text()).unwrap_or_default();
                        p.text(pos2(x, r.top() + 18.0), Align2::LEFT_CENTER, first, w::font_semibold(13.0), t.text);
                        p.text(pos2(x, r.top() + 37.0), Align2::LEFT_CENTER, format!("{} · {}", tk.area, tk.updated), w::font(12.0), t.text_dim);
                        let color = if tk.open { t.online } else { t.text_faint };
                        p.text(pos2(r.right() - 8.0, r.top() + 18.0), Align2::RIGHT_CENTER, &tk.status, w::font_semibold(11.5), color);
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&tk.title).clicked() {
                            if !ctx.state.support_messages.contains_key(&tk.id) {
                                ctx.send(Command::LoadSupportTicket { id: tk.id.clone(), url: tk.url.clone() });
                            }
                            self.open_ticket = Some(tk.id.clone());
                        }
                    }
                }
            }
            ui.add_space(6.0);
            if ui.link(w::text(format!("{}  {}", icons::EXTERNAL, t!("All requests on Steam Support")), 12.5, t.accent)).clicked() {
                let _ = open::that_detached(format!("{HELP}HelpRequests"));
            }
        });
    }

    fn conversation(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: &str) {
        // Notifications give the code without dashes; the list has them.
        let plain = |s: &str| s.replace('-', "");
        let ticket = ctx.state.support_tickets.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|x| plain(&x.id) == plain(id))).cloned();
        let id = ticket.as_ref().map(|tk| tk.id.clone()).unwrap_or_else(|| id.to_string());
        let id = id.as_str();
        if let Some(tk) = &ticket {
            if !ctx.state.support_messages.contains_key(&tk.id) && self.asked.insert(tk.id.clone()) {
                ctx.send(Command::LoadSupportTicket { id: tk.id.clone(), url: tk.url.clone() });
            }
        }
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Back"), w::Kind::Ghost, w::Size::Sm).clicked() {
                self.open_ticket = None;
            }
            if let Some(tk) = &ticket {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if tk.open { t!("Reply on Steam Support") } else { t!("Open on Steam Support") };
                    if w::button(ui, t, Some(icons::EXTERNAL), label, w::Kind::Secondary, w::Size::Sm).clicked() {
                        let _ = open::that_detached(&tk.url);
                    }
                });
            }
        });
        ui.add_space(8.0);
        if let Some(tk) = &ticket {
            ui.label(w::text_bold(&tk.title, 22.0, t.text));
            ui.label(w::text(format!("{} · {} · {}", tk.area, tk.status, tk.id), 13.0, t.text_dim));
            ui.add_space(12.0);
        }
        match ctx.state.support_messages.get(id) {
            None => {
                for _ in 0..2 {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::hover());
                    w::skeleton(ui, t, r.shrink2(vec2(0.0, 6.0)), t.corner());
                }
            }
            Some(Err(e)) => {
                ui.label(w::text(i18n::tr_dyn(e), 13.0, t.text_faint));
            }
            Some(Ok(messages)) => {
                let width = ui.available_width();
                for m in messages.clone() {
                    let fill = if m.from_steam { t.surface } else { t.raised };
                    egui::Frame::new()
                        .fill(fill)
                        .stroke(Stroke::new(1.0, if m.from_steam { t.accent.gamma_multiply(0.4) } else { t.border }))
                        .corner_radius(t.corner())
                        .inner_margin(Margin::same(16))
                        .show(ui, |ui| {
                            ui.set_width(width - 34.0);
                            let who = if m.from_steam { format!("{}  {}", icons::SHIELD, m.header) } else { m.header.clone() };
                            ui.label(w::text_semibold(who, 12.5, if m.from_steam { t.accent } else { t.text_dim }));
                            ui.add_space(6.0);
                            crate::builtin::store::blocks(ui, t, &store::html::parse(&m.html), width - 40.0);
                        });
                    ui.add_space(10.0);
                }
            }
        }
    }
}

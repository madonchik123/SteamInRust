//! Settings: account, appearance, plugins, extensions, overlay, library folders, about.

use i18n::t;
use client::Command;
use egui::{Margin, Sense, Ui, vec2};
use extensions::{Action, Ctx, Extension, Manifest, PageSpec};
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Section {
    #[default]
    Account,
    Friends,
    Family,
    Security,
    Notifications,
    Interface,
    Appearance,
    Store,
    Library,
    Downloads,
    Folders,
    Cloud,
    Overlay,
    Accessibility,
    Plugins,
    About,
}

/// Steam's settings list, in Steam's order; `None` draws a divider.
const SECTIONS: [Option<(Section, &str, &str)>; 19] = [
    Some((Section::Account, icons::USER_CIRCLE, "Account")),
    Some((Section::Friends, icons::USERS, "Friends & Chat")),
    Some((Section::Family, icons::USERS_THREE, "Family")),
    Some((Section::Security, icons::SHIELD, "Security")),
    None,
    Some((Section::Notifications, icons::BELL, "Notifications")),
    Some((Section::Interface, icons::MONITOR, "Interface")),
    Some((Section::Appearance, icons::PALETTE, "Appearance")),
    Some((Section::Store, icons::STORE, "Store")),
    Some((Section::Library, icons::BOOKS, "Library")),
    Some((Section::Downloads, icons::DOWNLOAD, "Downloads")),
    Some((Section::Folders, icons::DRIVES, "Storage")),
    Some((Section::Cloud, icons::CLOUD_CHECK, "Cloud")),
    Some((Section::Overlay, icons::CONTROLLER, "In game")),
    Some((Section::Accessibility, icons::EYE, "Accessibility")),
    None,
    Some((Section::Plugins, icons::PLUGS, "Plugins")),
    Some((Section::About, icons::INFO, "About")),
    None,
];

/// App-level options the Settings page edits (owned by the app, shared here).
#[derive(Debug, Clone, Copy, Default)]
pub struct AppFlags {
    pub close_to_tray: bool,
    pub ask_account: bool,
    /// Master switch: keep games updated automatically (like Steam's).
    pub auto_updates: bool,
    /// Steam Cloud for every game that supports it.
    pub cloud: bool,
    /// Pop-ups for new chat messages / Steam notifications.
    pub toast_messages: bool,
    pub toast_notifications: bool,
}

pub type SharedFlags = std::sync::Arc<std::sync::Mutex<AppFlags>>;

pub struct Settings {
    section: Section,
    flags: SharedFlags,
    /// Cached "are we the steam:// handler?" (registry lookups are slow).
    handler: Option<bool>,
}

impl Settings {
    pub fn new(flags: SharedFlags) -> Self {
        Self { section: Section::default(), flags, handler: None }
    }
}

impl Extension for Settings {
    fn manifest(&self) -> Manifest {
        Manifest { id: "settings", name: "Settings", author: "SteamInRust", description: "Appearance, plugins, account", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("settings", "Settings", icons::GEAR).hidden().order(1000)]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        if arg == Some("plugins") {
            self.section = Section::Plugins;
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        egui::Panel::left("settings-nav")
            .exact_size(240.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin { left: 14, right: 10, top: 22, bottom: 10 }))
            .show(ui, |ui| {
                ui.label(w::text_bold(t!("SETTINGS"), 18.0, t.accent));
                ui.add_space(12.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for entry in SECTIONS {
                        let Some((s, icon, label)) = entry else {
                            ui.add_space(4.0);
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                            ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
                            ui.add_space(4.0);
                            continue;
                        };
                        let on = self.section == s;
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                        if on || resp.hovered() {
                            ui.painter().rect_filled(r, t.corner_small(), if on { t.surface_hover } else { t.surface });
                        }
                        ui.painter().text(egui::pos2(r.left() + 18.0, r.center().y), egui::Align2::CENTER_CENTER, icon, w::font(17.0), if on { t.text } else { t.text_dim });
                        ui.painter().text(egui::pos2(r.left() + 40.0, r.center().y), egui::Align2::LEFT_CENTER, i18n::tr_dyn(label), w::font_medium(14.0), if on { t.text } else { t.text_dim });
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.section = s;
                        }
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(28.0);
                w::centered(ui, 820.0, 32.0, |ui| match self.section {
                    Section::Account => account(ui, &t, ctx),
                    Section::Friends => friends_chat(ui, &t, ctx),
                    Section::Family => family(ui, &t),
                    Section::Security => security(ui, &t, ctx),
                    Section::Notifications => notifications(ui, &t, &self.flags),
                    Section::Library => library(ui, &t, ctx),
                    Section::Downloads => downloads(ui, &t, ctx, &self.flags),
                    Section::Cloud => cloud(ui, &t, &self.flags),
                    Section::Accessibility => accessibility(ui, &t, ctx),
                    Section::Appearance => appearance(ui, &t, ctx),
                    Section::Plugins => {
                        w::page_title(ui, &t, t!("Plugins"), Some(t!("Scripts you install yourself. Each one runs in a sandbox and only gets the permissions you approve.")));
                        ctx.settings_for("plugins", ui);
                    }
                    Section::Interface => interface(ui, &t, ctx, &self.flags, &mut self.handler),
                    Section::Store => {
                        w::page_title(ui, &t, t!("Store"), None);
                        w::card(&t).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ctx.settings_for("store", ui);
                        });
                    }
                    Section::Overlay => overlay(ui, &t, ctx),
                    Section::Folders => folders(ui, &t, ctx),
                    Section::About => about(ui, &t),
                });
                ui.add_space(40.0);
            });
        });
    }
}

/// A row like Steam's settings: label, value, and a button on the right.
fn info_row(ui: &mut Ui, t: &Theme, label: &str, value: &str, button: Option<&str>) -> bool {
    let mut clicked = false;
    egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(w::text_medium(label, 14.0, t.text));
            ui.add_space(8.0);
            ui.label(w::text(value, 13.0, t.text_dim));
            if let Some(b) = button {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    clicked = w::button(ui, t, None, b, w::Kind::Secondary, w::Size::Sm).clicked();
                });
            }
        });
    });
    ui.add_space(6.0);
    clicked
}

/// A big square shortcut tile (icon, title, line under it).
fn tile(ui: &mut Ui, t: &Theme, icon: &str, title: &str, sub: &str, width: f32) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(width, 132.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter().rect_filled(r, t.corner_small(), w::mix(t.surface, t.surface_hover, hover));
    ui.painter().text(egui::pos2(r.center().x, r.top() + 42.0), egui::Align2::CENTER_CENTER, icon, w::font(34.0), t.text);
    ui.painter().text(egui::pos2(r.center().x, r.top() + 84.0), egui::Align2::CENTER_CENTER, title, w::font_semibold(14.5), t.text);
    let g = ui.painter().layout(sub.to_string(), w::font(12.0), t.text_dim, width - 20.0);
    ui.painter().galley(egui::pos2(r.center().x - g.size().x / 2.0, r.top() + 98.0), g, t.text_dim);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn web(url: &str) {
    let _ = open::that_detached(url);
}

/// Like Steam's Account page.
fn account(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    w::page_title(ui, t, t!("Account"), None);
    let me = ctx.state.persona(ctx.state.steam_id);
    let name = if ctx.state.persona_name.is_empty() { me.name.clone() } else { ctx.state.persona_name.clone() };
    ui.horizontal(|ui| {
        w::avatar(ui, t, &model::art::avatar_full(me.avatar_hash.as_deref()), 60.0, None);
        ui.vertical(|ui| {
            ui.add_space(8.0);
            ui.label(w::text_semibold(&name, 16.0, t.accent));
            ui.label(w::text(i18n::tr_dyn(me.state.label()), 13.0, crate::shell::status_color(t, &me)));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if w::button(ui, t, None, t!("View profile"), w::Kind::Secondary, w::Size::Md).clicked() {
                ctx.navigate("profile", None);
            }
        });
    });
    ui.add_space(14.0);
    let width = (ui.available_width() - 16.0) / 3.0;
    let balance = ctx.state.wallet.map(|(b, c)| model::format_money(b, c)).unwrap_or_else(|| "—".into());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if tile(ui, t, icons::COINS, t!("Add funds"), &t!("Current balance: {amount}", amount = balance), width) {
            web("https://store.steampowered.com/steamaccount/addfunds");
        }
        if tile(ui, t, icons::EYE_SLASH, t!("Privacy settings"), t!("Profile, games, inventory and more"), width) {
            web(&format!("https://steamcommunity.com/profiles/{}/edit/settings", ctx.state.steam_id));
        }
        if tile(ui, t, icons::NOTE, t!("Account details"), t!("Store, security, family settings, devices"), width) {
            web("https://store.steampowered.com/account/");
        }
    });
    ui.add_space(10.0);
    let code = (ctx.state.steam_id & 0xFFFF_FFFF).to_string();
    if info_row(ui, t, t!("Your friend code"), &code, Some(t!("Add friends"))) {
        ctx.navigate("community", None);
    }
    let email = ctx.state.email.as_ref().map(|(e, ok)| if *ok { e.clone() } else { format!("{e} ({})", t!("not verified")) }).unwrap_or_else(|| "—".into());
    if info_row(ui, t, t!("Email address"), &email, Some(t!("Change email address"))) {
        web("https://help.steampowered.com/wizard/HelpChangeEmail");
    }
    let vac = match ctx.state.profiles.get(&ctx.state.steam_id) {
        Some(p) if p.vac_banned => t!("VAC bans on record").to_string(),
        Some(_) => t!("No VAC bans").to_string(),
        None => "…".into(),
    };
    if info_row(ui, t, t!("VAC status"), &vac, Some(t!("Learn more"))) {
        web("https://help.steampowered.com/faqs/view/647C-5CC1-7EA9-3C29");
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(w::text(format!("{} · {} · {}", ctx.state.account, ctx.state.steam_id, ctx.state.country), 12.0, t.text_faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if w::button(ui, t, Some(icons::SIGN_OUT), t!("Sign out"), w::Kind::Danger, w::Size::Sm).clicked() {
                ctx.send(Command::LogOut);
            }
        });
    });
}

fn friends_chat(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    w::page_title(ui, t, t!("Friends & Chat"), None);
    let code = (ctx.state.steam_id & 0xFFFF_FFFF).to_string();
    if info_row(ui, t, t!("Your friend code"), &code, Some(t!("Copy"))) {
        ui.ctx().copy_text(code);
        ctx.toast(t!("Friend code copied"));
    }
    if info_row(ui, t, t!("Friends"), &ctx.state.friend_list().len().to_string(), Some(t!("Open friends list"))) {
        ctx.navigate("community", None);
    }
    if info_row(ui, t, t!("Who can see your status and send invites"), "", Some(t!("Privacy settings"))) {
        web(&format!("https://steamcommunity.com/profiles/{}/edit/settings", ctx.state.steam_id));
    }
}

fn family(ui: &mut Ui, t: &Theme) {
    w::page_title(ui, t, t!("Family"), Some(t!("Steam Families: share games and set limits for children. Managed on Steam's website.")));
    if info_row(ui, t, t!("Steam Family"), "", Some(t!("Manage family"))) {
        web("https://store.steampowered.com/account/familymanagement");
    }
}

fn security(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    w::page_title(ui, t, t!("Security"), None);
    if info_row(ui, t, t!("Steam Guard"), t!("Protects your account with codes from the Steam Mobile app"), Some(t!("Manage Steam Guard"))) {
        web("https://store.steampowered.com/twofactor/manage");
    }
    if info_row(ui, t, t!("Authorized devices"), t!("See and remove devices signed in to your account"), Some(t!("Manage devices"))) {
        web("https://store.steampowered.com/account/authorizeddevices");
    }
    if info_row(ui, t, t!("Password"), "", Some(t!("Change password"))) {
        web("https://help.steampowered.com/wizard/HelpChangePassword");
    }
    if info_row(ui, t, t!("This PC"), t!("Saved sign-in for this account is stored on this PC"), Some(t!("Sign out"))) {
        ctx.send(Command::LogOut);
    }
}

fn switch(ui: &mut Ui, t: &Theme, title: &str, body: &str, value: &mut bool) {
    egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(w::text_medium(title, 14.0, t.text));
                if !body.is_empty() {
                    ui.add(egui::Label::new(w::text(body, 12.5, t.text_dim)).wrap());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(toggle(value));
            });
        });
    });
    ui.add_space(6.0);
}

fn notifications(ui: &mut Ui, t: &Theme, flags: &SharedFlags) {
    w::page_title(ui, t, t!("Notifications"), None);
    let mut f = *flags.lock().unwrap();
    switch(ui, t, t!("New chat messages"), t!("Show a pop-up when a friend messages you."), &mut f.toast_messages);
    switch(ui, t, t!("Steam notifications"), t!("Show a pop-up for gifts, items, friend requests, sales and more."), &mut f.toast_notifications);
    *flags.lock().unwrap() = f;
}

fn library(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    w::page_title(ui, t, t!("Library"), None);
    if info_row(ui, t, t!("Collections"), t!("Steam's Favorites and Hidden are imported automatically"), Some(t!("Open library"))) {
        ctx.navigate("library", None);
    }
    if info_row(ui, t, t!("Add a non-Steam game"), "", Some(t!("Open library"))) {
        ctx.navigate("library", None);
    }
}

fn downloads(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, flags: &SharedFlags) {
    w::page_title(ui, t, t!("Downloads"), None);
    let mut f = *flags.lock().unwrap();
    switch(ui, t, t!("Automatic updates"), t!("Keep installed games up to date. Turn off to update only when you choose (or when you launch a game)."), &mut f.auto_updates);
    *flags.lock().unwrap() = f;
    if info_row(ui, t, t!("Download region, speed limit, schedule"), "", Some(t!("Open download settings"))) {
        ctx.navigate("downloads", None);
    }
}

fn cloud(ui: &mut Ui, t: &Theme, flags: &SharedFlags) {
    w::page_title(ui, t, t!("Cloud"), Some(t!("Steam Cloud keeps your game data the same on every PC.")));
    let mut f = *flags.lock().unwrap();
    switch(ui, t, t!("Enable Steam Cloud for games that support it"), t!("Each game can also be turned off in its Properties."), &mut f.cloud);
    *flags.lock().unwrap() = f;
}

fn accessibility(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    w::page_title(ui, t, t!("Accessibility"), None);
    egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(w::text_medium(t!("Interface size"), 14.0, t.text));
                ui.label(w::text(t!("Makes text and everything else bigger or smaller."), 12.5, t.text_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut zoom = ctx.zoom;
                if ui.add(egui::Slider::new(&mut zoom, 0.8..=1.6).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).changed() {
                    ctx.act(Action::SetZoom(zoom));
                }
            });
        });
    });
}

fn appearance(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    w::page_title(ui, t, t!("Appearance"), None);
    ui.label(w::text_semibold(t!("Theme"), 15.0, t.text));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        for preset in ui::Theme::presets() {
            let selected = preset.name == t.name;
            let (rect, resp) = ui.allocate_exact_size(vec2(150.0, 104.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(rect, t.corner(), preset.bg);
            let top = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 18.0));
            p.rect_filled(top, egui::CornerRadius { nw: t.radius, ne: t.radius, sw: 0, se: 0 }, preset.chrome);
            p.rect_filled(egui::Rect::from_min_size(rect.min + vec2(12.0, 28.0), vec2(56.0, 38.0)), egui::CornerRadius::same(5), preset.surface);
            p.rect_filled(egui::Rect::from_min_size(rect.min + vec2(76.0, 28.0), vec2(62.0, 38.0)), egui::CornerRadius::same(5), preset.surface);
            p.rect_filled(egui::Rect::from_min_size(rect.min + vec2(12.0, 72.0), vec2(36.0, 10.0)), egui::CornerRadius::same(5), preset.accent);
            p.text(rect.right_bottom() - vec2(10.0, 10.0), egui::Align2::RIGHT_BOTTOM, &preset.name, w::font_semibold(12.0), preset.text);
            let stroke = if selected { egui::Stroke::new(2.0, t.accent) } else { egui::Stroke::new(1.0, t.border) };
            p.rect_stroke(rect, t.corner(), stroke, egui::StrokeKind::Outside);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                ctx.act(Action::SetTheme(preset));
            }
        }
    });
    ui.add_space(16.0);
    let mut theme = t.clone();
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Grid::new("appearance").num_columns(2).spacing(vec2(32.0, 14.0)).show(ui, |ui| {
            ui.label(w::text_medium(t!("Accent color"), 14.0, t.text));
            ui.horizontal(|ui| {
                ui.color_edit_button_srgba(&mut theme.accent);
                ui.color_edit_button_srgba(&mut theme.accent_2);
            });
            ui.end_row();
            ui.label(w::text_medium(t!("Corner roundness"), 14.0, t.text));
            ui.add(egui::Slider::new(&mut theme.radius, 0..=20));
            ui.end_row();
            ui.label(w::text_medium(t!("Interface scale"), 14.0, t.text));
            let mut zoom = ctx.zoom;
            if ui.add(egui::Slider::new(&mut zoom, 0.8..=1.6).step_by(0.05)).drag_stopped() {
                ctx.act(Action::SetZoom(zoom));
            }
            ui.end_row();
        });
    });
    if &theme != t {
        ctx.act(Action::SetTheme(theme));
    }
}

fn interface(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, flags: &SharedFlags, handler: &mut Option<bool>) {
    w::page_title(ui, t, t!("Interface"), None);
    let mut f = *flags.lock().unwrap();
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let row = |ui: &mut Ui, title: &str, body: &str, value: &mut bool| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(w::text_semibold(title, 14.5, t.text));
                    ui.label(w::text(body, 12.5, t.text_dim));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(toggle(value));
                });
            });
            ui.add_space(8.0);
        };
        row(ui, t!("Close to tray"), t!("Keep SteamInRust running in the system tray when you close the window."), &mut f.close_to_tray);
        row(ui, t!("Ask which account to use"), t!("Show the account picker at startup instead of signing in automatically."), &mut f.ask_account);
    });
    *flags.lock().unwrap() = f;
    ui.add_space(12.0);
    let mut on = *handler.get_or_insert_with(crate::links::is_default_handler);
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(w::text_semibold(t!("Open steam:// links with SteamInRust"), 14.5, t.text));
                ui.add(egui::Label::new(w::text(t!("Store buttons, desktop shortcuts and links like steam://rungameid/570 will open here. Turning it off gives them back to the official Steam client."), 12.5, t.text_dim)).wrap());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(toggle(&mut on)).changed() {
                    let r = if on { crate::links::register() } else { crate::links::unregister() };
                    match r {
                        Ok(()) => ctx.toast(if on { t!("steam:// links now open in SteamInRust") } else { t!("steam:// links handed back") }),
                        Err(e) => ctx.toast(e),
                    }
                    *handler = None;
                }
            });
        });
    });
    ui.add_space(12.0);
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(w::text_semibold(t!("Language"), 15.0, t.text));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for lang in i18n::Lang::ALL {
                if w::chip(ui, t, lang.native_name(), i18n::language() == lang).clicked() {
                    ctx.act(Action::SetLanguage(lang.code().into()));
                }
            }
        });
    });
}

fn overlay(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    w::page_title(ui, t, t!("In-game overlay"), None);
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(w::text_semibold(t!("Shift+Tab"), 15.0, t.text));
        ui.add(egui::Label::new(w::text(
            t!("While a game you started from SteamInRust is running, press Shift+Tab to open the overlay: friends, quick chat and notes. It's a separate always-on-top window that never touches the game's process, so it's safe with anti-cheat. It shows over windowed and borderless games; exclusive fullscreen hides it."),
            13.5,
            t.text_dim,
        )).wrap());
        ui.add_space(10.0);
        if w::button(ui, t, Some(icons::EYE), t!("Preview overlay"), w::Kind::Secondary, w::Size::Md).clicked() {
            ctx.act(Action::ToggleOverlay);
        }
    });
}

fn folders(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    w::page_title(ui, t, t!("Library folders"), Some(t!("Folders where games are installed (read from your Steam setup).")));
    if ctx.local.libraries.is_empty() {
        w::empty(ui, t, icons::DRIVES, t!("No library folders found"), t!("Install location detection looks for an existing Steam install."));
    }
    for lib in &ctx.local.libraries {
        let games: Vec<_> = ctx.local.games.iter().filter(|g| &g.library == lib).collect();
        let size: u64 = games.iter().map(|g| g.size_on_disk).sum();
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(w::text(icons::DRIVES, 24.0, t.accent));
                ui.vertical(|ui| {
                    ui.label(w::text_semibold(lib.display().to_string(), 14.5, t.text));
                    ui.label(w::text(t!("{n} games · {size}", n = games.len(), size = ui::bytes(size)), 13.0, t.text_dim));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::button(ui, t, Some(icons::FOLDER), t!("Open"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        local::open_folder(lib);
                    }
                });
            });
        });
        ui.add_space(8.0);
    }
}

fn about(ui: &mut Ui, t: &Theme) {
    w::page_title(ui, t, t!("About"), None);
    crate::updater::settings_ui(ui, t);
    ui.add_space(12.0);
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            w::logo(ui, t, 40.0);
            ui.vertical(|ui| {
                ui.label(w::text_bold(format!("SteamInRust {}", env!("CARGO_PKG_VERSION")), 18.0, t.text));
                ui.label(w::text(t!("A native Steam client written in Rust."), 13.5, t.text_dim));
            });
        });
        ui.add_space(10.0);
        ui.label(w::text(t!("Not affiliated with Valve. Steam is a trademark of Valve Corporation."), 12.5, t.text_faint));
        ui.label(w::text(t!("Fonts: Inter (OFL). Icons: Phosphor (MIT). Protocol definitions: SteamTracking/Protobufs."), 12.5, t.text_faint));
        let dir = client::store::config_dir();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(w::text(t!("Data folder: {path}", path = dir.display()), 12.5, t.text_dim));
            if w::button(ui, t, Some(icons::FOLDER), t!("Open"), w::Kind::Ghost, w::Size::Sm).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                local::open_folder(&dir);
            }
        });
    });
}

/// iOS-style switch.
pub fn toggle(on: &mut bool) -> impl egui::Widget + '_ {
    move |ui: &mut Ui| {
        let size = vec2(40.0, 22.0);
        let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
        if resp.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        let k = ui.ctx().animate_bool_with_time(resp.id, *on, 0.15);
        let v = ui.visuals();
        let accent = v.selection.stroke.color;
        let off = v.widgets.inactive.bg_fill;
        let fill = w::mix(off, accent, k);
        let fill = if ui.is_enabled() { fill } else { fill.gamma_multiply(0.5) };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(11), fill);
        let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, k);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), 8.0, egui::Color32::WHITE);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

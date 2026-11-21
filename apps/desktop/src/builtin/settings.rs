//! Settings (Figma "settings" `475:6189`): a column of sections on the
//! left, groups of settings on the right. Profile Settings uses the same
//! layout (`layout`, `group`, `row` here).

use i18n::t;
use client::Command;
use egui::{Sense, Ui, vec2};
use extensions::{Action, Ctx, Extension, Manifest, PageSpec};
use model::ProfilePrivacy;
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Section {
    #[default]
    Account,
    Friends,
    Family,
    Security,
    Privacy,
    Wallet,
    Store,
    Language,
    Notifications,
    Interface,
    Appearance,
    Library,
    Downloads,
    Folders,
    Cloud,
    Overlay,
    Accessibility,
    Plugins,
    About,
}

/// The sections, in the design's order; `None` draws a divider.
const SECTIONS: [Option<(Section, &str)>; 21] = [
    Some((Section::Account, "Account")),
    Some((Section::Friends, "Friends & Chat")),
    Some((Section::Family, "Family")),
    Some((Section::Security, "Security")),
    Some((Section::Privacy, "Privacy")),
    Some((Section::Wallet, "Wallet & Purchases")),
    Some((Section::Store, "Store Preferences")),
    Some((Section::Language, "Language")),
    None,
    Some((Section::Notifications, "Notifications")),
    Some((Section::Interface, "Interface")),
    Some((Section::Appearance, "Appearance")),
    Some((Section::Library, "Library")),
    Some((Section::Downloads, "Downloads")),
    Some((Section::Folders, "Storage")),
    Some((Section::Cloud, "Cloud")),
    Some((Section::Overlay, "In game")),
    Some((Section::Accessibility, "Accessibility")),
    None,
    Some((Section::Plugins, "Plugins")),
    Some((Section::About, "About")),
];

/// Where the in-game FPS counter shows, like Steam's setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FpsCorner {
    #[default]
    Off,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

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
    /// Which version of the logo the tray icon shows.
    pub tray_icon: ui::brand::Logo,
    pub fps_corner: FpsCorner,
}

pub type SharedFlags = std::sync::Arc<std::sync::Mutex<AppFlags>>;

pub struct Settings {
    section: Section,
    /// Account details, purchase history, licenses, family.
    acct: super::settings_account::AccountPages,
    flags: SharedFlags,
    /// Cached "are we the steam:// handler?" (registry lookups are slow).
    handler: Option<bool>,
    /// The search box above the sections.
    search: String,
    /// Privacy asked for, and a change sent to Steam but not back yet.
    privacy_asked: bool,
    privacy_pending: Option<ProfilePrivacy>,
    /// The themes to pick from: built in, then the user's own (read from
    /// their folder when Appearance opens).
    user_themes: Option<Vec<Theme>>,
    /// Show the account name instead of dots.
    reveal_account: bool,
}

impl Settings {
    pub fn new(flags: SharedFlags) -> Self {
        Self {
            section: Section::default(),
            acct: Default::default(),
            flags,
            handler: None,
            search: String::new(),
            privacy_asked: false,
            privacy_pending: None,
            user_themes: None,
            reveal_account: false,
        }
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
        use super::settings_account::AccountView;
        match arg {
            Some("plugins") => self.section = Section::Plugins,
            Some("family") => self.section = Section::Family,
            Some("privacy") => self.section = Section::Privacy,
            Some("appearance") => self.section = Section::Appearance,
            Some("about") => self.section = Section::About,
            Some("ingame") => self.section = Section::Overlay,
            Some("wallet") => self.section = Section::Wallet,
            Some("notifications") => self.section = Section::Notifications,
            Some(a @ ("history" | "licenses" | "account-details")) => {
                self.section = if a == "account-details" { Section::Account } else { Section::Wallet };
                self.acct.open(match a {
                    "history" => AccountView::Purchases,
                    "licenses" => AccountView::Licenses,
                    _ => AccountView::Details,
                });
            }
            _ => {}
        }
        if self.section == Section::Appearance {
            self.user_themes = None;
        }
    }

    fn on_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        self.acct.family_manager.event(event, ctx);
        // Steam refused a privacy change: show what it has again.
        if let client::Event::ProfileSaved { what: model::ProfileSave::Privacy, result } = event {
            if result.is_err() {
                self.privacy_pending = None;
            }
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        let mut pick = None;
        let mut search = std::mem::take(&mut self.search);
        let current = self.section;
        layout(
            ui,
            &t,
            t!("Settings"),
            |_| {},
            |ui| {
                let width = ui.available_width();
                w::search(ui, &t, &mut search, t!("Search settings…"), width);
                ui.add_space(t.spacing.sm + 2.0);
                let q = search.trim().to_lowercase();
                for entry in SECTIONS {
                    match entry {
                        None if q.is_empty() => nav_divider(ui, &t),
                        None => {}
                        Some((s, en)) => {
                            let label = i18n::tr_dyn(en);
                            if !q.is_empty() && !label.to_lowercase().contains(&q) && !en.to_lowercase().contains(&q) {
                                continue;
                            }
                            if nav_item(ui, &t, &label, current == s).clicked() {
                                pick = Some(s);
                            }
                        }
                    }
                }
            },
            |ui| match self.section {
                Section::Account | Section::Wallet if self.acct.view != Default::default() => self.acct.show(ui, &t, ctx),
                Section::Account => self.account(ui, &t, ctx),
                Section::Wallet => wallet(ui, &t, ctx, &mut self.acct),
                Section::Friends => friends_chat(ui, &t, ctx),
                Section::Family => group(ui, &t, t!("Family"), |ui| self.acct.family(ui, &t, ctx)),
                Section::Security => security(ui, &t, ctx, &mut self.acct),
                Section::Privacy => self.privacy(ui, &t, ctx),
                Section::Notifications => notifications(ui, &t, &self.flags),
                Section::Library => library(ui, &t, ctx),
                Section::Downloads => downloads(ui, &t, ctx, &self.flags),
                Section::Cloud => cloud(ui, &t, &self.flags),
                Section::Accessibility => accessibility(ui, &t, ctx),
                Section::Appearance => self.appearance(ui, &t, ctx),
                Section::Language => language(ui, &t, ctx),
                Section::Plugins => group(ui, &t, t!("Plugins"), |ui| {
                    ui.label(w::body_m(&t, t!("Scripts you install yourself. Each one runs in a sandbox and only gets the permissions you approve."), t.text_dim));
                    ui.add_space(t.spacing.md);
                    ctx.settings_for("plugins", ui);
                }),
                Section::Interface => interface(ui, &t, ctx, &self.flags, &mut self.handler),
                Section::Store => group(ui, &t, t!("Store Preferences"), |ui| {
                    ctx.settings_for("store", ui);
                }),
                Section::Overlay => overlay(ui, &t, ctx, &self.flags),
                Section::Folders => folders(ui, &t, ctx),
                Section::About => about(ui, &t),
            },
        );
        self.search = search;
        if let Some(s) = pick {
            self.section = s;
            self.acct.open(Default::default());
            if s == Section::Appearance {
                self.user_themes = None;
            }
        }
    }
}

// ---------- the layout (shared with Profile Settings) ----------

/// The settings layout: the title (with `right` at its far end), the
/// section column (`nav`) on the left, and the groups (`body`) beside it.
pub(crate) fn layout(ui: &mut Ui, t: &Theme, title: &str, right: impl FnOnce(&mut Ui), nav: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui)) {
    w::paint_background(ui, t, ui.max_rect(), "page");
    egui::ScrollArea::vertical().id_salt(("settings-layout", title)).auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(t.spacing.xl + t.spacing.xl + t.spacing.sm);
        w::centered(ui, 1564.0, t.spacing.xl + t.spacing.sm, |ui| {
            ui.horizontal(|ui| {
                ui.label(w::heading_l(t, title, t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
            });
            ui.add_space(t.spacing.xl);
            let gap = t.spacing.lg + 2.0;
            let nav_w = (ui.available_width() * 0.2).clamp(200.0, 299.0);
            let body_w = (ui.available_width() - nav_w - gap).max(200.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui_with_layout(vec2(nav_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(nav_w);
                    w::component_frame(t, &t.components.settings_nav).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = t.spacing.sm + 2.0;
                        nav(ui);
                    });
                });
                ui.allocate_ui_with_layout(vec2(body_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(body_w);
                    ui.spacing_mut().item_spacing.y = t.spacing.lg + 4.0;
                    body(ui);
                });
            });
            ui.add_space(t.spacing.xl * 2.0);
        });
    });
}

/// A section in the left column.
pub(crate) fn nav_item(ui: &mut Ui, t: &Theme, label: &str, selected: bool) -> egui::Response {
    w::list_row(ui, t, &t.components.settings_nav_item, None, label, "", None, selected)
}

/// The line between groups of sections.
pub(crate) fn nav_divider(ui: &mut Ui, t: &Theme) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.surface_hover));
}

/// A group of settings ("Profile", "Contact Info").
pub(crate) fn group<R>(ui: &mut Ui, t: &Theme, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    w::titled_box(ui, t, &t.components.settings_group, title, "", |ui| {
        ui.spacing_mut().item_spacing.y = t.spacing.md - 1.0;
        if !title.is_empty() {
            ui.add_space(t.spacing.lg);
        }
        add(ui)
    })
}

/// The width of the name column of a row.
fn label_w(ui: &Ui) -> f32 {
    (ui.available_width() * 0.28).clamp(150.0, 265.0)
}

/// One setting: its name (with an "i" explaining it), its value, then
/// buttons or notes. `value` and `rest` draw left to right.
pub(crate) fn row(ui: &mut Ui, t: &Theme, label: &str, tip: &str, value: impl FnOnce(&mut Ui), rest: impl FnOnce(&mut Ui)) {
    row_sized(ui, t, label, tip, 0.0, value, rest);
}

/// A row's name in its column, `lw` wide: it wraps rather than being cut,
/// with the "i" (when there's a tip) after it.
fn row_label(ui: &mut Ui, t: &Theme, label: &str, tip: &str, lw: f32) {
    ui.allocate_ui_with_layout(vec2(lw, t.dense(44.0)), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(lw);
        ui.spacing_mut().item_spacing.x = t.spacing.xs + 1.0;
        let tip_w = if tip.is_empty() { 0.0 } else { t.typography.body_m.size + t.spacing.sm };
        ui.scope(|ui| {
            ui.set_max_width((lw - tip_w).max(40.0));
            ui.add(egui::Label::new(w::heading_s(t, label, t.text)).wrap());
        });
        if !tip.is_empty() {
            w::info_tip(ui, t, tip);
        }
    });
}

/// [`row`] whose value wants `wanted` points: it gets them, past the
/// design's column, out of the room the buttons leave (they're measured
/// the frame before). What still doesn't fit wraps.
pub(crate) fn row_sized(ui: &mut Ui, t: &Theme, label: &str, tip: &str, wanted: f32, value: impl FnOnce(&mut Ui), rest: impl FnOnce(&mut Ui)) {
    let lw = label_w(ui);
    let rest_id = ui.scope_id().with(("settings-row-rest", label));
    let rest_w = ui.ctx().data(|d| d.get_temp::<f32>(rest_id)).unwrap_or(0.0);
    // A soft highlight behind the row under the pointer (`settings_group`'s hover).
    let bg = ui.painter().add(egui::Shape::Noop);
    let r = ui.horizontal(|ui| {
        ui.set_min_height(t.dense(44.0));
        ui.spacing_mut().item_spacing.x = t.spacing.sm + 2.0;
        row_label(ui, t, label, tip, lw);
        let gap = ui.spacing().item_spacing.x;
        let design = (ui.available_width() * 0.35).clamp(120.0, 312.0);
        let room = (ui.available_width() - rest_w - gap).max(design);
        let vw = wanted.clamp(design, room);
        ui.allocate_ui_with_layout(vec2(vw, t.dense(44.0)), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(vw);
            value(ui);
        });
        let r = ui.horizontal(|ui| rest(ui)).response;
        let w = if r.rect.width() > 1.0 { r.rect.width() } else { 0.0 };
        if (w - rest_w).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(rest_id, w));
            ui.ctx().request_repaint();
        }
    });
    let rect = egui::Rect::from_min_max(r.response.rect.min, egui::pos2(ui.max_rect().right(), r.response.rect.max.y)).expand2(vec2(t.spacing.sm, 2.0));
    let hover = ui.ctx().animate_bool_with_time(r.response.id.with("row-hover"), ui.rect_contains_pointer(rect), 0.1);
    if hover > 0.0 {
        let look = t.look(&t.components.settings_group, ui::theme::Interact { hover, ..Default::default() });
        if let ui::theme::ResolvedFill::Color(c) = look.fill {
            ui.painter().set(bg, egui::Shape::rect_filled(rect, t.corner_small(), c));
        }
    }
}

/// A row whose value is text (in `color`) and whose only action is a
/// button; true when the button is pressed.
pub(crate) fn text_row(ui: &mut Ui, t: &Theme, label: &str, tip: &str, value: &str, color: egui::Color32, button: Option<&str>) -> bool {
    let mut clicked = false;
    let wanted = ui.painter().layout_no_wrap(value.to_string(), t.typography.body_l.font(), color).size().x + 2.0;
    row_sized(
        ui,
        t,
        label,
        tip,
        wanted,
        |ui| {
            ui.add(egui::Label::new(w::body_l(t, value, color)).wrap());
        },
        |ui| {
            if let Some(b) = button {
                clicked = secondary(ui, t, b);
            }
        },
    );
    clicked
}

/// The design's secondary button in settings rows.
pub(crate) fn secondary(ui: &mut Ui, t: &Theme, label: &str) -> bool {
    w::button(ui, t, None, label, w::Kind::Secondary, w::Size::Md).clicked()
}

/// A switch with a note after it.
fn switch_row(ui: &mut Ui, t: &Theme, label: &str, note: &str, value: &mut bool) -> bool {
    let mut changed = false;
    row(
        ui,
        t,
        label,
        "",
        |ui| changed = w::toggle(ui, t, value).changed(),
        |ui| {
            ui.add(egui::Label::new(w::body_m(t, note, t.text_dim)).wrap());
        },
    );
    changed
}

/// A row as Steam's account pages had it (name, value, one button): kept
/// for the account pages (`settings_account.rs`).
pub(super) fn info_row(ui: &mut Ui, t: &Theme, label: &str, value: &str, button: Option<&str>) -> bool {
    text_row(ui, t, label, "", value, t.text_dim, button)
}

pub(super) fn web(url: &str) {
    crate::links::open_http(url);
}

/// "••••@gmail.com": the address with its name part hidden.
fn masked_email(e: &str) -> String {
    match e.split_once('@') {
        Some((name, domain)) => format!("{}@{domain}", "*".repeat(name.chars().count().clamp(3, 8))),
        None => "*".repeat(e.chars().count().min(12)),
    }
}

// ---------- the profile's privacy (here and in Profile Settings) ----------

/// The labels for Steam's privacy levels (3 public, 2 friends, 1 private).
fn levels(max: u8) -> Vec<(u8, String)> {
    [(3u8, t!("Public")), (2, t!("Friends only")), (1, t!("Private"))].into_iter().filter(|(v, _)| *v <= max).map(|(v, l)| (v, l.to_string())).collect()
}

/// Steam's privacy settings, as its own page lays them out: your basic
/// details (always public), then your profile and, under it, game details
/// (with "keep my playtime private"), friends list, inventory (with "keep
/// gifts private") and comments, each explained. True when one changed.
pub(crate) fn privacy_rows(ui: &mut Ui, t: &Theme, p: &mut ProfilePrivacy) -> bool {
    let before = *p;
    let select_w = (ui.available_width() * 0.22).clamp(150.0, 200.0);
    let indent = t.spacing.xl + t.spacing.md;
    // One setting: its name and what it means on the left, the choice on
    // the right; `sub` ones sit under "My profile".
    let setting = |ui: &mut Ui, label: &str, note: &str, sub: bool, draw: &mut dyn FnMut(&mut Ui)| {
        ui.add_space(t.spacing.xs);
        ui.horizontal_top(|ui| {
            if sub {
                ui.add_space(indent);
            }
            let text_w = (ui.available_width() - select_w - t.spacing.xl).max(120.0);
            ui.allocate_ui_with_layout(vec2(text_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = t.spacing.xs;
                ui.add(egui::Label::new(w::heading_s(t, label, t.text)).wrap());
                if !note.is_empty() {
                    ui.add(egui::Label::new(w::body_m(t, note, t.text_dim)).wrap());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| draw(ui));
        });
    };
    // A check box under a setting ("keep my playtime private").
    let check = |ui: &mut Ui, on: &mut bool, label: &str| {
        ui.horizontal(|ui| {
            ui.add_space(indent * 2.0);
            w::check(ui, t, on, w::body_m(t, label, t.text));
        });
    };
    setting(ui, &t!("My basic details"), &t!("Your profile name, avatar, real name, location and summary. Anyone can see these; leave a field empty on the General page to hide it."), false, &mut |ui| {
        ui.label(w::body_l(t, t!("Public"), t.text_dim));
    });
    ui.separator();
    setting(ui, &t!("My profile"), &t!("Your level, badges, showcases, comments and groups. When it isn't public, people who can't see it only see your basic details."), false, &mut |ui| {
        w::select(ui, t, &mut p.profile, &levels(3), select_w);
    });
    let top = p.profile;
    // Nothing can be more public than the profile itself.
    p.games = p.games.min(top);
    p.friends = p.friends.min(top);
    p.inventory = p.inventory.min(top);
    setting(ui, &t!("My game details"), &t!("The games you own and wishlist, your achievements and playtime. It also decides whether people see when you're playing and what."), true, &mut |ui| {
        w::select(ui, t, &mut p.games, &levels(top), select_w);
    });
    if p.games > 1 {
        let mut hide = p.playtime == 1;
        check(ui, &mut hide, &t!("Always keep my total playtime private even if users can see my game details."));
        p.playtime = if hide { 1 } else { p.games };
    } else {
        p.playtime = p.games;
    }
    setting(ui, &t!("Friends List"), &t!("Who can see the list of your friends on your profile."), true, &mut |ui| {
        w::select(ui, t, &mut p.friends, &levels(top), select_w);
    });
    setting(ui, &t!("Inventory"), &t!("Your items, trading cards, gifts and anything else in your Steam inventory."), true, &mut |ui| {
        w::select(ui, t, &mut p.inventory, &levels(top), select_w);
    });
    if p.inventory > 1 {
        let mut hide = p.gifts == 1;
        check(ui, &mut hide, &t!("Always keep Steam Gifts private even if users can see my inventory."));
        p.gifts = if hide { 1 } else { p.inventory };
    } else {
        p.gifts = p.inventory;
    }
    // Comments: 1 everyone, 0 friends, 2 nobody.
    setting(ui, &t!("Can post comments on my profile"), "", true, &mut |ui| {
        w::select(ui, t, &mut p.comments, &[(1, t!("Public").to_string()), (0, t!("Friends only").to_string()), (2, t!("Private").to_string())], select_w);
    });
    ui.separator();
    setting(ui, &t!("Screenshots, videos and Workshop items"), &t!("Each has its own privacy setting: choose it when you upload, or change it later on the item's own page."), false, &mut |_| {});
    *p != before
}

// ---------- sections ----------

impl Settings {
    /// Account (the frame itself): profile, contact info, VAC, and the
    /// danger zone.
    fn account(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        use super::settings_account::AccountView;
        let me = ctx.state.persona(ctx.state.steam_id);
        let name = if ctx.state.persona_name.is_empty() { me.name.clone() } else { ctx.state.persona_name.clone() };
        let reveal = &mut self.reveal_account;
        group(ui, t, t!("Profile"), |ui| {
            row(
                ui,
                t,
                t!("Account name"),
                t!("The name you sign in with. Only you see it."),
                |ui| {
                    let shown = if *reveal { ctx.state.account.clone() } else { "•".repeat(ctx.state.account.chars().count().clamp(6, 12)) };
                    let r = ui.add(egui::Label::new(w::body_l(t, shown, t.text_dim)).sense(Sense::click()));
                    if r.on_hover_text(t!("Click to show or hide")).clicked() {
                        *reveal = !*reveal;
                    }
                },
                |ui| {
                    ui.label(w::body_l(t, t!("You can't change your account name."), t.text_dim));
                },
            );
            let (mut visit, mut edit) = (false, None);
            row(
                ui,
                t,
                t!("Profile name"),
                t!("The name everyone sees on your profile and in chat."),
                |ui| {
                    visit = ui.add(egui::Label::new(w::body_l(t, &name, t.accent)).sense(Sense::click()).truncate()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                },
                |ui| {
                    if secondary(ui, t, t!("Edit Profile")) {
                        edit = Some(Some("edit".to_string()));
                    }
                    if secondary(ui, t, t!("Visit Profile")) {
                        edit = Some(None);
                    }
                },
            );
            if let Some(arg) = edit.or(visit.then_some(None)) {
                ctx.navigate("profile", arg);
            }
            if text_row(ui, t, t!("Account details"), "", t!("Contact, Steam Guard, password, devices"), t.text_dim, Some(t!("View"))) {
                self.acct.open(AccountView::Details);
            }
        });
        group(ui, t, t!("Contact Info"), |ui| {
            row(
                ui,
                t,
                t!("Email"),
                t!("Where Steam sends receipts and Steam Guard codes."),
                |ui| match &ctx.state.email {
                    Some((e, ok)) => {
                        ui.label(w::body_l(t, masked_email(e), t.accent)).on_hover_text(e);
                        if *ok {
                            ui.label(w::body_l(t, icons::CHECK, t.play)).on_hover_text(t!("verified"));
                        } else {
                            ui.label(w::body_s(t, t!("not verified"), t.away));
                        }
                    }
                    None => {
                        ui.label(w::body_l(t, "—", t.text_dim));
                    }
                },
                |ui| {
                    if secondary(ui, t, t!("Change Email")) {
                        web("https://help.steampowered.com/wizard/HelpChangeEmail");
                    }
                    if secondary(ui, t, t!("Email Preferences")) {
                        web("https://store.steampowered.com/account/emailoptout");
                    }
                },
            );
            self.acct.ask_security(ctx);
            let phone = match &ctx.state.account_security {
                Some(Ok(s)) if s.phone_verified => t!("A verified phone number is on your account").to_string(),
                Some(Ok(_)) => t!("No phone number").to_string(),
                _ => "…".into(),
            };
            if text_row(ui, t, t!("Phone Number"), "", &phone, t.accent, Some(t!("Manage phone number"))) {
                web("https://store.steampowered.com/phone/manage");
            }
        });
        group(ui, t, t!("VAC"), |ui| {
            // The VAC status comes with your profile: ask for it once.
            static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !ctx.state.profiles.contains_key(&ctx.state.steam_id) && !ASKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                ctx.send(Command::LoadProfile { steam_id: ctx.state.steam_id });
            }
            let (vac, color) = match ctx.state.profiles.get(&ctx.state.steam_id) {
                Some(p) if p.vac_banned => (t!("VAC bans on record").to_string(), t.danger),
                Some(_) => (t!("No VAC bans on this account").to_string(), t.text),
                None => ("…".into(), t.text_dim),
            };
            if text_row(ui, t, t!("Status"), t!("Valve Anti-Cheat bans on this account."), &vac, color, Some(t!("Learn more"))) {
                web("https://help.steampowered.com/faqs/view/647C-5CC1-7EA9-3C29");
            }
        });
        group(ui, t, t!("Current sign-in"), |ui| {
            let id = format!("{} · {}", ctx.state.steam_id, ctx.state.country);
            if text_row(ui, t, t!("Steam ID"), "", &id, t.text_dim, Some(t!("Sign out"))) {
                ctx.send(Command::LogOut);
            }
        });
    }

    /// Privacy (Figma "privacy settings" `477:7327`): saved right away.
    fn privacy(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if !self.privacy_asked && ctx.state.profile_edit.is_none() {
            self.privacy_asked = true;
            ctx.send(Command::LoadProfileEdit { language: i18n::language().steam_name().into() });
        }
        let saved = ctx.state.profile_edit.as_ref().and_then(|r| r.as_ref().ok()).map(|e| e.privacy);
        if saved.is_some() && saved == self.privacy_pending {
            self.privacy_pending = None;
        }
        group(ui, t, t!("Profile Privacy"), |ui| match (self.privacy_pending.or(saved), &ctx.state.profile_edit) {
            (Some(mut p), _) => {
                if privacy_rows(ui, t, &mut p) {
                    self.privacy_pending = Some(p);
                    ctx.send(Command::SaveProfilePrivacy(p));
                }
                ui.label(w::body_s(t, t!("Changes are saved right away."), t.text_faint));
            }
            (None, Some(Err(e))) => {
                ui.label(w::body_m(t, t!("Couldn't load your profile settings"), t.text));
                ui.label(w::body_s(t, i18n::tr_dyn(e), t.text_dim));
                if secondary(ui, t, t!("Retry")) {
                    ctx.send(Command::LoadProfileEdit { language: i18n::language().steam_name().into() });
                }
            }
            (None, _) => {
                for _ in 0..5 {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                    w::skeleton(ui, t, r, t.corner_small());
                }
            }
        });
    }

    /// Appearance: the theme (built in, or from the themes folder), with
    /// the user's colors on top, and the tray icon.
    fn appearance(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let all = self.user_themes.get_or_insert_with(|| ui::themes::builtins().into_iter().chain(ui::themes::user_themes()).collect()).clone();
        let mut reload = false;
        group(ui, t, t!("Theme"), |ui| {
            ui.label(w::body_m(t, t!("Changes the look of the whole app. Your own colors below go on top of it."), t.text_dim));
            ui.add_space(t.spacing.sm);
            let mut pick = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.md, t.spacing.md);
                for preset in all {
                    if theme_card(ui, t, &preset, preset.id == t.id).clicked() && preset.id != t.id {
                        pick = Some(preset);
                    }
                }
            });
            if let Some(mut preset) = pick {
                // Keep the user's own colors and roundness on the new theme.
                let mine = ui::themes::choice_for(t);
                ui::themes::apply_overrides(&mut preset, &mine);
                ctx.act(Action::SetTheme(preset));
            }
            ui.add_space(t.spacing.sm);
            ui.horizontal(|ui| {
                if let Some(dir) = ui::themes::user_dir() {
                    if secondary(ui, t, t!("Open themes folder")) {
                        let _ = std::fs::create_dir_all(dir);
                        local::open_folder(dir);
                    }
                    if secondary(ui, t, t!("Reload")) {
                        reload = true;
                    }
                }
                if ui.link(w::body_m(t, t!("How to make a theme"), t.accent)).clicked() {
                    web("https://github.com/madonchik123/SteamInRust/blob/main/docs/themes.md");
                }
            });
        });
        if reload {
            self.user_themes = None;
            // Read the picked theme's files again too.
            if let Some(fresh) = ui::themes::find(&t.id) {
                let mut fresh = fresh;
                ui::themes::apply_overrides(&mut fresh, &ui::themes::choice_for(t));
                ctx.act(Action::SetTheme(fresh));
            }
        }
        let mut theme = t.clone();
        let mine = ui::themes::choice_for(t);
        group(ui, t, t!("Colors"), |ui| {
            row(ui, t, t!("Accent color"), t!("Links, the open section and focus."), |ui| {
                ui.color_edit_button_srgba(&mut theme.accent);
            }, |_| {});
            row(ui, t, t!("Second accent"), t!("Main buttons and the selected tab."), |ui| {
                ui.color_edit_button_srgba(&mut theme.accent_2);
            }, |_| {});
            row(ui, t, t!("Corner roundness"), "", |ui| {
                ui.add(egui::Slider::new(&mut theme.radius, 0..=20));
            }, |_| {});
            if mine.accent.is_some() || mine.accent_2.is_some() || mine.radius.is_some() {
                row(ui, t, "", "", |ui| {
                    if secondary(ui, t, t!("Use the theme's colors")) {
                        if let Some(base) = ui::themes::find(&t.id) {
                            theme = base;
                        }
                    }
                }, |_| {});
            }
        });
        if &theme != t {
            ctx.act(Action::SetTheme(theme));
        }
        group(ui, t, t!("Interface size"), |ui| {
            row(ui, t, t!("Interface scale"), t!("Makes text and everything else bigger or smaller."), |ui| {
                let mut zoom = ctx.zoom;
                if ui.add(egui::Slider::new(&mut zoom, 0.8..=1.6).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).drag_stopped() {
                    ctx.act(Action::SetZoom(zoom));
                }
            }, |_| {});
        });
        group(ui, t, t!("App icon"), |ui| tray_icon_picker(ui, t, &self.flags));
    }
}

/// A theme to pick: its preview picture (or a sketch in its colors), name
/// and author.
fn theme_card(ui: &mut Ui, t: &Theme, preset: &Theme, selected: bool) -> egui::Response {
    let size = vec2(t.dense(196.0), t.dense(168.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let pic = egui::Rect::from_min_size(rect.min, vec2(size.x, size.y - t.dense(48.0)));
    let p = ui.painter();
    let shown = preset.preview.as_deref().and_then(|path| ui::themes::image_uri(ui.ctx(), preset, path)).is_some_and(|uri| w::paint_cover(ui, pic, &uri, t.corner()));
    if !shown {
        // A sketch of the app in the theme's own colors.
        p.rect_filled(pic, t.corner(), preset.bg);
        let top = egui::Rect::from_min_size(pic.min, vec2(pic.width(), pic.height() * 0.16));
        p.rect_filled(top, egui::CornerRadius { nw: t.corner().nw, ne: t.corner().ne, sw: 0, se: 0 }, preset.chrome);
        let pad = pic.width() * 0.07;
        let card = |x: f32, wd: f32| egui::Rect::from_min_size(pic.min + vec2(x, top.height() + pad), vec2(wd, pic.height() * 0.36));
        p.rect_filled(card(pad, pic.width() * 0.38), preset.corner_small(), preset.surface);
        p.rect_filled(card(pad * 2.0 + pic.width() * 0.38, pic.width() - pad * 3.0 - pic.width() * 0.38), preset.corner_small(), preset.surface);
        let button = egui::Rect::from_min_size(egui::pos2(pic.left() + pad, pic.bottom() - pad - pic.height() * 0.12), vec2(pic.width() * 0.3, pic.height() * 0.12));
        p.rect_filled(button, preset.corner_small(), preset.accent_2);
        let link = egui::Rect::from_min_size(egui::pos2(button.right() + pad, button.center().y - 2.0), vec2(pic.width() * 0.22, 4.0));
        p.rect_filled(link, egui::CornerRadius::same(2), preset.accent);
        p.circle_filled(egui::pos2(top.right() - pad, top.center().y), top.height() * 0.22, preset.accent);
    }
    let ring = if selected { egui::Stroke::new(2.0, t.accent) } else { egui::Stroke::new(1.0, w::mix(t.surface_hover, t.text_dim, hover)) };
    p.rect_stroke(pic, t.corner(), ring, egui::StrokeKind::Outside);
    let name = if selected { format!("{}  {}", icons::CHECK, preset.name) } else { preset.name.clone() };
    let mut job = egui::text::LayoutJob::simple_singleline(name, t.font_of(&t.components.settings_nav_item), if selected { t.accent } else { t.text });
    job.wrap = egui::text::TextWrapping::truncate_at_width(size.x);
    p.galley(egui::pos2(rect.left(), pic.bottom() + t.spacing.sm), ui.painter().layout_job(job), t.text);
    let by = if preset.author.is_empty() { String::new() } else { t!("by {author}", author = preset.author) };
    let mut job = egui::text::LayoutJob::simple_singleline(by, egui::FontId::proportional(t.typography.body_s.size), t.text_dim);
    job.wrap = egui::text::TextWrapping::truncate_at_width(size.x);
    p.galley(egui::pos2(rect.left(), pic.bottom() + t.spacing.sm + t.typography.body_l.size + t.spacing.xs + 2.0), ui.painter().layout_job(job), t.text_dim);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The versions of the logo the tray icon can show, each on the top bar's
/// color, like the theme cards.
fn tray_icon_picker(ui: &mut Ui, t: &Theme, flags: &SharedFlags) {
    use ui::brand::Logo;
    ui.label(w::body_m(t, t!("The logo SteamInRust shows on the taskbar and in the system tray."), t.text_dim));
    ui.add_space(t.spacing.sm);
    let mut current = flags.lock().unwrap().tray_icon;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.spacing.md, t.spacing.md);
        for logo in Logo::ALL {
            let (rect, resp) = ui.allocate_exact_size(vec2(t.dense(112.0), t.dense(104.0)), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
            ui.painter().rect_filled(rect, t.corner(), w::mix(t.raised, t.surface_hover, hover * 0.5));
            let mark = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + rect.height() * 0.37), vec2(rect.height() * 0.46, rect.height() * 0.46));
            logo.image().paint_at(ui, mark);
            let on = logo == current;
            ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - rect.height() * 0.15), egui::Align2::CENTER_CENTER, i18n::tr_dyn(logo.name()), egui::FontId::proportional(t.typography.body_s.size), if on { t.text } else { t.text_dim });
            if on {
                ui.painter().rect_stroke(rect, t.corner(), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                current = logo;
            }
        }
    });
    flags.lock().unwrap().tray_icon = current;
}

fn friends_chat(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    group(ui, t, t!("Friends & Chat"), |ui| {
        let code = (ctx.state.steam_id & 0xFFFF_FFFF).to_string();
        if text_row(ui, t, t!("Your friend code"), t!("Friends can add you with this code."), &code, t.accent, Some(t!("Copy"))) {
            ui.ctx().copy_text(code);
            ctx.toast(t!("Friend code copied"));
        }
        if text_row(ui, t, t!("Friends"), "", &ctx.state.friend_list().len().to_string(), t.text, Some(t!("Open friends list"))) {
            ctx.navigate("friends", None);
        }
        if text_row(ui, t, t!("Privacy"), t!("Who can see your status and send invites"), "", t.text_dim, Some(t!("Privacy settings"))) {
            ctx.navigate("settings", Some("privacy".into()));
        }
    });
}

fn wallet(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, pages: &mut super::settings_account::AccountPages) {
    use super::settings_account::AccountView;
    group(ui, t, t!("Steam Wallet"), |ui| {
        let balance = ctx.state.wallet.map(|(b, c)| model::format_money(b, c)).unwrap_or_else(|| "—".into());
        if text_row(ui, t, t!("Balance"), "", &balance, t.text, Some(t!("Add funds"))) {
            web("https://store.steampowered.com/steamaccount/addfunds");
        }
        if ctx.state.wallet_pending > 0 {
            if let Some((_, c)) = ctx.state.wallet {
                text_row(ui, t, t!("Pending"), t!("Money from recent Market sales or refunds that Steam hasn't released yet. It's added to your balance automatically."), &model::format_money(ctx.state.wallet_pending, c), t.away, None);
            }
        }
        if text_row(ui, t, t!("Wallet code"), "", "", t.text_dim, Some(t!("Redeem a Steam Wallet code…"))) {
            web("https://store.steampowered.com/account/redeemwalletcode");
        }
    });
    group(ui, t, t!("Purchases"), |ui| {
        if text_row(ui, t, t!("Purchase history"), "", t!("Purchases, gifts, refunds, wallet and Market"), t.text_dim, Some(t!("View"))) {
            pages.open(AccountView::Purchases);
        }
        if text_row(ui, t, t!("Licenses"), "", t!("Everything on your account and how you got it"), t.text_dim, Some(t!("View"))) {
            pages.open(AccountView::Licenses);
        }
        if text_row(ui, t, t!("Market listings"), "", "", t.text_dim, Some(t!("View"))) {
            ctx.navigate("inventory", Some("listings".into()));
        }
    });
}

fn security(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, pages: &mut super::settings_account::AccountPages) {
    pages.ask_security(ctx);
    let (guard, password) = match &ctx.state.account_security {
        Some(Ok(s)) => (
            if s.authenticator {
                t!("Steam Guard Mobile Authenticator is on").to_string()
            } else if s.steam_guard {
                t!("Codes by email are on").to_string()
            } else {
                t!("Off: your account isn't protected").to_string()
            },
            match (s.password_changed, s.password_reset) {
                (Some(c), _) => t!("Last changed: {date}", date = ui::date(c)).to_string(),
                (None, Some(r)) => t!("Last reset: {date}", date = ui::date(r)).to_string(),
                _ => String::new(),
            },
        ),
        _ => (t!("Protects your account with codes from the Steam Mobile app").to_string(), String::new()),
    };
    group(ui, t, t!("Security"), |ui| {
        if text_row(ui, t, t!("Steam Guard"), "", &guard, t.text_dim, Some(t!("Manage Steam Guard"))) {
            web("https://store.steampowered.com/twofactor/manage");
        }
        if text_row(ui, t, t!("Authorized devices"), t!("See and remove devices signed in to your account"), "", t.text_dim, Some(t!("Manage devices"))) {
            web("https://store.steampowered.com/account/authorizeddevices");
        }
        if text_row(ui, t, t!("Password"), "", &password, t.text_dim, Some(t!("Change password"))) {
            web("https://help.steampowered.com/wizard/HelpChangePassword");
        }
        if text_row(ui, t, t!("This PC"), "", t!("Saved sign-in for this account is stored on this PC"), t.text_dim, Some(t!("Sign out"))) {
            ctx.send(Command::LogOut);
        }
    });
}

fn notifications(ui: &mut Ui, t: &Theme, flags: &SharedFlags) {
    let mut f = *flags.lock().unwrap();
    group(ui, t, t!("Notifications"), |ui| {
        switch_row(ui, t, t!("New chat messages"), t!("Show a pop-up when a friend messages you."), &mut f.toast_messages);
        switch_row(ui, t, t!("Steam notifications"), t!("Show a pop-up for gifts, items, friend requests, sales and more."), &mut f.toast_notifications);
    });
    *flags.lock().unwrap() = f;
}

fn library(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    group(ui, t, t!("Library"), |ui| {
        if text_row(ui, t, t!("Collections"), "", t!("Steam's Favorites and Hidden are imported automatically"), t.text_dim, Some(t!("Open library"))) {
            ctx.navigate("library", None);
        }
        if text_row(ui, t, t!("Add a non-Steam game"), "", "", t.text_dim, Some(t!("Open library"))) {
            ctx.navigate("library", None);
        }
    });
}

fn downloads(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, flags: &SharedFlags) {
    let mut f = *flags.lock().unwrap();
    group(ui, t, t!("Downloads"), |ui| {
        switch_row(ui, t, t!("Automatic updates"), t!("Keep installed games up to date. Turn off to update only when you choose (or when you launch a game)."), &mut f.auto_updates);
        if text_row(ui, t, t!("Download region, speed limit, schedule"), "", "", t.text_dim, Some(t!("Open download settings"))) {
            ctx.navigate("downloads", None);
        }
    });
    *flags.lock().unwrap() = f;
}

fn cloud(ui: &mut Ui, t: &Theme, flags: &SharedFlags) {
    let mut f = *flags.lock().unwrap();
    group(ui, t, t!("Cloud"), |ui| {
        ui.label(w::body_m(t, t!("Steam Cloud keeps your game data the same on every PC."), t.text_dim));
        switch_row(ui, t, t!("Steam Cloud"), t!("Enable Steam Cloud for games that support it. Each game can also be turned off in its Properties."), &mut f.cloud);
    });
    *flags.lock().unwrap() = f;
}

fn accessibility(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    group(ui, t, t!("Accessibility"), |ui| {
        row(ui, t, t!("Interface size"), t!("Makes text and everything else bigger or smaller."), |ui| {
            let mut zoom = ctx.zoom;
            if ui.add(egui::Slider::new(&mut zoom, 0.8..=1.6).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).changed() {
                ctx.act(Action::SetZoom(zoom));
            }
        }, |_| {});
    });
}

fn language(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    group(ui, t, t!("Language"), |ui| {
        row(ui, t, t!("Interface language"), t!("The language of SteamInRust's own text. Store pages follow it too."), |ui| {
            let mut lang = i18n::language();
            let options: Vec<(i18n::Lang, String)> = i18n::Lang::ALL.iter().map(|l| (*l, l.native_name().to_string())).collect();
            let width = ui.available_width().min(260.0);
            if w::select(ui, t, &mut lang, &options, width) {
                ctx.act(Action::SetLanguage(lang.code().into()));
            }
        }, |_| {});
    });
}

fn interface(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, flags: &SharedFlags, handler: &mut Option<bool>) {
    let mut f = *flags.lock().unwrap();
    group(ui, t, t!("Interface"), |ui| {
        switch_row(ui, t, t!("Close to tray"), t!("Keep SteamInRust running in the system tray when you close the window."), &mut f.close_to_tray);
        switch_row(ui, t, t!("Ask which account to use"), t!("Show the account picker at startup instead of signing in automatically."), &mut f.ask_account);
        let mut on = *handler.get_or_insert_with(crate::links::is_default_handler);
        if switch_row(ui, t, t!("Open steam:// links"), t!("Store buttons, desktop shortcuts and links like steam://rungameid/570 will open here. Turning it off gives them back to the official Steam client."), &mut on) {
            let r = if on { crate::links::register() } else { crate::links::unregister() };
            match r {
                Ok(()) => ctx.toast(if on { t!("steam:// links now open in SteamInRust") } else { t!("steam:// links handed back") }),
                Err(e) => ctx.toast(e),
            }
            *handler = None;
        }
        // macOS: what the system has to allow (Shift+Tab in games).
        #[cfg(target_os = "macos")]
        {
            let state = if crate::mac_permissions::missing() { t!("Some features need your permission.") } else { t!("Everything SteamInRust needs is allowed.") };
            if text_row(ui, t, t!("macOS permissions"), "", state, t.text_dim, Some(t!("Review…"))) {
                crate::mac_permissions::open();
            }
        }
    });
    *flags.lock().unwrap() = f;
}

fn overlay(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, flags: &SharedFlags) {
    group(ui, t, t!("In-game overlay"), |ui| {
        ui.label(w::heading_s(t, t!("Shift+Tab"), t.text));
        ui.add(egui::Label::new(w::body_m(
            t,
            t!("While a game you started from SteamInRust is running, press Shift+Tab to open the overlay: friends, chat, achievements, guides, notes and more. In most Windows games (Direct3D 11) it's drawn inside the game, so it works in exclusive fullscreen too. Other games get it as a window over the game."),
            t.text_dim,
        )).wrap());
        ui.add_space(t.spacing.md);
        if w::button(ui, t, Some(icons::EYE), t!("Preview overlay"), w::Kind::Secondary, w::Size::Md).clicked() {
            ctx.act(Action::ToggleOverlay);
        }
    });
    group(ui, t, t!("Screenshots"), |ui| {
        ui.add(egui::Label::new(w::body_m(
            t,
            t!("Press F12 in a game to take a screenshot. It's saved in Steam's screenshot folder, where Steam can upload it to your profile."),
            t.text_dim,
        )).wrap());
    });
    let mut f = *flags.lock().unwrap();
    group(ui, t, t!("FPS counter"), |ui| {
        ui.add(egui::Label::new(w::body_m(t, t!("Shows how many frames per second the game draws, in a corner of the screen."), t.text_dim)).wrap());
        ui.add_space(t.spacing.sm);
        ui.horizontal_wrapped(|ui| {
            for (corner, label) in [
                (FpsCorner::Off, t!("Off")),
                (FpsCorner::TopLeft, t!("Top left")),
                (FpsCorner::TopRight, t!("Top right")),
                (FpsCorner::BottomLeft, t!("Bottom left")),
                (FpsCorner::BottomRight, t!("Bottom right")),
            ] {
                if w::chip(ui, t, &label, f.fps_corner == corner).clicked() {
                    f.fps_corner = corner;
                }
            }
        });
    });
    *flags.lock().unwrap() = f;
}

fn folders(ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
    // Saved pictures (store and library art), like Steam's web browser cache.
    group(ui, t, t!("Picture cache"), |ui| {
        let size = ui.ctx().memory(|m| m.data.get_temp::<u64>(egui::Id::unique("image-cache-size")));
        let size = size.unwrap_or_else(|| {
            let s = crate::image_cache::size();
            ui.ctx().memory_mut(|m| m.data.insert_temp(egui::Id::unique("image-cache-size"), s));
            s
        });
        row(
            ui,
            t,
            &ui::bytes(size),
            "",
            |ui| {
                ui.add(egui::Label::new(w::body_m(t, t!("Store and library pictures, kept so pages open instantly and work offline."), t.text_dim)).wrap());
            },
            |ui| {
                if w::button(ui, t, Some(icons::TRASH), t!("Clear"), w::Kind::Secondary, w::Size::Md).clicked() {
                    crate::image_cache::clear(ui.ctx());
                    ui.ctx().memory_mut(|m| m.data.insert_temp(egui::Id::unique("image-cache-size"), 0u64));
                }
            },
        );
    });
    group(ui, t, t!("Library folders"), |ui| {
        ui.label(w::body_m(t, t!("Folders where games are installed (read from your Steam setup)."), t.text_dim));
        if ctx.local.libraries.is_empty() {
            w::empty(ui, t, icons::DRIVES, t!("No library folders found"), t!("Install location detection looks for an existing Steam install."));
        }
        for lib in &ctx.local.libraries {
            let games: Vec<_> = ctx.local.games.iter().filter(|g| &g.library == lib).collect();
            let size: u64 = games.iter().map(|g| g.size_on_disk).sum();
            let info = t!("{n} games · {size}", n = games.len(), size = ui::bytes(size));
            if text_row(ui, t, &lib.display().to_string(), "", &info, t.text_dim, Some(t!("Open"))) {
                local::open_folder(lib);
            }
        }
    });
}

fn about(ui: &mut Ui, t: &Theme) {
    crate::updater::settings_ui(ui, t);
    group(ui, t, t!("About"), |ui| {
        ui.horizontal(|ui| {
            w::app_icon(ui, t.dense(40.0));
            ui.vertical(|ui| {
                ui.label(w::heading_s(t, format!("SteamInRust {}", env!("CARGO_PKG_VERSION")), t.text));
                ui.label(w::body_m(t, t!("A native Steam client written in Rust."), t.text_dim));
            });
        });
        ui.label(w::body_s(t, t!("Not affiliated with Valve. Steam is a trademark of Valve Corporation."), t.text_faint));
        ui.label(w::body_s(t, t!("Fonts: Inter (OFL). Icons: Phosphor (MIT). Protocol definitions: SteamTracking/Protobufs."), t.text_faint));
        let dir = client::store::config_dir();
        if text_row(ui, t, t!("Data folder"), "", &dir.display().to_string(), t.text_dim, Some(t!("Open"))) {
            let _ = std::fs::create_dir_all(&dir);
            local::open_folder(&dir);
        }
    });
}

/// The old switch, for pages that haven't moved to `w::toggle` yet (it
/// takes the theme's `toggle` style; this one only egui's colors).
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
        let fill = w::mix(v.widgets.inactive.bg_fill, v.selection.stroke.color, k);
        let fill = if ui.is_enabled() { fill } else { fill.gamma_multiply(0.5) };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(11), fill);
        let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, k);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), 8.0, v.widgets.active.fg_stroke.color);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

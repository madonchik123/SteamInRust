//! Managing a group chat, like Steam's Friends & Chat: the header's
//! notifications, invite and settings buttons with the online and member
//! counts; the Notifications and Invite dialogs; the group settings window
//! (General, Channels, Permissions, Invite links, Invited accounts, Bans,
//! Leave); a member's right-click Manage menu; and making a new group chat.
//! Everything you aren't allowed to do shows greyed out with the reason.

use crate::friends_window::{Conversations, Pane};
use client::Command;
use egui::{Align, Layout, Margin, Sense, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::chat_admin::{AdminEvent, AdminOp, GroupAdmin, Notify, RolePerms};
use model::{ChatGroup, PersonaState, SteamId, art};
use std::collections::{BTreeSet, HashSet};
use ui::{Theme, icons, widgets as w};

/// The settings window's pages (Steam's left column).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Page {
    #[default]
    General,
    Channels,
    Permissions,
    Links,
    Invited,
    Bans,
}

/// Something that asks first.
#[derive(Clone, PartialEq, Debug)]
enum Confirm {
    Leave,
    Delete,
    Kick(SteamId),
    Ban(SteamId),
    DeleteRoom(u64, String),
    DeleteRole(u64, String),
}

#[derive(Clone, Debug)]
enum Dialog {
    Notify,
    Invite { search: String, picked: BTreeSet<SteamId>, expiry: Expiry, link: Option<String> },
    Settings(Box<Settings>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Expiry {
    Hour,
    #[default]
    Day,
    Never,
}

impl Expiry {
    fn seconds(self) -> Option<u32> {
        match self {
            Expiry::Hour => Some(3600),
            Expiry::Day => Some(86_400),
            Expiry::Never => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Settings {
    page: Page,
    /// Drafts of the name and tagline (None = not touched).
    name: Option<String>,
    tagline: Option<String>,
    new_text: String,
    /// A channel being renamed, and its new name.
    renaming: Option<(u64, String)>,
    /// The role being edited (Permissions page), its name draft.
    role: Option<u64>,
    role_name: Option<String>,
    new_role: Option<String>,
    link_expiry: Expiry,
}

/// The dialogs' state; lives in the Friends extension.
#[derive(Default)]
pub struct Admin {
    dialog: Option<(u64, Dialog)>,
    confirm: Option<(u64, Confirm)>,
    /// Groups whose settings were asked for.
    asked: HashSet<u64>,
    /// A new group chat: its name, the friend search and who's picked.
    new_name: String,
    new_search: String,
    new_picked: BTreeSet<SteamId>,
    creating: bool,
    /// `--group-admin=<steps>` was acted on (previews).
    previewed: bool,
    /// A member whose menu a preview opens.
    menu_for: Option<SteamId>,
}

fn admin_of<'a>(ctx: &Ctx<'a>, group: u64) -> Option<&'a GroupAdmin> {
    ctx.state.chat_admin.get(&group).and_then(|r| r.as_ref().ok())
}

fn name_of(state: &client::State, id: SteamId) -> String {
    let p = state.persona(id);
    if p.name.is_empty() { id.to_string() } else { p.display_name() }
}

/// "● 3 online  ● 8 members", like Steam's header.
pub fn counts(state: &client::State, group: u64) -> String {
    let members = state.chat_group_members.get(&group).map(|m| m.as_slice()).unwrap_or_default();
    let online = members.iter().filter(|id| **id == state.steam_id || state.persona(**id).state != PersonaState::Offline).count();
    format!("● {}   ● {}", t!("{n} online", n = online), t!("{n} members", n = members.len()))
}

fn when(time: u32) -> String {
    if time == 0 {
        return String::new();
    }
    ui::date_time(time as u64)
}

/// A greyed-out button or row says why on hover.
fn allowed(resp: egui::Response, ok: bool, why: &str) -> egui::Response {
    if ok { resp } else { resp.on_disabled_hover_text(why) }
}

fn no_right(what: &str) -> String {
    t!("Your role doesn't allow this ({what})", what = what).to_string()
}

fn notify_options(with_inherit: bool) -> Vec<(Notify, String)> {
    let mut v = vec![];
    if with_inherit {
        v.push((Notify::Inherit, t!("Don't override").to_string()));
    }
    v.extend([
        (Notify::All, t!("Every message").to_string()),
        (Notify::MentionAll, t!("@all, @online, and @mentions only").to_string()),
        (Notify::Mentions, t!("Direct @mentions only").to_string()),
        (Notify::None, t!("None").to_string()),
    ]);
    v
}

impl Admin {
    fn send(ctx: &Ctx, group: u64, op: AdminOp) {
        ctx.send(Command::ChatAdmin { group, op });
    }

    /// Steam's answers: errors as toasts, new links into the invite dialog,
    /// a new group opens.
    pub fn on_event(&mut self, e: &AdminEvent, ctx: &mut Ctx) {
        match e {
            AdminEvent::Done { result: Err(error), op, .. } => {
                if matches!(op, AdminOp::CreateGroup { .. }) {
                    self.creating = false;
                }
                ctx.toast(t!("That didn't work: {error}", error = i18n::tr_dyn(error)));
            }
            AdminEvent::Done { op: AdminOp::InviteFriend { steam_id, .. }, result: Ok(()), .. } => {
                ctx.toast(t!("Invited {name}", name = name_of(ctx.state, *steam_id)));
            }
            AdminEvent::Done { op: AdminOp::Leave | AdminOp::Delete, group, result: Ok(()) } => {
                if self.dialog.as_ref().is_some_and(|(g, _)| g == group) {
                    self.dialog = None;
                }
            }
            AdminEvent::LinkCreated { group, link } => {
                if let Some((g, Dialog::Invite { link: shown, .. })) = &mut self.dialog {
                    if g == group {
                        *shown = Some(link.url());
                    }
                }
            }
            AdminEvent::GroupCreated { group, .. } => {
                if std::mem::take(&mut self.creating) {
                    self.new_name.clear();
                    self.new_search.clear();
                    self.new_picked.clear();
                    ctx.navigate("friends", Some(format!("group:{}", group.id)));
                }
            }
            _ => {}
        }
    }

    /// Ask Steam for a group's settings once.
    fn want(&mut self, ctx: &Ctx, group: u64) {
        if self.asked.insert(group) && !ctx.state.chat_admin.contains_key(&group) {
            Self::send(ctx, group, AdminOp::Load);
        }
    }

    /// The header's buttons, drawn right to left: settings, invite,
    /// notifications.
    pub fn header_buttons(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, g: &ChatGroup) {
        self.want(ctx, g.id);
        self.preview(ctx, g.id);
        let perms = admin_of(ctx, g.id).map(|a| a.perms_of(ctx.state.steam_id));
        if w::icon_button(ui, t, icons::GEAR, t!("Group settings"), false, 0).clicked() {
            Self::send(ctx, g.id, AdminOp::Load);
            self.dialog = Some((g.id, Dialog::Settings(Box::default())));
        }
        let can_invite = perms.is_none_or(|p| p.invite);
        let r = ui.add_enabled_ui(can_invite, |ui| w::icon_button(ui, t, icons::USER_PLUS, t!("Invite friends"), false, 0)).inner;
        if allowed(r, can_invite, &no_right(t!("Invite new members").as_ref())).clicked() {
            self.open_invite(ctx, g.id);
        }
        if w::icon_button(ui, t, icons::BELL, t!("Notifications"), false, 0).clicked() {
            Self::send(ctx, g.id, AdminOp::Load);
            self.dialog = Some((g.id, Dialog::Notify));
        }
    }

    /// Previews of an off-screen test copy (`--group-admin=a,b`): `notify`,
    /// `invite`, `settings[:general|channels|permissions|links|invited|bans]`,
    /// `role:<id>`, `menu:<steamid>`, `confirm:leave|delete`, and changes
    /// made first: `kick:<steamid>`, `ban:<steamid>`, `room:<name>`.
    fn preview(&mut self, ctx: &Ctx, group: u64) {
        if self.previewed || !admin_of(ctx, group).is_some() {
            return;
        }
        self.previewed = true;
        let Some(steps) = std::env::args().find_map(|a| a.strip_prefix("--group-admin=").map(String::from)) else { return };
        for step in steps.split(',') {
            let (what, arg) = step.split_once(':').unwrap_or((step, ""));
            let id = arg.parse::<u64>().unwrap_or(0);
            match what {
                "notify" => self.dialog = Some((group, Dialog::Notify)),
                "invite" => self.open_invite(ctx, group),
                "settings" => {
                    let page = match arg {
                        "channels" => Page::Channels,
                        "permissions" => Page::Permissions,
                        "links" => Page::Links,
                        "invited" => Page::Invited,
                        "bans" => Page::Bans,
                        _ => Page::General,
                    };
                    self.dialog = Some((group, Dialog::Settings(Box::new(Settings { page, ..Default::default() }))));
                }
                "role" => self.dialog = Some((group, Dialog::Settings(Box::new(Settings { page: Page::Permissions, role: Some(id), ..Default::default() })))),
                "menu" => self.menu_for = Some(id),
                "confirm" => self.confirm = Some((group, if arg == "delete" { Confirm::Delete } else { Confirm::Leave })),
                "kick" => Self::send(ctx, group, AdminOp::Kick { steam_id: id }),
                "ban" => Self::send(ctx, group, AdminOp::Ban { steam_id: id, ban: true }),
                "room" => Self::send(ctx, group, AdminOp::CreateRoom { name: arg.to_string(), voice: false }),
                _ => {}
            }
        }
    }

    fn open_invite(&mut self, ctx: &Ctx, group: u64) {
        let expiry = Expiry::Day;
        // Like Steam, the dialog starts with a fresh link.
        Self::send(ctx, group, AdminOp::CreateInviteLink { seconds: expiry.seconds(), room: 0 });
        self.dialog = Some((group, Dialog::Invite { search: String::new(), picked: BTreeSet::new(), expiry, link: None }));
    }

    /// A member's right-click menu: Manage (roles), kick, ban, make admin.
    pub fn member_menu(&mut self, resp: &egui::Response, t: &Theme, ctx: &mut Ctx, group: u64, who: SteamId) {
        // Previews: the menu stays open beside the row.
        if self.menu_for == Some(who) {
            egui::Popup::menu(resp).id(resp.id.with("preview-menu")).open(true).show(|ui| self.member_menu_body(ui, t, ctx, group, who));
        }
        resp.context_menu(|ui| self.member_menu_body(ui, t, ctx, group, who));
    }

    fn member_menu_body(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, group: u64, who: SteamId) {
        let me = ctx.state.steam_id;
        ui.set_min_width(t.dense(240.0));
        if w::menu_item(ui, t, icons::USER, t!("View profile")).clicked() {
            ctx.navigate("profile", Some(who.to_string()));
            ui.close();
        }
        let Some(a) = admin_of(ctx, group) else {
            ui.label(w::body_s(t, t!("Loading…"), t.text_faint));
            return;
        };
        if who == me {
            return;
        }
        let p = a.perms_of(me);
        let owner = who == a.owner;
        ui.separator();
        ui.label(w::body_s(t, t!("Manage"), t.text_dim));
        let has = a.roles_of(who);
        for role in a.sorted_roles().into_iter().filter(|r| r.id != a.default_role) {
            let mut on = has.contains(&role.id);
            let ok = p.assign_roles && !owner;
            let r = ui.add_enabled_ui(ok, |ui| w::check(ui, t, &mut on, role.name.as_str())).inner;
            if allowed(r, ok, &if owner { t!("The owner has every permission").to_string() } else { no_right(t!("Assign roles").as_ref()) }).changed() {
                Self::send(ctx, group, if on { AdminOp::AddRole { steam_id: who, role: role.id } } else { AdminOp::RemoveRole { steam_id: who, role: role.id } });
            }
        }
        let is_admin = owner || has.iter().any(|r| a.role(*r).is_some_and(|r| r.perms.is_all()));
        let ok = p.assign_roles && p.edit_roles && !is_admin;
        let why = if is_admin { t!("Already an admin").to_string() } else { no_right(t!("Edit roles").as_ref()) };
        if allowed(ui.add_enabled_ui(ok, |ui| w::menu_item(ui, t, icons::SHIELD, t!("Make admin"))).inner, ok, &why).clicked() {
            Self::send(ctx, group, AdminOp::MakeAdmin { steam_id: who, name: t!("Admin").to_string() });
            ui.close();
        }
        ui.separator();
        let ok = p.kick && !owner;
        let why = if owner { t!("The owner can't be removed").to_string() } else { no_right(t!("Kick members").as_ref()) };
        if allowed(ui.add_enabled_ui(ok, |ui| w::menu_item_tinted(ui, t, icons::SIGN_OUT, t!("Kick…"), Some(t.danger))).inner, ok, &why).clicked() {
            self.confirm = Some((group, Confirm::Kick(who)));
            ui.close();
        }
        let ok = p.ban && !owner;
        let why = if owner { t!("The owner can't be removed").to_string() } else { no_right(t!("Ban members").as_ref()) };
        if allowed(ui.add_enabled_ui(ok, |ui| w::menu_item_tinted(ui, t, icons::LOCK, t!("Ban…"), Some(t.danger))).inner, ok, &why).clicked() {
            self.confirm = Some((group, Confirm::Ban(who)));
            ui.close();
        }
    }

    /// Whatever dialog is open (call once per frame from the group's view).
    pub fn dialogs(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, convs: &mut Conversations, g: &ChatGroup, name: &str) {
        if let Some((group, _)) = &self.dialog {
            if *group == g.id {
                let mut dialog = self.dialog.take();
                let keep = match &mut dialog {
                    Some((_, Dialog::Notify)) => self.notify_dialog(ui, t, ctx, g, name),
                    Some((_, Dialog::Invite { search, picked, expiry, link })) => self.invite_dialog(ui, t, ctx, g, name, search, picked, expiry, link),
                    Some((_, Dialog::Settings(s))) => self.settings_dialog(ui, t, ctx, g, name, s),
                    None => false,
                };
                if keep && self.dialog.is_none() {
                    self.dialog = dialog;
                }
            }
        }
        self.confirm_dialog(ui, t, ctx, convs, g, name);
    }

    fn modal<R>(ui: &Ui, t: &Theme, id: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> (R, bool) {
        let r = egui::Modal::new(egui::Id::unique(("group-admin", id))).frame(w::component_frame(t, &t.components.settings_group).inner_margin(Margin::same(t.spacing.xl as i8))).show(ui.ctx(), |ui| {
            ui.set_width(t.dense(width));
            add(ui)
        });
        let closed = r.should_close();
        (r.inner, closed)
    }

    /// A dialog's title with a close button on its right.
    fn title(ui: &mut Ui, t: &Theme, title: &str) -> bool {
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(w::heading_m(t, title, t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked();
            });
        });
        close
    }

    fn notify_dialog(&mut self, ui: &Ui, t: &Theme, ctx: &mut Ctx, g: &ChatGroup, name: &str) -> bool {
        let a = admin_of(ctx, g.id).cloned();
        let (keep, closed) = Self::modal(ui, t, "notify", 560.0, |ui| {
            let mut keep = !Self::title(ui, t, t!("Notifications").as_ref());
            ui.label(w::body_s(t, t!("For chat room {name}", name = name).to_uppercase(), t.text_dim));
            ui.add_space(t.spacing.lg);
            let Some(a) = a else {
                w::loading(ui, t, t!("Loading…"));
                return keep;
            };
            ui.label(w::body_s(t, t!("Notify me for:").to_uppercase(), t.text_dim));
            let mut level = a.notify;
            let width = ui.available_width();
            if w::select(ui, t, &mut level, &notify_options(false), width) {
                Self::send(ctx, g.id, AdminOp::SetNotify(level));
            }
            ui.add_space(t.spacing.md);
            ui.separator();
            ui.add_space(t.spacing.sm);
            let texts: Vec<_> = g.rooms.iter().filter(|r| !r.voice).collect();
            let col = t.dense(200.0);
            egui::Grid::new("room-notify").num_columns(2).spacing(vec2(t.spacing.md, t.spacing.sm)).show(ui, |ui| {
                ui.label(w::body_s(t, t!("Text channels").to_uppercase(), t.text_dim));
                ui.label(w::body_s(t, t!("Setting override:").to_uppercase(), t.text_dim));
                ui.end_row();
                let options = notify_options(true);
                for r in texts {
                    ui.allocate_ui_with_layout(vec2(col, t.dense(34.0)), Layout::left_to_right(Align::Center), |ui| {
                        ui.set_min_width(col);
                        ui.add(egui::Label::new(w::body_m(t, &r.name, t.text_dim)).truncate());
                    });
                    let mut level = a.room_notify(r.id);
                    let width = ui.available_width().max(t.dense(260.0));
                    if w::select(ui, t, &mut level, &options, width) {
                        Self::send(ctx, g.id, AdminOp::SetRoomNotify { room: r.id, level });
                    }
                    ui.end_row();
                }
            });
            ui.add_space(t.spacing.xl);
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.spacing_mut().interact_size.x = ui.available_width();
                if w::button(ui, t, None, t!("Close"), w::Kind::Primary, w::Size::Md).clicked() {
                    keep = false;
                }
            });
            keep
        });
        keep && !closed
    }

    #[allow(clippy::too_many_arguments)]
    fn invite_dialog(&mut self, ui: &Ui, t: &Theme, ctx: &mut Ctx, g: &ChatGroup, name: &str, search: &mut String, picked: &mut BTreeSet<SteamId>, expiry: &mut Expiry, link: &mut Option<String>) -> bool {
        let state = ctx.state;
        let a = admin_of(ctx, g.id);
        let can = a.is_none_or(|a| a.perms_of(state.steam_id).invite);
        let (keep, closed) = Self::modal(ui, t, "invite", 620.0, |ui| {
            let mut keep = !Self::title(ui, t, t!("Invite to group chat").as_ref());
            ui.horizontal(|ui| {
                let size = t.dense(28.0);
                let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
                super::paint_group_avatar(ui, t, g, name, r);
                ui.label(w::heading_s(t, name, t.accent));
            });
            ui.add_space(t.spacing.lg);
            if !can {
                ui.label(w::body_m(t, no_right(t!("Invite new members").as_ref()), t.text_dim));
                return keep;
            }
            ui.label(w::body_s(t, t!("Send a link").to_uppercase(), t.text_dim));
            ui.horizontal(|ui| {
                let mut shown = link.clone().unwrap_or_else(|| t!("Making a link…").to_string());
                let width = ui.available_width() - t.dense(120.0) - t.spacing.sm;
                ui.add_enabled_ui(false, |ui| w::text_input(ui, t, &mut shown, "", width, 1));
                let ok = link.is_some();
                if ui.add_enabled_ui(ok, |ui| w::button(ui, t, Some(icons::COPY), t!("Copy"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                    ui.ctx().copy_text(link.clone().unwrap_or_default());
                    ctx.toast(t!("Link copied"));
                }
            });
            ui.add_space(t.spacing.sm);
            ui.horizontal(|ui| {
                ui.label(w::body_s(t, t!("Expire this link in:").to_uppercase(), t.text_dim));
                let opts = [(Expiry::Hour, t!("One hour").to_string()), (Expiry::Day, t!("One day").to_string()), (Expiry::Never, t!("Never").to_string())];
                if w::select(ui, t, expiry, &opts, t.dense(180.0)) {
                    *link = None;
                    Self::send(ctx, g.id, AdminOp::CreateInviteLink { seconds: expiry.seconds(), room: 0 });
                }
            });
            ui.add_space(t.spacing.md);
            ui.separator();
            ui.label(w::body_s(t, t!("Or select from your friends list").to_uppercase(), t.text_dim));
            ui.add_space(t.spacing.xs);
            let width = ui.available_width();
            w::search(ui, t, search, t!("Start typing a name…").as_ref(), width);
            ui.add_space(t.spacing.sm);
            pick_friends(ui, t, state, g.id, search, picked, t.dense(280.0));
            ui.add_space(t.spacing.md);
            ui.columns(2, |cols| {
                let n = picked.len();
                let label = if n > 1 { t!("Invite ({n})", n = n).to_string() } else { t!("Invite").to_string() };
                cols[0].with_layout(Layout::top_down_justified(Align::Center), |ui| {
                    if ui.add_enabled_ui(n > 0, |ui| w::button(ui, t, Some(icons::USER_PLUS), &label, w::Kind::Primary, w::Size::Md)).inner.clicked() {
                        for id in std::mem::take(picked) {
                            Self::send(ctx, g.id, AdminOp::InviteFriend { steam_id: id, room: 0 });
                        }
                    }
                });
                cols[1].with_layout(Layout::top_down_justified(Align::Center), |ui| {
                    if w::button(ui, t, None, t!("Close"), w::Kind::Secondary, w::Size::Md).clicked() {
                        keep = false;
                    }
                });
            });
            keep
        });
        keep && !closed
    }

    fn settings_dialog(&mut self, ui: &Ui, t: &Theme, ctx: &mut Ctx, g: &ChatGroup, name: &str, s: &mut Settings) -> bool {
        let a = admin_of(ctx, g.id).cloned();
        let me = ctx.state.steam_id;
        let height = t.dense(560.0);
        let (keep, closed) = Self::modal(ui, t, "settings", 900.0, |ui| {
            let mut keep = true;
            ui.horizontal_top(|ui| {
                // Left column: the pages, then Leave at the bottom.
                ui.allocate_ui_with_layout(vec2(t.dense(210.0), height), Layout::top_down(Align::Min), |ui| {
                    w::component_frame(t, &t.components.settings_nav).show(ui, |ui| {
                        ui.set_min_size(vec2(ui.available_width(), height - t.spacing.xl));
                        ui.label(w::body_s(t, t!("Group settings").to_uppercase(), t.accent));
                        ui.add_space(t.spacing.sm);
                        for (p, label) in [
                            (Page::General, t!("General")),
                            (Page::Channels, t!("Channels")),
                            (Page::Permissions, t!("Permissions")),
                            (Page::Links, t!("Invite links")),
                            (Page::Invited, t!("Invited accounts")),
                            (Page::Bans, t!("Bans")),
                        ] {
                            if w::list_row(ui, t, &t.components.settings_nav_item, None, &label, "", None, s.page == p).clicked() {
                                s.page = p;
                                s.role = None;
                            }
                        }
                        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                            if a.as_ref().is_some_and(|a| a.owner == me) && w::menu_item_tinted(ui, t, icons::TRASH, t!("Delete group chat…").as_ref(), Some(t.danger)).clicked() {
                                self.confirm = Some((g.id, Confirm::Delete));
                            }
                            if w::menu_item_tinted(ui, t, icons::SIGN_OUT, t!("Leave group chat").as_ref(), Some(t.danger)).clicked() {
                                self.confirm = Some((g.id, Confirm::Leave));
                            }
                        });
                    });
                });
                ui.add_space(t.spacing.lg);
                ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::top_down(Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        let title = match s.page {
                            Page::General => t!("General"),
                            Page::Channels => t!("Channels"),
                            Page::Permissions => t!("Permissions"),
                            Page::Links => t!("Invite links"),
                            Page::Invited => t!("Invited accounts"),
                            Page::Bans => t!("Bans"),
                        };
                        ui.label(w::heading_l(t, title.to_uppercase(), t.text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked() {
                                keep = false;
                            }
                        });
                    });
                    ui.add_space(t.spacing.md);
                    let Some(a) = &a else {
                        match ctx.state.chat_admin.get(&g.id) {
                            Some(Err(e)) => {
                                ui.label(w::body_m(t, t!("Couldn't load this group chat's settings: {error}", error = i18n::tr_dyn(e)), t.danger));
                            }
                            _ => w::loading(ui, t, t!("Loading…")),
                        }
                        return;
                    };
                    egui::ScrollArea::vertical().id_salt(("group-settings", g.id, s.page)).auto_shrink([false, false]).show(ui, |ui| match s.page {
                        Page::General => self.general(ui, t, ctx, g, name, a, s),
                        Page::Channels => self.channels(ui, t, ctx, g, a, s),
                        Page::Permissions => self.permissions(ui, t, ctx, a, s),
                        Page::Links => self.links(ui, t, ctx, g, a, s),
                        Page::Invited => self.invited(ui, t, ctx, a),
                        Page::Bans => self.bans(ui, t, ctx, a),
                    });
                });
            });
            keep
        });
        keep && !closed
    }

    #[allow(clippy::too_many_arguments)]
    fn general(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, g: &ChatGroup, name: &str, a: &GroupAdmin, s: &mut Settings) {
        let me = ctx.state.steam_id;
        let can = a.perms_of(me).change_details;
        let why = no_right(t!("Change group name / tagline / avatar").as_ref());
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(w::body_s(t, t!("Icon").to_uppercase(), t.text_dim));
                let size = t.dense(180.0);
                let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
                super::paint_group_avatar(ui, t, g, name, r);
                ui.add_space(t.spacing.sm);
                let b = ui.add_enabled_ui(can, |ui| w::button(ui, t, Some(icons::IMAGE), t!("Change icon"), w::Kind::Secondary, w::Size::Md)).inner;
                if allowed(b, can, &why).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter(&*t!("Pictures"), &["png", "jpg", "jpeg", "gif"]).pick_file() {
                        Self::send(ctx, g.id, AdminOp::SetAvatar { path });
                        ctx.toast(t!("Uploading the new icon…"));
                    }
                }
            });
            ui.add_space(t.spacing.xl);
            ui.vertical(|ui| {
                let width = ui.available_width();
                ui.label(w::body_s(t, t!("Group chat name").to_uppercase(), t.text_dim));
                let draft = s.name.get_or_insert_with(|| g.name.clone());
                let r = ui.add_enabled_ui(can, |ui| w::text_input(ui, t, draft, name, width, 1)).inner;
                allowed(r, can, &why);
                ui.add_space(t.spacing.md);
                ui.label(w::body_s(t, t!("Tagline").to_uppercase(), t.text_dim));
                let draft = s.tagline.get_or_insert_with(|| g.tagline.clone());
                let r = ui.add_enabled_ui(can, |ui| w::text_input(ui, t, draft, "", width, 1)).inner;
                allowed(r, can, &why);
                let changed = s.name.as_deref().is_some_and(|n| n.trim() != g.name && !n.trim().is_empty()) || s.tagline.as_deref().is_some_and(|x| x.trim() != g.tagline);
                if can && changed {
                    ui.add_space(t.spacing.sm);
                    ui.horizontal(|ui| {
                        if w::button(ui, t, Some(icons::CHECK), t!("Save"), w::Kind::Primary, w::Size::Md).clicked() {
                            if let Some(n) = s.name.take().filter(|n| n.trim() != g.name && !n.trim().is_empty()) {
                                Self::send(ctx, g.id, AdminOp::Rename { name: n.trim().to_string() });
                            }
                            if let Some(x) = s.tagline.take().filter(|x| x.trim() != g.tagline) {
                                Self::send(ctx, g.id, AdminOp::SetTagline { tagline: x.trim().to_string() });
                            }
                        }
                        if w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked() {
                            s.name = None;
                            s.tagline = None;
                        }
                    });
                }
                ui.add_space(t.spacing.lg);
                ui.label(w::body_s(t, t!("Group chat owner").to_uppercase(), t.text_dim));
                person_row(ui, t, ctx, a.owner, true);
            });
        });
    }

    fn channels(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, g: &ChatGroup, a: &GroupAdmin, s: &mut Settings) {
        let can = a.perms_of(ctx.state.steam_id).channels;
        let why = no_right(t!("Create and delete channels").as_ref());
        ui.horizontal(|ui| {
            let width = (ui.available_width() - t.spacing.md) / 2.0;
            let r = ui.add_enabled_ui(can, |ui| w::text_input(ui, t, &mut s.new_text, t!("New channel name").as_ref(), width - t.dense(110.0), 1)).inner;
            allowed(r, can, &why);
            let ok = can && !s.new_text.trim().is_empty();
            let b = ui.add_enabled_ui(ok, |ui| w::button(ui, t, Some(icons::PLUS), t!("Create"), w::Kind::Secondary, w::Size::Md)).inner;
            if allowed(b, can, &why).clicked() {
                Self::send(ctx, g.id, AdminOp::CreateRoom { name: s.new_text.trim().to_string(), voice: false });
                s.new_text.clear();
            }
            let b = ui.add_enabled_ui(ok, |ui| w::button(ui, t, Some(icons::MICROPHONE), t!("Create new voice channel"), w::Kind::Secondary, w::Size::Md)).inner;
            if allowed(b, can, &why).clicked() {
                Self::send(ctx, g.id, AdminOp::CreateRoom { name: s.new_text.trim().to_string(), voice: true });
                s.new_text.clear();
            }
        });
        ui.add_space(t.spacing.lg);
        let rooms = |voice: bool| -> Vec<&model::ChatRoom> { g.rooms.iter().filter(|r| r.voice == voice).collect() };
        for (voice, heading) in [(false, t!("Text channels")), (true, t!("Voice channels"))] {
            let list = rooms(voice);
            if voice && list.is_empty() {
                continue;
            }
            ui.label(w::body_s(t, heading.to_uppercase(), t.text_dim));
            ui.separator();
            for (i, r) in list.iter().enumerate() {
                let home = r.id == g.default_room;
                w::component_frame(t, &t.components.settings_group).inner_margin(Margin::symmetric(t.spacing.md as i8, t.spacing.xs as i8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(w::body_m(t, if voice { icons::MICROPHONE } else { icons::LIST }, t.text_dim));
                        if let Some((id, draft)) = &mut s.renaming {
                            if *id == r.id {
                                w::text_input(ui, t, draft, &r.name, t.dense(240.0), 1);
                                if w::button(ui, t, Some(icons::CHECK), t!("Save"), w::Kind::Primary, w::Size::Sm).clicked() && !draft.trim().is_empty() {
                                    Self::send(ctx, g.id, AdminOp::RenameRoom { room: r.id, name: draft.trim().to_string() });
                                    s.renaming = None;
                                } else if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                    s.renaming = None;
                                }
                                return;
                            }
                        }
                        let label = if home { w::styled(&r.name, ui::theme::TextStyle { weight: ui::theme::Weight::Semibold, ..t.typography.body_m }, t.text).italics() } else { w::body_m(t, &r.name, t.text) };
                        ui.label(label);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if home {
                                ui.label(w::body_s(t, t!("Home channel"), t.text_dim)).on_hover_text(t!("The channel the group opens on; it can't be moved or deleted."));
                                return;
                            }
                            let up = i > 0 && !(i == 1 && list[0].id == g.default_room);
                            let down = i + 1 < list.len();
                            if ui.add_enabled_ui(can && down, |ui| w::icon_button(ui, t, icons::CARET_DOWN, t!("Move down"), false, 0)).inner.clicked() {
                                Self::send(ctx, g.id, AdminOp::MoveRoom { room: r.id, after: list[i + 1].id });
                            }
                            if ui.add_enabled_ui(can && up, |ui| w::icon_button(ui, t, icons::CARET_UP, t!("Move up"), false, 0)).inner.clicked() {
                                let after = if i >= 2 { list[i - 2].id } else { 0 };
                                Self::send(ctx, g.id, AdminOp::MoveRoom { room: r.id, after });
                            }
                            let b = ui.add_enabled_ui(can, |ui| w::button(ui, t, None, t!("Delete"), w::Kind::Ghost, w::Size::Sm)).inner;
                            if allowed(b, can, &why).clicked() {
                                self.confirm = Some((g.id, Confirm::DeleteRoom(r.id, r.name.clone())));
                            }
                            let b = ui.add_enabled_ui(can, |ui| w::button(ui, t, None, t!("Rename"), w::Kind::Ghost, w::Size::Sm)).inner;
                            if allowed(b, can, &why).clicked() {
                                s.renaming = Some((r.id, r.name.clone()));
                            }
                        });
                    });
                });
                ui.add_space(t.spacing.xs);
            }
            ui.add_space(t.spacing.lg);
        }
    }

    fn permissions(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, a: &GroupAdmin, s: &mut Settings) {
        let me = ctx.state.steam_id;
        let mine = a.perms_of(me);
        if let Some(id) = s.role {
            let Some(role) = a.role(id).cloned() else {
                s.role = None;
                return;
            };
            if w::button(ui, t, Some(icons::CARET_LEFT), t!("Back to permissions"), w::Kind::Ghost, w::Size::Sm).clicked() {
                s.role = None;
                s.role_name = None;
            }
            let default = role.id == a.default_role;
            let can = mine.edit_roles;
            let why = no_right(t!("Edit roles").as_ref());
            ui.label(w::body_s(t, t!("Role name").to_uppercase(), t.text_dim));
            ui.horizontal(|ui| {
                if default || !can {
                    ui.label(w::heading_m(t, &role.name, t.text));
                } else {
                    let draft = s.role_name.get_or_insert_with(|| role.name.clone());
                    w::text_input(ui, t, draft, &role.name, t.dense(300.0), 1);
                    if draft.trim() != role.name && !draft.trim().is_empty() && w::button(ui, t, Some(icons::CHECK), t!("Save"), w::Kind::Primary, w::Size::Md).clicked() {
                        Self::send(ctx, a.group, AdminOp::RenameRole { role: role.id, name: draft.trim().to_string() });
                        s.role_name = None;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::button(ui, t, Some(icons::TRASH), t!("Delete role"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            self.confirm = Some((a.group, Confirm::DeleteRole(role.id, role.name.clone())));
                        }
                    });
                }
            });
            if !can {
                ui.label(w::body_s(t, why.clone(), t.text_faint));
            }
            let mut p = role.perms;
            let before = p;
            let sections: [(String, Vec<(&mut bool, String, bool)>); 3] = {
                let RolePerms { chat, mention_all, broadcasts, kick, ban, invite, edit_roles, assign_roles, change_details, channels, .. } = &mut p;
                [
                    (
                        t!("General settings").to_string(),
                        vec![(chat, t!("Send messages").to_string(), true), (mention_all, t!("Mention all members with @all and @online").to_string(), true), (broadcasts, t!("Link and unlink broadcasts").to_string(), true)],
                    ),
                    (
                        t!("Members & roles").to_string(),
                        vec![
                            (kick, t!("Kick members").to_string(), true),
                            (ban, t!("Ban members").to_string(), true),
                            (invite, t!("Invite new members").to_string(), true),
                            // Steam keeps these off for everyone.
                            (edit_roles, t!("Edit roles").to_string(), !default),
                            (assign_roles, t!("Assign roles").to_string(), !default),
                        ],
                    ),
                    (t!("Chat room management").to_string(), vec![(change_details, t!("Change group name / tagline / avatar").to_string(), true), (channels, t!("Create and delete channels").to_string(), true)]),
                ]
            };
            for (heading, rows) in sections {
                ui.add_space(t.spacing.lg);
                ui.label(w::body_s(t, heading.to_uppercase(), t.text_dim));
                w::titled_box(ui, t, &t.components.settings_group, "", "", |ui| {
                    for (i, (on, label, editable)) in rows.into_iter().enumerate() {
                        if i > 0 {
                            ui.add_space(t.spacing.xs);
                        }
                        let ok = can && editable;
                        let r = ui.add_enabled_ui(ok, |ui| w::toggle_row(ui, t, on, &label)).inner;
                        allowed(r, ok, &if editable { why.clone() } else { t!("Not available for the Everyone role").to_string() });
                    }
                });
            }
            if p != before {
                Self::send(ctx, a.group, AdminOp::SetRolePerms { role: role.id, perms: p });
            }
            return;
        }
        w::component_frame(t, &t.components.settings_group).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut on = a.dms_allowed;
            if w::toggle_row(ui, t, &mut on, t!("Allow group members to send you direct messages").as_ref()).changed() {
                Self::send(ctx, a.group, AdminOp::SetDmsAllowed(on));
            }
        });
        ui.add_space(t.spacing.sm);
        ui.add(egui::Label::new(w::body_s(t, t!("To give someone roles, right-click them in the group's member list and pick the roles under Manage. A member can have more than one role."), t.text_dim)).wrap());
        ui.add_space(t.spacing.lg);
        ui.horizontal(|ui| {
            ui.label(w::body_s(t, t!("Roles").to_uppercase(), t.text_dim));
            if a.owner == me {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(w::body_s(t, format!("{} {}", icons::CROWN, t!("You are the owner of this group chat").to_uppercase()), t.text_dim));
                });
            }
        });
        ui.add_space(t.spacing.xs);
        let c = &t.components.settings_nav_item;
        ui.add_enabled_ui(false, |ui| w::list_row(ui, t, c, Some(icons::CROWN), t!("Owner").as_ref(), t!("Owner has all permissions").to_uppercase().as_str(), None, false));
        for role in a.sorted_roles() {
            let action = if mine.edit_roles { t!("Edit") } else { t!("View") };
            if w::list_row(ui, t, c, None, &role.name, action.to_uppercase().as_str(), Some(t.accent), false).clicked() {
                s.role = Some(role.id);
                s.role_name = None;
            }
        }
        ui.add_space(t.spacing.md);
        let why = no_right(t!("Edit roles").as_ref());
        ui.horizontal(|ui| {
            if let Some(draft) = &mut s.new_role {
                w::text_input(ui, t, draft, t!("New role name").as_ref(), t.dense(260.0), 1);
                if w::button(ui, t, Some(icons::CHECK), t!("Create"), w::Kind::Primary, w::Size::Md).clicked() && !draft.trim().is_empty() {
                    Self::send(ctx, a.group, AdminOp::CreateRole { name: draft.trim().to_string() });
                    s.new_role = None;
                } else if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                    s.new_role = None;
                }
            } else {
                let b = ui.add_enabled_ui(mine.edit_roles, |ui| w::button(ui, t, Some(icons::PLUS), t!("Add new role"), w::Kind::Primary, w::Size::Md)).inner;
                if allowed(b, mine.edit_roles, &why).clicked() {
                    s.new_role = Some(String::new());
                }
            }
        });
    }

    fn links(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, g: &ChatGroup, a: &GroupAdmin, s: &mut Settings) {
        let me = ctx.state.steam_id;
        let can = a.perms_of(me).invite;
        let why = no_right(t!("Invite new members").as_ref());
        ui.horizontal(|ui| {
            let opts = [(Expiry::Hour, t!("One hour").to_string()), (Expiry::Day, t!("One day").to_string()), (Expiry::Never, t!("Never").to_string())];
            ui.label(w::body_s(t, t!("Expire this link in:").to_uppercase(), t.text_dim));
            ui.add_enabled_ui(can, |ui| w::select(ui, t, &mut s.link_expiry, &opts, t.dense(160.0)));
            let b = ui.add_enabled_ui(can, |ui| w::button(ui, t, Some(icons::LINK), t!("Create link"), w::Kind::Primary, w::Size::Md)).inner;
            if allowed(b, can, &why).clicked() {
                Self::send(ctx, g.id, AdminOp::CreateInviteLink { seconds: s.link_expiry.seconds(), room: 0 });
            }
        });
        ui.add_space(t.spacing.md);
        if a.links.is_empty() {
            ui.label(w::body_m(t, t!("There are no invite links"), t.text_dim));
            return;
        }
        for l in &a.links {
            w::component_frame(t, &t.components.settings_group).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(w::body_m(t, l.url(), t.text));
                        let room = g.rooms.iter().find(|r| r.id == l.room).map(|r| format!(" · #{}", r.name)).unwrap_or_default();
                        let exp = if l.expires == 0 { t!("Never expires").to_string() } else { t!("Expires {time}", time = when(l.expires)).to_string() };
                        ui.label(w::body_s(t, format!("{} · {exp}{room}", t!("Made by {name}", name = name_of(ctx.state, l.creator))), t.text_dim));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let ok = can || l.creator == me;
                        let b = ui.add_enabled_ui(ok, |ui| w::button(ui, t, Some(icons::TRASH), t!("Revoke"), w::Kind::Ghost, w::Size::Sm)).inner;
                        if allowed(b, ok, &why).clicked() {
                            Self::send(ctx, g.id, AdminOp::DeleteInviteLink { code: l.code.clone() });
                        }
                        if w::button(ui, t, Some(icons::COPY), t!("Copy"), w::Kind::Secondary, w::Size::Sm).clicked() {
                            ui.ctx().copy_text(l.url());
                            ctx.toast(t!("Link copied"));
                        }
                    });
                });
            });
            ui.add_space(t.spacing.xs);
        }
    }

    fn invited(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, a: &GroupAdmin) {
        let can = a.perms_of(ctx.state.steam_id).invite;
        if a.invited.is_empty() {
            ui.label(w::body_m(t, t!("No one has been invited"), t.text_dim));
            return;
        }
        for i in &a.invited {
            let by = if i.by == 0 { String::new() } else { t!("Invited by {name}", name = name_of(ctx.state, i.by)).to_string() };
            if listed_row(ui, t, ctx, i.steam_id, &format!("{by}  {}", when(i.time)), t!("Revoke").as_ref(), can, &no_right(t!("Invite new members").as_ref())) {
                Self::send(ctx, a.group, AdminOp::RevokeInvite { steam_id: i.steam_id });
            }
        }
    }

    fn bans(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, a: &GroupAdmin) {
        let can = a.perms_of(ctx.state.steam_id).ban;
        if a.bans.is_empty() {
            ui.label(w::body_m(t, t!("No one has been banned"), t.text_dim));
            return;
        }
        for b in &a.bans {
            let by = if b.by == 0 { String::new() } else { t!("Banned by {name}", name = name_of(ctx.state, b.by)).to_string() };
            if listed_row(ui, t, ctx, b.steam_id, &format!("{by}  {}", when(b.time)), t!("Unban").as_ref(), can, &no_right(t!("Ban members").as_ref())) {
                Self::send(ctx, a.group, AdminOp::Ban { steam_id: b.steam_id, ban: false });
            }
        }
    }

    /// "Are you sure?" for leaving, deleting, kicking, banning and deleting
    /// channels and roles.
    fn confirm_dialog(&mut self, ui: &Ui, t: &Theme, ctx: &mut Ctx, convs: &mut Conversations, g: &ChatGroup, name: &str) {
        let Some((group, c)) = self.confirm.clone() else { return };
        if group != g.id {
            return;
        }
        let state = ctx.state;
        let (title, body, button) = match &c {
            Confirm::Leave => (t!("Leave group chat"), t!("Leave {name}? You'll need a new invite to come back.", name = name), t!("Leave")),
            Confirm::Delete => (
                t!("Delete group chat"),
                t!("Delete {name}? Everyone is removed, its invite links stop working and its messages are gone. This can't be undone.", name = name),
                t!("Delete"),
            ),
            Confirm::Kick(id) => (t!("Kick"), t!("Remove {name} from the group chat? They can come back with a new invite.", name = name_of(state, *id)), t!("Kick")),
            Confirm::Ban(id) => (t!("Ban"), t!("Ban {name}? They're removed and can't come back until unbanned.", name = name_of(state, *id)), t!("Ban")),
            Confirm::DeleteRoom(_, room) => (t!("Delete channel"), t!("Delete the channel {name} and its messages?", name = room), t!("Delete")),
            Confirm::DeleteRole(_, role) => (t!("Delete role"), t!("Delete the role {name}? Members who have it lose what it allows.", name = role), t!("Delete")),
        };
        let mut answer = None;
        let (_, closed) = Self::modal(ui, t, "confirm", 440.0, |ui| {
            ui.label(w::heading_s(t, &*title, t.text));
            ui.add_space(t.spacing.sm);
            ui.add(egui::Label::new(w::body_m(t, &*body, t.text_dim)).wrap());
            ui.add_space(t.spacing.lg);
            ui.horizontal(|ui| {
                if w::button(ui, t, None, button.as_ref(), w::Kind::Danger, w::Size::Md).clicked() {
                    answer = Some(true);
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked() {
                    answer = Some(false);
                }
            });
        });
        if closed {
            answer = Some(false);
        }
        let Some(yes) = answer else { return };
        self.confirm = None;
        if !yes {
            return;
        }
        let op = match c {
            Confirm::Leave => AdminOp::Leave,
            Confirm::Delete => AdminOp::Delete,
            Confirm::Kick(id) => AdminOp::Kick { steam_id: id },
            Confirm::Ban(id) => AdminOp::Ban { steam_id: id, ban: true },
            Confirm::DeleteRoom(room, _) => AdminOp::DeleteRoom { room },
            Confirm::DeleteRole(role, _) => AdminOp::DeleteRole { role },
        };
        if matches!(op, AdminOp::Leave | AdminOp::Delete) {
            self.dialog = None;
            convs.close(Pane::Group(group, 0));
        }
        Self::send(ctx, group, op);
    }

    /// The page for a new group chat: a name and friends to invite.
    pub fn new_group(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let x = t.spacing.xl as i8;
        egui::Frame::new().inner_margin(Margin { left: x, right: x, top: t.spacing.sm as i8, bottom: t.spacing.lg as i8 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::heading_m(t, t!("Create a group chat"), t.text));
            ui.add_space(t.spacing.md);
            ui.label(w::body_s(t, t!("Group chat name").to_uppercase(), t.text_dim));
            let width = ui.available_width();
            w::text_input(ui, t, &mut self.new_name, t!("Name your group chat").as_ref(), width, 1);
            ui.add_space(t.spacing.md);
            ui.label(w::body_s(t, t!("Invite friends").to_uppercase(), t.text_dim));
            w::search(ui, t, &mut self.new_search, t!("Start typing a name…").as_ref(), width);
            ui.add_space(t.spacing.sm);
            let h = (ui.available_height() - t.dense(70.0)).max(t.dense(120.0));
            pick_friends(ui, t, ctx.state, 0, &self.new_search, &mut self.new_picked, h);
            ui.add_space(t.spacing.md);
            let ok = !self.new_name.trim().is_empty() && !self.creating;
            let label = if self.creating { t!("Creating…") } else { t!("Create group chat") };
            if ui.add_enabled_ui(ok, |ui| w::button(ui, t, Some(icons::PLUS), &label, w::Kind::Primary, w::Size::Md)).inner.clicked() {
                self.creating = true;
                Self::send(ctx, 0, AdminOp::CreateGroup { name: self.new_name.trim().to_string(), invite: self.new_picked.iter().copied().collect() });
            }
        });
    }
}

/// Friends you can pick (not already in `group`), filtered by `search`.
fn pick_friends(ui: &mut Ui, t: &Theme, state: &client::State, group: u64, search: &str, picked: &mut BTreeSet<SteamId>, height: f32) {
    let members = state.chat_group_members.get(&group);
    let q = search.trim().to_lowercase();
    let mut friends: Vec<model::Persona> = state
        .friend_list()
        .into_iter()
        .filter(|p| members.is_none_or(|m| !m.contains(&p.steam_id)))
        .filter(|p| q.is_empty() || p.display_name().to_lowercase().contains(&q) || p.name.to_lowercase().contains(&q))
        .collect();
    friends.sort_by_key(|p| (!p.in_game(), p.state == PersonaState::Offline, p.name.to_lowercase()));
    egui::ScrollArea::vertical().id_salt(("pick-friends", group)).max_height(height).min_scrolled_height(height.min(t.dense(200.0))).auto_shrink([false, true]).show(ui, |ui| {
        if friends.is_empty() {
            ui.label(w::body_m(t, if q.is_empty() { t!("All your friends are already here") } else { t!("No friends match") }, t.text_dim));
        }
        for p in &friends {
            let on = picked.contains(&p.steam_id);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), t.dense(44.0)), Sense::click());
            let c = &t.components.friend_row;
            w::paint_box(ui, t, rect, c, &t.look(c, w::interact(ui, &resp, on)), if on { 1.0 } else { 0.0 });
            let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(t.spacing.sm, 4.0))).layout(Layout::left_to_right(Align::Center)));
            let color = crate::shell::status_color(t, p);
            w::presence_avatar(&mut row, t, &art::avatar(p.avatar_hash.as_deref()), rect.height() - 10.0, Some(color));
            row.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(egui::Label::new(super::strong(t, p.display_name(), color)).truncate());
                ui.add(egui::Label::new(w::body_s(t, crate::shell::status_text(state, p), color)).truncate());
            });
            row.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if on {
                    ui.label(w::body_l(t, icons::CHECK, t.accent));
                }
            });
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if on {
                    picked.remove(&p.steam_id);
                } else {
                    picked.insert(p.steam_id);
                }
            }
        }
    });
}

/// Someone's avatar and name (a click opens their profile).
fn person_row(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, crown: bool) {
    let p = ctx.state.persona(id);
    ui.horizontal(|ui| {
        let color = crate::shell::status_color(t, &p);
        let r = w::presence_avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), t.dense(36.0), Some(color));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let name = format!("{}{}", if crown { format!("{} ", icons::CROWN) } else { String::new() }, name_of(ctx.state, id));
            ui.label(super::strong(t, name, color));
            ui.label(w::body_s(t, crate::shell::status_text(ctx.state, &p), color));
        });
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            ctx.navigate("profile", Some(id.to_string()));
        }
    });
}

/// A row of the invited and banned lists; true when its button was clicked.
#[allow(clippy::too_many_arguments)]
fn listed_row(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, note: &str, button: &str, can: bool, why: &str) -> bool {
    let mut clicked = false;
    w::component_frame(t, &t.components.settings_group).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            person_row(ui, t, ctx, id, false);
            ui.label(w::body_s(t, note, t.text_faint));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let b = ui.add_enabled_ui(can, |ui| w::button(ui, t, None, button, w::Kind::Secondary, w::Size::Sm)).inner;
                clicked = allowed(b, can, why).clicked();
            });
        });
    });
    ui.add_space(t.spacing.xs);
    clicked
}

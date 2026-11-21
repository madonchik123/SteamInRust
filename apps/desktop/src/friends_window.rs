//! The Friends & Chat window, like Steam's (Figma "friends" `447:3850`):
//! the one place for friends and chat. The friends list is docked on the
//! theme's friends side (`t.layout.friends_side`, the right by default);
//! opening a conversation widens the window toward the other side, with the
//! open conversations as tabs. It keeps working while the main window is
//! hidden in the tray.
//!
//! What opens beside the list (chats, group chats, adding friends) is drawn
//! by the `friends` extension (`builtin/friends.rs`).

use crate::app::SteamApp;
use crate::builtin::friends::{Friends, fit, group_name, paint_group_avatar, strong};
use crate::shell::{FriendDialog, game_page, invite_options, send_invite, status_color, status_text};
use client::Command;
use egui::{Align, Align2, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::{Action, Env};
use i18n::t;
use model::{Persona, PersonaState, SteamId, art};
use ui::theme::Side;
use ui::{Theme, icons, widgets as w};

/// What shows beside the friends list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Chat(SteamId),
    /// A group chat and its room (0 until the group's default room is known).
    Group(u64, u64),
    Add,
    Invites,
    Blocked,
    /// Making a new group chat.
    NewGroup,
}

impl Pane {
    /// The same conversation (a group in any of its rooms).
    pub fn same(self, other: Pane) -> bool {
        match (self, other) {
            (Pane::Group(a, _), Pane::Group(b, _)) => a == b,
            (a, b) => a == b,
        }
    }
    fn is_conversation(self) -> bool {
        matches!(self, Pane::Chat(_) | Pane::Group(..))
    }
}

/// The open conversations (the tabs) and the one showing.
#[derive(Default)]
pub struct Conversations {
    pub open: Option<Pane>,
    pub tabs: Vec<Pane>,
}

impl Conversations {
    /// Show a conversation (adding its tab) or the friends pages.
    pub fn show(&mut self, p: Pane) {
        if p.is_conversation() {
            match self.tabs.iter_mut().find(|x| x.same(p)) {
                Some(x) => *x = p,
                None => self.tabs.push(p),
            }
        }
        self.open = Some(p);
    }

    /// Close a tab (or the friends pages); the next tab shows, if any.
    pub fn close(&mut self, p: Pane) {
        let at = self.tabs.iter().position(|x| x.same(p));
        self.tabs.retain(|x| !x.same(p));
        if self.open.is_some_and(|o| o.same(p)) {
            self.open = at.and_then(|i| self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).copied());
        }
    }
}

/// The list's two tabs (Figma: "Friends", "Activity").
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ListTab {
    #[default]
    Friends,
    Activity,
}

/// Something a friend did while the app was running.
#[derive(Clone)]
struct Seen {
    time: u64,
    who: SteamId,
    what: Did,
}

#[derive(Clone)]
enum Did {
    CameOnline,
    WentOffline,
    Playing(String),
}

#[derive(Default)]
pub struct FriendsWindow {
    pub open: bool,
    pub convs: Conversations,
    tab: ListTab,
    groups_hidden: bool,
    groups_asked: bool,
    seen: Vec<Seen>,
    /// The conversation side was showing last frame (to widen the window
    /// toward it once).
    wide: bool,
    /// Bring the window to the front next frame.
    focus: bool,
    /// The "New group chat" dialog, while open.
    new_group: Option<NewGroup>,
}

/// A group chat being made: its name and who's invited.
#[derive(Default)]
struct NewGroup {
    name: String,
    invited: Vec<SteamId>,
    filter: String,
}

impl FriendsWindow {
    /// Open the window on a conversation or friends page.
    pub fn show(&mut self, p: Pane) {
        self.convs.show(p);
        self.raise();
    }

    /// Open the window (or bring it to the front).
    pub fn raise(&mut self) {
        self.open = true;
        self.focus = true;
    }

    /// Is this friend's chat on screen?
    pub fn showing_chat(&self, id: SteamId) -> bool {
        self.open && self.convs.open == Some(Pane::Chat(id))
    }

    /// Open what a link asks for: a SteamID (their chat), `group:<id>`,
    /// `groups`, `activity`, `add`, `invites`, `blocked`, or nothing (the list).
    pub fn route(&mut self, arg: Option<&str>) {
        match arg.unwrap_or_default() {
            "add" => self.show(Pane::Add),
            "invites" => self.show(Pane::Invites),
            "blocked" => self.show(Pane::Blocked),
            "newgroup" => self.show(Pane::NewGroup),
            "activity" => {
                self.tab = ListTab::Activity;
                self.raise();
            }
            "groups" => {
                self.tab = ListTab::Friends;
                self.groups_hidden = false;
                self.raise();
            }
            a => {
                if let Some(g) = a.strip_prefix("group:").and_then(|g| g.parse().ok()) {
                    self.show(Pane::Group(g, 0));
                } else if let Ok(id) = a.parse::<SteamId>() {
                    self.show(Pane::Chat(id));
                } else {
                    self.raise();
                }
            }
        }
    }

    /// Note what friends started doing (from a persona update, before it's
    /// applied) for the Activity tab.
    pub fn note_personas(&mut self, state: &client::State, list: &[Persona]) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        for p in list {
            if state.friends.get(&p.steam_id) != Some(&model::Relationship::Friend) {
                continue;
            }
            let Some(old) = state.personas.get(&p.steam_id) else { continue };
            let was_on = old.state != PersonaState::Offline;
            let is_on = p.state != PersonaState::Offline;
            let what = if p.in_game() && (!old.in_game() || old.game_app_id != p.game_app_id) {
                let game = p.game_app_id.and_then(|id| state.app_name(id).map(String::from)).or_else(|| p.game_name.clone()).unwrap_or_else(|| t!("a game").to_string());
                Some(Did::Playing(game))
            } else if is_on && !was_on {
                Some(Did::CameOnline)
            } else if was_on && !is_on {
                Some(Did::WentOffline)
            } else {
                None
            };
            if let Some(what) = what {
                self.seen.push(Seen { time: now, who: p.steam_id, what });
            }
        }
        let extra = self.seen.len().saturating_sub(200);
        self.seen.drain(..extra);
    }
}

/// `--friends-size=WxH` (previews): the window's size, e.g. its narrowest.
fn size_flag() -> Option<[f32; 2]> {
    let v = std::env::args().find_map(|a| a.strip_prefix("--friends-size=").map(String::from))?;
    let (w, h) = v.split_once('x')?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

/// How wide the conversation side opens.
const PANE_WIDTH: f32 = 560.0;

pub fn show(app: &mut SteamApp, ctx: &egui::Context, actions: &mut Vec<Action>) {
    pending(app);
    if !app.friends_window.open {
        return;
    }
    let t = app.prefs.theme.clone();
    let id = egui::ViewportId::from_hash_of("friends-window");
    let list_w = t.layout.friends_width;
    let pane_w = t.dense(PANE_WIDTH);
    let has_pane = app.friends_window.convs.open.is_some();
    let width = list_w + if has_pane { pane_w } else { 0.0 };
    // Our own title bar, like the main window (no Windows frame).
    let mut builder = egui::ViewportBuilder::default()
        .with_title(t!("Friends & Chat"))
        .with_inner_size(size_flag().unwrap_or([width, 720.0]))
        .with_min_inner_size([if has_pane { list_w + t.dense(360.0) } else { list_w }, 420.0])
        .with_decorations(crate::chrome::SYSTEM_FRAME)
        .with_icon(crate::window_icon::icon_data(app.prefs.tray_icon, 64));
    // A preview copy keeps it off the screen too, beside the main window.
    if crate::app::offscreen() {
        // Created hidden: Windows can put a new window at its default spot
        // for a moment before it moves; it's shown once it's off the screen.
        builder = builder.with_position([crate::app::OFFSCREEN_POS[0] + 4000.0, crate::app::OFFSCREEN_POS[1]]).with_visible(false);
    }
    if !app.can_open_window(ctx, id) {
        return;
    }
    if std::mem::take(&mut app.friends_window.focus) && !crate::app::offscreen() {
        ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Focus);
    }
    ctx.show_viewport_immediate(id, builder, |ui, _| {
        if crate::app::offscreen() {
            match ui.input(|i| i.viewport().outer_rect) {
                Some(r) if r.right() < -1000.0 => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true)),
                _ => {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(crate::app::OFFSCREEN_POS[0] + 4000.0, crate::app::OFFSCREEN_POS[1])));
                    ui.ctx().request_repaint();
                }
            }
        }
        if ui.input(|i| i.viewport().close_requested()) {
            app.friends_window.open = false;
        }
        let side = t.layout.friends_side;
        // With the list on the right, the window grows (and shrinks) on its
        // left, so the list stays where it was.
        if has_pane != app.friends_window.wide {
            app.friends_window.wide = has_pane;
            let (outer, inner) = ui.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
            if let (Side::Right, Some(outer), Some(inner)) = (side, outer, inner) {
                let dx = if has_pane { -pane_w } else { (inner.width() - list_w).max(0.0) };
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::OuterPosition(outer.min + vec2(dx, 0.0)));
            }
        }
        let panel = match side {
            Side::Left => egui::Panel::left("fw-list"),
            Side::Right => egui::Panel::right("fw-list"),
        };
        panel.exact_size(list_w).resizable(false).frame(egui::Frame::new()).show(ui, |ui| list(app, ui, &t, actions));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
            // Nothing on the chat side may draw over the friends list.
            let clip = ui.max_rect().intersect(ui.clip_rect());
            ui.set_clip_rect(clip);
            if has_pane {
                // The window buttons sit over the tabs when the conversation
                // side is the right one.
                let reserve = if side == Side::Left && !crate::chrome::SYSTEM_FRAME { 3.0 * 46.0 } else { 0.0 };
                pane(app, ui, actions, reserve);
            } else {
                // Widened by hand with nothing open.
                crate::chrome::drag_area(ui, ui.max_rect(), true);
                w::empty(ui, &t, icons::CHATS, t!("Pick a friend to chat with"), "");
            }
        });
        // The window buttons in the top-right corner.
        let full = ui.ctx().content_rect();
        let corner = Rect::from_min_max(pos2(full.right() - 3.0 * 46.0, full.top()), pos2(full.right(), full.top() + 30.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(corner).layout(Layout::right_to_left(Align::Min)));
        crate::chrome::window_buttons(&mut c, &t, 30.0, true);
        crate::chrome::resize_edges(ui);
        let ctx = ui.ctx().clone();
        crate::chrome::border(&ctx, &t);
        crate::shell::friend_dialogs(app, &ctx);
        new_group_dialog(app, &ctx, &t);
    });
}

/// "New group chat": a name (optional) and the friends to invite.
fn new_group_dialog(app: &mut SteamApp, ctx: &egui::Context, t: &Theme) {
    let Some(mut ng) = app.friends_window.new_group.take() else { return };
    let mut keep = true;
    let width = (ctx.content_rect().width() - 64.0).clamp(240.0, 400.0);
    egui::Modal::new(egui::Id::unique("new-group-chat")).show(ctx, |ui| {
        ui.set_width(width);
        ui.label(w::heading_s(t, t!("New group chat"), t.text));
        ui.add_space(t.spacing.sm);
        ui.add(egui::TextEdit::singleline(&mut ng.name).hint_text(t!("Name (optional)")).desired_width(f32::INFINITY));
        ui.add_space(t.spacing.sm);
        ui.label(w::body_s(t, t!("Invite friends"), t.text_dim));
        w::search(ui, t, &mut ng.filter, t!("Search friends"), width);
        let q = ng.filter.trim().to_lowercase();
        let mut friends = app.state.friend_list();
        friends.retain(|p| q.is_empty() || p.display_name().to_lowercase().contains(&q));
        friends.sort_by_key(|p| (p.state == PersonaState::Offline, p.display_name().to_lowercase()));
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for p in friends {
                let mut on = ng.invited.contains(&p.steam_id);
                if w::check(ui, t, &mut on, &p.display_name()).changed() {
                    if on {
                        ng.invited.push(p.steam_id);
                    } else {
                        ng.invited.retain(|x| *x != p.steam_id);
                    }
                }
            }
        });
        ui.add_space(t.spacing.md);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!ng.invited.is_empty(), |ui| {
                if w::button(ui, t, Some(icons::CHATS), t!("Make the group chat"), w::Kind::Primary, w::Size::Sm).clicked() {
                    app.client.send(Command::CreateChatGroup { name: ng.name.trim().to_string(), partner: None, invitees: ng.invited.clone() });
                    keep = false;
                }
            });
            if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                keep = false;
            }
        });
    });
    if keep {
        app.friends_window.new_group = Some(ng);
    }
}

/// `--friends=<what>` (previews): open it once what it names has arrived.
/// Several can be given, separated by commas (`chat,activity`).
fn pending(app: &mut SteamApp) {
    let Some(all) = app.pending_friends.clone() else { return };
    let (what, rest) = all.split_once(',').unwrap_or((&all, ""));
    match what {
        // Every conversation with messages, as tabs; the first one showing.
        "chat" => {
            let mut ids: Vec<SteamId> = app.state.chats.iter().filter(|(_, m)| !m.is_empty()).map(|(id, _)| *id).collect();
            if ids.is_empty() {
                return;
            }
            ids.sort_unstable_by(|a, b| b.cmp(a));
            for id in ids {
                app.friends_window.show(Pane::Chat(id));
            }
        }
        "group" => match &app.state.chat_groups {
            Some(Ok(l)) if !l.is_empty() => {
                let g = l[0].id;
                app.friends_window.route(Some(&format!("group:{g}")));
            }
            _ => return,
        },
        other => app.friends_window.route(Some(other)),
    }
    app.pending_friends = (!rest.is_empty()).then(|| rest.to_string());
}

/// The conversation side, drawn by the `friends` extension.
fn pane(app: &mut SteamApp, ui: &mut Ui, actions: &mut Vec<Action>, reserve: f32) {
    let SteamApp { registry, friends_window, state, local, prefs, client, .. } = app;
    let env = Env { state, local, theme: &prefs.theme, zoom: prefs.zoom, client, actions: &mut *actions };
    registry.with_ext::<Friends, _>("friends", env, |f, ctx| f.pane(ui, ctx, &mut friends_window.convs, reserve));
}

/// Friends calling you and the call you're in, drawn by the `friends` extension.
fn voice_dock(app: &mut SteamApp, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
    let SteamApp { registry, state, local, prefs, client, .. } = app;
    let env = Env { state, local, theme: &prefs.theme, zoom: prefs.zoom, client, actions: &mut *actions };
    registry.with_ext::<Friends, _>("friends", env, |f, ctx| f.voice_dock(ui, ctx, rect));
}

/// The list column: you, your status and "Add a friend"; the Friends and
/// Activity tabs; the friends grouped by what they're doing, and group chats.
fn list(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let panel = &t.components.friends_panel;
    let full = ui.max_rect();
    ui.set_clip_rect(full.intersect(ui.clip_rect()));
    w::paint_box(ui, t, full, panel, &t.look(panel, Default::default()), 0.0);
    // The top strip moves the window (the window buttons sit in it).
    let strip = Rect::from_min_size(full.min, vec2(full.width(), 30.0));
    crate::chrome::drag_area(ui, strip, true);
    let pad = t.padding_of(panel);
    // Calls sit under the list, so they stay in view whatever is open.
    let dock = crate::builtin::friends::dock_height(&app.state, t);
    if dock > 0.0 {
        voice_dock(app, ui, Rect::from_min_max(pos2(full.left(), full.bottom() - dock), full.max), actions);
    }
    let inner = Rect::from_min_max(pos2(full.left() + pad.x, strip.bottom() + (pad.y - 10.0).max(0.0)), full.max - pad - vec2(0.0, dock));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    let ui = &mut ui;
    header(app, ui, t);
    ui.add_space(t.spacing.xl + t.spacing.lg);

    // Friends / Activity, each half the width, joined to the box below.
    let sort = &t.components.sort;
    let tab_h = t.height_of(sort, 39.0);
    let half = (ui.available_width() / 2.0).floor();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (tab, label) in [(ListTab::Friends, t!("Friends")), (ListTab::Activity, t!("Activity"))] {
            let (r, resp) = ui.allocate_exact_size(vec2(half, tab_h), Sense::click());
            let on = app.friends_window.tab == tab;
            let look = t.look(sort, w::interact(ui, &resp, on));
            w::paint_box(ui, t, r, sort, &look, if on { 1.0 } else { 0.0 });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, label, t.font_of(sort), look.text);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.friends_window.tab = tab;
            }
        }
    });
    ui.spacing_mut().item_spacing.y = 0.0;
    let boxc = &t.components.friends_box;
    let rect = Rect::from_min_max(ui.cursor().min, pos2(ui.max_rect().right(), ui.max_rect().bottom()));
    if rect.height() < 40.0 {
        return;
    }
    w::paint_box(ui, t, rect, boxc, &t.look(boxc, Default::default()), 0.0);
    let bp = t.padding_of(boxc);
    let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(rect.min + vec2(bp.x, bp.y), rect.max - vec2(bp.x, 0.0))).layout(Layout::top_down(Align::Min)));
    match app.friends_window.tab {
        ListTab::Friends => friends(app, &mut inside, t, actions),
        ListTab::Activity => activity(app, &mut inside, t, actions),
    }
}

/// You: avatar, name, your status (a menu), and "Add a friend".
fn header(app: &mut SteamApp, ui: &mut Ui, t: &Theme) {
    let me = app.state.me().cloned().unwrap_or_default();
    let size = t.dense(46.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.sm;
        // The button first (right to left), so the name takes what's left.
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let invites = app.state.friends.values().filter(|r| **r == model::Relationship::RequestRecipient).count();
            let managing = matches!(app.friends_window.convs.open, Some(Pane::Add | Pane::Invites | Pane::Blocked));
            let r = w::icon_button(ui, t, icons::USER_PLUS, t!("Add a friend"), managing, invites);
            if r.clicked() {
                if managing {
                    app.friends_window.convs.open = None;
                } else {
                    app.friends_window.convs.show(if invites > 0 { Pane::Invites } else { Pane::Add });
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                let color = status_color(t, &me);
                let av = w::presence_avatar(ui, t, &art::avatar(me.avatar_hash.as_deref()), size, Some(color));
                if crate::builtin::friends::hover_ring(ui, t, av).on_hover_text(t!("View profile")).clicked() {
                    app.navigate("profile".into(), None);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = t.spacing.xs;
                    ui.add(egui::Label::new(w::heading_s(t, &app.state.persona_name, t.text)).truncate());
                    status_menu(app, ui, t, &me);
                });
            });
        });
    });
}

/// "● Online ⌄": your status, and a menu to change it.
fn status_menu(app: &mut SteamApp, ui: &mut Ui, t: &Theme, me: &Persona) {
    let color = status_color(t, me);
    let font = w::font(t.typography.body_m.size);
    let label = i18n::tr_dyn(me.state.label());
    let g = ui.painter().layout_no_wrap(label.to_string(), font.clone(), color);
    let dot = t.dense(11.0);
    let caret = t.dense(12.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(dot + t.spacing.sm + g.size().x + t.spacing.sm + caret, g.size().y.max(dot)), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let ghost = &t.components.button_ghost;
    let look = t.look(ghost, w::interact(ui, &resp, open));
    w::paint_box(ui, t, rect.expand2(vec2(t.spacing.xs + 2.0, 2.0)), ghost, &look, 0.0);
    crate::shell::status_mark(ui.painter(), pos2(rect.left() + dot / 2.0, rect.center().y), dot / 2.0, me.state, color);
    ui.painter().galley(pos2(rect.left() + dot + t.spacing.sm, rect.center().y - g.size().y / 2.0), g, color);
    ui.painter().text(pos2(rect.right() - caret / 2.0, rect.center().y), Align2::CENTER_CENTER, if open { icons::CARET_UP } else { icons::CARET_DOWN }, w::font(caret), t.text_dim);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Change your status"));
    egui::Popup::menu(&resp).width(t.dense(200.0)).show(|ui| {
        for s in PersonaState::SELECTABLE {
            let r = w::menu_item(ui, t, "", &i18n::tr_dyn(s.label()));
            let c = status_color(t, &Persona { state: s, ..Default::default() });
            let pad = t.padding_of(&t.components.menu_item).x;
            crate::shell::status_mark(ui.painter(), pos2(r.rect.left() + pad, r.rect.center().y), 5.0, s, c);
            if r.clicked() {
                app.client.send(Command::SetPersonaState(s));
            }
        }
    });
}

/// The Friends tab: search, then Favorites, In game, Online and Offline,
/// then group chats.
fn friends(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let width = ui.available_width();
    w::search(ui, t, &mut app.friend_filter, t!("Search friends"), width);
    ui.add_space(t.spacing.sm);
    crate::offline_ui::friends_notice(app, ui, t);
    let list = app.state.friend_list();
    let filter = app.friend_filter.trim().to_lowercase();
    let ok = |p: &&Persona| {
        filter.is_empty()
            || p.name.to_lowercase().contains(&filter)
            || p.nickname.as_ref().is_some_and(|n| n.to_lowercase().contains(&filter))
            || p.game_name.as_deref().unwrap_or_default().to_lowercase().contains(&filter)
            || p.game_app_id.and_then(|id| app.state.app_name(id)).is_some_and(|g| g.to_lowercase().contains(&filter))
    };
    let favs = app.prefs.favorite_friends.clone();
    let fav = |p: &&Persona| favs.contains(&p.steam_id);
    let favorites: Vec<Persona> = list.iter().filter(ok).filter(fav).cloned().collect();
    let in_game: Vec<Persona> = list.iter().filter(ok).filter(|p| !fav(p) && p.in_game()).cloned().collect();
    let online: Vec<Persona> = list.iter().filter(ok).filter(|p| !fav(p) && !p.in_game() && p.state != PersonaState::Offline).cloned().collect();
    let offline: Vec<Persona> = list.iter().filter(ok).filter(|p| !fav(p) && p.state == PersonaState::Offline).cloned().collect();
    let selected = match app.friends_window.convs.open {
        Some(Pane::Chat(id)) => Some(id),
        _ => None,
    };
    if !app.friends_window.groups_asked && app.state.chat_groups.is_none() && app.state.online() {
        app.friends_window.groups_asked = true;
        app.client.send(Command::LoadChatGroups);
    }
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("friend-list").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(t.spacing.sm);
        if list.is_empty() {
            ui.label(w::body_m(t, t!("Loading friends…"), t.text_faint));
        } else if favorites.len() + in_game.len() + online.len() + offline.len() == 0 {
            ui.label(w::body_m(t, t!("No friends match."), t.text_faint));
        }
        for (title, group) in [(t!("Favorites"), &favorites), (t!("In game"), &in_game), (t!("Online"), &online), (t!("Offline"), &offline)] {
            if group.is_empty() {
                continue;
            }
            ui.label(w::body_m(t, format!("{title} ({})", group.len()), t.text_dim));
            ui.add_space(t.spacing.sm + 2.0);
            for p in group.iter() {
                if friend_row(app, ui, t, p, actions, selected == Some(p.steam_id)) {
                    clicked = Some(p.steam_id);
                }
            }
            ui.add_space(t.spacing.lg + t.spacing.xs);
        }
        group_chats(app, ui, t, &filter);
        ui.add_space(t.spacing.lg);
    });
    if let Some(id) = clicked {
        app.friends_window.convs.show(Pane::Chat(id));
        actions.push(Action::MarkRead(id));
    }
}

/// One friend: avatar outlined in their status color, name, and what
/// they're doing (the game is a link to its page). Right-click for more.
/// Returns true when clicked.
fn friend_row(app: &mut SteamApp, ui: &mut Ui, t: &Theme, p: &Persona, actions: &mut Vec<Action>, selected: bool) -> bool {
    let c = &t.components.friend_row;
    let offline = p.state == PersonaState::Offline;
    let color = status_color(t, p);
    let name = p.display_name();
    let typing = app.state.is_typing(p.steam_id);
    let status = if typing { t!("typing…").to_string() } else { status_text(&app.state, p) };
    let unread = app.state.unread.get(&p.steam_id).copied().unwrap_or(0);

    let h = t.height_of(c, 66.0);
    let pad = t.padding_of(c);
    let (slot, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    // The fill reaches into the box's padding, so the content lines up with
    // the group titles.
    let rect = slot.expand2(vec2(pad.x, 0.0));
    let look = t.look(c, w::interact(ui, &resp, selected));
    // Keep lit while its menu is open.
    let look = if resp.context_menu_opened() { t.look(c, ui::theme::Interact { hover: 1.0, selected, ..Default::default() }) } else { look };
    w::paint_box(ui, t, rect, c, &look, if selected { 1.0 } else { 0.0 });
    let content = Rect::from_min_max(pos2(slot.left(), slot.top() + pad.y), pos2(slot.right(), slot.bottom() - pad.y));
    let av = Rect::from_min_size(content.min, vec2(content.height(), content.height()));
    w::paint_presence_avatar(ui, t, av, &art::avatar(p.avatar_hash.as_deref()), Some(color));

    let x = av.right() + t.spacing.sm + 1.0;
    let right = if unread > 0 { content.right() - t.dense(26.0) } else { content.right() };
    let name_font = t.font_of(c);
    let status_style = if h >= 56.0 { t.typography.body_m } else { t.typography.body_s };
    let status_font = egui::FontId::new(status_style.size, status_style.weight.family());
    let name_c = if offline { t.text_dim } else { color };
    let status_c = if offline { t.text_dim.gamma_multiply(0.7) } else if typing { t.text_dim } else { color };
    let ng = fit(ui, name, name_font, name_c, right - x);
    let sg = fit(ui, status, status_font, status_c, right - x);
    let gap = t.spacing.xs + 2.0;
    let top = content.center().y - (ng.size().y + gap + sg.size().y) / 2.0;
    ui.painter().galley(pos2(x, top), ng, name_c);
    let status_rect = Rect::from_min_size(pos2(x, top + ui.fonts_mut(|f| f.row_height(&t.font_of(c))) + gap), sg.size());
    ui.painter().galley(status_rect.min, sg, status_c);
    // The game is a link of its own: underlined while the pointer is on it,
    // so it's clear a click opens the game, not the chat.
    if let Some(game) = p.game_app_id.filter(|_| p.in_game() && !typing) {
        let r = ui.interact(status_rect, resp.id.with("game"), Sense::click());
        if r.hovered() {
            ui.painter().hline(status_rect.x_range(), status_rect.bottom(), Stroke::new(1.0, color));
        }
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Open the game's page")).clicked() {
            actions.push(game_page(&app.state, game));
        }
    }
    if unread > 0 {
        w::badge(ui.painter(), t, pos2(content.right() - t.dense(10.0), content.center().y), unread);
    }
    friend_menu(app, &resp, t, p, actions);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A friend's right-click menu: message, join their game, invite them to
/// yours, profile, inventory, nickname, favorites, remove and block.
fn friend_menu(app: &mut SteamApp, resp: &egui::Response, t: &Theme, persona: &Persona, actions: &mut Vec<Action>) {
    let name = persona.display_name();
    let vp = resp.ctx.viewport_id();
    resp.context_menu(|ui| {
        ui.label(strong(t, &name, t.text));
        if persona.nickname.is_some() && !persona.name.is_empty() {
            ui.label(w::body_s(t, &persona.name, t.text_dim));
        }
        ui.separator();
        if w::menu_item(ui, t, icons::CHAT, t!("Send a message")).clicked() {
            app.friends_window.convs.show(Pane::Chat(persona.steam_id));
            actions.push(Action::MarkRead(persona.steam_id));
            ui.close();
        }
        // In a game: join them, see their server, watch their broadcast, like Steam.
        if let Some(game) = persona.game_app_id.filter(|_| persona.in_game()) {
            let game_name = app.state.app_name(game).map(String::from).unwrap_or_else(|| t!("the game").to_string());
            ui.separator();
            if let Some(args) = persona.join_args() {
                if w::menu_item(ui, t, icons::PLAY, t!("Join game")).on_hover_text(&args).clicked() {
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
                if w::menu_item(ui, t, icons::INFO, t!("View server info")).clicked() {
                    app.client.send(Command::QueryServer { addr: addr.clone() });
                    app.friend_dialog = Some((vp, FriendDialog::Server { name: persona.display_name(), game, addr }));
                    ui.close();
                }
            }
            if persona.broadcasting && w::menu_item(ui, t, icons::EYE, t!("Watch game")).on_hover_text(t!("Opens their Steam broadcast")).clicked() {
                crate::links::open_http(&format!("https://steamcommunity.com/broadcast/watch/{}", persona.steam_id));
                ui.close();
            }
            if w::menu_item(ui, t, icons::CONTROLLER, t!("View game info")).clicked() {
                actions.push(game_page(&app.state, game));
                ui.close();
            }
        }
        // While you're in a game: invite them to it, its lobby or its match.
        for (invite, label) in invite_options(&app.state) {
            if w::menu_item(ui, t, icons::USER_PLUS, &label).clicked() {
                send_invite(app, persona, invite, actions);
                ui.close();
            }
        }
        ui.separator();
        if w::menu_item(ui, t, icons::USER_CIRCLE, t!("View profile")).clicked() {
            actions.push(Action::Navigate { page: "profile".into(), arg: Some(persona.steam_id.to_string()) });
            ui.close();
        }
        if w::menu_item(ui, t, icons::BACKPACK, t!("Inventory")).clicked() {
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
                crate::builtin::steam_id_menu(ui, t, persona.steam_id);
            });
            ui.separator();
            if ui.button(w::body_m(t, t!("Remove friend…"), t.danger)).clicked() {
                app.friend_dialog = Some((vp, FriendDialog::Remove { id: persona.steam_id, name: persona.display_name() }));
                ui.close();
            }
            if ui.button(w::body_m(t, t!("Block all communication…"), t.danger)).clicked() {
                app.friend_dialog = Some((vp, FriendDialog::Block { id: persona.steam_id, name: persona.display_name() }));
                ui.close();
            }
        });
        let env = Env { state: &app.state, local: &app.local, theme: &app.prefs.theme, zoom: app.prefs.zoom, client: &app.client, actions: &mut *actions };
        app.registry.friend_menu(persona, ui, env);
    });
}

/// Your group chats under the friends, like Steam's list (a click on the
/// title folds them away).
fn group_chats(app: &mut SteamApp, ui: &mut Ui, t: &Theme, filter: &str) {
    let groups = match &app.state.chat_groups {
        Some(Ok(g)) => g.clone(),
        Some(Err(e)) => {
            ui.add(egui::Label::new(w::body_s(t, t!("Couldn't load your group chats: {error}", error = i18n::tr_dyn(e)), t.text_faint)).wrap());
            return;
        }
        _ => return,
    };
    let shown: Vec<_> = groups.iter().filter(|g| filter.is_empty() || group_name(&app.state, g).to_lowercase().contains(filter)).collect();
    if shown.is_empty() && !filter.is_empty() {
        return;
    }
    let hidden = app.friends_window.groups_hidden;
    let text = format!("{} {} ({})", if hidden { icons::CARET_RIGHT } else { icons::CARET_DOWN }, t!("Group chats"), shown.len());
    // Lit while the pointer is on it (where it was drawn last frame).
    let id = egui::Id::unique("fw-group-title");
    let lit = ui.data(|d| d.get_temp::<Rect>(id)).is_some_and(|r| ui.rect_contains_pointer(r));
    let title = ui
        .horizontal(|ui| {
            let title = ui.add(egui::Label::new(w::body_m(t, text, if lit { t.text } else { t.text_dim })).sense(Sense::click()));
            // Steam's "+": make a new group chat.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::icon_button(ui, t, icons::PLUS, t!("Create a group chat"), false, 0).clicked() {
                    app.friends_window.convs.show(Pane::NewGroup);
                }
            });
            title
        })
        .inner;
    ui.data_mut(|d| d.insert_temp(id, title.rect));
    if title.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        app.friends_window.groups_hidden = !hidden;
    }
    if hidden {
        return;
    }
    ui.add_space(t.spacing.sm + 2.0);
    let c = &t.components.friend_row;
    let open_group = match app.friends_window.convs.open {
        Some(Pane::Group(g, _)) => Some(g),
        _ => None,
    };
    for g in shown {
        let name = group_name(&app.state, g);
        let h = t.height_of(c, 66.0);
        let pad = t.padding_of(c);
        let (slot, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        let selected = open_group == Some(g.id);
        let look = t.look(c, w::interact(ui, &resp, selected));
        w::paint_box(ui, t, slot.expand2(vec2(pad.x, 0.0)), c, &look, 0.0);
        let content = Rect::from_min_max(pos2(slot.left(), slot.top() + pad.y), pos2(slot.right(), slot.bottom() - pad.y));
        let av = Rect::from_min_size(content.min, vec2(content.height(), content.height()));
        paint_group_avatar(ui, t, g, &name, av);
        let x = av.right() + t.spacing.sm + 1.0;
        let right = content.right() - if g.unread { t.dense(20.0) } else { 0.0 };
        let last = g.rooms.iter().max_by_key(|r| r.last_time);
        let preview = last
            .filter(|r| !r.last_message.is_empty())
            .map(|r| {
                let who = app.state.persona(r.last_sender).name;
                let (text, _) = model::chat::parse(&r.last_message);
                if who.is_empty() { text } else { format!("{who}: {text}") }
            })
            .unwrap_or_else(|| if g.active_members == 1 { t!("1 member online").to_string() } else { t!("{n} members online", n = g.active_members) });
        let status_style = if h >= 56.0 { t.typography.body_m } else { t.typography.body_s };
        let ng = fit(ui, name.clone(), t.font_of(c), t.text, right - x);
        let sg = fit(ui, preview, egui::FontId::new(status_style.size, status_style.weight.family()), t.text_dim, right - x);
        let gap = t.spacing.xs + 2.0;
        let top = content.center().y - (ng.size().y + gap + sg.size().y) / 2.0;
        let nh = ng.size().y;
        ui.painter().galley(pos2(x, top), ng, t.text);
        ui.painter().galley(pos2(x, top + nh + gap), sg, t.text_dim);
        if g.unread {
            ui.painter().circle_filled(pos2(content.right() - t.dense(8.0), content.center().y), t.dense(5.0), t.accent);
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            let room = if g.default_room != 0 { g.default_room } else { g.rooms.first().map(|r| r.id).unwrap_or(0) };
            app.friends_window.convs.show(Pane::Group(g.id, room));
        }
    }
}

/// One line of the Activity tab.
struct Line {
    time: u64,
    who: SteamId,
    text: String,
    open: Option<Pane>,
}

/// The Activity tab: what friends said and did lately, newest first, by day
/// (Figma: dates in the accent color over a line, small avatars).
fn activity(app: &mut SteamApp, ui: &mut Ui, t: &Theme, actions: &mut Vec<Action>) {
    let state = &app.state;
    let mut lines: Vec<Line> = Vec::new();
    // Each conversation's latest message.
    for (friend, msgs) in &state.chats {
        if let Some(m) = msgs.iter().rev().find(|m| !m.text.is_empty() || m.invite.is_some()) {
            let text = if m.invite.is_some() { t!("sent a game invite").to_string() } else { m.text.clone() };
            lines.push(Line { time: m.time, who: m.from, text: if m.from == state.steam_id { format!("→ {}: {text}", state.persona(*friend).display_name()) } else { text }, open: Some(Pane::Chat(*friend)) });
        }
    }
    // Each group chat's latest message.
    if let Some(Ok(groups)) = &state.chat_groups {
        for g in groups {
            if let Some(r) = g.rooms.iter().filter(|r| !r.last_message.is_empty()).max_by_key(|r| r.last_time) {
                let (text, _) = model::chat::parse(&r.last_message);
                lines.push(Line { time: r.last_time as u64, who: r.last_sender, text: format!("{} · {text}", group_name(state, g)), open: Some(Pane::Group(g.id, r.id)) });
            }
        }
    }
    // What friends did while the app was open.
    for s in &app.friends_window.seen {
        let text = match &s.what {
            Did::CameOnline => t!("is now online").to_string(),
            Did::WentOffline => t!("went offline").to_string(),
            Did::Playing(game) => t!("started playing {game}", game = game).to_string(),
        };
        lines.push(Line { time: s.time, who: s.who, text, open: Some(Pane::Chat(s.who)) });
    }
    lines.sort_by_key(|l| std::cmp::Reverse(l.time));
    let mut open = None;
    egui::ScrollArea::vertical().id_salt("friend-activity").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if lines.is_empty() {
            ui.add_space(t.spacing.sm);
            ui.add(egui::Label::new(w::body_m(t, t!("Messages and what your friends are up to show here."), t.text_faint)).wrap());
        }
        let today = ui::date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
        let mut day = String::new();
        for l in &lines {
            let d = ui::date(l.time);
            if d != day {
                if !day.is_empty() {
                    ui.add_space(t.spacing.xl);
                }
                let label = if d == today { t!("Today").to_string() } else { d.clone() };
                let r = ui.label(w::heading_s(t, label, t.accent));
                ui.painter().hline(r.rect.left()..=ui.max_rect().right(), r.rect.bottom() + t.spacing.xs, Stroke::new(1.0, t.accent_2));
                ui.add_space(t.spacing.lg + t.spacing.xs);
                day = d;
            }
            let p = state.persona(l.who);
            let av = t.dense(21.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), av + t.spacing.lg), Sense::click());
            if resp.hovered() {
                let c = &t.components.friend_row;
                w::paint_box(ui, t, rect.expand2(vec2(t.spacing.sm, 0.0)), c, &t.look(c, w::interact(ui, &resp, false)), 0.0);
            }
            let a = Rect::from_min_size(pos2(rect.left(), rect.top() + t.spacing.sm / 2.0), vec2(av, av));
            w::paint_presence_avatar(ui, t, a, &art::avatar(p.avatar_hash.as_deref()), Some(status_color(t, &p)));
            let x = a.right() + t.spacing.sm + 1.0;
            let wdt = rect.right() - x;
            let name = if p.name.is_empty() { t!("You").to_string() } else { p.display_name() };
            let mut job = egui::text::LayoutJob::default();
            let f = |s: ui::theme::TextStyle| egui::FontId::new(s.size, s.weight.family());
            job.append(&name, 0.0, egui::TextFormat::simple(f(ui::theme::TextStyle { weight: ui::theme::Weight::Bold, ..t.typography.body_m }), status_color(t, &p)));
            job.append(&format!(" {}", l.text), 0.0, egui::TextFormat::simple(f(t.typography.body_m), t.text));
            job.wrap = egui::text::TextWrapping { max_width: wdt.max(20.0), max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(x, a.top() + (av - ui.fonts_mut(|f2| f2.row_height(&f(t.typography.body_m)))) / 2.0), g.clone(), t.text);
            let extra = (g.size().y - av).max(0.0);
            if extra > 0.0 {
                ui.add_space(extra);
            }
            if let Some(p) = l.open.filter(|_| resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(ui::ago(l.time)).clicked()) {
                open = Some(p);
            }
        }
        ui.add_space(t.spacing.lg);
    });
    if let Some(p) = open {
        if let Pane::Chat(id) = p {
            actions.push(Action::MarkRead(id));
        }
        app.friends_window.convs.show(p);
    }
}

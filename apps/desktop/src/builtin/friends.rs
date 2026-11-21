//! Friends & Chat: what opens beside the friends list in the Friends & Chat
//! window (`friends_window.rs` draws the window and the list). One-to-one
//! chats, group chats (`friends_groups.rs`), and adding, accepting and
//! blocking friends, with the open conversations as tabs along the top.
//!
//! It's an extension without pages so it gets events (Quick Invite links,
//! profile search results) and a [`Ctx`] like any page.

use crate::builtin::chat;
use crate::friends_window::{Conversations, Pane};
use client::{Command, State};
use egui::{Align, Key, Layout, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use extensions::{Action, Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{SteamId, art};
use std::collections::{HashMap, HashSet};
use ui::theme::{TextStyle, Weight};
use ui::{Theme, icons, widgets as w};

#[path = "friends_groups.rs"]
mod groups;
#[path = "group_settings.rs"]
mod admin;
#[path = "friends_voice.rs"]
mod voice;
pub(crate) use groups::{group_name, paint_group_avatar};
pub(crate) use voice::{dock_height, settings as voice_settings};

#[derive(Default)]
pub struct Friends {
    /// What's typed in each conversation's box.
    inputs: HashMap<SteamId, String>,
    /// Conversations whose history was asked for.
    loaded: HashSet<SteamId>,
    group_input: String,
    /// Show a group's members beside its chat even when the window is narrow.
    members: bool,
    /// The group chat "Leave" asks about.
    leave_confirm: Option<u64>,
    groups_asked: bool,
    history_asked: HashSet<(u64, u64)>,
    /// The newest message time told to Steam as read, per room.
    acked: HashMap<(u64, u64), u32>,
    /// Add a friend: the friend code box.
    add: String,
    /// Add a friend: your Quick Invite link (asked for once per visit).
    quick: Option<Result<model::QuickInvite, String>>,
    quick_asked: bool,
    /// Add a friend: profile search, what was last sent, when the text last
    /// changed (searching waits for a pause in typing), and the results.
    find: String,
    find_sent: String,
    find_typed: f64,
    found: Option<Result<model::UserSearch, String>>,
    finding: bool,
    /// Invites sent from the search results this session.
    invited: HashSet<SteamId>,
    /// Group chat management: its dialogs and the new group page.
    admin: admin::Admin,
}

impl Extension for Friends {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "friends",
            name: "Friends & Chat",
            author: "SteamInRust",
            description: "Friends, chat and group chats",
            core: true,
            ..Manifest::DEFAULT
        }
    }

    fn pages(&self) -> Vec<PageSpec> {
        Vec::new()
    }

    fn on_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        match event {
            client::Event::QuickInvite(r) => self.quick = Some(r.clone()),
            client::Event::QuickInviteRedeemed { result, .. } => match result {
                Ok(()) => ctx.toast(t!("You're now friends")),
                Err(e) => ctx.toast(t!("That invite link didn't work: {error}", error = e)),
            },
            client::Event::UserSearch(r) => {
                self.finding = false;
                match (r, &mut self.found) {
                    // Only results for what's in the box now.
                    (Ok(page), _) if page.text != self.find_sent => {}
                    // The next page adds to the list.
                    (Ok(page), Some(Ok(shown))) if page.page > 1 && page.text == shown.text => {
                        shown.users.extend(page.users.iter().cloned());
                        shown.page = page.page;
                    }
                    (r, found) => *found = Some(r.clone()),
                }
            }
            client::Event::ChatAdmin(e) => self.admin.on_event(e, ctx),
            _ => {}
        }
    }

    fn as_any(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Body Medium in a heavier weight: names in chats and rows.
pub(crate) fn strong(t: &Theme, s: impl Into<String>, color: egui::Color32) -> RichText {
    w::styled(s, TextStyle { weight: Weight::Semibold, ..t.typography.body_m }, color)
}

impl Friends {
    /// The conversation side of the window: the open conversations as tabs,
    /// then the one showing. `reserve` keeps room at the right end of the
    /// tabs for the window buttons.
    pub fn pane(&mut self, ui: &mut Ui, ctx: &mut Ctx, convs: &mut Conversations, reserve: f32) {
        let t = ctx.theme.clone();
        // A link to a group (no room yet): its default room once groups are known.
        if let Some(Pane::Group(g, 0)) = convs.open {
            if !self.groups_asked {
                self.groups_asked = true;
                ctx.send(Command::LoadChatGroups);
            }
            if let Some(found) = ctx.state.chat_groups.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|x| x.id == g)) {
                let room = if found.default_room != 0 { found.default_room } else { found.rooms.first().map(|r| r.id).unwrap_or(0) };
                convs.show(Pane::Group(g, room));
            }
        }
        self.tabs(ui, &t, ctx, convs, reserve);
        let Some(pane) = convs.open else { return };
        match pane {
            Pane::Chat(id) => self.conversation(ui, &t, ctx, convs, id),
            Pane::Group(g, room) => {
                self.group_conversation(ui, &t, ctx, convs, g, room);
                self.leave_dialog(ui, &t, ctx, convs);
            }
            Pane::Add | Pane::Invites | Pane::Blocked => self.manage(ui, &t, ctx, convs, pane),
            Pane::NewGroup => self.admin.new_group(ui, &t, ctx),
        }
    }

    /// The open conversations, one tab each (avatar, name, unread count and
    /// a close button), like Steam's chat tabs.
    fn tabs(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, convs: &mut Conversations, reserve: f32) {
        let c = &t.components.sort;
        let h = t.height_of(c, 39.0);
        let pad = t.spacing.sm;
        let strip = Rect::from_min_size(ui.max_rect().min, vec2(ui.max_rect().width(), h + pad * 2.0));
        // Empty parts of the strip move the window.
        crate::chrome::drag_area(ui, strip, true);
        let room = Rect::from_min_max(strip.min + vec2(t.spacing.lg, pad), pos2(strip.right() - reserve - pad, strip.bottom() - pad));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(room).layout(Layout::left_to_right(Align::Center)));
        let mut close = None;
        egui::ScrollArea::horizontal().id_salt("chat-tabs").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.xs + 2.0;
            for tab in convs.tabs.clone() {
                let (name, unread) = match tab {
                    Pane::Chat(id) => (ctx.state.persona(id).display_name(), ctx.state.unread.get(&id).copied().unwrap_or(0)),
                    Pane::Group(g, _) => {
                        let found = ctx.state.chat_groups.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|x| x.id == g));
                        (found.map(|x| group_name(ctx.state, x)).unwrap_or_else(|| t!("Group chat").to_string()), found.is_some_and(|x| x.unread) as usize)
                    }
                    _ => continue,
                };
                let selected = convs.open.is_some_and(|o| o.same(tab));
                let font = t.font_of(c);
                let mut job = egui::text::LayoutJob::simple_singleline(name, font, egui::Color32::PLACEHOLDER);
                job.wrap = egui::text::TextWrapping::truncate_at_width(t.dense(150.0));
                let g = ui.painter().layout_job(job);
                let px = t.spacing.md;
                let av = (h - t.spacing.md).max(16.0);
                let x_w = h * 0.5;
                let width = px + av + t.spacing.sm + g.size().x + t.spacing.sm + x_w + px * 0.5;
                let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
                let look = t.look(c, w::interact(ui, &resp, selected));
                w::paint_box(ui, t, rect, c, &look, if selected { 1.0 } else { 0.0 });
                let av_rect = Rect::from_min_size(pos2(rect.left() + px, rect.center().y - av / 2.0), vec2(av, av));
                match tab {
                    Pane::Chat(id) => {
                        let p = ctx.state.persona(id);
                        w::paint_presence_avatar(ui, t, av_rect, &art::avatar(p.avatar_hash.as_deref()), Some(crate::shell::status_color(t, &p)));
                    }
                    Pane::Group(g, _) => {
                        if let Some(found) = ctx.state.chat_groups.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|x| x.id == g)) {
                            paint_group_avatar(ui, t, found, &group_name(ctx.state, found), av_rect);
                        }
                    }
                    _ => {}
                }
                let text_x = av_rect.right() + t.spacing.sm;
                ui.painter().galley(pos2(text_x, rect.center().y - g.size().y / 2.0), g.clone(), look.text);
                // The close button, or the unread count until the pointer comes.
                let x_rect = Rect::from_center_size(pos2(rect.right() - px * 0.5 - x_w / 2.0, rect.center().y), vec2(x_w, x_w));
                let x_resp = ui.interact(x_rect, resp.id.with("close"), Sense::click());
                if unread > 0 && !selected && !resp.hovered() && !x_resp.hovered() {
                    w::badge(ui.painter(), t, x_rect.center(), unread);
                } else if selected || resp.hovered() || x_resp.hovered() {
                    let color = if x_resp.hovered() { t.text } else { look.text.gamma_multiply(0.7) };
                    w::icon_centered(ui.painter(), x_rect.center(), icons::X, t.dense(13.0), color);
                }
                if x_resp.on_hover_text(t!("Close conversation")).clicked() || resp.middle_clicked() {
                    close = Some(tab);
                } else if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    convs.show(tab);
                }
            }
        });
        if let Some(tab) = close {
            convs.close(tab);
        }
        ui.advance_cursor_after_rect(strip);
    }

    fn conversation(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, convs: &mut Conversations, id: SteamId) {
        if self.loaded.insert(id) {
            ctx.send(Command::LoadHistory { friend: id });
        }
        if ctx.state.unread.contains_key(&id) {
            ctx.act(Action::MarkRead(id));
        }
        let p = ctx.state.persona(id);
        let color = crate::shell::status_color(t, &p);
        let x = t.spacing.xl as i8;
        egui::Frame::new().inner_margin(Margin { left: x, right: x, top: t.spacing.sm as i8, bottom: t.spacing.md as i8 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // The buttons first (right to left), so a long status is cut
                // off instead of covering them.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::X, t!("Close conversation"), false, 0).clicked() {
                        convs.close(Pane::Chat(id));
                    }
                    let r = w::icon_button(ui, t, icons::COPY, t!("Copy SteamID"), false, 0);
                    if let Some(Some(what)) = egui::Popup::from_toggle_button_response(&r).show(|ui| super::steam_id_menu(ui, t, id)).map(|r| r.inner) {
                        ctx.toast(t!("Copied: {what}", what = what));
                    }
                    if w::icon_button(ui, t, icons::USER_CIRCLE, t!("View profile"), false, 0).clicked() {
                        ctx.navigate("profile", Some(id.to_string()));
                    }
                    self.call_button(ui, t, ctx, id);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let size = t.dense(46.0);
                        let av = w::presence_avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), size, Some(color));
                        if hover_ring(ui, t, av).on_hover_text(t!("View profile")).clicked() {
                            ctx.navigate("profile", Some(id.to_string()));
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = t.spacing.xs;
                            let name = if p.display_name().is_empty() { id.to_string() } else { p.display_name() };
                            let r = ui.add(egui::Label::new(w::heading_s(t, name, t.text)).truncate().sense(Sense::click()));
                            if link_hover(ui, r, t.text).on_hover_text(t!("View profile")).clicked() {
                                ctx.navigate("profile", Some(id.to_string()));
                            }
                            let status = crate::shell::status_text(ctx.state, &p);
                            let r = ui.add(egui::Label::new(w::body_m(t, &status, color)).truncate().sense(Sense::click()));
                            if let Some(game) = p.game_app_id.filter(|_| p.in_game()) {
                                if link_hover(ui, r, color).on_hover_text(t!("Open the game's page")).clicked() {
                                    ctx.act(crate::shell::game_page(ctx.state, game));
                                }
                            } else {
                                r.on_hover_text(&status);
                            }
                        });
                    });
                });
            });
        });
        egui::Frame::new().inner_margin(Margin { left: x, right: x, top: 0, bottom: t.spacing.lg as i8 }).show(ui, |ui| {
            let h = ui.available_height();
            let input = self.inputs.entry(id).or_default();
            let msgs: Vec<chat::Msg> = ctx.state.chats.get(&id).map(|c| c.iter().map(chat::Msg::friend).collect()).unwrap_or_default();
            let sender = CtxSender(ctx);
            let env = chat::Env { state: ctx.state, client: &sender, steam_root: Some(&ctx.local.steam_root), language: i18n::language().steam_name().to_string() };
            let acts = chat::conversation(ui, t, &env, client::ChatTarget::Friend(id), &msgs, input, h, None, false);
            for a in acts {
                ctx.act(a);
            }
        });
    }

    /// Adding friends, invites and blocked people, with tabs to move between them.
    fn manage(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, convs: &mut Conversations, pane: Pane) {
        let state = ctx.state;
        let invites_n = state.friends.values().filter(|r| **r == model::Relationship::RequestRecipient).count();
        let blocked_n = state.friends.values().filter(|r| matches!(r, model::Relationship::Blocked | model::Relationship::Ignored)).count();
        let x = t.spacing.xl as i8;
        egui::Frame::new().inner_margin(Margin { left: x, right: x, top: 0, bottom: t.spacing.lg as i8 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // The close button first, then the tabs in what's left, on a
            // second line when they don't fit (long names in some languages).
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked() {
                        convs.close(pane);
                    }
                    ui.add_space(t.spacing.sm);
                    ui.with_layout(Layout::left_to_right(Align::Min).with_main_wrap(true), |ui| {
                        ui.spacing_mut().item_spacing = vec2(t.spacing.lg, t.spacing.xs);
                        let h = t.height_of(&t.components.nav_tab, 36.0);
                        for (p, label) in [
                            (Pane::Add, t!("Add a friend").to_string()),
                            (Pane::Invites, if invites_n > 0 { format!("{} ({invites_n})", t!("Invites")) } else { t!("Invites").to_string() }),
                            (Pane::Blocked, if blocked_n > 0 { format!("{} ({blocked_n})", t!("Blocked")) } else { t!("Blocked").to_string() }),
                        ] {
                            if w::nav_tab(ui, t, &label, pane == p, h).clicked() {
                                convs.show(p);
                            }
                        }
                    });
                });
            });
            ui.add_space(t.spacing.md);
            egui::ScrollArea::vertical().id_salt("friends-manage").auto_shrink([false; 2]).show(ui, |ui| match pane {
                Pane::Add => self.add_friend(ui, t, ctx),
                Pane::Invites => self.invites(ui, t, ctx),
                _ => self.blocked(ui, t, ctx),
            });
        });
    }
}

/// Extensions reach the client through `Ctx::send`.
pub struct CtxSender<'a, 'b>(pub &'a Ctx<'b>);

pub use crate::builtin::chat::Sender;

impl Sender for CtxSender<'_, '_> {
    fn send(&self, cmd: Command) {
        self.0.send(cmd);
    }
}

/// A one-to-one chat: its messages and the message box, `height` tall. The
/// overlay shows it too. Returns what the app should do (join a game, open
/// a page).
pub fn chat_body(ui: &mut Ui, t: &Theme, state: &State, client: &impl Sender, friend: SteamId, input: &mut String, height: f32) -> Vec<Action> {
    let msgs: Vec<chat::Msg> = state.chats.get(&friend).map(|c| c.iter().map(chat::Msg::friend).collect()).unwrap_or_default();
    let env = chat::Env { state, client, steam_root: None, language: i18n::language().steam_name().to_string() };
    chat::conversation(ui, t, &env, client::ChatTarget::Friend(friend), &msgs, input, height, None, false)
}

impl Friends {
    fn add_friend(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.set_max_width(ui.available_width().min(t.dense(680.0)));
        self.friend_code_panel(ui, t, ctx);
        ui.add_space(t.spacing.md);
        self.quick_invite_panel(ui, t, ctx);
        ui.add_space(t.spacing.md);
        self.search_panel(ui, t, ctx);
        ui.add_space(t.spacing.lg);
    }

    /// Your friend code, and a box for theirs (or a profile or invite link).
    fn friend_code_panel(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let code = (ctx.state.steam_id & 0xFFFF_FFFF).to_string();
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::caption_text(t, t!("YOUR FRIEND CODE")));
            ui.add_space(t.spacing.xs);
            // Narrow: the box and its button on their own lines.
            let narrow = ui.available_width() < t.dense(440.0);
            ends(ui, |ui| {
                if action(ui, t, icons::COPY, t!("Copy"), w::Kind::Primary, ui.available_width() < t.dense(260.0)).clicked() {
                    ui.ctx().copy_text(code.clone());
                    ctx.toast(t!("Friend code copied"));
                }
            }, |ui| {
                ui.label(w::heading_l(t, &code, t.text));
            });
            ui.add_space(t.spacing.sm);
            ui.add(egui::Label::new(w::body_s(t, t!("Enter your friend's friend code to send them an invite. A profile link or a Quick Invite link works too."), t.text_dim)).wrap());
            ui.add_space(t.spacing.sm);
            let invite_link = client::friends_add::parse_invite_link(&self.add);
            let target = parse_friend(&self.add);
            let ready = target.is_some() || invite_link.is_some();
            let mut send = false;
            if narrow {
                let r = w::search(ui, t, &mut self.add, t!("Enter a friend code"), ui.available_width());
                send |= ready && r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(t.spacing.sm);
                ui.add_enabled_ui(ready, |ui| {
                    send |= w::button(ui, t, Some(icons::USER_PLUS), t!("Send invite"), w::Kind::Primary, w::Size::Md).clicked();
                });
            } else {
                // The button first (right to left), so the box takes what's left.
                ui.allocate_ui_with_layout(vec2(ui.available_width(), t.height_of(&t.components.button_primary, 40.0)), Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(ready, |ui| {
                        send |= w::button(ui, t, Some(icons::USER_PLUS), t!("Send invite"), w::Kind::Primary, w::Size::Md).clicked();
                    });
                    ui.add_space(t.spacing.sm);
                    let r = w::search(ui, t, &mut self.add, t!("Enter a friend code"), ui.available_width());
                    send |= ready && r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                });
            }
            if send {
                match (invite_link, target) {
                    (Some((steam_id, token)), _) => ctx.send(Command::RedeemQuickInvite { steam_id, token }),
                    (None, Some(id)) => {
                        ctx.send(Command::AddFriend { steam_id: id });
                        ctx.toast(t!("Friend invite sent"));
                    }
                    _ => {}
                }
                self.add.clear();
            }
            if !self.add.trim().is_empty() && !ready {
                ui.add(egui::Label::new(w::body_s(t, t!("Enter a friend code (numbers) or a link like steamcommunity.com/profiles/7656…"), t.away)).wrap());
            }
        });
    }

    /// A link that makes you friends with whoever opens it, like Steam's.
    fn quick_invite_panel(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if !self.quick_asked && ctx.state.steam_id != 0 {
            self.quick_asked = true;
            ctx.send(Command::LoadQuickInvite { new: false });
        }
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::caption_text(t, t!("OR SEND A QUICK INVITE")));
            ui.add_space(t.spacing.xs);
            ui.add(egui::Label::new(w::body_s(t, t!("Share this link by message or email. Whoever opens it becomes your friend once they accept. Be careful sharing it in public places."), t.text_dim)).wrap());
            ui.label(w::body_s(t, t!("Each link works once and stops working after 30 days."), t.text_faint));
            ui.add_space(t.spacing.sm);
            match &self.quick {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(w::body_m(t, t!("Getting your link…"), t.text_dim));
                    });
                }
                Some(Err(e)) => {
                    ui.label(w::body_s(t, t!("Couldn't get a link: {error}", error = e), t.danger));
                }
                Some(Ok(q)) => {
                    let link = q.link.clone();
                    egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(Margin::symmetric(t.spacing.md as i8, t.spacing.xs as i8 + 2)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let narrow = ui.available_width() < t.dense(360.0);
                        ends(ui, |ui| {
                            if action(ui, t, icons::COPY, t!("Copy"), w::Kind::Primary, narrow).clicked() {
                                ui.ctx().copy_text(link.clone());
                                ctx.toast(t!("Invite link copied"));
                            }
                        }, |ui| {
                            ui.add(egui::Label::new(strong(t, &link, t.text)).truncate()).on_hover_text(&link);
                        });
                    });
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    let days = (q.expires as u64).saturating_sub(now).div_ceil(86400);
                    ui.add_space(t.spacing.xs);
                    ui.label(w::body_s(t, t!("Works for {n} more days", n = days), t.text_faint));
                }
            }
            ui.add_space(t.spacing.sm);
            if w::button(ui, t, Some(icons::REFRESH), t!("Generate new link"), w::Kind::Secondary, w::Size::Sm).clicked() {
                self.quick = None;
                ctx.send(Command::LoadQuickInvite { new: true });
            }
        });
    }

    /// Find someone by their profile name, as on Steam Community.
    fn search_panel(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::caption_text(t, t!("OR SEARCH FOR YOUR FRIEND")));
            ui.add_space(t.spacing.sm);
            let before = self.find.clone();
            let r = w::search(ui, t, &mut self.find, t!("Profile name"), ui.available_width());
            let now = ui.input(|i| i.time);
            if self.find != before {
                self.find_typed = now;
            }
            let text = self.find.trim().to_string();
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            // Search once typing pauses (or right away on Enter).
            if text.chars().count() >= 2 && text != self.find_sent && (enter || now - self.find_typed > 0.5) {
                self.find_sent = text.clone();
                self.finding = true;
                self.found = None;
                ctx.send(Command::SearchUsers { text: text.clone(), page: 1 });
            } else if text != self.find_sent && !text.is_empty() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            }
            if text.is_empty() {
                self.found = None;
                self.find_sent.clear();
                return;
            }
            ui.add_space(t.spacing.sm);
            let found = match &self.found {
                None if self.finding => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(w::body_m(t, t!("Searching…"), t.text_dim));
                    });
                    return;
                }
                None => return,
                Some(Err(e)) => {
                    ui.label(w::body_s(t, t!("Search didn't work: {error}", error = e), t.danger));
                    return;
                }
                Some(Ok(found)) => found.clone(),
            };
            // Your own friends with a matching name first: Steam ranks
            // strangers with the same name above them.
            let needle = found.text.to_lowercase();
            let mine: Vec<model::FoundUser> = ctx
                .state
                .friends
                .iter()
                .filter(|(_, r)| **r == model::Relationship::Friend)
                .map(|(id, _)| ctx.state.persona(*id))
                .filter(|p| p.name.to_lowercase().contains(&needle))
                .map(|p| model::FoundUser { steam_id: p.steam_id, avatar: art::avatar(p.avatar_hash.as_deref()), name: p.name, ..Default::default() })
                .collect();
            if found.users.is_empty() && mine.is_empty() {
                ui.label(w::body_m(t, t!("No one found with that name."), t.text_faint));
                return;
            }
            if !mine.is_empty() {
                ui.label(w::caption_text(t, t!("YOUR FRIENDS")));
                ui.add_space(t.spacing.xs);
                for u in &mine {
                    self.found_row(ui, t, ctx, u);
                }
                ui.add_space(t.spacing.sm);
            }
            ui.label(w::body_s(t, t!("{n} profiles found", n = found.total), t.text_faint));
            ui.add_space(t.spacing.xs);
            for u in found.users.iter().filter(|u| !mine.iter().any(|m| m.steam_id == u.steam_id)) {
                self.found_row(ui, t, ctx, u);
            }
            if (found.users.len() as u32) < found.total && found.page < 50 {
                ui.add_space(t.spacing.sm);
                if self.finding {
                    ui.spinner();
                } else if w::button(ui, t, None, t!("Show more"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    self.finding = true;
                    ctx.send(Command::SearchUsers { text: found.text.clone(), page: found.page + 1 });
                }
            }
        });
    }

    /// One profile in the search results: avatar, name, details, and an
    /// invite button that knows whether you're already friends.
    fn found_row(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, u: &model::FoundUser) {
        egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(Margin::symmetric(t.spacing.md as i8, t.spacing.sm as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let narrow = ui.available_width() < t.dense(420.0);
            let mut open = false;
            ends(ui, |ui| {
                    let rel = ctx.state.friends.get(&u.steam_id).copied();
                    if u.steam_id == ctx.state.steam_id {
                        ui.label(w::body_s(t, t!("That's you"), t.text_faint));
                    } else if rel == Some(model::Relationship::Friend) {
                        ui.label(w::body_s(t, format!("{}  {}", icons::CHECK, t!("Friends")), t.online));
                    } else if rel == Some(model::Relationship::RequestRecipient) {
                        if action(ui, t, icons::CHECK, t!("Accept"), w::Kind::Primary, narrow).clicked() {
                            ctx.send(Command::AddFriend { steam_id: u.steam_id });
                        }
                    } else if rel == Some(model::Relationship::RequestInitiator) || self.invited.contains(&u.steam_id) {
                        ui.label(w::body_s(t, t!("Invite sent"), t.text_dim));
                    } else if action(ui, t, icons::USER_PLUS, t!("Add as friend"), w::Kind::Primary, narrow).clicked() {
                        ctx.send(Command::AddFriend { steam_id: u.steam_id });
                        self.invited.insert(u.steam_id);
                        ctx.toast(t!("Friend invite sent to {name}", name = u.name));
                    }
            }, |ui| {
                let av = w::presence_avatar(ui, t, &u.avatar, t.dense(44.0), None);
                open |= hover_ring(ui, t, av).clicked();
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    let name = ui.add(egui::Label::new(strong(t, &u.name, t.text)).sense(Sense::click()).truncate());
                    open |= link_hover(ui, name, t.text).clicked();
                    for line in u.details.lines().take(2) {
                        ui.add(egui::Label::new(w::body_s(t, line, t.text_faint)).truncate());
                    }
                });
            });
            if open {
                ctx.navigate("profile", Some(u.steam_id.to_string()));
            }
        });
        ui.add_space(t.spacing.xs);
    }

    fn invites(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let pick = |rel: model::Relationship| -> Vec<model::Persona> { ctx.state.friends.iter().filter(|(_, r)| **r == rel).map(|(id, _)| ctx.state.persona(*id)).collect() };
        let received = pick(model::Relationship::RequestRecipient);
        let sent = pick(model::Relationship::RequestInitiator);
        if received.is_empty() && sent.is_empty() {
            w::empty(ui, t, icons::USER_PLUS, t!("No pending invites."), "");
        }
        for (title, list, incoming) in [(t!("RECEIVED"), received, true), (t!("SENT"), sent, false)] {
            if list.is_empty() {
                continue;
            }
            ui.label(w::caption_text(t, &format!("{title}  ·  {}", list.len())));
            ui.add_space(t.spacing.sm);
            for p in &list {
                egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(Margin::symmetric(t.spacing.md as i8, t.spacing.sm as i8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // Text buttons while they fit beside a readable name; icons
                    // (with the words on hover) in a narrow window.
                    let narrow = ui.available_width() < t.dense(if incoming { 470.0 } else { 400.0 });
                    let mut open = false;
                    ends(ui, |ui| {
                        if incoming {
                            if action(ui, t, icons::X, t!("Ignore"), w::Kind::Ghost, narrow).clicked() {
                                ctx.send(client::Command::RemoveFriend { steam_id: p.steam_id });
                            }
                            if action(ui, t, icons::CHECK, t!("Accept"), w::Kind::Primary, narrow).clicked() {
                                ctx.send(client::Command::AddFriend { steam_id: p.steam_id });
                            }
                        } else if action(ui, t, icons::X, t!("Cancel invite"), w::Kind::Ghost, narrow).clicked() {
                            ctx.send(client::Command::RemoveFriend { steam_id: p.steam_id });
                        }
                    }, |ui| {
                        let av = w::presence_avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), t.dense(36.0), None);
                        open |= hover_ring(ui, t, av).on_hover_text(t!("View profile")).clicked();
                        let name = if p.name.is_empty() { p.steam_id.to_string() } else { p.name.clone() };
                        let r = ui.add(egui::Label::new(strong(t, name, t.text)).truncate().sense(Sense::click()));
                        open |= link_hover(ui, r, t.text).on_hover_text(t!("View profile")).clicked();
                    });
                    if open {
                        ctx.navigate("profile", Some(p.steam_id.to_string()));
                    }
                });
                ui.add_space(t.spacing.sm);
            }
            ui.add_space(t.spacing.md);
        }
    }

    fn blocked(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        ui.label(w::body_m(t, t!("They can't message you or send invites."), t.text_dim));
        ui.add_space(t.spacing.md);
        let list: Vec<model::Persona> = ctx.state.friends.iter().filter(|(_, r)| matches!(r, model::Relationship::Blocked | model::Relationship::Ignored)).map(|(id, _)| ctx.state.persona(*id)).collect();
        if list.is_empty() {
            ui.label(w::body_m(t, t!("You haven't blocked anyone."), t.text_faint));
        }
        let narrow = ui.available_width() < t.dense(320.0);
        for p in &list {
            let mut open = false;
            ends(ui, |ui| {
                if action(ui, t, icons::LOCK_SIMPLE, t!("Unblock"), w::Kind::Ghost, narrow).clicked() {
                    ctx.send(Command::BlockFriend { steam_id: p.steam_id, block: false });
                }
            }, |ui| {
                let av = w::presence_avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), t.dense(32.0), None);
                open |= hover_ring(ui, t, av).on_hover_text(t!("View profile")).clicked();
                let name = if p.name.is_empty() { p.steam_id.to_string() } else { p.name.clone() };
                let r = ui.add(egui::Label::new(strong(t, name, t.text)).truncate().sense(Sense::click()));
                open |= link_hover(ui, r, t.text).on_hover_text(t!("View profile")).clicked();
            });
            if open {
                ctx.navigate("profile", Some(p.steam_id.to_string()));
            }
            ui.add_space(t.spacing.xs);
        }
    }
}

/// A friend code (account id), SteamID64, or profile link -> SteamID64.
fn parse_friend(input: &str) -> Option<SteamId> {
    let s = input.trim().trim_end_matches('/');
    let s = s.rsplit_once("/profiles/").map(|x| x.1).unwrap_or(s);
    let n: u64 = s.trim_end_matches('/').parse().ok()?;
    match n {
        1..=0xFFFF_FFFF => Some(n + 76561197960265728),
        76561197960265729..=76561202255233023 => Some(n),
        _ => None,
    }
}

/// A row with things at its right end (`right`, laid out first and right to
/// left) and the rest on the left in what's left over, so a long name gets
/// cut off instead of pushing the buttons out of the panel.
pub(crate) fn ends<R>(ui: &mut Ui, right: impl FnOnce(&mut Ui), left: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            right(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), left).inner
        })
        .inner
    })
    .inner
}

/// A button with its words, or just its icon (the words on hover) when
/// `narrow`.
pub(crate) fn action(ui: &mut Ui, t: &Theme, icon: &str, label: &str, kind: w::Kind, narrow: bool) -> egui::Response {
    if narrow { w::icon_button(ui, t, icon, label, false, 0) } else { w::button(ui, t, Some(icon), label, kind, w::Size::Sm) }
}

/// A clickable picture: an outline while the pointer is on it.
pub(crate) fn hover_ring(ui: &Ui, t: &Theme, r: egui::Response) -> egui::Response {
    if r.hovered() {
        let radius = t.radius_of(&t.components.friend_avatar);
        let radius = egui::CornerRadius::same(radius.nw.min((r.rect.width() / 2.0) as u8 + 2));
        ui.painter().rect_stroke(r.rect.expand(2.0), radius, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Clickable text: underlined while the pointer is on it.
pub(crate) fn link_hover(ui: &Ui, r: egui::Response, color: egui::Color32) -> egui::Response {
    if r.hovered() {
        ui.painter().hline(r.rect.x_range(), r.rect.bottom(), egui::Stroke::new(1.0, color));
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Text that fits `width` on one line (cut with "…").
pub(crate) fn fit(ui: &Ui, text: String, font: egui::FontId, color: egui::Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    ui.painter().layout_job(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoSend;
    impl Sender for NoSend {
        fn send(&self, _: Command) {}
    }

    /// The chat inside a resizable window (like the overlay's) must keep the
    /// window's size; it used to grow a little every frame.
    #[test]
    fn chat_keeps_window_size() {
        let ctx = egui::Context::default();
        let t = Theme::default();
        let mut state = State::default();
        let friend: SteamId = 76561197960287930;
        state.chats.insert(friend, (0..30).map(|i| model::ChatMessage { friend, from: friend, text: format!("message {i}"), time: 1_700_000_000 + i * 600, invite: (i % 7 == 0).then(|| model::GameInvite { app_id: 477160, connect: "+connect_lobby 1".into() }), ..Default::default() }).collect());
        let mut input = String::new();
        let mut heights = Vec::new();
        for frame in 0..40 {
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1920.0, 1080.0))), time: Some(frame as f64 / 60.0), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let r = egui::Window::new("chat")
                    .title_bar(false)
                    .frame(egui::Frame::new().inner_margin(Margin::same(14)))
                    .default_pos(pos2(100.0, 100.0))
                    .default_size(vec2(440.0, 520.0))
                    .min_size(vec2(260.0, 160.0))
                    .resizable(true)
                    .show(ui.ctx(), |ui| {
                        ui.horizontal(|ui| ui.label("title"));
                        ui.separator();
                        ui.horizontal(|ui| ui.label("status"));
                        ui.add_space(6.0);
                        let h = ui.available_height();
                        let _ = chat_body(ui, &t, &state, &NoSend, friend, &mut input, h);
                    });
                heights.push(r.map(|r| r.response.rect.height()).unwrap_or(0.0));
            });
            out.textures_delta.clear();
        }
        let last = heights[heights.len() - 1];
        assert!((last - heights[5]).abs() < 0.5, "window grew: {:?}", &heights[..]);
    }

    #[test]
    fn friend_codes() {
        assert_eq!(parse_friend("22202"), Some(76561197960287930));
        assert_eq!(parse_friend("https://steamcommunity.com/profiles/76561197960287930/"), Some(76561197960287930));
        assert_eq!(parse_friend("hello"), None);
    }
}

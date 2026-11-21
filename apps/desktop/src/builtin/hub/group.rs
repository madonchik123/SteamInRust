//! A Steam group's page: who it is, how many are around, what it's about,
//! its announcements and some of its members; joining, leaving and
//! answering an invite.

use super::{HubView, Io, View, columns, error, files, rich};
use client::community_actions::{ClanRelationship as Clan, GroupButton, JoinKind, group_button};
use client::hub::{self, Request};
use client::{Command, State};
use egui::{Align, Layout, Sense, Ui, vec2};
use i18n::t;
use model::{Relationship, SteamId};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use ui::{Theme, icons, widgets as w};

/// Group ids start here; the rest is the clan's account id.
const CLAN_BASE: u64 = 103582791429521408;

pub(super) fn show(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, key: &str) {
    let req = || Request::Group { key: key.to_string() };
    let have = io.state.hub.groups.contains_key(key);
    hv.ask(io, format!("group:{key}"), have, req);
    let g = match io.state.hub.groups.get(key) {
        None => return w::loading(ui, t, &t!("Loading…")),
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, req());
            }
            return;
        }
        Some(Ok(g)) => g.clone(),
    };
    ui.add_space(16.0);
    header(hv, ui, t, io, &g, key);
    leave_dialog(hv, ui, t, io, &g);
    ui.add_space(18.0);
    columns(
        ui,
        300.0,
        hv,
        io,
        |ui, hv, io| {
            if !g.summary.trim().is_empty() {
                w::card(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(w::text_bold(t!("About"), 15.0, t.text));
                    ui.add_space(6.0);
                    if let Some(click) = rich::show(ui, t, &rich::parse(&g.summary), 14.0) {
                        hv.follow(io, click);
                    }
                });
                ui.add_space(16.0);
            }
            ui.label(w::text_bold(t!("Announcements"), 17.0, t.text));
            ui.add_space(8.0);
            if g.announcements.is_empty() {
                ui.label(w::text(t!("No announcements yet."), 13.0, t.text_faint));
            } else {
                files::news_cards(hv, ui, t, io, &g.announcements);
            }
        },
        |ui, _hv, io| members(ui, t, io, &g),
    );
}

fn header(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, g: &hub::Group, key: &str) {
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
            w::image_or_skeleton(ui, t, r, &g.avatar, t.corner_small());
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add(egui::Label::new(w::text_bold(&g.name, 26.0, t.text)).wrap());
                if !g.headline.is_empty() && g.headline != g.name {
                    ui.add(egui::Label::new(w::text(&g.headline, 14.0, t.text_dim)).wrap());
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let stat = |ui: &mut Ui, n: u32, label: String, color: egui::Color32| {
                        ui.label(w::text_bold(crate::builtin::store::thousands(n as u64), 14.0, color));
                        ui.label(w::text(label, 13.0, t.text_dim));
                        ui.add_space(10.0);
                    };
                    stat(ui, g.members, t!("members").to_string(), t.text);
                    stat(ui, g.in_game, t!("in game").to_string(), t.in_game);
                    stat(ui, g.online, t!("online").to_string(), t.online);
                    stat(ui, g.in_chat, t!("in group chat").to_string(), t.text);
                });
            });
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                let path = hub::group_path(&g.url).unwrap_or_else(|| format!("gid/{}", g.id));
                if w::button(ui, t, Some(icons::EXTERNAL), t!("Open on Steam"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    crate::links::open_http(&format!("https://steamcommunity.com/{path}"));
                }
                // Its group chat, when you're in it.
                let clan = g.id.checked_sub(CLAN_BASE).unwrap_or(0) as u32;
                let chat = io.state.chat_groups.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|c| c.clan_id != 0 && c.clan_id == clan)).map(|c| c.id);
                if let Some(chat) = chat {
                    if w::button(ui, t, Some(icons::CHAT), t!("Open group chat"), w::Kind::Primary, w::Size::Sm).clicked() {
                        io.go(View::Page("community".into(), Some(format!("group:{chat}"))));
                    }
                }
                if w::button(ui, t, Some(icons::CHATS), t!("Discussions"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    io.go(View::GroupForum { group: key.to_string(), forum: String::new() });
                }
                membership(hv, ui, t, io, g, key);
            });
        });
        // What Steam said about the last change.
        if let Some(notice) = hv.group_notice.get(&g.id) {
            ui.add_space(t.spacing.sm);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.add(egui::Label::new(w::body_m(t, notice, t.danger)).wrap());
            });
        }
    });
}

/// Friends in the group first, then the first members Steam lists.
fn members(ui: &mut Ui, t: &Theme, io: &mut Io, g: &hub::Group) {
    let friend = |id: &u64| io.state.friends.get(id) == Some(&Relationship::Friend);
    let friends: Vec<u64> = g.member_ids.iter().copied().filter(|id| friend(id)).collect();
    let others: Vec<u64> = g.member_ids.iter().copied().filter(|id| !friend(id)).take(24).collect();
    let mut open = None;
    let mut list = |ui: &mut Ui, title: String, ids: &[u64]| {
        if ids.is_empty() {
            return;
        }
        ui.label(w::text_bold(title, 15.0, t.text));
        ui.add_space(6.0);
        for id in ids {
            let p = io.state.persona(*id);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
            ui.painter().rect_filled(r, t.corner_small(), w::mix(t.bg, t.surface_hover, hover));
            let av = egui::Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(32.0, 32.0));
            let url = model::art::avatar(p.avatar_hash.as_deref());
            w::image_or_skeleton(ui, t, av, &url, t.corner_small());
            let name = if p.name.is_empty() { id.to_string() } else { p.name.clone() };
            let font = w::font(13.5);
            let max = r.width() - 52.0;
            let galley = ui.painter().layout(name, font, t.text, max);
            ui.painter().galley(egui::pos2(av.right() + 10.0, r.center().y - galley.size().y / 2.0), galley, t.text);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                open = Some(*id);
            }
        }
        ui.add_space(12.0);
    };
    list(ui, t!("Friends in this group").to_string(), &friends);
    list(ui, t!("Members").to_string(), &others);
    if let Some(id) = open {
        io.go(View::Page("profile".into(), Some(id.to_string())));
    }
}

// ---------- joining, leaving and invites ----------

/// What `--demo` pretends Steam now says, per group.
static DEMO: Mutex<Option<HashMap<SteamId, Clan>>> = Mutex::new(None);
/// Invites answered, until Steam's friends list says so too.
static ANSWERED: Mutex<Option<HashSet<SteamId>>> = Mutex::new(None);

/// Where you stand with a group.
pub fn clan_state(state: &State, id: SteamId) -> Clan {
    if super::is_demo() {
        if let Some(c) = DEMO.lock().unwrap().as_ref().and_then(|m| m.get(&id)) {
            return *c;
        }
    }
    state.clans.get(&id).copied().unwrap_or_default()
}

/// Groups that invited you and you haven't answered.
pub fn pending_invites(state: &State) -> Vec<SteamId> {
    let mut answered = ANSWERED.lock().unwrap();
    let answered = answered.get_or_insert_with(Default::default);
    answered.retain(|id| clan_state(state, *id) == Clan::Invited);
    let mut out: Vec<SteamId> = state.clans.keys().copied().filter(|id| clan_state(state, *id) == Clan::Invited && !answered.contains(id)).collect();
    out.sort_unstable();
    out
}

/// Accept or ignore an invite. The command to send (none in `--demo`,
/// where it only changes here).
pub fn answer_invite(id: SteamId, accept: bool) -> Option<Command> {
    if super::is_demo() {
        DEMO.lock().unwrap().get_or_insert_with(Default::default).insert(id, if accept { Clan::Member } else { Clan::None });
        return None;
    }
    ANSWERED.lock().unwrap().get_or_insert_with(Default::default).insert(id);
    Some(Command::Hub(Request::AnswerGroupInvite { group: id, accept }))
}

/// Steam didn't take the answer: the invite shows again.
pub(super) fn invite_failed(id: SteamId) {
    if let Some(a) = ANSWERED.lock().unwrap().as_mut() {
        a.remove(&id);
    }
}

/// A change sent to Steam, until the friends list shows it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Busy {
    from: Clan,
    since: f64,
    /// Answers that had come back before this one.
    sent: u32,
    /// Steam said yes; the friends list is still to come.
    taken: bool,
}

/// After Steam says yes, how long to wait for the friends list to show it.
const CONFIRM_SECS: f64 = 12.0;
/// How long to wait for any answer at all.
const ANSWER_SECS: f64 = 30.0;

/// What a change waiting on Steam comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Settled {
    Waiting,
    Done,
    Failed(String),
    NotConfirmed,
    NoAnswer,
}

fn settle(busy: &mut Busy, now_state: Clan, answer: Option<&(u32, Result<(), String>)>, now: f64) -> Settled {
    if now_state != busy.from {
        return Settled::Done;
    }
    if let Some((n, result)) = answer.filter(|(n, _)| *n > busy.sent) {
        busy.sent = *n;
        match result {
            Ok(()) => busy.taken = true,
            Err(e) => return Settled::Failed(e.clone()),
        }
    }
    let waited = now - busy.since;
    if busy.taken && waited > CONFIRM_SECS {
        Settled::NotConfirmed
    } else if waited > ANSWER_SECS {
        Settled::NoAnswer
    } else {
        Settled::Waiting
    }
}

/// Send a change, or in `--demo` make it here.
fn change(hv: &mut HubView, ui: &Ui, io: &mut Io, g: &hub::Group, to: Clan, cmd: Request) {
    hv.group_notice.remove(&g.id);
    if super::is_demo() {
        DEMO.lock().unwrap().get_or_insert_with(Default::default).insert(g.id, to);
        return;
    }
    let sent = io.state.hub.group_changed.get(&g.id).map_or(0, |(n, _)| *n);
    hv.group_busy.insert(g.id, Busy { from: clan_state(io.state, g.id), since: ui.input(|i| i.time), sent, taken: false });
    io.send(Command::Hub(cmd));
}

/// The group's button: join, ask to join, leave, answer an invite, or
/// where you stand when there's nothing to press.
fn membership(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, g: &hub::Group, key: &str) {
    if io.state.steam_id == 0 {
        return;
    }
    let now = ui.input(|i| i.time);
    let state_now = clan_state(io.state, g.id);
    if let Some(busy) = hv.group_busy.get_mut(&g.id) {
        match settle(busy, state_now, io.state.hub.group_changed.get(&g.id), now) {
            Settled::Waiting => {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::REFRESH), t!("Working…"), w::Kind::Secondary, w::Size::Sm));
                return;
            }
            Settled::Done => {}
            Settled::Failed(e) => {
                hv.group_notice.insert(g.id, i18n::tr_dyn(&e));
            }
            Settled::NotConfirmed => {
                hv.group_notice.insert(g.id, t!("Steam took it but hasn't updated your groups yet. Check again in a moment.").to_string());
            }
            Settled::NoAnswer => {
                hv.group_notice.insert(g.id, t!("Steam didn't answer. Try again.").to_string());
            }
        }
        hv.group_busy.remove(&g.id);
    }
    let join_key = if g.url.is_empty() { key.to_string() } else { g.url.clone() };
    let still = |ui: &mut Ui, icon: &str, label: &str| {
        ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icon), label, w::Kind::Secondary, w::Size::Sm));
    };
    // Right to left: the main action ends up nearest the edge.
    match group_button(state_now, g.join) {
        GroupButton::Join => {
            if w::button(ui, t, Some(icons::PLUS), t!("Join group"), w::Kind::Primary, w::Size::Sm).clicked() {
                change(hv, ui, io, g, Clan::Member, Request::JoinGroup { group: g.id, key: join_key });
            }
        }
        GroupButton::RequestToJoin => {
            if w::button(ui, t, Some(icons::USER_PLUS), t!("Request to join"), w::Kind::Primary, w::Size::Sm).on_hover_text(t!("An officer of this group has to approve you.")).clicked() {
                change(hv, ui, io, g, Clan::PendingApproval, Request::JoinGroup { group: g.id, key: join_key });
            }
        }
        GroupButton::Leave => {
            if w::button(ui, t, Some(icons::SIGN_OUT), t!("Leave group"), w::Kind::Secondary, w::Size::Sm).clicked() {
                hv.leave_confirm = Some(g.id);
            }
            ui.label(w::body_s(t, format!("{}  {}", icons::CHECK, t!("You're a member")), t.play));
        }
        GroupButton::Invited => {
            if w::button(ui, t, Some(icons::X), t!("Ignore"), w::Kind::Ghost, w::Size::Sm).clicked() {
                change(hv, ui, io, g, Clan::None, Request::AnswerGroupInvite { group: g.id, accept: false });
            }
            if w::button(ui, t, Some(icons::CHECK), t!("Accept invite"), w::Kind::Primary, w::Size::Sm).clicked() {
                change(hv, ui, io, g, Clan::Member, Request::AnswerGroupInvite { group: g.id, accept: true });
            }
            ui.label(w::body_s(t, t!("You're invited to this group"), t.accent));
        }
        GroupButton::InviteOnly => still(ui, icons::LOCK_SIMPLE, t!("Invitation only")),
        GroupButton::RequestPending => still(ui, icons::CLOCK, t!("Request sent")),
        GroupButton::RequestDenied => still(ui, icons::X, t!("Request declined")),
        GroupButton::Removed => still(ui, icons::X, t!("Removed from this group")),
        GroupButton::Blocked => still(ui, icons::LOCK_SIMPLE, t!("Blocked from this group")),
    }
}

/// "Leave this group?"
fn leave_dialog(hv: &mut HubView, ui: &Ui, t: &Theme, io: &mut Io, g: &hub::Group) {
    if hv.leave_confirm != Some(g.id) {
        return;
    }
    let (mut leave, mut cancel) = (false, false);
    egui::Modal::new(egui::Id::unique(("leave-steam-group", g.id))).show(ui.ctx(), |ui| {
        ui.set_width((ui.ctx().content_rect().width() - 64.0).clamp(240.0, 420.0));
        ui.add(egui::Label::new(w::heading_s(t, t!("Leave {name}?", name = g.name), t.text)).wrap());
        let why = if g.join == JoinKind::InviteOnly {
            t!("This group is by invitation only: you'd need a new invite to come back.")
        } else if g.join == JoinKind::Request {
            t!("To come back you'd have to ask to join again.")
        } else {
            t!("You can join it again at any time.")
        };
        ui.add(egui::Label::new(w::body_s(t, why, t.text_dim)).wrap());
        ui.add_space(t.spacing.md);
        ui.horizontal(|ui| {
            leave = w::button(ui, t, Some(icons::SIGN_OUT), t!("Leave group"), w::Kind::Danger, w::Size::Sm).clicked();
            cancel = w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked();
        });
    });
    if leave {
        change(hv, ui, io, g, Clan::None, Request::LeaveGroup { group: g.id });
    }
    if leave || cancel {
        hv.leave_confirm = None;
    }
}

/// The title over a group's forum, with the way back to its page.
pub(super) fn forum_header(ui: &mut Ui, t: &Theme, io: &mut Io, key: &str) {
    ui.add_space(t.spacing.xl);
    let name = match io.state.hub.groups.get(key) {
        Some(Ok(g)) => g.name.clone(),
        _ => String::new(),
    };
    ui.horizontal(|ui| {
        if w::button(ui, t, Some(icons::CARET_LEFT), t!("Group page"), w::Kind::Ghost, w::Size::Sm).clicked() {
            io.go(View::Group(key.to_string()));
        }
        if !name.is_empty() {
            ui.label(w::heading_m(t, name, t.text));
        }
    });
    ui.add_space(t.spacing.md);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn busy(from: Clan) -> Busy {
        Busy { from, since: 100.0, sent: 2, taken: false }
    }

    #[test]
    fn a_change_ends_when_the_friends_list_shows_it() {
        let mut b = busy(Clan::None);
        assert_eq!(settle(&mut b, Clan::None, None, 101.0), Settled::Waiting);
        // An old answer from before this change doesn't count.
        assert_eq!(settle(&mut b, Clan::None, Some(&(2, Err("old".into()))), 101.0), Settled::Waiting);
        // Steam said yes, the list hasn't caught up: keep waiting a while.
        assert_eq!(settle(&mut b, Clan::None, Some(&(3, Ok(()))), 102.0), Settled::Waiting);
        assert!(b.taken);
        assert_eq!(settle(&mut b, Clan::Member, Some(&(3, Ok(()))), 103.0), Settled::Done);
    }

    #[test]
    fn an_answered_invite_stays_gone_unless_steam_refuses() {
        const ID: SteamId = 103582791429521999;
        let mut state = State::default();
        state.clans.insert(ID, Clan::Invited);
        assert_eq!(pending_invites(&state), vec![ID]);
        let Some(Command::Hub(Request::AnswerGroupInvite { group, accept })) = answer_invite(ID, true) else {
            panic!("the answer should go to Steam");
        };
        assert_eq!((group, accept), (ID, true));
        assert!(pending_invites(&state).is_empty());
        invite_failed(ID);
        assert_eq!(pending_invites(&state), vec![ID]);
    }

    #[test]
    fn a_change_says_when_it_went_wrong() {
        let mut b = busy(Clan::Member);
        assert_eq!(settle(&mut b, Clan::Member, Some(&(3, Err("Nope".into()))), 101.0), Settled::Failed("Nope".into()));
        let mut b = busy(Clan::None);
        assert_eq!(settle(&mut b, Clan::None, Some(&(3, Ok(()))), 100.0 + CONFIRM_SECS + 1.0), Settled::NotConfirmed);
        let mut b = busy(Clan::None);
        assert_eq!(settle(&mut b, Clan::None, None, 100.0 + ANSWER_SECS + 1.0), Settled::NoAnswer);
    }
}

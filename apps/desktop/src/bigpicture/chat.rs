//! Big Picture's chat: the normal window's messages (emoticons, reactions,
//! spoilers, code, quotes, pictures, link cards), for friends and group
//! chats, driven by a controller, the keyboard or the mouse.
//!
//! Up and down pick a message and A opens what you can do with it: reply,
//! react, copy, show a spoiler, open a picture or a link, join a game,
//! report. Beside the message box: "+" (a picture, your latest screenshot,
//! your trade link), the emoticon picker and the slash commands.

use super::focus::{Dir, Focus, Place, Scroller, ease};
use super::input::{Device, Press};
use super::paint::{self, Kind};
use super::{BigPicture, Frame, Page, screens};
use crate::builtin::chat::{self, ChatUi, Msg, PickerTab};
use client::{ChatTarget, Command};
use egui::{Align, Align2, Color32, Id, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use extensions::Action;
use i18n::t;
use model::chat::Part;
use model::{AppId, SteamId, art};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use ui::{icons, widgets as w};

/// A message: its time and ordinal.
type MsgKey = (u64, u32);

/// What Big Picture's chat remembers.
#[derive(Default)]
pub struct ChatState {
    mem: ChatUi,
    target: Option<ChatTarget>,
    /// What was being typed in the other conversations.
    drafts: HashMap<ChatTarget, String>,
    /// The message picked with up/down.
    selected: Option<MsgKey>,
    scroll_to_selected: bool,
    /// Up/down waiting to move the selection.
    step: i32,
    /// Where each message was last frame (for the mouse).
    rows: HashMap<MsgKey, Rect>,
    at_first: bool,
    at_last: bool,
    has_older: bool,
    want_older: bool,
    /// The on-screen keyboard is up.
    pub keys_up: bool,
    /// Tab on a real keyboard: complete the slash command.
    pub complete: bool,
    /// The list of slash commands is showing, and one of its rows has focus.
    pub slash_open: bool,
    pub slash_focused: bool,
    /// Replying to: (who, what they said, the quote to send).
    reply: Option<(String, String, String)>,
    asked: HashSet<ChatTarget>,
    acked: HashMap<(u64, u64), u32>,
    emoticons_asked: bool,
    pub groups_asked: bool,
    was_empty: bool,
    pub sheet: Option<Sheet>,
    /// Your own screenshots, while choosing one to send.
    thumbs: Thumbs,
    /// The normal-window page showing inside Big Picture.
    embedded: Option<(String, Option<String>)>,
}

/// The conversation's history (up/down pick a message while it has focus).
pub fn history_id() -> Id {
    Id::unique("bp-chat-history")
}

/// The message box (selecting it brings up the keyboard).
pub fn input_id() -> Id {
    Id::unique("bp-chat-input")
}

fn slash_id(i: usize) -> Id {
    Id::unique(("bp-chat-slash", i))
}

impl ChatState {
    /// Up or down on the history: pick another message. Past the newest,
    /// on to the message box; past the oldest, load older ones or move on.
    /// True when the move was used here.
    pub fn on_move(&mut self, focus: &mut Option<Id>, dir: Dir) -> bool {
        if *focus != Some(history_id()) {
            return false;
        }
        match dir {
            Dir::Up if !self.at_first => self.step -= 1,
            Dir::Up if self.has_older => self.want_older = true,
            Dir::Down if !self.at_last => self.step += 1,
            Dir::Down => *focus = Some(input_id()),
            _ => return false,
        }
        true
    }

    /// Enter on a real keyboard sends, unless a message or a list row is
    /// what's selected (there it selects, like everywhere else).
    pub fn enter_sends(&self, focus: Option<Id>) -> bool {
        focus != Some(history_id()) && !self.slash_focused
    }
}

/// Commands go to Steam; the demo answers itself and sends nothing.
struct Outbox<'a> {
    client: &'a client::Client,
    demo: Option<&'a std::sync::mpsc::Sender<client::Event>>,
    me: SteamId,
}

impl chat::Sender for Outbox<'_> {
    fn send(&self, cmd: Command) {
        match self.demo {
            Some(tx) => crate::demo::echo(tx, self.me, cmd),
            None => self.client.send(cmd),
        }
    }
}

/// Send a command to Steam (in the demo, it answers itself).
pub(super) fn send(f: &Frame, cmd: Command) {
    chat::Sender::send(&Outbox { client: &f.app.client, demo: f.app.demo_sender(), me: f.app.state.steam_id }, cmd);
}

fn with_env<R>(f: &Frame, g: impl FnOnce(&chat::Env) -> R) -> R {
    let outbox = Outbox { client: &f.app.client, demo: f.app.demo_sender(), me: f.app.state.steam_id };
    let env = chat::Env { state: &f.app.state, client: &outbox, steam_root: Some(&f.app.local.steam_root), language: i18n::language().steam_name().to_string() };
    g(&env)
}

fn messages(f: &Frame, to: ChatTarget) -> Vec<Msg> {
    let state = &f.app.state;
    let all: Vec<Msg> = match to {
        ChatTarget::Friend(id) => state.chats.get(&id).map(|c| c.iter().map(Msg::friend).collect()).unwrap_or_default(),
        ChatTarget::Group { group, room } => state.group_messages.get(&(group, room)).map(|c| c.iter().map(Msg::group).collect()).unwrap_or_default(),
    };
    all.into_iter().filter(|m| !m.raw.trim().is_empty() || m.invite.is_some()).collect()
}

/// Opening a conversation: its draft, its history, and marking it read.
fn enter(bp: &mut BigPicture, f: &mut Frame, to: ChatTarget, msgs: &[Msg]) {
    let st = &mut bp.chat;
    if st.target != Some(to) {
        if let Some(old) = st.target {
            st.drafts.insert(old, std::mem::take(&mut bp.message));
        }
        bp.message = st.drafts.remove(&to).unwrap_or_default();
        st.target = Some(to);
        st.selected = None;
        st.reply = None;
        st.rows.clear();
        st.was_empty = bp.message.is_empty();
    }
    let first_visit = st.asked.insert(to);
    let ask_emoticons = f.app.state.emoticons.is_none() && !std::mem::replace(&mut st.emoticons_asked, true);
    with_env(f, |env| {
        if first_visit {
            env.client.send(match to {
                ChatTarget::Friend(friend) => Command::LoadHistory { friend },
                ChatTarget::Group { group, room } => Command::LoadGroupHistory { group, room, before: None },
            });
        }
        if ask_emoticons {
            env.client.send(Command::LoadEmoticons);
        }
    });
    match to {
        ChatTarget::Friend(id) => {
            if f.app.state.unread.get(&id).is_some_and(|n| *n > 0) {
                f.actions.push(Action::MarkRead(id));
            }
        }
        ChatTarget::Group { group, room } => {
            if let Some(last) = msgs.last().map(|m| m.time as u32) {
                if st.acked.insert((group, room), last) != Some(last) {
                    with_env(f, |env| env.client.send(Command::AckGroupRoom { group, room, time: last }));
                }
            }
        }
    }
}

/// Who you're talking to; in a group, its rooms too. Returns where it ends.
fn header(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, to: ChatTarget) -> f32 {
    let s = f.s;
    let (left, right) = (f.view.left() + f.pad, f.view.right() - f.pad);
    let p = ui.painter().clone();
    let av = Rect::from_min_size(pos2(left, f.view.top() + 6.0 * s), vec2(64.0 * s, 64.0 * s));
    let tx = av.right() + 22.0 * s;
    let (name, status, color) = match to {
        ChatTarget::Friend(id) => {
            let fr = f.app.state.persona(id);
            if !w::paint_cover(ui, av, &art::avatar_full(fr.avatar_hash.as_deref()), paint::radius(32.0 * s)) {
                p.circle_filled(av.center(), 32.0 * s, f.t.raised);
            }
            let color = crate::shell::status_color(&f.t, &fr);
            p.circle_stroke(av.center(), 34.0 * s, egui::Stroke::new(3.0 * s, color));
            let status = if f.app.state.is_typing(id) { t!("typing…").to_string() } else { crate::shell::status_text(&f.app.state, &fr) };
            (fr.display_name(), status, color)
        }
        ChatTarget::Group { group, .. } => {
            let g = f.app.state.chat_groups.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|g| g.id == group)).cloned();
            let name = g.as_ref().map(|g| crate::builtin::friends::group_name(&f.app.state, g)).unwrap_or_default();
            if let Some(g) = &g {
                crate::builtin::friends::paint_group_avatar(ui, &f.t, g, &name, av);
            }
            let members = f.app.state.chat_group_members.get(&group).map(|m| m.len()).unwrap_or(0);
            let mut sub = t!("{n} members", n = members);
            if let Some(tag) = g.as_ref().map(|g| g.tagline.as_str()).filter(|t| !t.is_empty()) {
                sub = format!("{sub} · {tag}");
            }
            (name, sub, f.t.text_dim)
        }
    };
    paint::line(&p, pos2(tx, av.center().y - 2.0 * s), Align2::LEFT_BOTTOM, &name, w::font_bold(32.0 * s), f.t.text, (right - tx) * 0.55);
    paint::line(&p, pos2(tx, av.center().y + 4.0 * s), Align2::LEFT_TOP, &status, w::font_medium(20.0 * s), color, (right - tx) * 0.55);
    // A group's rooms, at the right (voice rooms aren't supported yet).
    if let ChatTarget::Group { group, room } = to {
        let rooms: Vec<(u64, String, bool)> = f
            .app
            .state
            .chat_groups
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|l| l.iter().find(|g| g.id == group))
            .map(|g| g.rooms.iter().filter(|r| !r.voice).map(|r| (r.id, if r.name.is_empty() { t!("general").to_string() } else { r.name.clone() }, r.unread)).collect())
            .unwrap_or_default();
        if rooms.len() > 1 {
            let mut x = right;
            for (id, label, unread) in rooms.iter().rev() {
                let text = format!("# {label}");
                let wd = paint::text_width(&p, &text, w::font_semibold(21.0 * s)) + 40.0 * s;
                x -= wd;
                let r = Rect::from_min_size(pos2(x, av.center().y - 26.0 * s), vec2(wd, 52.0 * s));
                x -= 12.0 * s;
                let hit = bp.focus.item(ui, Id::unique(("bp-chat-room", *id)), r, ui.clip_rect(), Place::Page);
                let on = *id == room;
                let fill = if on { f.t.accent.gamma_multiply(0.75) } else { w::mix(Color32::from_black_alpha(110), Color32::from_white_alpha(30), hit.glow) };
                p.rect_filled(r, paint::radius(26.0 * s), fill);
                p.text(r.center(), Align2::CENTER_CENTER, text, w::font_semibold(21.0 * s), f.t.text);
                if *unread && !on {
                    p.circle_filled(r.right_top() + vec2(-8.0 * s, 8.0 * s), 6.0 * s, f.t.accent);
                }
                if hit.focused {
                    bp.focus.ring_around(r, 26.0 * s);
                }
                if hit.activated && !on {
                    let keep = bp.focus.current;
                    if let Some(top) = bp.stack.last_mut() {
                        *top = Page::Chat(ChatTarget::Group { group, room: *id });
                    }
                    bp.focus.current = keep;
                }
            }
        }
    }
    av.bottom() + 16.0 * s
}

/// A round button beside the message box.
fn round_button(bp: &mut BigPicture, ui: &Ui, f: &Frame, salt: &str, r: Rect, icon: &str) -> bool {
    let s = f.s;
    let hit = bp.focus.item(ui, Id::unique(("bp-chat-tool", salt)), r, ui.clip_rect(), Place::Page);
    let shown = paint::grow(r, 1.0 + 0.05 * hit.glow);
    ui.painter().circle_filled(shown.center(), shown.width() / 2.0, w::mix(f.t.raised, f.t.accent, hit.glow * 0.7));
    if icon == "/" {
        ui.painter().text(shown.center(), Align2::CENTER_CENTER, "/", w::font_bold(32.0 * s), f.t.text);
    } else {
        w::icon_centered(ui.painter(), shown.center(), icon, 32.0 * s, f.t.text);
    }
    if hit.focused {
        bp.focus.ring_around(shown, shown.width() / 2.0);
    }
    hit.activated
}

/// The whole chat page.
pub fn page(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, to: ChatTarget) -> f32 {
    let s = f.s;
    let (left, right) = (f.view.left() + f.pad, f.view.right() - f.pad);
    let msgs = messages(f, to);
    enter(bp, f, to, &msgs);
    if bp.chat.sheet.is_none() && !bp.chat.thumbs.asked.is_empty() && bp.chat.thumbs.running.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        bp.chat.thumbs = Thumbs::default();
    }
    let head_bottom = header(bp, ui, f, to);
    let p = ui.painter().clone();
    let mut out: Vec<Action> = Vec::new();

    // The keyboard slides up while the message box or one of its keys is focused.
    let want_keys = bp.focus.current == Some(input_id()) || screens::is_key(bp.focus.current);
    bp.chat.keys_up = want_keys;
    let k = ease(ui.ctx().animate_bool_with_time(Id::unique("bp-chat-keys"), want_keys, 0.25));
    let unit = 84.0 * s;
    let keys_w = 11.0 * unit - 10.0 * s;
    let keys_h = 5.0 * unit + 40.0 * s;
    let panel_top = f.view.bottom() - keys_h * k;
    let mut send = std::mem::take(&mut bp.send_now);
    if k > 0.01 {
        let screen = ui.max_rect();
        let panel = Rect::from_min_max(pos2(screen.left(), panel_top), pos2(screen.right(), f.view.bottom() + keys_h));
        p.rect_filled(panel, 0.0, f.t.chrome.gamma_multiply(0.96));
        let edge = Rect::from_min_size(panel.min, vec2(panel.width(), 2.0 * s));
        w::gradient(&p, edge, f.t.accent.gamma_multiply(0.0), f.t.accent.gamma_multiply(0.8), true);
        let mut ky = panel_top + 26.0 * s;
        match screens::keyboard(bp, ui, f, f.view.center().x - keys_w / 2.0, &mut ky, t!("Send")) {
            Some(screens::Key::Char(ch)) => bp.message.push(ch),
            Some(screens::Key::Del) => {
                bp.message.pop();
            }
            Some(screens::Key::Go) => send = true,
            None => {}
        }
    }
    if f.x {
        bp.message.pop();
    }
    if f.y {
        bp.message.push(' ');
    }

    // The tools, the message box (it grows to four lines, then keeps the
    // end of the text in view) and Send.
    let bottom = panel_top - 20.0 * s;
    let tool = 72.0 * s;
    let tool_gap = 10.0 * s;
    let mut open = None;
    for (i, (salt, icon, kind)) in [("attach", icons::PLUS, SheetKind::Attach), ("emoticons", icons::SMILEY, SheetKind::Picker), ("commands", "/", SheetKind::Commands)].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(left + i as f32 * (tool + tool_gap), bottom - tool), vec2(tool, tool));
        if round_button(bp, ui, f, salt, r, icon) {
            open = Some(kind);
        }
    }
    let field_left = left + 3.0 * (tool + tool_gap) + 4.0 * s;
    let send_label = t!("Send");
    let send_w = paint::pill_width(ui, true, send_label, s);
    let field_w = right - field_left - send_w - 18.0 * s;
    let font = w::font_semibold(26.0 * s);
    let text_w = (field_w - 64.0 * s).max(40.0 * s);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(egui::text::LayoutJob::simple(bp.message.clone(), font.clone(), f.t.text, text_w)));
    let line_h = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let lines = (galley.size().y / line_h).round().clamp(1.0, 4.0);
    let field_h = (lines * line_h + 34.0 * s).max(tool);
    let field = Rect::from_min_max(pos2(field_left, bottom - field_h), pos2(field_left + field_w, bottom));
    let hit = bp.focus.item(ui, input_id(), field, ui.clip_rect(), Place::Page);
    let radius = (36.0 * s).min(field.height() / 2.0);
    p.rect_filled(field, paint::radius(radius), Color32::from_black_alpha(130));
    let stroke = w::mix(f.t.accent.gamma_multiply(0.45), f.t.accent, hit.glow);
    p.rect_stroke(field, paint::radius(radius), egui::Stroke::new((2.0 + hit.glow) * s, stroke), egui::StrokeKind::Inside);
    if hit.focused {
        bp.focus.ring_around(field, radius);
    }
    let inner = field.shrink2(vec2(32.0 * s, 17.0 * s));
    let hint = match to {
        ChatTarget::Friend(id) => t!("Message {name}", name = f.app.state.persona(id).display_name()),
        ChatTarget::Group { .. } => t!("Message the group").to_string(),
    };
    let caret = if bp.message.is_empty() {
        paint::line(&p, pos2(inner.left(), inner.bottom() - line_h / 2.0), Align2::LEFT_CENTER, &hint, w::font(26.0 * s), f.t.text_faint, inner.width());
        Rect::from_min_size(pos2(inner.left(), inner.bottom() - line_h), vec2(0.0, line_h))
    } else {
        let at = pos2(inner.left(), inner.bottom() - galley.size().y);
        let end = galley.pos_from_cursor(egui::text::CCursor::new(bp.message.chars().count()));
        p.with_clip_rect(inner.expand2(vec2(4.0 * s, 0.0))).galley(at, galley, f.t.text);
        end.translate(at.to_vec2())
    };
    let blink = ((f.now * 3.0).sin() as f32 * 0.5 + 0.5).powf(0.6);
    p.vline(caret.right() + 3.0 * s, (caret.top() + 3.0 * s)..=(caret.bottom() - 3.0 * s), egui::Stroke::new(3.0 * s, f.t.accent.gamma_multiply(blink)));
    if hit.activated {
        bp.focus.current = Some(Id::unique(("bp-key", 1, 0)));
    }
    let empty_box = bp.message.trim().is_empty();
    let (_, send_hit) = screens::button(bp, ui, f, Id::unique("bp-chat-send"), pos2(field.right() + 18.0 * s, bottom - tool), send_w, Some(icons::SEND), send_label, if empty_box { Kind::Secondary } else { Kind::Primary }, None);
    send |= send_hit.activated;
    bp.preferred = Some(input_id());

    // Tell a friend you're typing, when you start.
    if let ChatTarget::Friend(friend) = to {
        if bp.chat.was_empty && !bp.message.is_empty() {
            with_env(f, |env| env.client.send(Command::Typing { to: friend }));
        }
    }
    bp.chat.was_empty = bp.message.is_empty();

    // Slash commands: the list over the box while typing one.
    let focus_now = bp.focus.current;
    let on_list = (0..8).any(|i| focus_now == Some(slash_id(i)));
    let typing_here = focus_now == Some(input_id()) || screens::is_key(focus_now) || on_list;
    let rows = if typing_here {
        let (mem, message) = (&mut bp.chat.mem, &bp.message);
        with_env(f, |env| chat::slash_rows(ui.ctx(), env, message, mem))
    } else {
        Vec::new()
    };
    bp.chat.slash_open = !rows.is_empty();
    bp.chat.slash_focused = on_list;
    let complete = std::mem::take(&mut bp.chat.complete);
    if (send || complete) && !on_list {
        if let Some(row) = rows.first().filter(|r| r.text.is_some() || r.game.is_some()) {
            // A whole command with its space already typed has no row, so
            // this only ever finishes a command or picks a game.
            let message = &mut bp.message;
            with_env(f, |env| chat::use_slash_row(env, to, message, row));
            send = false;
        }
    }
    if complete {
        send = false;
    }

    // Sending.
    if send && !bp.message.trim().is_empty() {
        if let Some((_, _, quote)) = bp.chat.reply.take() {
            bp.message = format!("{quote}{}", bp.message);
        }
        let (mem, message) = (&mut bp.chat.mem, &mut bp.message);
        with_env(f, |env| chat::send(env, to, message, mem, &mut out));
        bp.chat.selected = None;
    }
    {
        let mem = &mut bp.chat.mem;
        with_env(f, |env| chat::share_trade_when_ready(env, to, mem, &mut out));
    }

    // Replying: what's being quoted, just above the box.
    let mut top_of_box = field.top();
    if let Some((name, said, _)) = bp.chat.reply.clone() {
        let h = 54.0 * s;
        let r = Rect::from_min_max(pos2(field.left(), field.top() - h - 8.0 * s), pos2(field.right(), field.top() - 8.0 * s));
        p.rect_filled(r, paint::radius(12.0 * s), Color32::from_black_alpha(130));
        p.rect_filled(Rect::from_min_size(r.min, vec2(4.0 * s, r.height())), 2.0, f.t.accent_2);
        let x_rect = Rect::from_center_size(pos2(r.right() - 30.0 * s, r.center().y), vec2(40.0 * s, 40.0 * s));
        paint::line(&p, pos2(r.left() + 18.0 * s, r.center().y), Align2::LEFT_CENTER, &t!("Replying to {name}: {text}", name = name, text = said.replace('\n', " ")), w::font(19.0 * s), f.t.text_dim, r.width() - 80.0 * s);
        let hit = bp.focus.item(ui, Id::unique("bp-chat-noreply"), x_rect, ui.clip_rect(), Place::Page);
        w::icon_centered(&p, x_rect.center(), icons::X, 22.0 * s, f.t.text);
        if hit.focused {
            bp.focus.ring_around(x_rect, 20.0 * s);
        }
        if hit.activated {
            bp.chat.reply = None;
            bp.focus.current = Some(input_id());
        }
        top_of_box = r.top();
    }

    // The history fills the rest.
    let area = Rect::from_min_max(pos2(left, head_bottom), pos2(right, top_of_box - 16.0 * s));
    if area.height() > 40.0 * s {
        history(bp, ui, f, to, &msgs, area, &mut out);
    }

    // The list goes over the history.
    if !rows.is_empty() {
        let row_h = 74.0 * s;
        let shown = rows.len().min(6);
        let w_ = field.width().min(820.0 * s);
        let list = Rect::from_min_size(pos2(field.left(), top_of_box - 10.0 * s - row_h * shown as f32 - 16.0 * s), vec2(w_, row_h * shown as f32 + 16.0 * s));
        p.rect_filled(list, paint::radius(14.0 * s), f.t.raised);
        for (i, row) in rows.iter().take(shown).enumerate() {
            let r = Rect::from_min_size(list.min + vec2(8.0 * s, 8.0 * s + i as f32 * row_h), vec2(list.width() - 16.0 * s, row_h));
            let hit = bp.focus.item(ui, slash_id(i), r, ui.clip_rect(), Place::Page);
            if hit.glow > 0.0 {
                p.rect_filled(r, paint::radius(10.0 * s), f.t.accent.gamma_multiply(0.35 * hit.glow));
            }
            paint::line(&p, pos2(r.left() + 16.0 * s, r.center().y - 2.0 * s), Align2::LEFT_BOTTOM, &row.label, w::font_semibold(23.0 * s), f.t.text, r.width() - 32.0 * s);
            paint::line(&p, pos2(r.left() + 16.0 * s, r.center().y + 2.0 * s), Align2::LEFT_TOP, &row.help, w::font(17.0 * s), f.t.text_dim, r.width() - 32.0 * s);
            if hit.focused {
                bp.focus.ring_around(r, 10.0 * s);
            }
            if hit.activated {
                let message = &mut bp.message;
                with_env(f, |env| chat::use_slash_row(env, to, message, row));
                bp.focus.current = Some(input_id());
            }
        }
    }

    if let Some(kind) = open {
        bp.chat.sheet = Some(Sheet::new(kind, f.now));
    }
    if let Some(src) = bp.chat.mem.take_viewing() {
        bp.chat.sheet = Some(Sheet::new(SheetKind::Viewer(src), f.now));
    }
    for a in out {
        route(bp, f, ui.ctx(), a);
    }
    f.view.height()
}

/// The messages, with the normal chat's own drawing, sized for the TV.
#[allow(clippy::too_many_arguments)]
fn history(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, to: ChatTarget, msgs: &[Msg], area: Rect, out: &mut Vec<Action>) {
    let s = f.s;
    let hist = bp.focus.quiet_item(ui, history_id(), area, Place::Page);
    ui.painter().rect_filled(area, paint::radius(18.0 * s), Color32::from_black_alpha(90));

    // Which message is picked.
    let st = &mut bp.chat;
    if hist.focused && !msgs.is_empty() {
        let at = st.selected.and_then(|k| msgs.iter().position(|m| m.key() == k));
        let was = at.unwrap_or(msgs.len() - 1);
        let now = (was as i32 + st.step).clamp(0, msgs.len() as i32 - 1) as usize;
        if at != Some(now) {
            st.selected = Some(msgs[now].key());
            st.scroll_to_selected = true;
        }
        st.at_first = now == 0;
        st.at_last = now + 1 == msgs.len();
    } else {
        st.at_first = true;
        st.at_last = true;
    }
    st.step = 0;
    if let ChatTarget::Group { group, room } = to {
        st.has_older = f.app.state.group_more.get(&(group, room)) == Some(&true);
        if std::mem::take(&mut st.want_older) && st.has_older {
            if let Some(first) = msgs.first() {
                let before = Some((first.time as u32, first.ordinal));
                with_env(f, |env| env.client.send(Command::LoadGroupHistory { group, room, before }));
            }
        }
    } else {
        st.has_older = false;
        st.want_older = false;
    }

    if msgs.is_empty() {
        ui.painter().text(area.center(), Align2::CENTER_CENTER, t!("No messages yet"), w::font(24.0 * s), f.t.text_faint);
    }
    let tv = f.t.for_tv((1.55 * s).max(1.0));
    let inner = area.shrink2(vec2(20.0 * s, 8.0 * s));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(inner.intersect(ui.clip_rect()));
    child.style_mut().interaction.selectable_labels = false;
    let selected = st.selected.filter(|_| hist.focused);
    let mut scroll = std::mem::take(&mut st.scroll_to_selected);
    let before = std::mem::take(&mut st.rows);
    let mut rows = HashMap::with_capacity(msgs.len());
    let mut clicked = None;
    let mut picked_rect = None;
    let mem = &mut st.mem;
    with_env(f, |env| {
        egui::ScrollArea::vertical().id_salt(("bp-chat", to)).auto_shrink([false, false]).stick_to_bottom(true).show(&mut child, |ui| {
            ui.set_width(ui.available_width());
            let mut last: Option<(SteamId, u64)> = None;
            for m in msgs {
                let key = m.key();
                // Under the message, so its links and spoilers get their clicks.
                if let Some(r) = before.get(&key) {
                    if ui.interact(*r, Id::unique(("bp-chat-msg", key)), Sense::click()).clicked() {
                        clicked = Some(key);
                    }
                }
                let shade = ui.painter().add(egui::Shape::Noop);
                let first = last.is_none_or(|(from, time)| from != m.from || m.time.saturating_sub(time) > 300);
                let r = chat::message_row(ui, &tv, env, to, m, first, mem, out);
                last = Some((m.from, m.time));
                rows.insert(key, r);
                if selected == Some(key) {
                    ui.painter().set(shade, egui::Shape::rect_filled(r, paint::radius(10.0 * s), f.t.accent.gamma_multiply(0.16)));
                    picked_rect = Some(r);
                    if std::mem::take(&mut scroll) {
                        ui.scroll_to_rect(r, Some(Align::Center));
                    }
                }
            }
        });
    });
    let st = &mut bp.chat;
    st.rows = rows;
    if let Some(r) = picked_rect {
        let shown = r.intersect(inner);
        if shown.is_positive() {
            bp.focus.ring_around(shown, 10.0 * s);
        }
    }
    let open = if hist.activated { bp.chat.selected.filter(|k| msgs.iter().any(|m| m.key() == *k)) } else { None };
    if let Some(key) = clicked {
        bp.focus.current = Some(history_id());
        bp.chat.selected = Some(key);
        bp.chat.sheet = Some(Sheet::new(SheetKind::Actions(key), f.now));
    } else if let Some(key) = open {
        bp.chat.sheet = Some(Sheet::new(SheetKind::Actions(key), f.now));
    }
}

// ---------- links: inside Big Picture where it can show them ----------

/// Do what a message's card or link asks, in Big Picture when it has the
/// page; trades and anything else go to the normal window.
pub fn route(bp: &mut BigPicture, f: &mut Frame, ctx: &egui::Context, action: Action) {
    let Action::Navigate { page, arg } = action else {
        f.actions.push(action);
        return;
    };
    let now = f.now;
    let number = arg.as_deref().and_then(|a| a.strip_prefix("app:").unwrap_or(a).parse::<u64>().ok());
    match (page.as_str(), number) {
        ("store", Some(id)) => bp.open(Page::StoreApp(id as AppId), now),
        ("library", Some(id)) => bp.open(Page::Game(id as AppId), now),
        ("profile", Some(id)) if f.app.state.friends.contains_key(&id) => bp.open(Page::Friend(id), now),
        ("profile" | "hub" | "workshop" | "workshop_item" | "guide" | "guides" | "discussions" | "thread" | "achievements" | "user_files" | "group", _) => bp.open(Page::Embedded(page, arg), now),
        _ => {
            bp.leave(ctx);
            f.actions.push(Action::Navigate { page, arg });
        }
    }
}

/// A Workshop, community or profile page from the normal window, inside
/// Big Picture and sized for the TV. Up and down scroll it, B goes back.
pub fn embedded(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, page: &str, arg: Option<&str>) -> f32 {
    let s = f.s;
    let want = (page.to_string(), arg.map(String::from));
    if bp.chat.embedded.as_ref() != Some(&want) {
        f.app.registry.navigate(page, arg);
        bp.chat.embedded = Some(want);
    }
    let area = f.view.shrink2(vec2(f.pad * 0.5, 6.0 * s));
    let _ = bp.focus.quiet_item(ui, Id::unique("bp-embedded"), area, Place::Page);
    bp.focus.hide_ring();
    ui.painter().rect_filled(area, paint::radius(14.0 * s), f.t.bg);
    let tv = f.t.for_tv((1.3 * s).max(1.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    let mut acts = Vec::new();
    {
        let app = &mut *f.app;
        let env = extensions::Env { state: &app.state, local: &app.local, theme: &tv, zoom: app.prefs.zoom, client: &app.client, actions: &mut acts };
        if !app.registry.show_page(page, &mut child, env) {
            ui.painter().text(area.center(), Align2::CENTER_CENTER, t!("This page isn't available"), w::font(24.0 * s), f.t.text_dim);
        }
    }
    for a in acts {
        route(bp, f, ui.ctx(), a);
    }
    f.view.height()
}

// ---------- sheets: what opens over the chat ----------

#[derive(Clone)]
pub enum Outgoing {
    Picture(PathBuf),
    TradeLink,
}

#[derive(Clone)]
pub enum SheetKind {
    Actions(MsgKey),
    React(MsgKey),
    Picker,
    Attach,
    Commands,
    Pictures,
    Confirm(Outgoing),
    Report(MsgKey),
    Viewer(String),
}

/// A panel over the chat, with its own focus.
pub struct Sheet {
    kind: SheetKind,
    /// Where B goes back to.
    back: Vec<SheetKind>,
    focus: Focus,
    opened: f64,
    dir: Option<Dir>,
    press: bool,
    scroll: Scroller,
    tab: PickerTab,
    reason: i32,
}

impl Sheet {
    fn new(kind: SheetKind, now: f64) -> Self {
        Self { kind, back: Vec::new(), focus: Focus::default(), opened: now, dir: None, press: false, scroll: Scroller::default(), tab: PickerTab::default(), reason: 28 }
    }

    /// The presses while it's open. True when it should close.
    pub fn input(&mut self, presses: &[(Press, bool)]) -> bool {
        for (p, _) in presses {
            match p {
                Press::Move(d) => self.dir = Some(*d),
                Press::A => self.press = true,
                Press::B => match self.back.pop() {
                    Some(kind) => self.show(kind),
                    None => return true,
                },
                _ => {}
            }
        }
        false
    }

    fn show(&mut self, kind: SheetKind) {
        self.kind = kind;
        self.focus = Focus::default();
        self.scroll = Scroller::default();
    }
}

/// What a choice in a sheet does.
#[derive(Clone)]
enum Do {
    Open(SheetKind),
    Quote(MsgKey),
    React { key: MsgKey, name: String, sticker: bool, add: bool },
    Copy(String),
    Reveal(MsgKey),
    View(String),
    Link(String),
    Join(AppId, String),
    Reason(i32),
    Report(MsgKey),
    Close,
    Pick(String, bool),
    Emoji(&'static str),
    Effect(String),
    Command(&'static str),
    SendPicture(PathBuf),
    SendTrade,
    Browse,
    Tab(PickerTab),
}

/// One cell of a grid.
struct Cell {
    /// One of your own screenshots.
    file: Option<PathBuf>,
    url: Option<String>,
    glyph: Option<&'static str>,
    mine: bool,
    /// Fill the cell (screenshots) instead of fitting inside it (emoticons).
    cover: bool,
    act: Do,
}

/// Lays out a sheet's rows and grids top-down, scrolled.
struct Body<'a> {
    ui: Ui,
    focus: &'a mut Focus,
    thumbs: &'a mut Thumbs,
    s: f32,
    t: &'a ui::Theme,
    x: f32,
    width: f32,
    y: f32,
    picked: Option<Do>,
    first: Option<Id>,
}

impl Body<'_> {
    fn title(&mut self, text: &str) {
        self.ui.painter().text(pos2(self.x, self.y), Align2::LEFT_TOP, text, w::font_bold(30.0 * self.s), self.t.text);
        self.y += 52.0 * self.s;
    }

    fn note(&mut self, text: &str, rows: usize) {
        let r = paint::wrapped(self.ui.painter(), pos2(self.x, self.y), text, w::font(20.0 * self.s), self.t.text_dim, self.width, rows);
        self.y = r.bottom() + 18.0 * self.s;
    }

    fn row(&mut self, salt: &str, icon: &str, label: &str, tint: Option<Color32>, enabled: bool, act: Do) {
        let s = self.s;
        let id = Id::unique(("bp-sheet-row", salt));
        let r = Rect::from_min_size(pos2(self.x, self.y), vec2(self.width, 76.0 * s));
        self.y += 84.0 * s;
        let clip = self.ui.clip_rect();
        let hit = self.focus.item(&self.ui, id, r, clip, Place::Page);
        self.first.get_or_insert(id);
        let p = self.ui.painter();
        p.rect_filled(r, paint::radius(14.0 * s), w::mix(Color32::TRANSPARENT, Color32::from_white_alpha(22), hit.glow));
        let c = if !enabled { self.t.text_faint } else { tint.unwrap_or(self.t.text) };
        w::icon_centered(p, pos2(r.left() + 40.0 * s, r.center().y), icon, 28.0 * s, c);
        paint::line(p, pos2(r.left() + 78.0 * s, r.center().y), Align2::LEFT_CENTER, label, w::font_semibold(24.0 * s), c, r.width() - 96.0 * s);
        if hit.focused {
            self.focus.ring_around(r, 14.0 * s);
        }
        if hit.activated && enabled {
            self.picked = Some(act);
        }
    }

    fn grid(&mut self, salt: &str, cells: Vec<Cell>, size: egui::Vec2) {
        let s = self.s;
        let gap = 10.0 * s;
        let cols = (((self.width + gap) / (size.x + gap)).floor() as usize).max(1);
        let clip = self.ui.clip_rect();
        let n = cells.len();
        for (i, cell) in cells.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(self.x + (i % cols) as f32 * (size.x + gap), self.y + (i / cols) as f32 * (size.y + gap)), size);
            let id = Id::unique(("bp-sheet-cell", salt, i));
            let hit = self.focus.item(&self.ui, id, r, clip, Place::Page);
            self.first.get_or_insert(id);
            if r.bottom() < clip.top() || r.top() > clip.bottom() {
                continue;
            }
            let shown = paint::grow(r, 1.0 + 0.06 * hit.glow);
            let radius = paint::radius(10.0 * s);
            self.ui.painter().rect_filled(shown, radius, w::mix(self.t.surface, self.t.surface_hover, hit.glow));
            if let Some(path) = &cell.file {
                let ctx = self.ui.ctx().clone();
                match self.thumbs.get(&ctx, path) {
                    Some(tex) => paint_texture(&self.ui, shown, tex, cell.cover, radius),
                    None => w::skeleton(&self.ui, self.t, shown, radius),
                }
            }
            if let Some(url) = &cell.url {
                egui::Image::new(url.clone()).paint_at(&self.ui, shown.shrink(8.0 * s));
            }
            if let Some(g) = cell.glyph {
                self.ui.painter().text(shown.center(), Align2::CENTER_CENTER, g, w::font(size.y * 0.55), self.t.text);
            }
            if cell.mine {
                self.ui.painter().rect_stroke(shown, radius, egui::Stroke::new(2.5 * s, self.t.accent), egui::StrokeKind::Inside);
            }
            if hit.focused {
                self.focus.ring_around(shown, 10.0 * s);
            }
            if hit.activated {
                self.picked = Some(cell.act);
            }
        }
        self.y += n.div_ceil(cols) as f32 * (size.y + gap) + 8.0 * s;
    }
}

/// Your screenshots, decoded small on a background thread. The app has no
/// `file://` image loader on purpose (a chat message must never be able to
/// point a picture at a file or network share), so these are read here,
/// only for the files this screen listed or you picked.
#[derive(Default)]
struct Thumbs {
    ready: HashMap<PathBuf, egui::TextureHandle>,
    done: std::sync::Arc<std::sync::Mutex<Vec<(PathBuf, Option<egui::ColorImage>)>>>,
    asked: HashSet<PathBuf>,
    running: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Thumbs {
    fn get(&mut self, ctx: &egui::Context, path: &std::path::Path) -> Option<egui::load::SizedTexture> {
        use std::sync::atomic::Ordering::SeqCst;
        for (done, image) in self.done.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default() {
            if let Some(image) = image {
                let tex = ctx.load_texture(format!("bp-shot-{}", done.display()), image, egui::TextureOptions::LINEAR);
                self.ready.insert(done, tex);
            }
        }
        if let Some(tex) = self.ready.get(path) {
            return Some(egui::load::SizedTexture::from_handle(tex));
        }
        // Two at a time: a full folder shouldn't start a thread per picture.
        if !self.asked.contains(path) && self.running.load(SeqCst) < 2 {
            self.asked.insert(path.to_path_buf());
            self.running.fetch_add(1, SeqCst);
            let (path, done, running, ctx) = (path.to_path_buf(), self.done.clone(), self.running.clone(), ctx.clone());
            let spawned = std::thread::Builder::new().name("chat-picture".into()).spawn(move || {
                let image = decode_small(&path);
                if let Ok(mut d) = done.lock() {
                    d.push((path, image));
                }
                running.fetch_sub(1, SeqCst);
                ctx.request_repaint();
            });
            if spawned.is_err() {
                self.running.fetch_sub(1, SeqCst);
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        None
    }
}

/// A picture file, at most 960 pixels wide or tall.
fn decode_small(path: &std::path::Path) -> Option<egui::ColorImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 << 20);
    let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    reader.limits(limits);
    let img = reader.decode().ok()?.thumbnail(960, 960).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// Draw a texture filling `rect` (cropped) or fitting inside it.
fn paint_texture(ui: &Ui, rect: Rect, tex: egui::load::SizedTexture, cover: bool, radius: egui::CornerRadius) {
    let (ta, ra) = (tex.size.x / tex.size.y.max(1.0), rect.width() / rect.height().max(1.0));
    if cover {
        let uv = if ta > ra {
            let w = ra / ta;
            Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
        } else {
            let h = ta / ra;
            Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
        };
        egui::Image::from_texture(tex).uv(uv).corner_radius(radius).paint_at(ui, rect);
    } else {
        let k = (rect.width() / tex.size.x).min(rect.height() / tex.size.y);
        let target = Rect::from_center_size(rect.center(), tex.size * k);
        egui::Image::from_texture(tex).corner_radius(radius).paint_at(ui, target);
    }
}

/// The pictures in a message (inside quotes and spoilers too).
fn pictures(parts: &[Part], out: &mut Vec<String>) {
    for p in parts {
        match p {
            Part::Image { src, .. } => out.push(src.clone()),
            Part::Link { url, .. } if chat::is_image(url) => out.push(url.clone()),
            Part::Spoiler(i) | Part::Me(i) | Part::Quote { parts: i, .. } => pictures(i, out),
            _ => {}
        }
    }
}

fn emoticon_cell(name: &str, sticker: bool, mine: bool, act: Do) -> Cell {
    let url = if sticker { model::chat::sticker_url(name) } else { model::chat::emoticon_url(name, false) };
    Cell { file: None, url: Some(url), glyph: None, mine, cover: false, act }
}

/// Draw the open sheet and do what was chosen.
pub fn show_sheet(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, screen: Rect) {
    let Some(mut sh) = bp.chat.sheet.take() else { return };
    let Some(to) = bp.chat.target else { return };
    let s = f.s;
    let t = f.t.clone();
    let e = ease(((f.now - sh.opened) as f32 / 0.22).min(1.0));
    if e < 1.0 {
        ui.ctx().request_repaint();
    }
    let p = ui.painter().clone();
    let viewer = matches!(sh.kind, SheetKind::Viewer(_));
    p.rect_filled(screen, 0.0, Color32::from_black_alpha(((if viewer { 225.0 } else { 150.0 }) * e) as u8));
    let backdrop = ui.interact(screen, Id::unique("bp-sheet-backdrop"), Sense::click());
    let msgs = messages(f, to);

    // The large picture viewer has no panel.
    if let SheetKind::Viewer(src) = &sh.kind {
        sh.focus.begin(ui, None, false);
        let area = screen.shrink2(vec2(60.0 * s, 50.0 * s));
        let sized = crate::sized_images::inline_url(src, area.width(), area.height(), ui.ctx().pixels_per_point());
        let mut c = paint::faded(ui, screen, e);
        c.set_clip_rect(screen);
        if !w::paint_contain(&c, area.shrink2(vec2(0.0, 30.0 * s)), &sized, Align2::CENTER_CENTER) {
            w::skeleton(&c, &t, Rect::from_center_size(area.center(), vec2(640.0 * s, 360.0 * s)), paint::radius(12.0 * s));
        }
        paint::line(c.painter(), pos2(screen.center().x, screen.bottom() - 34.0 * s), Align2::CENTER_CENTER, t!("Press Back to close"), w::font_medium(20.0 * s), t.text_dim, screen.width());
        let press = std::mem::take(&mut sh.press);
        sh.dir = None;
        if !(press || backdrop.clicked()) {
            bp.chat.sheet = Some(sh);
        }
        return;
    }

    let width = 660.0 * s;
    let panel = Rect::from_min_size(pos2(screen.right() - width * e, screen.top()), vec2(width, screen.height()));
    paint::shadow(&p, panel, 0.0, s, 1.0, Color32::from_black_alpha(200));
    p.rect_filled(panel, 0.0, t.chrome);
    w::gradient(&p, Rect::from_min_size(panel.min, vec2(3.0 * s, panel.height())), t.accent, t.accent_2, false);
    let on_panel = ui.interact(panel, Id::unique("bp-sheet-panel"), Sense::click());
    let outside = backdrop.clicked() && !on_panel.hovered();

    let inner = panel.shrink2(vec2(36.0 * s, 44.0 * s));
    let dir = sh.dir.take();
    let press = std::mem::take(&mut sh.press);
    sh.focus.begin(ui, dir, press);
    let mut clip_ui = ui.new_child(UiBuilder::new().max_rect(inner));
    clip_ui.set_clip_rect(inner);
    let top = inner.top() - sh.scroll.pos;
    let (kind, tab, reason) = (sh.kind.clone(), sh.tab, sh.reason);
    let mut b = Body { ui: clip_ui, focus: &mut sh.focus, thumbs: &mut bp.chat.thumbs, s, t: &t, x: inner.left(), width: inner.width(), y: top, picked: None, first: None };
    let find = |key: &MsgKey| msgs.iter().find(|m| m.key() == *key);
    match &kind {
        SheetKind::Actions(key) => {
            if let Some(m) = find(key) {
                let name = chat::sender_name(&f.app.state, m);
                b.title(&name);
                b.note(&m.plain.replace('\n', " "), 3);
                b.row("quote", icons::CHAT, t!("Reply with a quote"), None, true, Do::Quote(*key));
                b.row("react", icons::SMILEY, t!("React"), None, true, Do::Open(SheetKind::React(*key)));
                b.row("copy", icons::COPY, t!("Copy"), None, true, Do::Copy(m.plain.clone()));
                if bp.chat.mem.has_hidden_spoiler(m) {
                    b.row("spoiler", icons::EYE, t!("Show spoiler"), None, true, Do::Reveal(*key));
                }
                let parts = model::chat::rich(&m.raw);
                let mut pics = Vec::new();
                pictures(&parts, &mut pics);
                for (i, src) in pics.into_iter().take(4).enumerate() {
                    b.row(&format!("pic{i}"), icons::IMAGE, t!("View picture"), None, true, Do::View(src));
                }
                let links: Vec<String> = model::chat::links(&parts).into_iter().filter(|u| !chat::is_image(u)).take(4).collect();
                for (i, url) in links.into_iter().enumerate() {
                    let (icon, label) = with_env(f, |env| chat::link_label(env, &url));
                    b.row(&format!("link{i}"), icon, &t!("Open: {name}", name = label), None, true, Do::Link(url));
                }
                if let Some(inv) = m.invite.as_ref().filter(|_| m.from != f.app.state.steam_id) {
                    let label = if inv.connect.is_empty() { t!("Play") } else { t!("Join game") };
                    b.row("join", icons::PLAY, label, Some(t.play), true, Do::Join(inv.app_id, inv.connect.clone()));
                }
                if m.from != f.app.state.steam_id {
                    b.row("report", icons::WARNING, t!("Report…"), Some(t.danger), true, Do::Open(SheetKind::Report(*key)));
                }
            } else {
                b.picked = Some(Do::Close);
            }
        }
        SheetKind::React(key) => {
            b.title(t!("React"));
            let mut cells: Vec<Cell> = Vec::new();
            let mut seen: Vec<String> = Vec::new();
            if let Some(m) = find(key) {
                for r in &m.reactions {
                    seen.push(r.name.clone());
                    cells.push(emoticon_cell(&r.name, r.sticker, r.mine, Do::React { key: *key, name: r.name.clone(), sticker: r.sticker, add: !r.mine }));
                }
            }
            let mem = &bp.chat.mem;
            let mine = with_env(f, |env| {
                let mut v = chat::picker_items(env, mem, PickerTab::Recent);
                v.retain(|(_, sticker)| !sticker);
                v.extend(chat::picker_items(env, mem, PickerTab::Emoticons));
                v
            });
            for (name, _) in mine {
                if !seen.contains(&name) {
                    seen.push(name.clone());
                    cells.push(emoticon_cell(&name, false, false, Do::React { key: *key, name: name.clone(), sticker: false, add: true }));
                }
            }
            if cells.is_empty() {
                b.note(t!("You don't have any emoticons yet. You get them from trading cards and the Points Shop."), 4);
            }
            b.grid("react", cells, vec2(84.0 * s, 84.0 * s));
        }
        SheetKind::Picker => {
            let tabs = [(PickerTab::Recent, icons::CLOCK), (PickerTab::Emoticons, icons::SMILEY), (PickerTab::Stickers, icons::IMAGE), (PickerTab::Effects, icons::SPARKLE)];
            let tw = (b.width - 30.0 * s) / 4.0;
            for (i, (which, icon)) in tabs.iter().enumerate() {
                let r = Rect::from_min_size(pos2(b.x + i as f32 * (tw + 10.0 * s), b.y), vec2(tw, 62.0 * s));
                let id = Id::unique(("bp-sheet-tab", i));
                let clip = b.ui.clip_rect();
                let hit = b.focus.item(&b.ui, id, r, clip, Place::Bar);
                if *which == tab {
                    b.first.get_or_insert(id);
                }
                let on = *which == tab;
                b.ui.painter().rect_filled(r, paint::radius(12.0 * s), if on { t.accent.gamma_multiply(0.7) } else { w::mix(t.surface, t.surface_hover, hit.glow) });
                w::icon_centered(b.ui.painter(), r.center(), icon, 28.0 * s, t.text);
                if hit.focused {
                    b.focus.ring_around(r, 12.0 * s);
                    if !on {
                        b.picked = Some(Do::Tab(*which));
                    }
                }
            }
            b.y += 80.0 * s;
            let mem = &bp.chat.mem;
            let items = with_env(f, |env| chat::picker_items(env, mem, tab));
            match tab {
                PickerTab::Effects => {
                    if items.is_empty() {
                        b.note(t!("You don't have any chat effects yet. They come from the Points Shop."), 3);
                    }
                    for (i, (name, _)) in items.into_iter().enumerate() {
                        b.row(&format!("fx{i}"), icons::SPARKLE, &name.clone(), None, true, Do::Effect(name));
                    }
                }
                PickerTab::Stickers => {
                    if items.is_empty() {
                        b.note(t!("You don't have any stickers yet. They come from the Points Shop."), 3);
                    }
                    let cells = items.into_iter().map(|(n, _)| emoticon_cell(&n, true, false, Do::Pick(n.clone(), true))).collect();
                    b.grid("stickers", cells, vec2(130.0 * s, 130.0 * s));
                }
                _ => {
                    let mut cells: Vec<Cell> = items.into_iter().map(|(n, sticker)| emoticon_cell(&n, sticker, false, Do::Pick(n.clone(), sticker))).collect();
                    if tab == PickerTab::Emoticons {
                        cells.extend(chat::EMOJI.iter().map(|e| Cell { file: None, url: None, glyph: Some(*e), mine: false, cover: false, act: Do::Emoji(e) }));
                    }
                    b.grid("emoticons", cells, vec2(84.0 * s, 84.0 * s));
                }
            }
        }
        SheetKind::Attach => {
            b.title(t!("Send"));
            let root = f.app.local.steam_root.clone();
            let newest = client::chat::newest_screenshot(&root, f.app.state.steam_id);
            b.row("pictures", icons::IMAGE, t!("Send a picture…"), None, true, Do::Open(SheetKind::Pictures));
            let label = if newest.is_some() { t!("Send my latest screenshot") } else { t!("No screenshots found") };
            let act = newest.map(|p| Do::Open(SheetKind::Confirm(Outgoing::Picture(p))));
            let enabled = act.is_some();
            b.row("latest", icons::CAMERA, label, None, enabled, act.unwrap_or(Do::Close));
            b.row("trade", icons::SWAP, t!("Share my trade offer link"), None, true, Do::Open(SheetKind::Confirm(Outgoing::TradeLink)));
        }
        SheetKind::Pictures => {
            b.title(t!("Send a picture…"));
            let root = f.app.local.steam_root.clone();
            let shots = client::chat::recent_screenshots(&root, f.app.state.steam_id, 18);
            if shots.is_empty() {
                b.note(t!("No screenshots found"), 2);
            }
            let cells = shots.into_iter().map(|path| Cell { file: Some(path.clone()), url: None, glyph: None, mine: false, cover: true, act: Do::Open(SheetKind::Confirm(Outgoing::Picture(path))) }).collect();
            b.grid("shots", cells, vec2(280.0 * s, 158.0 * s));
            // A file dialog needs a mouse or keyboard.
            if bp.device != Device::Pad {
                b.row("browse", icons::FOLDER, t!("Browse files…"), None, true, Do::Browse);
            }
        }
        SheetKind::Confirm(what) => {
            match what {
                Outgoing::Picture(path) => {
                    b.title(t!("Send this picture?"));
                    let r = Rect::from_min_size(pos2(b.x, b.y), vec2(b.width, 330.0 * s));
                    let ctx = b.ui.ctx().clone();
                    match b.thumbs.get(&ctx, path) {
                        Some(tex) => paint_texture(&b.ui, r, tex, false, paint::radius(10.0 * s)),
                        None => w::skeleton(&b.ui, &t, r, paint::radius(10.0 * s)),
                    }
                    b.y += 350.0 * s;
                }
                Outgoing::TradeLink => {
                    b.title(t!("Share my trade offer link"));
                    b.note(t!("Anyone in this chat can use it to send you trade offers."), 3);
                }
            }
            let act = match what {
                Outgoing::Picture(path) => Do::SendPicture(path.clone()),
                Outgoing::TradeLink => Do::SendTrade,
            };
            b.row("send", icons::SEND, t!("Send"), Some(t.accent), true, act);
            b.row("cancel", icons::X, t!("Cancel"), None, true, Do::Close);
        }
        SheetKind::Commands => {
            b.title(t!("Chat commands"));
            for c in model::chat::COMMANDS {
                let id = b.y;
                let _ = id;
                let label = format!("/{c}  ·  {}", chat::command_help(c));
                b.row(c, "/", &label, None, true, Do::Command(c));
            }
        }
        SheetKind::Report(key) => {
            b.title(t!("Report this message"));
            b.note(t!("Steam's moderators will see the message and who sent it. They won't be told who reported it."), 3);
            for (code, label) in chat::reasons() {
                b.row(&format!("r{code}"), if code == reason { icons::CHECK } else { icons::CIRCLE }, label, None, true, Do::Reason(code));
            }
            b.y += 8.0 * s;
            b.row("send", icons::WARNING, t!("Report"), Some(t.danger), true, Do::Report(*key));
            b.row("cancel", icons::X, t!("Cancel"), None, true, Do::Close);
        }
        SheetKind::Viewer(_) => {}
    }
    let height = b.y - top;
    let picked = b.picked.take();
    let first = b.first;
    let first_list: Vec<Id> = first.into_iter().collect();
    drop(b);
    let last = first.unwrap_or(Id::NULL);
    sh.focus.end(&first_list, last);
    // Follow the focus, or the wheel.
    if let (true, Some((r, _))) = (sh.focus.moved, sh.focus.focused()) {
        let y = r.top() - inner.top() + sh.scroll.pos;
        sh.scroll.reveal(y, y + r.height(), inner.height(), 24.0 * s);
    }
    if ui.rect_contains_pointer(panel) {
        sh.scroll.target -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    sh.scroll.clamp(height, inner.height());
    sh.scroll.step(ui);
    sh.focus.paint_ring(ui, &t, s, 14.0 * s);

    let mut close = outside;
    let mut back_to_input = false;
    let key_msg = |key: &MsgKey| msgs.iter().find(|m| m.key() == *key).cloned();
    if let Some(what) = picked {
        match what {
            Do::Open(kind) => {
                let cur = sh.kind.clone();
                sh.back.push(cur);
                sh.show(kind);
            }
            Do::Close => match sh.back.pop() {
                Some(kind) => sh.show(kind),
                None => close = true,
            },
            Do::Quote(key) => {
                if let Some(m) = key_msg(&key) {
                    let name = chat::sender_name(&f.app.state, &m);
                    let quote = model::chat::quote(&name, &m.plain);
                    bp.chat.reply = Some((name, m.plain.clone(), quote));
                }
                close = true;
                back_to_input = true;
            }
            Do::React { key, name, sticker, add } => {
                let (time, ordinal) = (key.0 as u32, key.1);
                with_env(f, |env| env.client.send(Command::ReactToMessage { to, time, ordinal, reaction: name, sticker, add }));
                close = true;
            }
            Do::Copy(text) => {
                ui.ctx().copy_text(text);
                f.actions.push(Action::Toast(t!("Copied").to_string()));
                close = true;
            }
            Do::Reveal(key) => {
                if let Some(m) = key_msg(&key) {
                    bp.chat.mem.reveal_spoilers(&m);
                }
                close = true;
            }
            Do::View(src) => {
                let cur = sh.kind.clone();
                sh.back.push(cur);
                sh.show(SheetKind::Viewer(src));
            }
            Do::Link(url) => {
                close = true;
                if let Some(a) = crate::builtin::chat::open_link(&url) {
                    route(bp, f, ui.ctx(), a);
                }
            }
            Do::Join(app_id, connect) => {
                f.actions.push(Action::JoinGame { app_id, connect });
                close = true;
            }
            Do::Reason(code) => sh.reason = code,
            Do::Report(key) => {
                if let Some(m) = key_msg(&key) {
                    let (reason, from) = (sh.reason, m.from);
                    with_env(f, |env| env.client.send(Command::ReportMessage { to, from, time: m.time as u32, ordinal: m.ordinal, reason, text: String::new(), language: env.language.clone() }));
                }
                close = true;
            }
            Do::Pick(name, sticker) => {
                let (mem, message) = (&mut bp.chat.mem, &mut bp.message);
                with_env(f, |env| chat::use_picked(env, to, message, mem, &name, sticker));
                close = sticker;
            }
            Do::Emoji(e) => bp.message.push_str(e),
            Do::Effect(name) => {
                with_env(f, |env| env.client.send(Command::SendChat { to, text: model::chat::effect(&name) }));
                close = true;
            }
            Do::Command(c) => {
                bp.message = format!("/{c} ");
                close = true;
                back_to_input = true;
            }
            Do::SendPicture(path) => {
                with_env(f, |env| env.client.send(Command::SendChatImage { to, path }));
                f.actions.push(Action::Toast(t!("Sending the picture…").to_string()));
                close = true;
            }
            Do::SendTrade => {
                let mem = &mut bp.chat.mem;
                with_env(f, |env| chat::ask_trade_link(env, mem));
                close = true;
            }
            Do::Browse => {
                if let Some(path) = rfd::FileDialog::new().add_filter(t!("Pictures"), &["png", "jpg", "jpeg", "gif", "webp"]).pick_file() {
                    let cur = sh.kind.clone();
                    sh.back.push(cur);
                    sh.show(SheetKind::Confirm(Outgoing::Picture(path)));
                }
            }
            Do::Tab(which) => {
                sh.tab = which;
                sh.scroll = Scroller::default();
            }
        }
    }
    if back_to_input {
        bp.focus.current = Some(input_id());
    }
    if !close && bp.chat.sheet.is_none() && matches!(bp.page(), Page::Chat(_)) {
        bp.chat.sheet = Some(sh);
    }
}

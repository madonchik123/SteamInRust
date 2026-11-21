//! The chat itself, like Steam's: messages with emoticons, stickers,
//! spoilers, code, quotes, `/me`, `/random` and `/flip`, links with preview
//! cards and pictures you can open large; reactions and a menu on every
//! message (copy, select, react, quote, report); and the message box with
//! slash commands, an emoticon picker and attachments. The Friends & Chat
//! window and the in-game overlay both show it.

mod cards;
mod input;
mod render;

pub use cards::init;
pub(crate) use cards::{is_image, link_label};
pub(crate) use input::{EMOJI, ask_trade_link, command_help, picker_items, send, share_trade_when_ready, slash_rows, use_picked, use_slash_row};

use client::{ChatTarget, Command, State};
use egui::{Align, Layout, Margin, Rect, Sense, Ui, pos2, vec2};
use extensions::Action;
use i18n::t;
use model::{ChatMessage, GameInvite, GroupMessage, Reaction, SteamId, art};
use std::collections::HashSet;
use ui::{Theme, icons, widgets as w};

/// Sends commands to Steam (the window's `Ctx`, or the overlay's `Client`).
pub trait Sender {
    fn send(&self, cmd: Command);
}
impl Sender for client::Client {
    fn send(&self, cmd: Command) {
        client::Client::send(self, cmd);
    }
}

/// What the chat needs from where it's shown.
pub struct Env<'a> {
    pub state: &'a State,
    pub client: &'a dyn Sender,
    /// Steam's folder, for "send my latest screenshot".
    pub steam_root: Option<&'a std::path::Path>,
    /// Steam's name for the interface language ("english"), for reports.
    pub language: String,
}

/// A message as the chat shows it, one-to-one or in a group.
#[derive(Clone)]
pub struct Msg {
    pub from: SteamId,
    pub time: u64,
    pub ordinal: u32,
    /// Steam's markup.
    pub raw: String,
    /// Plain text (copying, quoting).
    pub plain: String,
    pub invite: Option<GameInvite>,
    pub reactions: Vec<Reaction>,
}

impl Msg {
    pub fn friend(m: &ChatMessage) -> Msg {
        let raw = if m.raw.is_empty() { model::chat::escape(&m.text) } else { m.raw.clone() };
        Msg { from: m.from, time: m.time, ordinal: m.ordinal, raw, plain: m.text.clone(), invite: m.invite.clone(), reactions: m.reactions.clone() }
    }
    pub fn group(m: &GroupMessage) -> Msg {
        let (plain, invite) = model::chat::parse(&m.text);
        Msg { from: m.sender, time: m.time as u64, ordinal: m.ordinal, raw: m.text.clone(), plain, invite, reactions: m.reactions.clone() }
    }
    pub(crate) fn key(&self) -> (u64, u32) {
        (self.time, self.ordinal)
    }
}

/// How many spoilers a message has (inside quotes and `/me` too).
fn count_spoilers(parts: &[model::chat::Part]) -> usize {
    use model::chat::Part;
    parts
        .iter()
        .map(|p| match p {
            Part::Spoiler(i) => 1 + count_spoilers(i),
            Part::Me(i) | Part::Quote { parts: i, .. } => count_spoilers(i),
            _ => 0,
        })
        .sum()
}

impl ChatUi {
    /// The picture clicked in a message, for a viewer of the caller's own.
    pub(crate) fn take_viewing(&mut self) -> Option<String> {
        self.viewing.take()
    }

    /// Does this message still hide a spoiler?
    pub(crate) fn has_hidden_spoiler(&self, m: &Msg) -> bool {
        let n = count_spoilers(&model::chat::rich(&m.raw));
        (0..n).any(|i| !self.revealed.contains(&(m.time, m.ordinal, i)))
    }

    /// Show every spoiler in this message.
    pub(crate) fn reveal_spoilers(&mut self, m: &Msg) {
        for i in 0..count_spoilers(&model::chat::rich(&m.raw)) {
            self.revealed.insert((m.time, m.ordinal, i));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PickerTab {
    #[default]
    Recent,
    Emoticons,
    Stickers,
    Effects,
}

/// A report being written: whose message, and the reason picked.
#[derive(Clone)]
struct Report {
    from: SteamId,
    time: u64,
    ordinal: u32,
    reason: i32,
    text: String,
}

/// What a conversation remembers between frames (kept in egui's memory).
#[derive(Clone, Default)]
pub(crate) struct ChatUi {
    /// Spoilers shown: (message time, ordinal, which spoiler).
    revealed: HashSet<(u64, u32, usize)>,
    /// The message shown as selectable text.
    selecting: Option<(u64, u32)>,
    /// The picture open in the large viewer.
    viewing: Option<String>,
    report: Option<Report>,
    picker_tab: PickerTab,
    picker_search: String,
    /// The highlighted slash command or game.
    slash_pick: usize,
    /// "Share my trade link" waits for Steam to give it.
    share_trade: bool,
    /// `/store`: the search that was sent, and when the text last changed.
    store_sent: String,
    store_typed: f64,
    /// Emoticons you used this session, newest first.
    recent: Vec<(String, bool)>,
}

/// One conversation: its messages and the message box, `height` tall.
/// `older` (group chats) shows an "Older messages" button at the top and is
/// set when it's clicked; `loading` shows a spinner instead of "No messages
/// yet". Returns what the app should do (join a game, open a page).
#[allow(clippy::too_many_arguments)]
pub fn conversation(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, msgs: &[Msg], input: &mut String, height: f32, older: Option<&mut bool>, loading: bool) -> Vec<Action> {
    let id = ui.scope_id().with(("chat-ui", to));
    let mut mem: ChatUi = ui.data_mut(|d| d.get_temp_mut_or_default::<ChatUi>(id).clone());
    let mut out = Vec::new();
    if env.state.emoticons.is_none() {
        let asked = egui::Id::unique("emoticons-asked");
        if !ui.data(|d| d.get_temp::<bool>(asked).unwrap_or(false)) {
            ui.data_mut(|d| d.insert_temp(asked, true));
            env.client.send(Command::LoadEmoticons);
        }
    }
    // The messages get exactly what the message box (measured last frame)
    // leaves, so the whole thing never needs more than `height`. Inside a
    // resizable window, needing even a little more made it grow every frame.
    let composer_id = id.with("composer-h");
    let composer = ui.data(|d| d.get_temp::<f32>(composer_id)).unwrap_or(46.0);
    let gap = t.spacing.sm;
    let list = (height - composer - gap - ui.spacing().item_spacing.y).floor().max(40.0);
    let mut older_clicked = false;
    let has_older = older.is_some();
    egui::ScrollArea::vertical()
        .id_salt(("chat", to))
        .max_height(list)
        .min_scrolled_height(list)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if has_older && !msgs.is_empty() {
                ui.vertical_centered(|ui| {
                    older_clicked = w::button(ui, t, Some(icons::CARET_UP), t!("Older messages"), w::Kind::Ghost, w::Size::Sm).clicked();
                });
            }
            if msgs.is_empty() {
                if loading {
                    w::loading(ui, t, t!("Loading…"));
                } else {
                    w::empty(ui, t, icons::CHAT, t!("No messages yet"), t!("Say hi!"));
                }
            }
            let mut last: Option<(SteamId, u64)> = None;
            for m in msgs.iter().filter(|m| !m.raw.trim().is_empty() || m.invite.is_some()) {
                let first = last.is_none_or(|(from, time)| from != m.from || m.time.saturating_sub(time) > 300);
                message(ui, t, env, to, m, first, true, &mut mem, &mut out);
                last = Some((m.from, m.time));
            }
            if let ChatTarget::Friend(friend) = to {
                if env.state.is_typing(friend) {
                    ui.add_space(t.spacing.sm);
                    ui.label(w::body_s(t, t!("{name} is typing…", name = env.state.persona(friend).name), t.text_faint).italics());
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                }
            }
        });
    if let Some(o) = older {
        *o = older_clicked;
    }
    ui.add_space(gap);
    let r = input::composer(ui, t, env, to, input, &mut mem, &mut out);
    if (r.height() - composer).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(composer_id, r.height()));
        ui.ctx().request_repaint();
    }
    report_dialog(ui, t, env, to, &mut mem);
    cards::viewer(ui, t, &mut mem);
    ui.data_mut(|d| d.insert_temp(id, mem));
    out
}

/// One message (with its sender's avatar, name and time when it starts a
/// run), its reactions, and the react button and menu while the pointer is
/// on it.
#[allow(clippy::too_many_arguments)]
fn message(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, m: &Msg, first: bool, buttons: bool, mem: &mut ChatUi, out: &mut Vec<Action>) -> Rect {
    let state = env.state;
    let av = t.dense(34.0);
    if first {
        ui.add_space(t.spacing.md);
    }
    // Lit while the pointer is on it; the fill goes behind what's drawn next.
    let bg = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().min;
    let p = state.persona(m.from);
    let me = m.from == state.steam_id;
    let name = if p.name.is_empty() { if me { t!("You").to_string() } else { m.from.to_string() } } else { p.display_name() };
    let inner = ui
        .horizontal_top(|ui| {
            if first {
                w::presence_avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), av, None);
            } else {
                ui.add_space(av);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // Room for the buttons at the right end.
                ui.set_max_width((ui.available_width() - t.dense(64.0)).max(80.0));
                if first {
                    ui.horizontal(|ui| {
                        ui.label(crate::builtin::friends::strong(t, &name, if me { t.accent } else { crate::shell::status_color(t, &p) }));
                        ui.label(w::body_s(t, ui::ago(m.time), t.text_faint)).on_hover_text(ui::date_time(m.time));
                    });
                }
                if mem.selecting == Some(m.key()) {
                    let mut text = m.plain.clone();
                    ui.add(egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(ui.available_width()).font(egui::FontId::new(t.typography.body_m.size, t.typography.body_m.weight.family())));
                    if w::button(ui, t, Some(icons::CHECK), t!("Done"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        mem.selecting = None;
                    }
                } else {
                    render::body(ui, t, env, to, m, &name, mem, out);
                }
                reactions(ui, t, env, to, m);
            });
        })
        .response;
    let rect = Rect::from_min_max(top, pos2(ui.max_rect().right(), inner.rect.bottom())).expand2(vec2(t.spacing.xs, 2.0));
    let menu_id = ui.scope_id().with(("msg-menu", m.time, m.ordinal));
    let open = egui::Popup::is_id_open(ui.ctx(), menu_id) || egui::Popup::is_id_open(ui.ctx(), menu_id.with("react"));
    if ui.rect_contains_pointer(rect) || open {
        let look = t.look(&t.components.friend_row, ui::theme::Interact { hover: 1.0, ..Default::default() });
        if let ui::theme::ResolvedFill::Color(c) = look.fill {
            ui.painter().set(bg, egui::Shape::rect_filled(rect, t.corner_small(), c));
        }
        if buttons {
            actions(ui, t, env, to, m, &name, rect, menu_id, mem);
        }
    }
    rect
}

/// One message drawn like the normal chat's, without the mouse-only buttons
/// (Big Picture has its own menu for a message). Returns where it is.
#[allow(clippy::too_many_arguments)]
pub(crate) fn message_row(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, m: &Msg, first: bool, mem: &mut ChatUi, out: &mut Vec<Action>) -> Rect {
    message(ui, t, env, to, m, first, false, mem, out)
}

/// The name a message is quoted under.
pub(crate) fn sender_name(state: &State, m: &Msg) -> String {
    let p = state.persona(m.from);
    if p.name.is_empty() {
        if m.from == state.steam_id { t!("You").to_string() } else { m.from.to_string() }
    } else {
        p.display_name()
    }
}

/// The react button and the "⋮" menu at the top right of a message.
#[allow(clippy::too_many_arguments)]
fn actions(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, m: &Msg, name: &str, rect: Rect, menu_id: egui::Id, mem: &mut ChatUi) {
    let size = t.dense(26.0);
    let bar = Rect::from_min_size(pos2(rect.right() - size * 2.0 - t.spacing.xs * 2.0, rect.top() + 2.0), vec2(size * 2.0 + t.spacing.xs, size));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(Layout::left_to_right(Align::Center)));
    child.spacing_mut().item_spacing.x = t.spacing.xs;
    let small = |ui: &mut Ui, icon: &str, tip: &str| {
        let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let c = &t.components.icon_button;
        let look = t.look(c, w::interact(ui, &resp, false));
        w::paint_box(ui, t, r, c, &look, 0.0);
        w::icon_centered(ui.painter(), r.center(), icon, size * 0.55, look.icon);
        resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
    };
    let react = small(&mut child, icons::SMILEY, t!("React"));
    let more = small(&mut child, icons::DOTS, t!("More"));
    egui::Popup::menu(&react).id(menu_id.with("react")).show(|ui| {
        if let Some((name, sticker)) = input::reaction_grid(ui, t, env, mem) {
            env.client.send(Command::ReactToMessage { to, time: m.time as u32, ordinal: m.ordinal, reaction: name, sticker, add: true });
            ui.close();
        }
    });
    egui::Popup::menu(&more).id(menu_id).width(t.dense(220.0)).show(|ui| {
        if w::menu_item(ui, t, icons::COPY, t!("Copy")).clicked() {
            ui.ctx().copy_text(m.plain.clone());
            ui.close();
        }
        if w::menu_item(ui, t, icons::NOTE, t!("Select message")).clicked() {
            mem.selecting = Some(m.key());
            ui.close();
        }
        if w::menu_item(ui, t, icons::CHAT, t!("Quote")).clicked() {
            ui.data_mut(|d| d.insert_temp(egui::Id::unique(("chat-quote", to)), model::chat::quote(name, &m.plain)));
            ui.close();
        }
        if m.from != env.state.steam_id {
            ui.separator();
            if w::menu_item_tinted(ui, t, icons::WARNING, t!("Report…"), Some(t.danger)).clicked() {
                mem.report = Some(Report { from: m.from, time: m.time, ordinal: m.ordinal, reason: 28, text: String::new() });
                ui.close();
            }
        }
    });
}

/// The reactions under a message: each emoticon or sticker with its count,
/// yours marked; a click adds yours or takes it back.
fn reactions(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, m: &Msg) {
    if m.reactions.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
        for r in &m.reactions {
            let c = &t.components.chip;
            let img = t.dense(18.0);
            let g = ui.painter().layout_no_wrap(r.count.to_string(), egui::FontId::new(t.typography.body_s.size, t.typography.body_s.weight.family()), egui::Color32::PLACEHOLDER);
            let pad = t.spacing.sm;
            let (rect, resp) = ui.allocate_exact_size(vec2(pad + img + t.spacing.xs + g.size().x + pad, img + t.spacing.xs * 2.0), Sense::click());
            let look = t.look(c, w::interact(ui, &resp, r.mine));
            w::paint_box(ui, t, rect, c, &look, if r.mine { 1.0 } else { 0.0 });
            if r.mine {
                ui.painter().rect_stroke(rect, t.radius_of(c), egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            }
            let url = if r.sticker { model::chat::sticker_url(&r.name) } else { model::chat::emoticon_url(&r.name, false) };
            let ir = Rect::from_min_size(pos2(rect.left() + pad, rect.center().y - img / 2.0), vec2(img, img));
            egui::Image::new(url).paint_at(ui, ir);
            ui.painter().galley(pos2(ir.right() + t.spacing.xs, rect.center().y - g.size().y / 2.0), g, look.text);
            let tip = if r.mine { t!("You reacted with :{name}: (click to take it back)", name = r.name) } else { t!("React with :{name}:", name = r.name) };
            if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                env.client.send(Command::ReactToMessage { to, time: m.time as u32, ordinal: m.ordinal, reaction: r.name.clone(), sticker: r.sticker, add: !r.mine });
            }
        }
    });
}

/// Steam's report reasons offered for a message, in its own words.
pub(crate) fn reasons() -> [(i32, &'static str); 7] {
    [
        (28, t!("Spam")),
        (3, t!("Harassment")),
        (13, t!("Trying to scam me")),
        (12, t!("Phishing or account theft")),
        (14, t!("Links to malicious content")),
        (22, t!("Hate or discrimination")),
        (23, t!("Other abuse")),
    ]
}

fn report_dialog(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, mem: &mut ChatUi) {
    let Some(r) = mem.report.as_mut() else { return };
    let mut close = false;
    let width = (ui.ctx().content_rect().width() - 64.0).clamp(240.0, 420.0);
    egui::Modal::new(egui::Id::unique(("report-message", r.time, r.ordinal))).show(ui.ctx(), |ui| {
        ui.set_width(width);
        ui.label(w::heading_s(t, t!("Report this message"), t.text));
        ui.add(egui::Label::new(w::body_s(t, t!("Steam's moderators will see the message and who sent it. They won't be told who reported it."), t.text_dim)).wrap());
        ui.add_space(t.spacing.sm);
        for (code, label) in reasons() {
            ui.radio_value(&mut r.reason, code, w::body_m(t, label, t.text));
        }
        ui.add_space(t.spacing.sm);
        ui.add(egui::TextEdit::multiline(&mut r.text).hint_text(t!("Anything else they should know (optional)")).desired_rows(2).desired_width(f32::INFINITY));
        ui.add_space(t.spacing.md);
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::WARNING), t!("Report"), w::Kind::Danger, w::Size::Sm).clicked() {
                env.client.send(Command::ReportMessage { to, from: r.from, time: r.time as u32, ordinal: r.ordinal, reason: r.reason, text: r.text.trim().to_string(), language: env.language.clone() });
                close = true;
            }
            if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Sm).clicked() {
                close = true;
            }
        });
    });
    if close {
        mem.report = None;
    }
}

/// A game invite in a message, as a card with a Play or Join button.
fn invite_card(ui: &mut Ui, t: &Theme, state: &State, from: SteamId, inv: &GameInvite, out: &mut Vec<Action>) {
    let mine = from == state.steam_id;
    let name = state.app_name(inv.app_id).map(String::from).unwrap_or_else(|| t!("App {id}", id = inv.app_id).to_string());
    let width = ui.available_width().min(t.dense(380.0));
    w::component_frame(t, &t.components.card).inner_margin(Margin::same(t.spacing.sm as i8)).show(ui, |ui| {
        ui.set_width((width - t.spacing.lg).max(0.0));
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(t.dense(107.0), t.dense(50.0)), Sense::hover());
            w::image_or_title(ui, t, r, &art::header(inv.app_id), &name, t.corner_small());
            ui.add_space(t.spacing.xs);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let head = if mine { t!("YOU SENT A GAME INVITE") } else { t!("INVITED YOU TO PLAY") };
                ui.label(w::caption_text(t, head));
                ui.add(egui::Label::new(crate::builtin::friends::strong(t, &name, t.text)).truncate());
                if !mine {
                    let label = if inv.connect.is_empty() { t!("Play") } else { t!("Join game") };
                    if w::button(ui, t, Some(icons::PLAY), label, w::Kind::Primary, w::Size::Sm).clicked() {
                        out.push(Action::JoinGame { app_id: inv.app_id, connect: inv.connect.clone() });
                    }
                }
            });
        });
    });
}

/// Where a link in a chat opens: Store pages, Workshop items, profiles,
/// groups, discussions and trade links in the app; anything else in the
/// browser (`None`).
pub fn open_link(url: &str) -> Option<Action> {
    let nav = |page: &str, arg: String| Some(Action::Navigate { page: page.into(), arg: Some(arg) });
    if let Some(app) = cards::store_app(url) {
        return nav("store", format!("app:{app}"));
    }
    if let Some((partner, _)) = client::trade::parse_link(url) {
        return nav("inventory", format!("trade:{partner}"));
    }
    if let Some(id) = cards::profile_id(url) {
        return nav("profile", id.to_string());
    }
    if let Some(view) = crate::builtin::hub::route(url) {
        let (page, arg) = view.to_page();
        return Some(Action::Navigate { page, arg });
    }
    crate::links::open_http(url);
    None
}

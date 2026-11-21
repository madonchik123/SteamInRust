//! The message box: what you type (`:emoticons:` become Steam's), slash
//! commands with a list that pops up on `/`, `/store` with a game search,
//! the emoticon picker (recent, emoticons and emoji, stickers, chat
//! effects, with a search), and the "+" menu (a picture, your latest
//! screenshot, your trade link, inviting a friend in).

use super::{ChatUi, Env, PickerTab};
use client::{ChatTarget, Command};
use egui::{Align, Key, Layout, Margin, Rect, Sense, Ui, pos2, vec2};
use extensions::Action;
use i18n::t;
use model::AppId;
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

/// Everyday emoji offered beside Steam's emoticons.
pub(crate) const EMOJI: &[&str] = &[
    "😀", "😁", "😂", "🤣", "😊", "😍", "😘", "😎", "🤔", "😏", "😴", "😢", "😭", "😡", "😱", "🥳", "🤯", "🙄", "😅", "😇", "👍", "👎", "👌", "👏", "🙏", "💪", "👋", "✌", "🤝", "❤", "💔", "🔥", "⭐", "✨", "🎉", "🎮", "🕹", "🏆", "💀", "👀", "💯", "✅", "❌", "❓", "❗", "☕", "🍕", "🍺", "🌙", "☀",
];

/// What a slash command does, in the user's language.
pub(crate) fn command_help(name: &str) -> &'static str {
    match name {
        "me" => t!("Say what you're doing (in the third person)"),
        "code" => t!("Show the rest as code"),
        "pre" => t!("Keep the rest as typed (spaces and lines)"),
        "quote" => t!("Show the rest as a quote"),
        "spoiler" => t!("Hide the rest until clicked"),
        "random" => t!("A random number (from 1 to 100, or /random 1 6)"),
        "flip" => t!("Flip a coin"),
        "store" => t!("Share a game's Store page (/store name)"),
        _ => "",
    }
}

/// The message box with its buttons; returns its rect.
pub(super) fn composer(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, input: &mut String, mem: &mut ChatUi, out: &mut Vec<Action>) -> Rect {
    // A quote picked from a message's menu goes in front of what's typed.
    let quote_id = egui::Id::unique(("chat-quote", to));
    if let Some(q) = ui.data_mut(|d| d.remove_temp::<String>(quote_id)) {
        *input = format!("{q}{input}");
    }
    share_trade_when_ready(env, to, mem, out);
    let preview = preview_flag();
    if preview.as_deref() == Some("slash") && input.is_empty() {
        input.push('/');
    }
    let c = &t.components.input;
    let pad = t.padding_of(c);
    let row = t.height_of(&t.components.icon_button, 36.0);
    let frame = w::component_frame(t, c).inner_margin(Margin::symmetric(pad.x.clamp(t.spacing.xs, t.spacing.md) as i8, t.spacing.xs as i8));
    let mut edit_rect = Rect::NOTHING;
    let mut submit = false;
    let mut edit_id = None;
    let r = frame.show(ui, |ui| {
        ui.allocate_ui_with_layout(vec2(ui.available_width(), row), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.xs;
            attach_menu(ui, t, env, to, mem, out);
            // The buttons at the right end first, so the box takes what's left.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                submit |= w::icon_button(ui, t, icons::SEND, t!("Send"), false, 0).clicked();
                let smile = w::icon_button(ui, t, icons::SMILEY, t!("Emoticons, stickers and effects"), false, 0);
                if preview.as_deref() == Some("picker") {
                    egui::Popup::open_id(ui.ctx(), egui::Id::unique(("chat-picker", to)));
                }
                egui::Popup::menu(&smile).id(egui::Id::unique(("chat-picker", to))).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| picker(ui, t, env, to, input, mem));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let before = input.len();
                    let te = ui
                        .add(
                            egui::TextEdit::singleline(input)
                                .id_salt(("chat-input", to))
                                .hint_text(w::body_m(t, hint(env, to), t.text_faint))
                                .font(egui::FontId::new(t.typography.body_m.size, t.typography.body_m.weight.family()))
                                .text_color(t.text)
                                .frame(egui::Frame::NONE)
                                .align(egui::Align2::LEFT_CENTER)
                                .min_size(vec2(0.0, row))
                                .desired_width(ui.available_width()),
                        )
                        .with_edit_menu();
                    edit_rect = te.rect;
                    edit_id = Some(te.id);
                    if preview.as_deref() == Some("slash") {
                        te.request_focus();
                    }
                    if let ChatTarget::Friend(f) = to {
                        if input.len() > before && before == 0 {
                            env.client.send(Command::Typing { to: f });
                        }
                    }
                    let popup_open = model::chat::typing_command(input).is_some() || model::chat::store_command(input).is_some();
                    if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !popup_open {
                        submit = true;
                    }
                });
            });
        });
    });
    // Slash commands and `/store` suggestions over the box.
    let picked = slash_popup(ui, t, env, to, input, mem, r.response.rect, edit_id);
    if let Some(cmd) = picked {
        *input = cmd;
        if let Some(id) = edit_id {
            ui.memory_mut(|m| m.request_focus(id));
            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(input.chars().count()))));
                st.store(ui.ctx(), id);
            }
        }
    }
    if submit && !input.trim().is_empty() {
        send(env, to, input, mem, out);
        if let Some(id) = edit_id {
            ui.memory_mut(|m| m.request_focus(id));
        }
    }
    r.response.rect
}

/// Ask Steam for your trade link, to send it once it's here.
pub(crate) fn ask_trade_link(env: &Env, mem: &mut ChatUi) {
    if env.state.trade_link.as_ref().is_none_or(|r| r.is_err()) {
        env.client.send(Command::LoadTradeLink { new: false });
    }
    mem.share_trade = true;
}

/// The trade link, once Steam gives it.
pub(crate) fn share_trade_when_ready(env: &Env, to: ChatTarget, mem: &mut ChatUi, out: &mut Vec<Action>) {
    if !mem.share_trade {
        return;
    }
    match &env.state.trade_link {
        Some(Ok(link)) => {
            env.client.send(Command::SendChat { to, text: model::chat::escape(link) });
            mem.share_trade = false;
        }
        Some(Err(e)) => {
            out.push(Action::Toast(t!("Couldn't get your trade link: {error}", error = e).to_string()));
            mem.share_trade = false;
        }
        None => {}
    }
}

/// `--chat-preview=picker` or `=slash` (previews of an off-screen copy,
/// which no mouse can reach): open the picker or the command list once.
fn preview_flag() -> Option<String> {
    static DONE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let v = std::env::args().find_map(|a| a.strip_prefix("--chat-preview=").map(String::from))?;
    // A few frames, so it stays open once the window is up.
    (DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 600).then_some(v)
}

fn hint(env: &Env, to: ChatTarget) -> String {
    match to {
        ChatTarget::Friend(f) => t!("Message {name}", name = env.state.persona(f).name).to_string(),
        ChatTarget::Group { group, room } => {
            let g = env.state.chat_groups.as_ref().and_then(|r| r.as_ref().ok()).and_then(|l| l.iter().find(|g| g.id == group));
            let name = g.and_then(|g| g.rooms.iter().find(|r| r.id == room)).map(|r| r.name.clone()).filter(|n| !n.is_empty()).or_else(|| g.map(|g| crate::builtin::friends::group_name(env.state, g))).unwrap_or_default();
            t!("Message {name}", name = name).to_string()
        }
    }
}

/// Is this emoticon yours (only those can be sent)?
fn owns(env: &Env, name: &str) -> bool {
    env.state.emoticons.as_ref().is_some_and(|e| e.emoticons.iter().any(|x| x.name == name))
}

pub(crate) fn send(env: &Env, to: ChatTarget, input: &mut String, mem: &mut ChatUi, out: &mut Vec<Action>) {
    // `/store <game>` sends that game's Store link.
    if let Some(name) = model::chat::store_command(input) {
        match store_matches(env, name).first() {
            Some((id, _)) => {
                env.client.send(Command::SendChat { to, text: format!("https://store.steampowered.com/app/{id}/") });
                input.clear();
            }
            None => out.push(Action::Toast(t!("No game found for “{name}”", name = name).to_string())),
        }
        return;
    }
    // A quote from a message's menu is markup; only what's typed is escaped.
    let (quote, typed) = model::chat::split_quote(input);
    let text = format!("{quote}{}", model::chat::compose(typed, |n| owns(env, n)));
    for name in model::chat::rich(&text).iter().filter_map(|p| if let model::chat::Part::Emoticon(n) = p { Some(n.clone()) } else { None }) {
        remember(mem, &name, false);
    }
    env.client.send(Command::SendChat { to, text });
    input.clear();
}

/// Keep what you just used at the front of "Recent".
fn remember(mem: &mut ChatUi, name: &str, sticker: bool) {
    mem.recent.retain(|(n, s)| !(n == name && *s == sticker));
    mem.recent.insert(0, (name.to_string(), sticker));
    mem.recent.truncate(24);
}

/// Games whose name matches, yours first, then the Store's suggestions.
fn store_matches(env: &Env, name: &str) -> Vec<(AppId, String)> {
    let q = name.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(AppId, String)> = Vec::new();
    if let Some(apps) = &env.state.apps {
        let mut mine: Vec<(AppId, String)> = apps.values().filter(|a| a.name.to_lowercase().contains(&q)).map(|a| (a.app_id, a.name.clone())).collect();
        mine.sort_by_key(|(_, n)| (!n.to_lowercase().starts_with(&q), n.len()));
        out.extend(mine.into_iter().take(5));
    }
    if let Some(Some(Ok(found))) = super::cards::search_cached(&q) {
        for (id, n) in found {
            if !out.iter().any(|(i, _)| *i == id) {
                out.push((id, n));
            }
        }
    }
    out.truncate(8);
    out
}

/// A row of the list over the message box.
#[derive(Clone)]
pub(crate) struct SlashRow {
    pub label: String,
    pub help: String,
    /// The text it puts in the box.
    pub text: Option<String>,
    /// A game whose Store page it sends.
    pub game: Option<AppId>,
}

/// What the list shows for what's typed: commands for `/…`, games for
/// `/store …` (asking the Store once typing pauses).
pub(crate) fn slash_rows(ctx: &egui::Context, env: &Env, input: &str, mem: &mut ChatUi) -> Vec<SlashRow> {
    let mut rows = Vec::new();
    if let Some(typed) = model::chat::typing_command(input) {
        let typed = typed.to_lowercase();
        for c in model::chat::COMMANDS.iter().filter(|c| c.starts_with(&typed)) {
            rows.push(SlashRow { label: format!("/{c}"), help: command_help(c).to_string(), text: Some(format!("/{c} ")), game: None });
        }
    } else if let Some(name) = model::chat::store_command(input) {
        let now = ctx.input(|i| i.time);
        if name != mem.store_sent {
            if mem.store_typed == 0.0 || mem.store_sent.is_empty() && name.is_empty() {
                mem.store_typed = now;
            }
            if now - mem.store_typed > 0.4 && name.chars().count() >= 2 {
                super::cards::search(&name.to_lowercase(), &env.state.country);
                mem.store_sent = name.to_string();
                mem.store_typed = 0.0;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
        for (id, n) in store_matches(env, name) {
            rows.push(SlashRow { label: n, help: t!("Send its Store page").to_string(), text: None, game: Some(id) });
        }
        if rows.is_empty() && !name.is_empty() {
            rows.push(SlashRow { label: t!("Searching the Store…").to_string(), help: String::new(), text: None, game: None });
        }
    }
    rows
}

/// Do what a row says: complete the command, or send the game's page.
pub(crate) fn use_slash_row(env: &Env, to: ChatTarget, input: &mut String, row: &SlashRow) {
    if let Some(id) = row.game {
        env.client.send(Command::SendChat { to, text: format!("https://store.steampowered.com/app/{id}/") });
        input.clear();
    } else if let Some(text) = &row.text {
        *input = text.clone();
    }
}

/// The list over the box while typing `/…`, or games for `/store …`.
/// Arrows move, Tab or Enter picks. Returns the new text.
#[allow(clippy::too_many_arguments)]
fn slash_popup(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, input: &mut String, mem: &mut ChatUi, anchor: Rect, edit: Option<egui::Id>) -> Option<String> {
    let has_focus = edit.is_some_and(|id| ui.memory(|m| m.has_focus(id)));
    let rows = slash_rows(ui.ctx(), env, input, mem);
    if rows.is_empty() || !has_focus {
        mem.slash_pick = 0;
        return None;
    }
    mem.slash_pick = mem.slash_pick.min(rows.len() - 1);
    let (up, down, pick) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, Key::Tab) || i.key_pressed(Key::Enter)));
    if up {
        mem.slash_pick = mem.slash_pick.checked_sub(1).unwrap_or(rows.len() - 1);
    }
    if down {
        mem.slash_pick = (mem.slash_pick + 1) % rows.len();
    }
    let mut chosen = pick.then_some(mem.slash_pick);
    let width = anchor.width().min(t.dense(460.0));
    let row_h = t.height_of(&t.components.menu_item, 38.0) + t.typography.body_s.size;
    let height = row_h * rows.len() as f32 + t.spacing.sm * 2.0;
    egui::Area::new(egui::Id::unique(("chat-slash", to)))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(anchor.left(), anchor.top() - height - t.spacing.xs))
        .show(ui.ctx(), |ui| {
            w::component_frame(t, &t.components.menu).show(ui, |ui| {
                ui.set_width(width);
                for (i, SlashRow { label, help, .. }) in rows.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(width, row_h), Sense::click());
                    let c = &t.components.menu_item;
                    let look = t.look(c, w::interact(ui, &resp, i == mem.slash_pick));
                    w::paint_box(ui, t, r, c, &look, 0.0);
                    let pad = t.padding_of(c).x;
                    let name = crate::builtin::friends::fit(ui, label.clone(), t.font_of(c), look.text, r.width() - pad * 2.0);
                    let nh = name.size().y;
                    let help = crate::builtin::friends::fit(ui, help.clone(), egui::FontId::new(t.typography.body_s.size, t.typography.body_s.weight.family()), t.text_dim, r.width() - pad * 2.0);
                    let top = r.center().y - (nh + help.size().y) / 2.0;
                    ui.painter().galley(pos2(r.left() + pad, top), name, look.text);
                    ui.painter().galley(pos2(r.left() + pad, top + nh), help, t.text_dim);
                    if resp.hovered() {
                        mem.slash_pick = i;
                    }
                    if resp.clicked() {
                        chosen = Some(i);
                    }
                }
            });
        });
    let row = rows.get(chosen?)?.clone();
    mem.slash_pick = 0;
    if row.game.is_some() {
        use_slash_row(env, to, input, &row);
        return None;
    }
    row.text
}

/// A grid of pictures (emoticons or stickers); returns the one clicked.
fn grid(ui: &mut Ui, t: &Theme, items: &[(String, bool)], size: f32) -> Option<(String, bool)> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
        for (name, sticker) in items {
            let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
            }
            let url = if *sticker { model::chat::sticker_url(name) } else { model::chat::emoticon_url(name, false) };
            egui::Image::new(url).paint_at(ui, r.shrink(t.spacing.xs / 2.0));
            let tip = if *sticker { name.clone() } else { format!(":{name}:") };
            if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some((name.clone(), *sticker));
            }
        }
    });
    clicked
}

/// Recent first (yours this session, then what Steam says you used last).
fn recent(env: &Env, mem: &ChatUi) -> Vec<(String, bool)> {
    let mut out = mem.recent.clone();
    if let Some(e) = &env.state.emoticons {
        let mut used: Vec<(u32, String, bool)> = e.emoticons.iter().filter(|x| x.last_used > 0).map(|x| (x.last_used, x.name.clone(), false)).collect();
        used.extend(e.stickers.iter().filter(|x| x.last_used > 0).map(|x| (x.last_used, x.name.clone(), true)));
        used.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, n, s) in used {
            if !out.iter().any(|(m, ms)| *m == n && *ms == s) {
                out.push((n, s));
            }
        }
    }
    out.truncate(24);
    out
}

/// What a tab of the picker offers: (name, is it a sticker). Recent falls
/// back to your emoticons so it's never empty when you have some.
pub(crate) fn picker_items(env: &Env, mem: &ChatUi, tab: PickerTab) -> Vec<(String, bool)> {
    let Some(e) = &env.state.emoticons else { return Vec::new() };
    match tab {
        PickerTab::Recent => {
            let items = recent(env, mem);
            if items.is_empty() { e.emoticons.iter().map(|x| (x.name.clone(), false)).collect() } else { items }
        }
        PickerTab::Emoticons => e.emoticons.iter().map(|x| (x.name.clone(), false)).collect(),
        PickerTab::Stickers => e.stickers.iter().map(|x| (x.name.clone(), true)).collect(),
        PickerTab::Effects => e.effects.iter().map(|x| (x.name.clone(), false)).collect(),
    }
}

/// An emoticon goes in the box; a sticker is sent right away.
pub(crate) fn use_picked(env: &Env, to: ChatTarget, input: &mut String, mem: &mut ChatUi, name: &str, sticker: bool) {
    use_item(env, to, input, mem, name, sticker);
}

/// The emoticon grid for reacting to a message.
pub(super) fn reaction_grid(ui: &mut Ui, t: &Theme, env: &Env, mem: &mut ChatUi) -> Option<(String, bool)> {
    let Some(e) = &env.state.emoticons else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(w::body_s(t, t!("Loading your emoticons…"), t.text_dim));
        });
        return None;
    };
    ui.set_width(t.dense(300.0));
    if e.emoticons.is_empty() {
        ui.add(egui::Label::new(w::body_s(t, t!("You don't have any emoticons yet. You get them from trading cards and the Points Shop."), t.text_dim)).wrap());
        return None;
    }
    let mut items: Vec<(String, bool)> = recent(env, mem).into_iter().filter(|(_, s)| !s).take(8).collect();
    for x in &e.emoticons {
        if !items.iter().any(|(n, _)| *n == x.name) {
            items.push((x.name.clone(), false));
        }
    }
    let picked = egui::ScrollArea::vertical().max_height(t.dense(220.0)).show(ui, |ui| grid(ui, t, &items, t.dense(30.0))).inner;
    if let Some((n, s)) = &picked {
        remember(mem, n, *s);
    }
    picked
}

/// The emoticon picker, like Steam's: Recent, Emoticons (and emoji),
/// Stickers and Effects, with a search.
fn picker(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, input: &mut String, mem: &mut ChatUi) {
    let width = t.dense(340.0);
    ui.set_width(width);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.xs;
        for (tab, icon, tip) in [
            (PickerTab::Recent, icons::CLOCK, t!("Recent")),
            (PickerTab::Emoticons, icons::SMILEY, t!("Emoticons")),
            (PickerTab::Stickers, icons::IMAGE, t!("Stickers")),
            (PickerTab::Effects, icons::SPARKLE, t!("Chat effects")),
        ] {
            if w::icon_box(ui, t, icon, tip, mem.picker_tab == tab).clicked() {
                mem.picker_tab = tab;
            }
        }
    });
    ui.add_space(t.spacing.xs);
    w::search(ui, t, &mut mem.picker_search, t!("Search"), width);
    ui.add_space(t.spacing.xs);
    let q = mem.picker_search.trim().to_lowercase();
    let Some(e) = env.state.emoticons.clone() else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(w::body_s(t, t!("Loading your emoticons…"), t.text_dim));
        });
        return;
    };
    let matches = |n: &str| q.is_empty() || n.to_lowercase().contains(&q);
    let empty = |ui: &mut Ui, text: &str| {
        ui.add(egui::Label::new(w::body_s(t, text, t.text_dim)).wrap());
    };
    egui::ScrollArea::vertical().max_height(t.dense(260.0)).show(ui, |ui| {
        ui.set_width(width);
        match mem.picker_tab {
            PickerTab::Recent => {
                let mut items: Vec<(String, bool)> = recent(env, mem).into_iter().filter(|(n, _)| matches(n)).collect();
                if items.is_empty() {
                    // Nothing used yet: your emoticons, so the tab isn't empty.
                    items = e.emoticons.iter().filter(|x| matches(&x.name)).map(|x| (x.name.clone(), false)).collect();
                }
                if items.is_empty() {
                    empty(ui, t!("Emoticons and stickers you use show here."));
                }
                if let Some((n, sticker)) = grid(ui, t, &items, t.dense(34.0)) {
                    use_item(env, to, input, mem, &n, sticker);
                }
            }
            PickerTab::Emoticons => {
                let items: Vec<(String, bool)> = e.emoticons.iter().filter(|x| matches(&x.name)).map(|x| (x.name.clone(), false)).collect();
                if e.emoticons.is_empty() {
                    empty(ui, t!("You don't have any emoticons yet. You get them from trading cards and the Points Shop."));
                }
                if let Some((n, _)) = grid(ui, t, &items, t.dense(34.0)) {
                    use_item(env, to, input, mem, &n, false);
                }
                if q.is_empty() {
                    ui.add_space(t.spacing.sm);
                    ui.label(w::caption_text(t, t!("EMOJI")));
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
                        for e in EMOJI {
                            let (r, resp) = ui.allocate_exact_size(vec2(t.dense(34.0), t.dense(34.0)), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
                            }
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, e, w::font(t.dense(22.0)), t.text);
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                input.push_str(e);
                            }
                        }
                    });
                }
            }
            PickerTab::Stickers => {
                let items: Vec<(String, bool)> = e.stickers.iter().filter(|x| matches(&x.name)).map(|x| (x.name.clone(), true)).collect();
                if e.stickers.is_empty() {
                    empty(ui, t!("You don't have any stickers yet. They come from the Points Shop."));
                }
                if let Some((n, _)) = grid(ui, t, &items, t.dense(72.0)) {
                    use_item(env, to, input, mem, &n, true);
                    ui.close();
                }
            }
            PickerTab::Effects => {
                if e.effects.is_empty() {
                    empty(ui, t!("You don't have any chat effects yet. They come from the Points Shop."));
                }
                for x in e.effects.iter().filter(|x| matches(&x.name)) {
                    if w::menu_item(ui, t, icons::SPARKLE, &x.name).on_hover_text(t!("Send this effect to the chat")).clicked() {
                        env.client.send(Command::SendChat { to, text: model::chat::effect(&x.name) });
                        ui.close();
                    }
                }
            }
        }
    });
}

/// An emoticon goes into the box; a sticker is sent right away.
fn use_item(env: &Env, to: ChatTarget, input: &mut String, mem: &mut ChatUi, name: &str, sticker: bool) {
    remember(mem, name, sticker);
    if sticker {
        env.client.send(Command::SendChat { to, text: model::chat::sticker(name) });
    } else {
        if !input.is_empty() && !input.ends_with(' ') {
            input.push(' ');
        }
        input.push_str(&format!(":{name}: "));
    }
}

/// "+": send a picture or your latest screenshot, share your trade link,
/// or invite a friend in (a one-to-one chat becomes a group chat).
fn attach_menu(ui: &mut Ui, t: &Theme, env: &Env, to: ChatTarget, mem: &mut ChatUi, out: &mut Vec<Action>) {
    let plus = w::icon_button(ui, t, icons::PLUS, t!("Send a picture, a screenshot, your trade link…"), false, 0);
    egui::Popup::menu(&plus).width(t.dense(280.0)).show(|ui| {
        if w::menu_item(ui, t, icons::IMAGE, t!("Send a picture…")).clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new().add_filter(t!("Pictures"), &["png", "jpg", "jpeg", "gif", "webp"]).pick_file() {
                env.client.send(Command::SendChatImage { to, path });
                out.push(Action::Toast(t!("Sending the picture…").to_string()));
            }
        }
        let shot = env.steam_root.and_then(|root| client::chat::newest_screenshot(root, env.state.steam_id));
        ui.add_enabled_ui(shot.is_some(), |ui| {
            let r = w::menu_item(ui, t, icons::CAMERA, t!("Send my latest screenshot"));
            let r = if shot.is_none() { r.on_disabled_hover_text(t!("No screenshots found")) } else { r };
            if r.clicked() {
                if let Some(path) = shot.clone() {
                    env.client.send(Command::SendChatImage { to, path });
                    out.push(Action::Toast(t!("Sending the screenshot…").to_string()));
                }
                ui.close();
            }
        });
        if w::menu_item(ui, t, icons::SWAP, t!("Share my trade offer link")).clicked() {
            if env.state.trade_link.as_ref().is_none_or(|r| r.is_err()) {
                env.client.send(Command::LoadTradeLink { new: false });
            }
            mem.share_trade = true;
            ui.close();
        }
        ui.separator();
        let (label, exclude): (&str, Vec<model::SteamId>) = match to {
            ChatTarget::Friend(f) => (t!("Invite a friend (makes a group chat)"), vec![f]),
            ChatTarget::Group { group, .. } => (t!("Invite a friend"), env.state.chat_group_members.get(&group).cloned().unwrap_or_default()),
        };
        ui.menu_button(format!("{}  {label}", icons::USER_PLUS), |ui| {
            let mut friends = env.state.friend_list();
            friends.retain(|p| !exclude.contains(&p.steam_id));
            friends.sort_by_key(|p| (p.state == model::PersonaState::Offline, p.display_name().to_lowercase()));
            if friends.is_empty() {
                ui.label(w::body_s(t, t!("Everyone's already here"), t.text_dim));
            }
            egui::ScrollArea::vertical().max_height(t.dense(300.0)).show(ui, |ui| {
                for p in friends {
                    if w::menu_item(ui, t, icons::USER, &p.display_name()).clicked() {
                        match to {
                            ChatTarget::Friend(f) => env.client.send(Command::CreateChatGroup { name: String::new(), partner: Some(f), invitees: vec![p.steam_id] }),
                            ChatTarget::Group { group, .. } => env.client.send(Command::InviteToChatGroup { group, friend: p.steam_id }),
                        }
                        out.push(Action::Toast(t!("Invited {name}", name = p.display_name()).to_string()));
                        ui.close();
                    }
                }
            });
        });
    });
}

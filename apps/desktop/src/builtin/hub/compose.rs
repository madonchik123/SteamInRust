//! Starting a new discussion: a title, the text with Steam's formatting
//! (BBCode) and a preview, which forum it goes in, and Steam's answer.

use super::{HubView, Io, View, rich};
use client::community_actions::{NewTopicForm, new_topic_request};
use client::hub::{self, Request};
use egui::text::{CCursor, CCursorRange};
use egui::{Align, Layout, Sense, Ui, vec2};
use i18n::t;
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

/// A discussion being written.
#[derive(Debug, Default)]
pub(super) struct Compose {
    /// The forum's topics key (without the page).
    pub key: String,
    subject: String,
    body: String,
    subforum: Option<String>,
    preview: bool,
    /// How many answers had come back when this one was sent.
    sent: Option<u32>,
    /// The cursor has been put in the title, once, on opening.
    focused: bool,
    error: Option<String>,
}

impl Compose {
    pub fn new(key: String) -> Self {
        Self { key, ..Default::default() }
    }
}

/// The forum's formatting buttons: what each puts around the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Bold,
    Italic,
    Underline,
    Strike,
    Heading,
    Spoiler,
    Quote,
    Code,
    NoParse,
    Link,
    List,
    Rule,
}

impl Format {
    const ALL: [Format; 12] = [Format::Bold, Format::Italic, Format::Underline, Format::Strike, Format::Heading, Format::Spoiler, Format::Quote, Format::Code, Format::NoParse, Format::Link, Format::List, Format::Rule];

    fn tags(self) -> (&'static str, &'static str) {
        match self {
            Format::Bold => ("[b]", "[/b]"),
            Format::Italic => ("[i]", "[/i]"),
            Format::Underline => ("[u]", "[/u]"),
            Format::Strike => ("[strike]", "[/strike]"),
            Format::Heading => ("[h1]", "[/h1]"),
            Format::Spoiler => ("[spoiler]", "[/spoiler]"),
            Format::Quote => ("[quote]", "[/quote]"),
            Format::Code => ("[code]", "[/code]"),
            Format::NoParse => ("[noparse]", "[/noparse]"),
            Format::Link => ("[url=https://]", "[/url]"),
            Format::List => ("[list]\n[*]", "\n[/list]"),
            Format::Rule => ("[hr]", "[/hr]"),
        }
    }

    /// The button's short face and what it does.
    fn face(self) -> (&'static str, String) {
        match self {
            Format::Bold => ("B", t!("Bold").to_string()),
            Format::Italic => ("I", t!("Italic").to_string()),
            Format::Underline => ("U", t!("Underline").to_string()),
            Format::Strike => ("S", t!("Strikethrough").to_string()),
            Format::Heading => ("H1", t!("Heading").to_string()),
            Format::Spoiler => (icons::EYE_SLASH, t!("Spoiler").to_string()),
            Format::Quote => ("❝", t!("Quote").to_string()),
            Format::Code => ("</>", t!("Code").to_string()),
            Format::NoParse => ("[ ]", t!("Show tags as plain text").to_string()),
            Format::Link => (icons::LINK, t!("Link").to_string()),
            Format::List => (icons::LIST, t!("List").to_string()),
            Format::Rule => ("—", t!("Horizontal line").to_string()),
        }
    }
}

/// Put `format`'s tags around the characters `from..to` of `text`. Returns
/// the selection after it: the same words, inside the tags.
fn apply_format(text: &mut String, from: usize, to: usize, format: Format) -> (usize, usize) {
    let count = text.chars().count();
    let (from, to) = (from.min(to).min(count), from.max(to).min(count));
    let byte = |text: &str, c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    let (open, close) = format.tags();
    let (a, b) = (byte(text, from), byte(text, to));
    text.insert_str(b, close);
    text.insert_str(a, open);
    let shift = open.chars().count();
    (from + shift, to + shift)
}

/// The writing box, above the forum's topics.
pub(super) fn show(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, form: &NewTopicForm) {
    answered(hv, io);
    let Some(mut c) = hv.compose.take() else { return };
    let mut close = false;
    let mut submit = false;
    let sending = c.sent.is_some();
    let body_id = egui::Id::unique("hub-new-topic-body");
    w::post_card(t).inner_margin(egui::Margin::same(t.spacing.lg as i8 + 3)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = t.spacing.md;
        ui.horizontal(|ui| {
            ui.label(w::heading_s(t, t!("Start a new discussion"), t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked();
            });
        });
        // The title.
        let input = &t.components.input;
        let field = egui::Margin::symmetric(t.padding_of(input).x as i8, t.spacing.sm as i8 + 2);
        w::component_frame(t, input).inner_margin(field).show(ui, |ui| {
            let title = ui
                .add_enabled(
                    !sending,
                    egui::TextEdit::singleline(&mut c.subject).hint_text(w::body_m(t, t!("Title"), t.text_dim)).font(t.typography.body_l.font()).text_color(t.text).frame(egui::Frame::NONE).desired_width(f32::INFINITY),
                )
                .with_edit_menu();
            if !std::mem::replace(&mut c.focused, true) {
                title.request_focus();
            }
        });
        // Write / Preview, and the formatting buttons while writing.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.xs, t.spacing.xs);
            let mut preview = c.preview;
            w::segmented(ui, t, &mut preview, &[(false, t!("Write")), (true, t!("Preview"))]);
            c.preview = preview;
            if !c.preview {
                ui.add_space(t.spacing.sm);
                for f in Format::ALL {
                    let (face, tip) = f.face();
                    if format_button(ui, t, face, !sending).on_hover_text(tip).clicked() {
                        let chars = c.body.chars().count();
                        let (a, b) = egui::TextEdit::load_state(ui.ctx(), body_id).and_then(|s| s.cursor.char_range()).map_or((chars, chars), |r| (r.primary.index.into(), r.secondary.index.into()));
                        let (from, to) = apply_format(&mut c.body, a, b, f);
                        let mut state = egui::TextEdit::load_state(ui.ctx(), body_id).unwrap_or_default();
                        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(from), CCursor::new(to))));
                        state.store(ui.ctx(), body_id);
                        ui.memory_mut(|m| m.request_focus(body_id));
                    }
                }
            }
        });
        // The text, or how it will look.
        if c.preview {
            w::component_frame(t, input).inner_margin(egui::Margin::same(t.spacing.md as i8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(t.dense(170.0));
                if !c.subject.trim().is_empty() {
                    ui.add(egui::Label::new(w::heading_m(t, c.subject.trim(), t.text)).wrap());
                }
                if c.body.trim().is_empty() {
                    ui.label(w::body_m(t, t!("Nothing to preview yet."), t.text_dim));
                } else if let Some(click) = rich::show(ui, t, &rich::parse(c.body.trim()), t.typography.body_l.size) {
                    hv.follow(io, click);
                }
            });
        } else {
            w::component_frame(t, input).inner_margin(field).show(ui, |ui| {
                ui.add_enabled(
                    !sending,
                    egui::TextEdit::multiline(&mut c.body)
                        .id(body_id)
                        .hint_text(w::body_m(t, t!("Say something…"), t.text_dim))
                        .font(t.typography.body_m.font())
                        .text_color(t.text)
                        .frame(egui::Frame::NONE)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY),
                )
                .with_edit_menu();
            });
        }
        if let Some(e) = &c.error {
            ui.add(egui::Label::new(w::body_m(t, e, t.danger)).wrap());
        }
        // Which forum, and Post.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.sm;
            // As tall as the dropdown, so the label sits level with it.
            ui.set_row_height(t.height_of(&t.components.sort, 39.0));
            if form.subforums.len() > 1 {
                ui.label(w::body_m(t, t!("Post in"), t.text_dim));
                let current = c.subforum.as_deref().and_then(|v| form.subforums.iter().find(|s| s.value == v)).or_else(|| form.default_subforum());
                let r = w::dropdown(ui, t, current.map_or("", |s| s.name.as_str()), t.dense(200.0));
                egui::Popup::menu(&r).width(280.0).show(|ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for s in &form.subforums {
                            if w::menu_item(ui, t, icons::CHATS, &s.name).clicked() {
                                c.subforum = Some(s.value.clone());
                                ui.close();
                            }
                        }
                    });
                });
            }
            ui.label(w::body_s(t, t!("Everyone can see what you post here."), t.text_faint));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ready = !sending && !c.subject.trim().is_empty() && !c.body.trim().is_empty();
                let label = if sending { t!("Posting…") } else { t!("Post discussion") };
                submit = ui.add_enabled_ui(ready, |ui| w::button(ui, t, Some(icons::SEND), label, w::Kind::Primary, w::Size::Md).clicked()).inner;
                if !sending && w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                    close = true;
                }
            });
        });
    });
    ui.add_space(t.spacing.lg);
    if submit {
        send(hv, io, &mut c, form);
    }
    // `send` may have finished it (in the demo).
    if !close && !c.key.is_empty() {
        hv.compose = Some(c);
    }
}

/// A small formatting button.
fn format_button(ui: &mut Ui, t: &Theme, face: &str, enabled: bool) -> egui::Response {
    let c = &t.components.chip;
    let h = t.height_of(c, 30.0);
    let (r, resp) = ui.allocate_exact_size(vec2(h.max(ui.painter().layout_no_wrap(face.to_string(), t.font_of(c), t.text).size().x + t.spacing.md), h), if enabled { Sense::click() } else { Sense::hover() });
    let look = t.look(c, w::interact(ui, &resp, false));
    w::paint_box(ui, t, r, c, &look, 0.0);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, face, t.font_of(c), look.text);
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default })
}

/// Check what Steam would refuse, then send (in `--demo`, it's only added
/// to this forum's list).
fn send(hv: &mut HubView, io: &mut Io, c: &mut Compose, form: &NewTopicForm) {
    let subforum = c.subforum.clone().or_else(|| form.default_subforum().map(|s| s.value.clone()));
    if let Err(e) = new_topic_request(form, "", &c.subject, &c.body, subforum.as_deref()) {
        c.error = Some(i18n::tr_dyn(&e));
        return;
    }
    c.error = None;
    if super::is_demo() {
        let me = io.state.persona(io.state.steam_id);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let topic = hub::Topic { id: now, title: c.subject.trim().to_string(), author: me.name, last_post: now, preview: c.body.trim().chars().take(200).collect(), ..Default::default() };
        hv.local_topics.entry(c.key.clone()).or_default().insert(0, topic);
        io.actions.push(extensions::Action::Toast(t!("Discussion posted").to_string()));
        c.key.clear();
        return;
    }
    c.sent = Some(io.state.hub.topic_created.get(&c.key).map_or(0, |(n, _)| *n));
    io.send(client::Command::Hub(Request::NewTopic { key: c.key.clone(), form: form.clone(), subject: c.subject.trim().to_string(), text: c.body.trim().to_string(), subforum }));
}

/// Steam answered: on to the new topic, or show why not (keeping the text).
fn answered(hv: &mut HubView, io: &mut Io) {
    let Some(c) = hv.compose.as_mut() else { return };
    let Some(sent) = c.sent else { return };
    let Some((n, result)) = io.state.hub.topic_created.get(&c.key) else { return };
    if *n <= sent {
        return;
    }
    c.sent = None;
    match result {
        Ok(url) => {
            let owner = format!("{}/", c.key.split('/').next().unwrap_or(&c.key));
            hv.asked.retain(|k| !k.starts_with(&owner));
            hv.compose = None;
            io.actions.push(extensions::Action::Toast(t!("Discussion posted").to_string()));
            if let Some(v @ View::Thread(_)) = super::route(url) {
                io.go(v);
            }
        }
        Err(e) => c.error = Some(t!("Couldn't post your discussion: {error}", error = i18n::tr_dyn(e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_selection() {
        let mut s = "say hello now".to_string();
        let (a, b) = apply_format(&mut s, 4, 9, Format::Bold);
        assert_eq!(s, "say [b]hello[/b] now");
        assert_eq!(s.chars().skip(a).take(b - a).collect::<String>(), "hello");
        // Selected backwards, past the end, and in other alphabets.
        let mut s = "привет мир".to_string();
        let (a, b) = apply_format(&mut s, 10, 7, Format::Spoiler);
        assert_eq!(s, "привет [spoiler]мир[/spoiler]");
        assert_eq!(s.chars().skip(a).take(b - a).collect::<String>(), "мир");
        let mut s = "x".to_string();
        apply_format(&mut s, 50, 90, Format::Rule);
        assert_eq!(s, "x[hr][/hr]");
    }

    #[test]
    fn a_list_starts_with_an_item() {
        let mut s = "one".to_string();
        apply_format(&mut s, 0, 3, Format::List);
        assert_eq!(s, "[list]\n[*]one\n[/list]");
    }
}

//! Discussions: a game's forums with their topics (Figma `505:8934`,
//! `505:9758`), and threads with pages (`504:7960`).

use super::{HubView, Io, View, error, rich};
use client::hub::{self, Request};
use egui::{Align, Align2, Layout, Sense, Ui, vec2};
use i18n::t;
use model::AppId;
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

/// Width of the forums column.
const SIDE: f32 = 285.0;

/// The forums column and the main column beside it (stacked when narrow).
fn side_by_side(ui: &mut Ui, t: &Theme, side: impl FnOnce(&mut Ui), main: impl FnOnce(&mut Ui)) {
    let total = ui.available_width();
    let gap = t.spacing.lg + 2.0;
    if total < 820.0 {
        main(ui);
        ui.add_space(gap);
        side(ui);
        return;
    }
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(vec2(SIDE, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(SIDE);
            side(ui);
        });
        let rest = ui.available_width();
        ui.allocate_ui_with_layout(vec2(rest, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(rest);
            main(ui);
        });
    });
}

/// A list of forums with their thread counts; returns the one clicked.
fn forum_list(ui: &mut Ui, t: &Theme, title: &str, count: &str, forums: &[hub::Forum], current: impl Fn(&hub::Forum) -> bool) -> Option<hub::Forum> {
    let mut picked = None;
    w::filter_bar(t).inner_margin(egui::Margin::same(t.spacing.lg as i8 - 2)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = t.spacing.xs;
        ui.horizontal(|ui| {
            ui.label(w::body_m(t, title, t.text_dim));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(w::body_m(t, count, t.text_dim));
            });
        });
        ui.add_space(t.spacing.xs);
        for f in forums {
            let on = current(f);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 45.0), Sense::click());
            let c = &t.components.sort;
            let look = t.look(c, w::interact(ui, &resp, on));
            w::paint_box(ui, t, r, c, &look, if on { 1.0 } else { 0.0 });
            let pad = t.spacing.md + 1.0;
            let mut job = egui::text::LayoutJob::simple_singleline(f.name.clone(), t.typography.body_l.font(), t.text);
            job.wrap.max_width = r.width() - pad * 2.0 - 70.0;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(egui::pos2(r.left() + pad, r.center().y - g.size().y / 2.0), g, t.text);
            if f.threads > 0 {
                ui.painter().text(egui::pos2(r.right() - pad, r.center().y), Align2::RIGHT_CENTER, super::files::group(f.threads), t.typography.body_m.font(), t.text_dim);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                picked = Some(f.clone());
            }
        }
    });
    picked
}

/// Whose forum it is.
#[derive(Clone, Copy)]
pub(super) enum Place<'a> {
    App(AppId),
    Group(&'a str),
}

impl Place<'_> {
    fn key(self, forum: &str, page: u32) -> String {
        match self {
            Place::App(app) => hub::topics_key(app, forum, page),
            Place::Group(group) => hub::group_topics_key(group, forum, page),
        }
    }

    fn request(self, forum: &str, page: u32, language: String) -> Request {
        match self {
            Place::App(app_id) => Request::Topics { app_id, forum: forum.to_string(), page, language },
            Place::Group(group) => Request::GroupTopics { group: group.to_string(), forum: forum.to_string(), page, language },
        }
    }

    fn view(self, forum: String) -> View {
        match self {
            Place::App(app) => View::Forum { app, forum },
            Place::Group(group) => View::GroupForum { group: group.to_string(), forum },
        }
    }
}

pub(super) fn forum(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, place: Place, forum: &str) {
    let page_key = format!("forum{}", place.key(forum, 0));
    let page = hv.pages.get(&page_key).copied().unwrap_or(1);
    let key = place.key(forum, page);
    // The forum's key without its page, for a discussion written here.
    let forum_key = key.rsplit_once('#').map_or(key.clone(), |(k, _)| k.to_string());
    let lang = io.lang();
    let req = || place.request(forum, page, lang.clone());
    let have = io.state.hub.topics.contains_key(&key);
    hv.ask(io, key.clone(), have, req);
    let data = match io.state.hub.topics.get(&key) {
        None => return w::loading(ui, t, &t!("Loading…")),
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, req());
            }
            return;
        }
        Some(Ok(d)) => d.clone(),
    };
    let mut go = None;
    let mut picked = None;
    side_by_side(
        ui,
        t,
        |ui| {
            let current = |f: &hub::Forum| f.name == data.forum || (forum.is_empty() && f.path == "discussions/0/") || f.path == forum;
            if let Some(f) = forum_list(ui, t, &t!("Forums"), &t!("Threads"), &data.forums, current) {
                picked = Some(place.view(f.path));
            }
        },
        |ui| {
            ui.horizontal(|ui| {
                ui.label(w::heading_s(t, if data.forum.is_empty() { t!("Discussions").to_string() } else { data.forum.clone() }, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    new_topic_button(hv, ui, t, io, data.new_topic.as_ref(), &forum_key);
                    let from = (page as u64 - 1) * data.page_size as u64 + 1;
                    let to = (from + data.topics.len() as u64).saturating_sub(1);
                    if data.total > 0 {
                        ui.label(w::body_s(t, t!("Showing {from}–{to} of {total} topics", from = from, to = to, total = data.total), t.text_dim));
                    }
                });
            });
            ui.add_space(t.spacing.sm);
            if let Some(form) = data.new_topic.as_ref().filter(|_| hv.compose.as_ref().is_some_and(|c| c.key == forum_key)) {
                super::compose::show(hv, ui, t, io, form);
            }
            let mine = hv.local_topics.get(&forum_key).cloned().unwrap_or_default();
            if data.topics.is_empty() && mine.is_empty() {
                w::empty(ui, t, icons::CHATS, t!("No discussions yet"), t!("Nobody has started a topic here."));
                return;
            }
            for topic in mine.iter().chain(&data.topics) {
                if topic_row(ui, t, topic) && !topic.url.is_empty() {
                    go = Some(View::Thread(topic.url.clone()));
                }
            }
            let pages = (data.total.div_ceil(data.page_size.max(1) as u64) as u32).max(1);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if let Some(p) = w::page_numbers(ui, t, page, pages) {
                        hv.pages.insert(page_key.clone(), p);
                    }
            });
        },
    );
    if let Some(v) = go.or(picked) {
        io.go(v);
    }
}

/// "Start a new discussion", or why you can't here.
fn new_topic_button(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &Io, form: Option<&client::community_actions::NewTopicForm>, forum_key: &str) {
    let Some(form) = form else { return };
    if io.state.steam_id == 0 || !form.has_form {
        ui.label(w::body_s(t, t!("Sign in to start a discussion"), t.text_dim));
    } else if form.banned {
        ui.label(w::body_s(t, t!("You're banned from posting in this forum."), t.danger));
    } else if !form.can_post {
        ui.label(w::body_s(t, t!("You can't post in this forum."), t.text_dim));
    } else if !hv.compose.as_ref().is_some_and(|c| c.key == forum_key) && w::button(ui, t, Some(icons::PLUS), t!("Start a new discussion"), w::Kind::Primary, w::Size::Sm).clicked() {
        hv.compose = Some(super::compose::Compose::new(forum_key.to_string()));
    }
    ui.add_space(t.spacing.md);
}

/// One topic (Figma `505:8934`): its title and author, when it was last
/// answered and how many replies. True when clicked.
fn topic_row(ui: &mut Ui, t: &Theme, topic: &hub::Topic) -> bool {
    let pad = t.spacing.lg + 3.0;
    let id = egui::Id::unique(("topic", topic.id, &topic.url));
    let margin = Some(egui::Margin::symmetric(pad as i8, t.spacing.lg as i8));
    let (_, resp) = w::hover_frame(ui, t, &t.components.post_card, id, margin, |ui| {
        ui.set_width(ui.available_width());
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
        let right_w = 330.0f32.min(r.width() * 0.45);
        let text_w = (r.width() - right_w).max(80.0);
        let mut x = r.left();
        if topic.pinned || topic.locked {
            let icon = if topic.locked { icons::LOCK_SIMPLE } else { icons::PUSH_PIN };
            w::icon_centered(ui.painter(), egui::pos2(x + 8.0, r.top() + 11.0), icon, 16.0, t.accent);
            x += 24.0;
        }
        let mut job = egui::text::LayoutJob::simple_singleline(topic.title.clone(), w::font_bold(t.typography.body_l.size), t.text);
        job.wrap.max_width = text_w - (x - r.left());
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(x, r.top()), g, t.text);
        ui.painter().text(egui::pos2(r.left(), r.bottom()), Align2::LEFT_BOTTOM, &topic.author, t.typography.body_s.font(), t.accent);
        // Replies, then when it was last answered, from the right.
        let y = r.center().y;
        let replies = super::files::group(topic.replies as u64);
        let g = ui.painter().layout_no_wrap(replies, t.typography.body_l.font(), t.accent);
        let rx = r.right() - g.size().x;
        let gw = g.size().x;
        ui.painter().galley(egui::pos2(rx, y - g.size().y / 2.0), g, t.accent);
        w::icon_centered(ui.painter(), egui::pos2(rx - 14.0, y), icons::CHAT, 18.0, t.text_dim);
        if topic.last_post > 0 {
            ui.painter().text(egui::pos2(rx - 14.0 - 12.0 - 22.0, y), Align2::RIGHT_CENTER, ui::ago(topic.last_post), t.typography.body_l.font(), t.accent);
        }
        let _ = gw;
    });
    ui.add_space(t.spacing.lg - ui.spacing().item_spacing.y);
    let resp = if topic.preview.is_empty() { resp } else { resp.on_hover_text(&topic.preview) };
    resp.clicked()
}

pub(super) fn thread(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, url: &str) {
    reply_answered(hv, io, url);
    let page = hv.pages.get(url).copied().unwrap_or(1);
    let key = hub::thread_key(url, page);
    let lang = io.lang();
    let req = || Request::Thread { url: url.to_string(), page, language: lang.clone() };
    let have = io.state.hub.threads.contains_key(&key);
    hv.ask(io, key.clone(), have, req);
    ui.add_space(if io.overlay { t.spacing.sm } else { t.spacing.xl + t.spacing.lg });
    let data = match io.state.hub.threads.get(&key) {
        None => return w::loading(ui, t, &t!("Loading…")),
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, req());
            }
            return;
        }
        Some(Ok(d)) => d.clone(),
    };
    let app = View::Thread(url.to_string()).app();
    // Breadcrumb: Discussions > the game > the forum.
    let forum_view = || match super::route(&data.forum_url) {
        Some(v) => Some(v),
        None => app.map(|app| View::Forum { app, forum: String::new() }),
    };
    let mut go = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.xs;
        let crumb = |ui: &mut Ui, label: &str| ui.add(egui::Label::new(w::body_s(t, label, t.accent)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        if !io.overlay && crumb(ui, &t!("Discussions")) {
            go = Some(View::Page(crate::builtin::community_home::PAGE.into(), Some("discussions".into())));
        }
        if let Some(app) = app {
            ui.label(w::body_s(t, ">", t.text_dim));
            let game = io.state.app_name(app).unwrap_or_default().to_string();
            if crumb(ui, &game) {
                go = Some(View::Forum { app, forum: String::new() });
            }
        }
        if !data.forum.is_empty() {
            ui.label(w::body_s(t, ">", t.text_dim));
            if crumb(ui, &data.forum) {
                go = forum_view();
            }
        }
        ui.label(w::body_s(t, ">", t.text_dim));
        ui.label(w::body_s(t, t!("Topic"), t.text_dim));
    });
    ui.add_space(t.spacing.xs);
    ui.horizontal(|ui| {
        ui.label(w::heading_m(t, if data.forum.is_empty() { t!("Discussions").to_string() } else { data.forum.clone() }, t.text));
        if data.locked {
            ui.label(w::body_m(t, format!("{}  {}", icons::LOCK_SIMPLE, t!("Locked")), t.text_dim));
        }
    });
    ui.add_space(t.spacing.xl + t.spacing.lg);
    // The game's other forums, when they were loaded.
    let forums: Vec<hub::Forum> = app
        .and_then(|a| io.state.hub.topics.iter().find(|(k, v)| k.starts_with(&format!("{a}/")) && v.is_ok()).and_then(|(_, v)| v.as_ref().ok()).map(|p| p.forums.clone()))
        .unwrap_or_default();
    let pages = data.total.div_ceil(data.page_size.max(1)).max(1);
    let body = |ui: &mut Ui, hv: &mut HubView, io: &mut Io| {
        if page == 1 {
            post(hv, ui, t, io, &data.op, Some(&data.title));
        }
        let first = (page - 1) * data.page_size;
        let shown = |ui: &mut Ui, hv: &mut HubView| {
            ui.horizontal(|ui| {
                if data.total > 0 {
                    let to = (first + data.posts.len() as u32).max(first + 1);
                    ui.label(w::body_l(t, t!("Showing {from}–{to} of {total} comments", from = first + 1, to = to, total = data.total), t.text_dim));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(p) = w::page_numbers(ui, t, page, pages) {
                            hv.pages.insert(url.to_string(), p);
                        }
                });
            });
            ui.add_space(t.spacing.lg);
        };
        shown(ui, hv);
        for p in &data.posts {
            post(hv, ui, t, io, p, None);
        }
        if pages > 1 {
            shown(ui, hv);
        }
        if let Some(target) = &data.reply {
            // The new reply lands on the last page.
            let last = (data.total + 1).div_ceil(data.page_size.max(1)).max(1);
            reply_box(hv, ui, t, io, url, target, last);
        }
    };
    if forums.is_empty() || io.overlay {
        body(ui, hv, io);
    } else {
        let mut picked = None;
        side_by_side(
            ui,
            t,
            |ui| {
                let current = |f: &hub::Forum| f.name == data.forum;
                picked = forum_list(ui, t, &t!("Forums"), &t!("Threads"), &forums, current);
            },
            |ui| {
                body(ui, hv, io);
            },
        );
        if let (Some(f), Some(app)) = (picked, app) {
            go = Some(View::Forum { app, forum: f.path });
        }
    }
    if let Some(v) = go {
        io.go(v);
    }
}

/// A reply was answered: on success forget the old pages and show the last one.
fn reply_answered(hv: &mut HubView, io: &Io, url: &str) {
    let Some((n, result)) = io.state.hub.replied.get(url) else { return };
    if hv.reply_seen.get(url) == Some(n) {
        return;
    }
    hv.reply_seen.insert(url.to_string(), *n);
    let page = hv.reply_sending.remove(url);
    if result.is_ok() {
        hv.reply_text.remove(url);
        let prefix = format!("{url}#");
        hv.asked.retain(|k| !k.starts_with(&prefix));
        if let Some(p) = page {
            hv.pages.insert(url.to_string(), p);
        }
    }
}

fn reply_box(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, url: &str, target: &hub::ReplyTarget, last: u32) {
    ui.add_space(16.0);
    ui.label(w::heading_s(t, t!("Reply"), t.text));
    ui.add_space(6.0);
    let sending = hv.reply_sending.contains_key(url);
    let draft = hv.reply_text.entry(url.to_string()).or_default();
    ui.add_enabled(!sending, egui::TextEdit::multiline(draft).hint_text(t!("Write a reply…")).desired_rows(3).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
    ui.add_space(4.0);
    let text = draft.trim().to_string();
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!sending && !text.is_empty(), |ui| {
            let label = if sending { t!("Posting…") } else { t!("Post reply") };
            if w::button(ui, t, Some(icons::SEND), label, w::Kind::Primary, w::Size::Sm).clicked() {
                hv.reply_sending.insert(url.to_string(), last);
                io.send(client::Command::Hub(Request::Reply { thread_url: url.to_string(), target: target.clone(), text }));
            }
        });
        ui.label(w::body_s(t, t!("Everyone can see your reply."), t.text_faint));
    });
    if let Some((_, Err(e))) = io.state.hub.replied.get(url).filter(|_| !sending) {
        ui.label(w::body_s(t, t!("Couldn't post your reply: {error}", error = i18n::tr_dyn(e)), t.danger));
    }
}

/// One post (Figma `504:7960`): avatar, name, when; the topic's title on
/// the opening post; the text.
fn post(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, p: &hub::Post, title: Option<&str>) {
    w::post_card(t).inner_margin(egui::Margin::same(t.spacing.lg as i8 + 3)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.md;
            let avatar = p.avatar.clone().unwrap_or_else(|| model::art::avatar(None));
            let r = w::avatar(ui, t, &avatar, 40.0, None);
            let name = ui.add(egui::Label::new(w::body_l(t, &p.name, t.accent)).sense(Sense::click()));
            if (r.clicked() || name.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()) && p.author != 0 {
                io.go(View::Page("profile".into(), Some(p.author.to_string())));
            }
            if p.time > 0 {
                ui.label(w::body_s(t, t!("Posted on {date}", date = ui::date_time(p.time)), t.text_dim)).on_hover_text(ui::ago(p.time));
            }
        });
        if let Some(title) = title {
            ui.add_space(t.spacing.lg);
            ui.add(egui::Label::new(w::heading_m(t, title, t.text)).wrap());
        }
        ui.add_space(t.spacing.lg);
        let doc = rich::parse(&p.html);
        if let Some(click) = rich::show(ui, t, &doc, t.typography.body_l.size) {
            hv.follow(io, click);
        }
    });
    ui.add_space(t.spacing.lg);
}

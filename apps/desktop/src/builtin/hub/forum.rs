//! Discussions: a game's forums with their topics, and threads with pages.
//! Reading only; replying and starting topics are left to Steam for now.

use super::{HubView, Io, View, columns, error, pager, rich};
use client::hub::{self, Request};
use egui::{Align, Align2, CornerRadius, Layout, Sense, Ui, vec2};
use i18n::t;
use model::AppId;
use ui::{Theme, icons, widgets as w};

pub(super) fn forum(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId, forum: &str) {
    let page_key = format!("forum{app}/{forum}");
    let page = hv.pages.get(&page_key).copied().unwrap_or(1);
    let key = hub::topics_key(app, forum, page);
    let lang = io.lang();
    let req = || Request::Topics { app_id: app, forum: forum.to_string(), page, language: lang.clone() };
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
    let side = if io.overlay { 220.0 } else { 280.0 };
    columns(
        ui,
        side,
        hv,
        io,
        |ui, hv, io| {
            ui.horizontal(|ui| {
                ui.label(w::text_bold(if data.forum.is_empty() { t!("Discussions").to_string() } else { data.forum.clone() }, 20.0, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let from = (page as u64 - 1) * data.page_size as u64 + 1;
                    let to = (from + data.topics.len() as u64).saturating_sub(1);
                    if data.total > 0 {
                        ui.label(w::text(t!("Showing {from}–{to} of {total} topics", from = from, to = to, total = data.total), 12.0, t.text_faint));
                    }
                });
            });
            ui.add_space(6.0);
            if data.topics.is_empty() {
                w::empty(ui, t, icons::CHATS, t!("No discussions yet"), t!("Nobody has started a topic here."));
                return;
            }
            for topic in &data.topics {
                if topic_row(ui, t, topic) {
                    io.go(View::Thread(topic.url.clone()));
                }
            }
            ui.add_space(10.0);
            let pages = (data.total.div_ceil(data.page_size.max(1) as u64) as u32).max(1);
            if let Some(p) = pager(ui, t, page, pages) {
                hv.pages.insert(page_key.clone(), p);
            }
        },
        |ui, _hv, io| {
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(w::text_bold(t!("SUB FORUMS"), 11.0, t.text_faint));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(w::text_bold(t!("THREADS"), 11.0, t.text_faint));
                    });
                });
                ui.add_space(4.0);
                for f in &data.forums {
                    let current = f.name == data.forum || (forum.is_empty() && f.path == "discussions/0/") || f.path == forum;
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                    if current || resp.hovered() {
                        ui.painter().rect_filled(r, t.corner_small(), if current { t.accent.gamma_multiply(0.16) } else { t.surface_hover });
                    }
                    let name = ui.painter().layout(f.name.clone(), w::font_medium(13.0), if current { t.accent } else { t.text }, r.width() - 70.0);
                    ui.painter().galley(egui::pos2(r.left() + 8.0, r.center().y - name.size().y / 2.0), name, t.text);
                    ui.painter().text(egui::pos2(r.right() - 8.0, r.center().y), Align2::RIGHT_CENTER, super::files::group(f.threads), w::font(12.0), t.text_faint);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !current {
                        io.go(View::Forum { app, forum: f.path.clone() });
                    }
                }
            });
            ui.add_space(10.0);
            ui.label(w::text(t!("Reading only for now: replying and starting topics happen on Steam."), 11.5, t.text_faint));
        },
    );
}

/// One topic in the list. True when clicked.
fn topic_row(ui: &mut Ui, t: &Theme, topic: &hub::Topic) -> bool {
    let width = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(width, 54.0), Sense::click());
    let hover = resp.hovered();
    ui.painter().rect_filled(r, CornerRadius::same(3), if hover { t.surface_hover } else if topic.pinned { t.surface } else { t.bg });
    let icon = if topic.locked { icons::LOCK_SIMPLE } else if topic.pinned { icons::PUSH_PIN } else { icons::CHAT };
    ui.painter().text(egui::pos2(r.left() + 18.0, r.center().y), Align2::CENTER_CENTER, icon, w::font(16.0), if topic.pinned { t.accent } else { t.text_faint });
    let right_w = 190.0;
    let text_w = (width - 40.0 - right_w).max(80.0);
    let mut title = topic.title.clone();
    if topic.pinned {
        title = format!("{}: {title}", t!("Pinned"));
    }
    let g = ui.painter().layout_no_wrap(title, if topic.pinned { w::font_semibold(14.0) } else { w::font_medium(14.0) }, t.text);
    let g = if g.size().x > text_w { ui.painter().layout(topic.title.clone(), w::font_medium(14.0), t.text, text_w) } else { g };
    let clip = ui.painter().with_clip_rect(egui::Rect::from_min_size(egui::pos2(r.left() + 36.0, r.top()), vec2(text_w, r.height())));
    clip.galley(egui::pos2(r.left() + 36.0, r.top() + 8.0), g, t.text);
    clip.text(egui::pos2(r.left() + 36.0, r.bottom() - 12.0), Align2::LEFT_CENTER, &topic.author, w::font(12.0), t.text_faint);
    // Replies and last post on the right.
    let x = r.right() - 10.0;
    ui.painter().text(egui::pos2(x - 110.0, r.center().y), Align2::RIGHT_CENTER, format!("{}  {}", icons::CHAT, topic.replies), w::font(12.5), t.text_dim);
    if topic.last_post > 0 {
        ui.painter().text(egui::pos2(x, r.center().y), Align2::RIGHT_CENTER, ui::ago(topic.last_post), w::font(12.0), t.text_faint);
    }
    ui.add_space(2.0);
    let resp = if topic.preview.is_empty() { resp } else { resp.on_hover_text(&topic.preview) };
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

pub(super) fn thread(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, url: &str) {
    let page = hv.pages.get(url).copied().unwrap_or(1);
    let key = hub::thread_key(url, page);
    let lang = io.lang();
    let req = || Request::Thread { url: url.to_string(), page, language: lang.clone() };
    let have = io.state.hub.threads.contains_key(&key);
    hv.ask(io, key.clone(), have, req);
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
    // Breadcrumb back to the forum.
    ui.horizontal(|ui| {
        if !data.forum.is_empty() && ui.link(w::text(format!("{}  {}", icons::CARET_LEFT, data.forum), 13.0, t.accent)).clicked() {
            match super::route(&data.forum_url) {
                Some(v) => io.go(v),
                None => {
                    if let Some(app) = url.split("/app/").nth(1).and_then(|s| s.split('/').next()).and_then(|s| s.parse().ok()) {
                        io.go(View::Forum { app, forum: String::new() });
                    }
                }
            }
        }
        if data.locked {
            ui.label(w::text(format!("{}  {}", icons::LOCK_SIMPLE, t!("Locked")), 12.5, t.text_faint));
        }
    });
    ui.add_space(4.0);
    ui.add(egui::Label::new(w::text_bold(&data.title, if io.overlay { 19.0 } else { 24.0 }, t.text)).wrap());
    ui.add_space(10.0);
    if page == 1 {
        post(hv, ui, t, io, &data.op, None, true);
        ui.add_space(14.0);
    }
    let pages = data.total.div_ceil(data.page_size.max(1)).max(1);
    ui.horizontal(|ui| {
        ui.label(w::text_bold(t!("{n} replies", n = data.total), 13.0, t.text_dim));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(p) = pager_compact(ui, t, page, pages) {
                hv.pages.insert(url.to_string(), p);
            }
        });
    });
    ui.add_space(6.0);
    let first = (page - 1) * data.page_size;
    for (i, p) in data.posts.iter().enumerate() {
        post(hv, ui, t, io, p, Some(first + i as u32 + 1), false);
        ui.add_space(8.0);
    }
    if let Some(p) = pager(ui, t, page, pages) {
        hv.pages.insert(url.to_string(), p);
    }
}

fn pager_compact(ui: &mut Ui, t: &Theme, page: u32, pages: u32) -> Option<u32> {
    if pages <= 1 {
        return None;
    }
    let mut to = None;
    ui.add_enabled_ui(page < pages, |ui| {
        if w::icon_button(ui, t, icons::CARET_RIGHT, &t!("Next"), false, 0).clicked() {
            to = Some(page + 1);
        }
    });
    ui.label(w::text(format!("{page} / {pages}"), 12.5, t.text_dim));
    ui.add_enabled_ui(page > 1, |ui| {
        if w::icon_button(ui, t, icons::CARET_LEFT, &t!("Previous"), false, 0).clicked() {
            to = Some(page - 1);
        }
    });
    to
}

/// One post: author, time, number and the text.
fn post(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, p: &hub::Post, number: Option<u32>, op: bool) {
    let fill = if op { t.surface } else { t.surface.gamma_multiply(0.6) };
    egui::Frame::new().fill(fill).corner_radius(t.corner_small()).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let avatar = p.avatar.clone().unwrap_or_else(|| model::art::avatar(None));
            let r = w::avatar(ui, t, &avatar, if op { 40.0 } else { 32.0 }, None);
            let name = ui.add(egui::Label::new(w::text_semibold(&p.name, 14.0, if op { t.accent } else { t.text })).sense(Sense::click()));
            if (r.clicked() || name.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()) && p.author != 0 {
                io.go(View::Page("profile".into(), Some(p.author.to_string())));
            }
            if p.time > 0 {
                ui.label(w::text(ui::ago(p.time), 12.0, t.text_faint)).on_hover_text(ui::date(p.time));
            }
            if let Some(n) = number {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(w::text(format!("#{n}"), 12.0, t.text_faint));
                });
            }
        });
        ui.add_space(6.0);
        let doc = rich::parse(&p.html);
        if let Some(link) = rich::show(ui, t, &doc, 14.0) {
            hv.follow(io, &link);
        }
    });
}

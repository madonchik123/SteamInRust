//! A post in a social feed (Figma `494:5547`): the Community home's friends
//! activity, Explore and Developers sections, and a game's hub all draw
//! their posts with [`card`], laid out two to a row by [`masonry`].
//!
//! Read-only: the counts under a post open it, they don't vote.

use client::State;
use client::feed::{Post, PostKind};
use egui::{Align, CornerRadius, Layout, Rect, Sense, Ui, vec2};
use i18n::t;
use model::{AppId, SteamId, art};
use ui::{Theme, icons, widgets as w};

/// What was clicked in a post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// The post itself (a Steam link: a shared file, a review, an
    /// announcement, a broadcast).
    Open(String),
    /// The game's community hub.
    Hub(AppId),
    /// The game's store page.
    Store(AppId),
    Profile(SteamId),
    /// The game's news (an announcement).
    News(AppId),
}

/// What kind of post it is, for the line under a game hub's or Explore's
/// cards (Figma `495:6863`).
pub fn kind_label(kind: PostKind) -> String {
    match kind {
        PostKind::Screenshot => t!("Screenshot"),
        PostKind::Artwork => t!("Artwork"),
        PostKind::Video => t!("Video"),
        PostKind::Broadcast => t!("Broadcast"),
        PostKind::Workshop => t!("Workshop item"),
        PostKind::Guide => t!("Guide"),
        PostKind::Review => t!("Review"),
        PostKind::Announcement => t!("Announcement"),
        PostKind::Discussion => t!("Discussion"),
        PostKind::Achievement | PostKind::Purchase | PostKind::Status | PostKind::Other => return String::new(),
    }
    .to_string()
}

/// Whether a post has anything to show (never draw an empty card).
pub fn showable(p: &Post) -> bool {
    p.kind == PostKind::Review || !p.title.trim().is_empty() || !p.text.trim().is_empty() || !p.images.is_empty() || !p.action.trim().is_empty()
}

/// Where a click on the post itself goes.
fn open_target(p: &Post) -> Option<Click> {
    // Announcements read best in the game's news (in the app).
    if p.kind == PostKind::Announcement && p.app_id != 0 {
        return Some(Click::News(p.app_id));
    }
    (!p.url.is_empty()).then(|| Click::Open(p.url.clone())).or((p.app_id != 0).then_some(Click::Hub(p.app_id)))
}

/// The game's name for a post (Steam's, else the one we know).
fn game_name(state: &State, p: &Post) -> String {
    if !p.game.is_empty() {
        return p.game.clone();
    }
    state.app_name(p.app_id).unwrap_or_default().to_string()
}

/// "Posted on 12 Sep 2023", "Broadcasting now", or the day Steam gave.
fn when(p: &Post) -> String {
    if p.live {
        return t!("Live now").to_string();
    }
    if p.time > 0 {
        return t!("Posted on {date}", date = ui::date(p.time));
    }
    p.when.trim_start_matches("Posted: ").to_string()
}

/// 50600 -> "50,600".
pub fn number(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The post's main picture and its height for `width`.
fn picture(p: &Post, width: f32) -> Option<(String, f32)> {
    let url = match p.kind {
        PostKind::Review | PostKind::Status | PostKind::Achievement | PostKind::Other => return None,
        // A purchase shows the game's header, not Steam's small capsule.
        PostKind::Purchase if p.app_id != 0 => art::header(p.app_id),
        PostKind::Guide => return None,
        _ => p.images.first()?.clone(),
    };
    let ratio = match p.kind {
        PostKind::Purchase => 215.0 / 460.0,
        PostKind::Announcement => 0.45,
        _ => 402.0 / 737.0,
    };
    Some((url, (width * ratio).round()))
}

/// A rough height for a post before it was drawn once (the masonry then
/// uses the real one).
fn guess_height(t: &Theme, p: &Post, width: f32) -> f32 {
    let mut h = 15.0 * 2.0 + 27.0 + 20.0 + t.spacing.xl * 2.0 + 37.0;
    if let Some((_, ph)) = picture(p, width) {
        h += ph;
    }
    let chars = (width / 8.0).max(20.0);
    let lines = p.text.lines().map(|l| (l.chars().count() as f32 / chars).ceil().max(1.0)).sum::<f32>().min(14.0);
    h + lines * 19.0
}

/// One post, `width` wide.
/// `show_game`: the game's name and links on top (not inside its own hub).
pub fn card(ui: &mut Ui, t: &Theme, state: &State, p: &Post, width: f32, show_game: bool) -> Option<Click> {
    let mut click = None;
    let pad = t.padding_of(&t.components.post_card).x;
    let inner = (width - pad * 2.0).max(60.0);
    let id = egui::Id::unique(("post-card", &p.url, p.file_id, &p.title, &p.action, p.author));
    let (_, frame) = w::hover_frame(ui, t, &t.components.post_card, id, None, |ui| {
        ui.set_width(inner);
        ui.spacing_mut().item_spacing.y = t.spacing.lg;
        let game = game_name(state, p);
        // The game, with links to its hub and store page.
        if show_game && p.app_id != 0 && !game.is_empty() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.xs + 1.0;
                let r = ui.add(egui::Label::new(w::heading_s(t, &game, t.text)).truncate().sense(Sense::click()));
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    click = Some(Click::Hub(p.app_id));
                }
                if w::icon_button(ui, t, icons::GLOBE, t!("Community hub"), false, 0).clicked() {
                    click = Some(Click::Hub(p.app_id));
                }
                if w::icon_button(ui, t, icons::STOREFRONT, t!("Store page"), false, 0).clicked() {
                    click = Some(Click::Store(p.app_id));
                }
            });
        }
        if let Some(c) = author_row(ui, t, state, p) {
            click = Some(c);
        }
        if let Some(c) = body(ui, t, p, inner) {
            click = Some(c);
        }
        if let Some(c) = footer(ui, t, p) {
            click = Some(c);
        }
    });
    // Anywhere else on the card opens the post.
    if click.is_none() && frame.clicked() {
        click = open_target(p);
    }
    click
}

/// Avatar, name, what they did; the date on the right.
fn author_row(ui: &mut Ui, t: &Theme, state: &State, p: &Post) -> Option<Click> {
    let mut click = None;
    let persona = (p.author != 0).then(|| state.persona(p.author));
    let name = persona.as_ref().map(|x| x.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| p.author_name.clone());
    let avatar = p.avatar.clone().or_else(|| persona.as_ref().filter(|x| x.avatar_hash.is_some()).map(|x| art::avatar(x.avatar_hash.as_deref())));
    // A one-line event ("X and Y are now friends") reads as a sentence.
    let starts = |s: &str| !s.is_empty() && p.action.starts_with(s);
    let sentence = starts(&name) || starts(&p.author_name);
    let detail = match p.kind {
        PostKind::Broadcast if p.views > 0 => t!("{n} viewers", n = number(p.views)),
        _ if sentence => String::new(),
        // A friend's post says what they did; others say what they are.
        _ if p.action.is_empty() => kind_label(p.kind),
        _ => p.action.clone(),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.sm;
        let date = when(p);
        let date_w = if date.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(date.clone(), t.typography.body_s.font(), t.text_dim).size().x + t.spacing.md };
        let room = (ui.available_width() - date_w).max(40.0);
        ui.allocate_ui_with_layout(vec2(room, 20.0), Layout::left_to_right(Align::Center), |ui| {
            if let Some(a) = &avatar {
                let r = w::avatar(ui, t, a, 18.0, None);
                if p.author != 0 && r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    click = Some(Click::Profile(p.author));
                }
            } else if p.kind == PostKind::Announcement {
                ui.label(w::body_m(t, super::NEWSPAPER, t.text_dim));
            }
            let label = if sentence { p.action.trim_end_matches([':', ' ']).to_string() } else { name };
            if !label.is_empty() {
                let r = ui.add(egui::Label::new(w::body_m(t, label, t.text)).truncate().sense(Sense::click()));
                if p.author != 0 && r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    click = Some(Click::Profile(p.author));
                }
            }
            if !detail.is_empty() {
                ui.add(egui::Label::new(w::body_m(t, detail, t.text_dim)).truncate());
            }
        });
        if !date.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(w::body_s(t, date, t.text_dim));
            });
        }
    });
    click
}

fn body(ui: &mut Ui, t: &Theme, p: &Post, width: f32) -> Option<Click> {
    let mut click = None;
    let open = || open_target(p);
    match p.kind {
        PostKind::Review => {
            review_box(ui, t, p);
            if !p.text.is_empty() {
                text_block(ui, t, &p.text, width, 14, t.typography.body_l);
            }
        }
        PostKind::Achievement => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.md, t.spacing.sm);
                for (i, img) in p.images.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                    w::image_or_skeleton(ui, t, r, img, t.corner_small());
                    if let Some(name) = p.text.split("\n\n").nth(i) {
                        resp.on_hover_text(name);
                    }
                }
            });
            if let Some(first) = p.text.split("\n\n").next().filter(|s| !s.is_empty()) {
                let mut lines = first.lines();
                if let Some(name) = lines.next() {
                    ui.label(w::body_l(t, name, t.text));
                }
                let rest: Vec<&str> = lines.collect();
                if !rest.is_empty() {
                    ui.add_space(-t.spacing.md);
                    ui.label(w::body_m(t, rest.join(" "), t.text_dim));
                }
            }
        }
        _ => {
            if let Some((url, h)) = picture(p, width) {
                let (r, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
                if !w::paint_cover(ui, r, &url, t.corner_small()) {
                    w::skeleton(ui, t, r, t.corner_small());
                }
                if matches!(p.kind, PostKind::Video | PostKind::Broadcast) {
                    play_mark(ui, t, r);
                }
                if p.live {
                    live_badge(ui, t, r);
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    click = open();
                }
            }
            match p.kind {
                PostKind::Announcement | PostKind::Guide | PostKind::Discussion => {
                    if !p.title.is_empty() {
                        let r = ui.add(egui::Label::new(w::heading_s(t, &p.title, t.text)).wrap().sense(Sense::click()));
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            click = open();
                        }
                    }
                    if !p.text.is_empty() {
                        ui.add_space(-t.spacing.sm);
                        text_block(ui, t, &p.text, width, 5, t.typography.body_m);
                    }
                }
                PostKind::Status | PostKind::Other | PostKind::Purchase => {
                    if !p.text.is_empty() {
                        text_block(ui, t, &p.text, width, 8, t.typography.body_m);
                    }
                }
                _ => {}
            }
        }
    }
    click
}

/// Up to `rows` lines of text, cut with "…".
fn text_block(ui: &mut Ui, t: &Theme, text: &str, width: f32, rows: usize, style: ui::theme::TextStyle) {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), style.font(), t.text, width);
    job.wrap.max_rows = rows;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    let (r, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(r.min, galley, t.text);
}

/// The review's verdict: thumb, "Recommended", hours (Figma `494:5465`).
fn review_box(ui: &mut Ui, t: &Theme, p: &Post) {
    let up = p.recommended.unwrap_or(true);
    w::filter_bar(t).inner_margin(egui::Margin::same(t.spacing.xs as i8 + 1)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.md;
            let (r, _) = ui.allocate_exact_size(vec2(49.0, 49.0), Sense::hover());
            ui.painter().rect_filled(r, t.corner_small(), if up { t.accent_2 } else { t.danger });
            w::paint_icon(ui.painter(), r.center(), if up { icons::THUMBS_UP } else { icons::THUMBS_DOWN }, 30.0, t.text);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = t.spacing.xs;
                ui.add_space(t.spacing.xs);
                ui.label(w::body_m(t, if up { t!("Recommended") } else { t!("Not Recommended") }, t.text));
                if !p.hours.is_empty() {
                    ui.label(w::body_m(t, &p.hours, t.text_dim));
                }
            });
        });
    });
}

fn play_mark(ui: &Ui, t: &Theme, r: Rect) {
    let b = Rect::from_center_size(r.center(), vec2(58.0, 58.0));
    ui.painter().rect_filled(b, t.corner_small(), t.overlay_dark(2));
    w::paint_icon(ui.painter(), b.center(), icons::PLAY, 26.0, t.text);
}

/// "● LIVE" in the corner of a broadcast (Figma `494:5794`).
fn live_badge(ui: &Ui, t: &Theme, r: Rect) {
    let font = t.typography.body_m.font();
    let g = ui.painter().layout_no_wrap(t!("LIVE").to_string(), font, t.danger);
    let size = vec2(g.size().x + 10.0 + 17.0 + 1.0, 27.0);
    let b = Rect::from_min_size(r.min + vec2(9.0, 10.0), size);
    ui.painter().rect_filled(b, t.corner_small(), t.raised);
    ui.painter().circle_filled(egui::pos2(b.left() + 5.0 + 8.5, b.center().y), 5.5, t.danger);
    ui.painter().galley(egui::pos2(b.left() + 5.0 + 17.0 + 1.0, b.center().y - g.size().y / 2.0), g, t.danger);
}

/// The caption and the counts (Figma `494:5549`); for a review "Was this
/// review helpful?" and its counts (`495:5835`).
fn footer(ui: &mut Ui, t: &Theme, p: &Post) -> Option<Click> {
    let mut click = None;
    let open = open_target(p);
    let counts = |ui: &mut Ui, click: &mut Option<Click>| {
        let mut chip = |ui: &mut Ui, icon: &str, label: Option<&str>, n: u32| {
            if w::stat_chip(ui, t, icon, label, Some(&number(n))).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *click = open.clone();
            }
        };
        if p.kind == PostKind::Review {
            chip(ui, icons::THUMBS_UP, Some(&t!("Yes")), p.votes_up);
            chip(ui, icons::SMILEY, Some(&t!("Funny")), p.funny);
            chip(ui, icons::CHAT, None, p.comments);
        } else {
            chip(ui, icons::THUMBS_UP, None, p.votes_up);
            chip(ui, icons::CHAT, None, p.comments);
        }
    };
    if p.kind == PostKind::Review {
        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
        ui.add_space(-t.spacing.sm);
        ui.label(w::body_s(t, t!("Was this review helpful?"), t.text_dim));
        ui.add_space(-t.spacing.sm);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.md - 2.0;
            counts(ui, &mut click);
        });
        return click;
    }
    let caption = match p.kind {
        PostKind::Screenshot | PostKind::Artwork | PostKind::Video | PostKind::Workshop => p.title.clone(),
        _ => String::new(),
    };
    let show_counts = p.votes_up > 0 || p.comments > 0 || p.file_id != 0;
    if caption.is_empty() && !show_counts {
        // Keep the card's bottom padding even.
        ui.add_space(-t.spacing.lg);
        return None;
    }
    ui.horizontal(|ui| {
        if show_counts {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.md - 2.0;
                // Right to left: the last one first.
                let mut c = None;
                let mut chip = |ui: &mut Ui, icon: &str, n: u32| {
                    if w::stat_chip(ui, t, icon, None, Some(&number(n))).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        c = open.clone();
                    }
                };
                chip(ui, icons::CHAT, p.comments);
                chip(ui, icons::THUMBS_UP, p.votes_up);
                if c.is_some() {
                    click = c;
                }
                if !caption.is_empty() {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if caption_link(ui, t, &caption) {
                            click = open.clone();
                        }
                    });
                }
            });
        } else if caption_link(ui, t, &caption) {
            click = open.clone();
        }
    });
    let _ = counts;
    click
}

fn caption_link(ui: &mut Ui, t: &Theme, caption: &str) -> bool {
    let r = ui.add(egui::Label::new(w::body_l(t, caption, t.accent)).truncate().sense(Sense::click()));
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Posts in two columns (one when narrow), each going under the shorter
/// column, like a social feed. Heights come from the last frame.
pub fn masonry(ui: &mut Ui, t: &Theme, state: &State, posts: &[&Post], salt: &str, show_game: bool) -> Option<Click> {
    let gap = t.spacing.xl - t.spacing.xs;
    let total = ui.available_width();
    let cols = if total >= 900.0 { 2 } else { 1 };
    let col_w = ((total - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let id = egui::Id::unique(("feed-heights", salt));
    let mut heights: std::collections::HashMap<String, f32> = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let key = |p: &Post| format!("{:?}{}{}{}{}{}", p.kind, p.url, p.file_id, p.title, p.action, p.author);
    // Which column each post goes in.
    let mut fill = vec![0.0f32; cols];
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); cols];
    for (i, p) in posts.iter().enumerate().filter(|(_, p)| showable(p)) {
        let h = heights.get(&key(p)).copied().unwrap_or_else(|| guess_height(t, p, col_w));
        let c = (0..cols).min_by(|a, b| fill[*a].total_cmp(&fill[*b])).unwrap_or(0);
        fill[c] += h + gap;
        columns[c].push(i);
    }
    let mut click = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for col in &columns {
            ui.allocate_ui_with_layout(vec2(col_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(col_w);
                ui.spacing_mut().item_spacing.y = gap;
                for &i in col {
                    let p = posts[i];
                    let top = ui.cursor().top();
                    if let Some(c) = card(ui, t, state, p, col_w, show_game) {
                        click = Some(c);
                    }
                    heights.insert(key(p), ui.cursor().top() - top - gap);
                }
            });
        }
    });
    ui.data_mut(|d| d.insert_temp(id, heights));
    click
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_get_thousands_separators() {
        assert_eq!(number(5), "5");
        assert_eq!(number(1234), "1,234");
        assert_eq!(number(50600), "50,600");
        assert_eq!(number(1000000), "1,000,000");
    }

    #[test]
    fn pictures_follow_the_kind() {
        let shot = Post { kind: PostKind::Screenshot, images: vec!["https://x/a.jpg".into()], ..Default::default() };
        assert_eq!(picture(&shot, 737.0).map(|(u, h)| (u, h as u32)), Some(("https://x/a.jpg".to_string(), 402)));
        let review = Post { kind: PostKind::Review, images: vec!["https://x/a.jpg".into()], ..Default::default() };
        assert!(picture(&review, 700.0).is_none());
        let bought = Post { kind: PostKind::Purchase, app_id: 620, ..Default::default() };
        assert_eq!(picture(&bought, 460.0).map(|(u, _)| u), Some(art::header(620)));
    }
}

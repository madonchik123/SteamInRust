//! Watching a Steam broadcast in the app: the video in the native player,
//! who is broadcasting what, and the chat beside it (read-only).
//!
//! Opened with `broadcast` and the broadcaster's 64-bit id. The video and the
//! background work end as soon as the page isn't drawn any more.

use client::Command;
use client::broadcast::{self, ChatState, Phase, Stop, View, Wait, Watch};
use client::feed::{ExploreQuery, PostKind, Section};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Layout, Rect, Sense, Ui, UiBuilder, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, SteamId, art};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use ui::{Theme, icons, widgets as w};

use super::community_home::{self, BarTab};
use super::hub;

pub const PAGE: &str = "broadcast";
/// With no Steam session to answer (signed out, `--demo`), public broadcasts
/// are read without one after this long.
const ANONYMOUS_AFTER: Duration = Duration::from_secs(3);

/// One broadcast on screen. Dropping it stops the video and the polling.
struct Session {
    watch: Watch,
    seen: u64,
    view: View,
    opened: Instant,
    /// The stream's link and its player.
    player: Option<(String, video::Player)>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.watch.stop();
    }
}

pub struct Broadcast {
    rt: tokio::runtime::Handle,
    ctx: egui::Context,
    target: Option<SteamId>,
    session: Option<Session>,
    /// The page was drawn since the last tick.
    drawn: bool,
    /// Ticks in a row without the page being drawn.
    missed: u8,
    bar_search: String,
}

impl Broadcast {
    pub fn new(rt: tokio::runtime::Handle, ctx: egui::Context) -> Self {
        Self { rt, ctx, target: None, session: None, drawn: false, missed: 0, bar_search: String::new() }
    }
}

impl Extension for Broadcast {
    fn manifest(&self) -> Manifest {
        Manifest { id: "broadcast", name: "Broadcasts", author: "SteamInRust", description: "Watch Steam broadcasts in the app", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new(PAGE, "Broadcast", icons::MONITOR).hidden().order(961)]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        self.target = arg.and_then(|a| a.parse().ok());
        if self.session.as_ref().is_some_and(|s| Some(s.watch.steam_id()) != self.target) {
            self.session = None;
        }
    }

    fn page_bar(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        community_home::page_bar(ui, ctx, BarTab::Broadcasts, &mut self.bar_search);
        true
    }

    /// Left the page: free the video right away.
    fn tick(&mut self, _ctx: &mut Ctx) {
        if self.session.is_none() {
            return;
        }
        if std::mem::take(&mut self.drawn) {
            self.missed = 0;
            return;
        }
        self.missed += 1;
        if self.missed >= 2 {
            tracing::debug!("broadcast: page left, stopping");
            self.session = None;
        } else {
            self.ctx.request_repaint();
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        self.drawn = true;
        let t = ctx.theme.clone();
        let Some(id) = self.target else {
            w::empty(ui, &t, icons::MONITOR, t!("Broadcasts"), t!("Check back later."));
            return;
        };
        if self.session.is_none() {
            let repaint = self.ctx.clone();
            let watch = Watch::new(id, move || repaint.request_repaint());
            ctx.send(Command::Hub(client::hub::Request::WatchBroadcast(watch.clone())));
            self.session = Some(Session { watch, seen: 0, view: View::default(), opened: Instant::now(), player: None });
        }
        let Some(s) = self.session.as_mut() else { return };
        if !s.watch.started() {
            if s.opened.elapsed() >= ANONYMOUS_AFTER {
                self.rt.spawn(broadcast::run(Arc::new(client::market::Web::anonymous()), s.watch.clone()));
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
        }
        if let Some(v) = s.watch.view_if_changed(&mut s.seen) {
            s.view = v;
        }

        // The player follows what Steam says about the stream.
        let wanted = match &s.view.phase {
            Phase::Live(stream) => Some(stream.url.clone()),
            _ => None,
        };
        if wanted.as_deref() != s.player.as_ref().map(|(u, _)| u.as_str()) {
            s.player = wanted.map(|url| {
                let mut opts = video::widget::options(ui.ctx());
                // A hidden test copy never makes a sound.
                opts.muted |= crate::app::offscreen();
                let p = video::Player::new(&self.rt, ui.ctx(), video::Source::Live(url.clone()), opts);
                (url, p)
            });
        }

        let area = ui.available_rect_before_wrap().shrink(t.spacing.xl);
        if area.width() <= 0.0 || area.height() <= 0.0 {
            return;
        }
        let gap = t.spacing.lg;
        let beside = area.width() >= t.dense(900.0);
        let chat_w = if beside { (area.width() * 0.27).clamp(t.dense(280.0), t.dense(380.0)) } else { 0.0 };
        let left_w = if beside { area.width() - chat_w - gap } else { area.width() };
        let details_h = t.dense(76.0);
        let room = (area.height() - details_h - gap).max(t.dense(140.0));
        let video_h = (left_w * 9.0 / 16.0).min(if beside { room } else { room * 0.62 }).round();
        let video_rect = Rect::from_min_size(area.min, vec2(left_w, video_h));
        let details_rect = Rect::from_min_size(video_rect.left_bottom() + vec2(0.0, gap), vec2(left_w, details_h));
        let chat_rect = if beside {
            Rect::from_min_size(egui::pos2(video_rect.right() + gap, area.top()), vec2(chat_w, area.height()))
        } else {
            Rect::from_min_max(details_rect.left_bottom() + vec2(0.0, gap), area.right_bottom())
        };
        ui.allocate_rect(area, Sense::hover());

        let name = shown_name(ctx, s, id);
        video_area(ui, &t, ctx, s, video_rect, &name);
        // Nothing to say about an account Steam can't show.
        let known = !s.view.name.is_empty() || !ctx.state.persona(id).name.is_empty() || s.view.info != broadcast::Info::default();
        if known {
            details(ui, &t, ctx, s, details_rect, id, &name);
        }
        if chat_rect.height() >= t.dense(90.0) {
            chat(ui, &t, &s.view, chat_rect, id);
        }
    }
}

/// The name Steam knows them by, or the one on their broadcast page.
fn shown_name(ctx: &Ctx, s: &Session, id: SteamId) -> String {
    let persona = ctx.state.persona(id).name.clone();
    if !persona.is_empty() {
        return persona;
    }
    if !s.view.name.is_empty() {
        return s.view.name.clone();
    }
    t!("Broadcaster").to_string()
}

/// (still working on it, headline, details) for a broadcast with no picture.
fn message(phase: &Phase, name: &str) -> (bool, String, String) {
    let stop = |title: String| (false, title, String::new());
    match phase {
        Phase::Connecting | Phase::Live(_) => (true, t!("Connecting to the broadcast…").to_string(), String::new()),
        Phase::Waiting(Wait::Answer) => (true, t!("Waiting for a response from {name}", name = name), String::new()),
        Phase::Waiting(Wait::Start) => (true, t!("Waiting for {name}'s broadcast to start", name = name), String::new()),
        Phase::Waiting(Wait::Reconnect) => (true, t!("Waiting for {name} to reconnect to Steam", name = name), String::new()),
        Phase::Stopped(Stop::Ended) => stop(t!("{name}'s broadcast has ended", name = name)),
        // Steam gives the same answer for all of these.
        Phase::Stopped(Stop::Unavailable) => (false, t!("This broadcast isn't available").to_string(), t!("It may have ended, or it may be private or for friends only.").to_string()),
        Phase::Stopped(Stop::NoServers) => stop(t!("Steam has no free broadcast servers near {name} right now. Try again in a few minutes.", name = name)),
        Phase::Stopped(Stop::NotSupported) => stop(t!("Steam doesn't support broadcasting on {name}'s system", name = name)),
        Phase::Stopped(Stop::Restricted) => stop(t!("{name}'s account can't broadcast right now", name = name)),
        Phase::Stopped(Stop::OutOfDate) => stop(t!("{name}'s Steam needs an update to broadcast", name = name)),
        Phase::Stopped(Stop::PoorUpload) => stop(t!("{name}'s connection is too unstable to broadcast", name = name)),
        Phase::Stopped(Stop::NeedsSubscription) => stop(t!("You need a subscription to watch this broadcast").to_string()),
        Phase::Stopped(Stop::Failed) => stop(t!("This broadcast couldn't be loaded").to_string()),
    }
}

fn video_area(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, s: &mut Session, rect: Rect, name: &str) {
    if let Some((_, p)) = s.player.as_mut() {
        let title = if s.view.info.title.is_empty() { name.to_string() } else { s.view.info.title.clone() };
        let shown = video::widget::show(ui, t, rect, p, &s.view.info.thumbnail, &title);
        // It stopped: Steam knows whether it ended or just hiccuped.
        if shown.ended || p.live_lost() {
            s.watch.recheck();
        }
        return;
    }
    // No picture: say why, over the last thing we saw of it.
    let radius = t.corner();
    if s.view.info.thumbnail.is_empty() || !w::paint_cover(ui, rect, &s.view.info.thumbnail, radius) {
        ui.painter().rect_filled(rect, radius, t.raised);
    } else {
        ui.painter().rect_filled(rect, radius, t.overlay_dark(2));
    }
    let (busy, title, body) = message(&s.view.phase, name);
    let width = (rect.width() - t.spacing.xl * 2.0).max(t.dense(120.0));
    let block = Rect::from_center_size(rect.center(), vec2(width, t.dense(150.0)));
    ui.scope_builder(UiBuilder::new().max_rect(block).layout(Layout::top_down(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.y = t.spacing.sm;
        if busy {
            ui.add(egui::Spinner::new().size(t.dense(28.0)).color(t.accent));
        } else {
            ui.label(w::heading_l(t, icons::MONITOR, t.text_dim));
        }
        ui.add(egui::Label::new(w::heading_s(t, title, t.text)).wrap());
        if !body.is_empty() {
            ui.add(egui::Label::new(w::body_m(t, body, t.text_dim)).wrap());
        }
        if !busy {
            ui.add_space(t.spacing.xs);
            ui.horizontal(|ui| {
                // Two buttons, centered by hand: a row doesn't center itself.
                let row = t.dense(250.0).min(ui.available_width());
                ui.add_space(((ui.available_width() - row) / 2.0).max(0.0));
                if w::button(ui, t, None, t!("Try again"), w::Kind::Primary, w::Size::Sm).clicked() {
                    s.watch.recheck();
                }
                if w::button(ui, t, None, t!("Back to broadcasts"), w::Kind::Tertiary, w::Size::Sm).clicked() {
                    ctx.navigate(community_home::PAGE, Some("broadcasts".into()));
                }
            });
        }
    });
}

/// Title, who, the game and how many are watching.
fn details(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, s: &Session, rect: Rect, id: SteamId, name: &str) {
    let info = &s.view.info;
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)), |ui| {
        ui.spacing_mut().item_spacing.y = t.spacing.xs;
        let headline = if !info.title.is_empty() { info.title.clone() } else if !info.game.is_empty() { info.game.clone() } else { name.to_string() };
        ui.add(egui::Label::new(w::heading_s(t, headline, t.text)).truncate());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.sm;
            let persona = ctx.state.persona(id);
            if persona.avatar_hash.is_some() {
                let a = art::avatar(persona.avatar_hash.as_deref());
                if w::avatar(ui, t, &a, t.dense(24.0), None).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    ctx.navigate("profile", Some(id.to_string()));
                }
            }
            let who = ui.add(egui::Label::new(w::body_m(t, name, t.text)).truncate().sense(Sense::click()));
            if who.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                ctx.navigate("profile", Some(id.to_string()));
            }
            if !info.game.is_empty() && info.game != *headline_game(info) {
                ui.label(w::body_m(t, "·", t.text_dim));
                ui.add(egui::Label::new(w::body_m(t, &info.game, t.text_dim)).truncate());
            }
            if info.viewers > 0 {
                ui.label(w::body_m(t, "·", t.text_dim));
                ui.label(w::body_m(t, t!("{n} viewers", n = hub::post::number(info.viewers)), t.text_dim));
            }
            if info.app_id != 0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::button(ui, t, None, t!("Community hub"), w::Kind::Tertiary, w::Size::Sm).clicked() {
                        ctx.navigate("hub", Some(info.app_id.to_string()));
                    }
                    if w::button(ui, t, None, t!("Store page"), w::Kind::Tertiary, w::Size::Sm).clicked() {
                        ctx.navigate("store", Some(format!("app:{}", info.app_id)));
                    }
                });
            }
        });
    });
}

/// The game's name when the headline already shows it (no title set).
fn headline_game(info: &broadcast::Info) -> &str {
    if info.title.is_empty() { &info.game } else { "" }
}

/// The broadcast's chat, newest at the bottom. Read-only.
fn chat(ui: &mut Ui, t: &Theme, view: &View, rect: Rect, broadcaster: SteamId) {
    ui.painter().rect_filled(rect, t.corner(), t.surface);
    let inner = rect.shrink(t.spacing.md);
    ui.scope_builder(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)), |ui| {
        ui.spacing_mut().item_spacing.y = t.spacing.sm;
        ui.horizontal(|ui| {
            ui.label(w::heading_s(t, t!("Chat"), t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(w::body_s(t, t!("Read only"), t.text_dim));
            });
        });
        let over = matches!(view.phase, Phase::Stopped(_)) && view.chat.is_empty();
        let note = match view.chat_state {
            ChatState::Unavailable => Some(t!("Chat isn't available for this broadcast")),
            _ if over => Some(t!("Chat isn't available for this broadcast")),
            ChatState::Connecting => Some(t!("Connecting to chat…")),
            ChatState::On if view.chat.is_empty() => Some(t!("No messages yet")),
            ChatState::On => None,
        };
        if let Some(note) = note {
            ui.label(w::body_s(t, note, t.text_dim));
            return;
        }
        let font = t.typography.body_s.font();
        egui::ScrollArea::vertical().id_salt("broadcast-chat").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = t.spacing.xs;
            let width = ui.available_width();
            for line in &view.chat {
                let mut job = LayoutJob::default();
                job.wrap.max_width = width;
                let color = if line.steam_id == broadcaster { t.accent } else { t.text_dim };
                job.append(&line.name, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
                job.append("  ", 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
                job.append(&line.text, 0.0, TextFormat { font_id: font.clone(), color: t.text, ..Default::default() });
                ui.add(egui::Label::new(job).wrap());
            }
        });
    });
}

/// On a game's store page: its most watched live broadcast, if there is one.
pub fn store_strip(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, asked: &mut HashSet<AppId>) {
    let q = ExploreQuery { section: Section::Broadcasts, page: 1, language: i18n::language().steam_name().to_string(), app, ..Default::default() };
    let key = q.key();
    if asked.insert(app) {
        ctx.send(Command::Hub(client::hub::Request::Explore(q)));
    }
    let Some(Ok(feed)) = ctx.state.hub.explore.get(&key) else { return };
    let Some(post) = feed.posts.iter().filter(|p| p.kind == PostKind::Broadcast && p.live).max_by_key(|p| p.views) else { return };
    let Some((page, Some(arg))) = hub::route(&post.url).map(|v| v.to_page()).filter(|(p, _)| p == PAGE) else { return };
    let persona = (post.author != 0).then(|| ctx.state.persona(post.author).name.clone()).filter(|n| !n.is_empty());
    let name = persona.unwrap_or_else(|| post.author_name.clone());
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.sm;
            ui.label(w::body_m(t, "●", t.danger));
            ui.label(w::body_m(t, t!("LIVE"), t.danger));
            let what = if name.is_empty() { t!("Someone is broadcasting this game").to_string() } else { t!("{name} is broadcasting this game", name = name) };
            ui.add(egui::Label::new(w::body_m(t, what, t.text)).truncate());
            if post.views > 0 {
                ui.label(w::body_m(t, t!("{n} viewers", n = hub::post::number(post.views)), t.text_dim));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::button(ui, t, Some(icons::PLAY), t!("Watch live"), w::Kind::Primary, w::Size::Sm).clicked() {
                    ctx.navigate(&page, Some(arg.clone()));
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_why_there_is_no_picture() {
        let ended = message(&Phase::Stopped(Stop::Ended), "Pixel");
        assert!(!ended.0);
        assert!(ended.1.contains("Pixel"));
        // Steam doesn't tell ended, private and friends-only apart: say so.
        let private = message(&Phase::Stopped(Stop::Unavailable), "Pixel");
        assert!(!private.0);
        assert!(!private.2.is_empty());
        assert!(message(&Phase::Waiting(Wait::Reconnect), "Pixel").0);
        assert!(message(&Phase::Connecting, "Pixel").0);
    }

    #[test]
    fn a_broadcast_link_opens_this_page() {
        let v = hub::route("https://steamcommunity.com/broadcast/watch/76561197960266732/").unwrap();
        assert_eq!(v.to_page(), (PAGE.to_string(), Some("76561197960266732".to_string())));
        assert!(hub::route("https://steamcommunity.com/broadcast/watch/notanid").is_none());
    }
}

//! Big Picture's pages. Each lays itself out in screen coordinates from the
//! top of its scrolled content and returns how tall it is.

use super::focus::{Hit, Place, since};
use super::paint::{self, Kind};
use super::{BigPicture, Frame, Page, Tab};
use crate::app::SteamApp;
use crate::builtin::library::{Library, MainAction};
use crate::builtin::store::Store;
use client::Command;
use egui::{Align2, Color32, Id, Rect, Ui, Vec2, pos2, vec2};
use extensions::Action;
use i18n::t;
use model::{AppId, PersonaState, art};
use ui::{icons, widgets as w};

/// Run `f` with the Library extension (its play/install logic and data).
pub fn with_library<R>(app: &mut SteamApp, actions: &mut Vec<Action>, f: impl FnOnce(&mut Library, &mut extensions::Ctx) -> R) -> Option<R> {
    let env = extensions::Env { state: &app.state, local: &app.local, theme: &app.prefs.theme, zoom: app.prefs.zoom, client: &app.client, actions };
    app.registry.with_ext::<Library, R>("library", env, f)
}

/// Run `f` with the Store extension (its data and caches).
pub fn with_store<R>(app: &mut SteamApp, actions: &mut Vec<Action>, f: impl FnOnce(&mut Store, &mut extensions::Ctx) -> R) -> Option<R> {
    let env = extensions::Env { state: &app.state, local: &app.local, theme: &app.prefs.theme, zoom: app.prefs.zoom, client: &app.client, actions };
    app.registry.with_ext::<Store, R>("store", env, f)
}

pub fn draw(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, pos: f32) -> f32 {
    let top = f.view.top() - pos;
    match bp.page() {
        Page::Tab(Tab::Home) => home(bp, ui, f, top),
        Page::Tab(Tab::Library) => library_tab(bp, ui, f, top),
        Page::Tab(Tab::Store) => store_tab(bp, ui, f, top),
        Page::Tab(Tab::Downloads) => downloads(bp, ui, f, top),
        Page::Tab(Tab::Friends) => friends(bp, ui, f, top),
        Page::Game(id) => game(bp, ui, f, top, id),
        Page::StoreApp(id) => store_app(bp, ui, f, top, id),
        Page::Search => search(bp, ui, f, top),
        Page::Shots(id, i) => shots(bp, ui, f, id, i),
        Page::Settings => settings(bp, ui, f, top),
        Page::Friend(id) => friend_page(bp, ui, f, top, id),
        Page::Chat(id) => chat_page(bp, ui, f, top, id),
    }
}

/// What X does on this page, for the hints.
pub fn x_hint(bp: &BigPicture) -> Option<String> {
    match bp.page() {
        Page::Tab(Tab::Library) => Some(t!("Installed / All").into()),
        Page::Search | Page::Chat(_) => Some(t!("Delete").into()),
        _ => None,
    }
}

// ---------- shared pieces ----------

/// Fade and rise in, one after another, when a page opens.
fn appear(ui: &Ui, bp: &BigPicture, order: usize) -> (f32, f32) {
    let k = since(ui, bp.page_since + 0.045 * order.min(10) as f64, 0.4);
    (k, (1.0 - k) * 26.0)
}

fn heading(ui: &Ui, f: &Frame, y: &mut f32, title: &str) {
    ui.painter().text(pos2(f.view.left() + f.pad, *y), Align2::LEFT_TOP, title, w::font_bold(30.0 * f.s), f.t.text);
    *y += 54.0 * f.s;
}

/// A shelf of `n` tiles that scrolls sideways to follow the focus. Returns
/// the tiles near the screen: (index, rect, hit).
fn shelf(bp: &mut BigPicture, ui: &mut Ui, f: &Frame, y: &mut f32, key: &str, n: usize, tile: Vec2, id: impl Fn(usize) -> Id) -> Vec<(usize, Rect, Hit)> {
    let gap = 26.0 * f.s;
    let width = f.view.width() - 2.0 * f.pad;
    let pos = bp.scroller(key).pos;
    let clip = ui.clip_rect();
    let mut out = Vec::new();
    for i in 0..n {
        let at = i as f32 * (tile.x + gap);
        let r = Rect::from_min_size(pos2(f.view.left() + f.pad + at - pos, *y), tile);
        let hit = bp.focus.item(ui, id(i), r, clip, Place::Page);
        if hit.focused && bp.focus.moved {
            bp.scroller(key).reveal(at, at + tile.x, width, tile.x * 0.6 + gap);
        }
        if r.right() > clip.left() - tile.x && r.left() < clip.right() + tile.x {
            out.push((i, r, hit));
        }
    }
    let sc = bp.scroller(key);
    sc.clamp(n as f32 * (tile.x + gap) - gap, width);
    sc.step(ui);
    *y += tile.y;
    out
}

/// A game's portrait art tile (with a caption under it) and what A does.
fn capsule(bp: &mut BigPicture, ui: &mut Ui, f: &Frame, r: Rect, hit: &Hit, order: usize, id: AppId, name: &str, caption: &str, dim: bool) {
    let (k, dy) = appear(ui, bp, order);
    let r = r.translate(vec2(0.0, dy * f.s));
    let mut c = paint::faded(ui, r.expand(40.0 * f.s), k);
    let sheen = (f.now - bp.focus.changed_at) as f32 / 0.7;
    let shown = paint::art_tile(&mut c, &f.t, r, &[&art::capsule(id), &art::header(id)], name, hit, f.s, sheen);
    if dim {
        c.painter().rect_filled(shown, paint::radius(10.0 * f.s), Color32::from_black_alpha((110.0 * (1.0 - hit.glow)) as u8));
    }
    if hit.focused {
        bp.focus.ring_around(shown, 10.0 * f.s);
        bp.bg_want = Some(id);
    }
    let text_y = r.bottom() + 12.0 * f.s + 6.0 * f.s * hit.glow;
    let color = w::mix(f.t.text_dim, f.t.text, hit.glow);
    paint::line(c.painter(), pos2(r.left(), text_y), Align2::LEFT_TOP, name, w::font_semibold(19.0 * f.s), color, r.width());
    if !caption.is_empty() {
        paint::line(c.painter(), pos2(r.left(), text_y + 26.0 * f.s), Align2::LEFT_TOP, caption, w::font(16.0 * f.s), f.t.text_faint, r.width());
    }
}

/// A wide picture tile (store capsules, news) with up to two lines under it.
#[allow(clippy::too_many_arguments)]
fn wide(bp: &mut BigPicture, ui: &mut Ui, f: &Frame, r: Rect, hit: &Hit, order: usize, urls: &[&str], title: &str, sub: &str, sub_color: Color32) -> Rect {
    let (k, dy) = appear(ui, bp, order);
    let r = r.translate(vec2(0.0, dy * f.s));
    let mut c = paint::faded(ui, r.expand(40.0 * f.s), k);
    let sheen = (f.now - bp.focus.changed_at) as f32 / 0.7;
    let shown = paint::art_tile(&mut c, &f.t, r, urls, title, hit, f.s, sheen);
    if hit.focused {
        bp.focus.ring_around(shown, 10.0 * f.s);
    }
    let text_y = r.bottom() + 12.0 * f.s + 6.0 * f.s * hit.glow;
    let color = w::mix(f.t.text_dim, f.t.text, hit.glow);
    paint::line(c.painter(), pos2(r.left(), text_y), Align2::LEFT_TOP, title, w::font_semibold(19.0 * f.s), color, r.width());
    if !sub.is_empty() {
        paint::line(c.painter(), pos2(r.left(), text_y + 26.0 * f.s), Align2::LEFT_TOP, sub, w::font_medium(17.0 * f.s), sub_color, r.width());
    }
    shown
}

/// A pill button that's focusable. Returns its hit.
#[allow(clippy::too_many_arguments)]
fn button(bp: &mut BigPicture, ui: &Ui, f: &Frame, id: Id, at: egui::Pos2, min_w: f32, icon: Option<&str>, label: &str, kind: Kind, progress: Option<f32>) -> (Rect, Hit) {
    let h = 72.0 * f.s;
    let wdt = paint::pill_width(ui, icon.is_some(), label, f.s).max(min_w);
    let r = Rect::from_min_size(at, vec2(wdt, h));
    let hit = bp.focus.item(ui, id, r, ui.clip_rect(), Place::Page);
    paint::pill(ui, &f.t, r, icon, label, kind, &hit, f.s, progress);
    if hit.focused {
        bp.focus.ring_around(paint::grow(r, 1.0 + 0.04 * hit.glow), h / 2.0);
    }
    (r, hit)
}

/// A big empty-state message with one way forward.
fn empty(bp: &mut BigPicture, ui: &Ui, f: &Frame, y: f32, icon: &str, title: &str, body: &str, action: Option<(&str, &str)>) -> Option<bool> {
    let c = pos2(f.view.center().x, y + 150.0 * f.s);
    let p = ui.painter();
    p.text(c, Align2::CENTER_CENTER, icon, w::font(96.0 * f.s), f.t.text_faint);
    p.text(c + vec2(0.0, 90.0 * f.s), Align2::CENTER_CENTER, title, w::font_bold(34.0 * f.s), f.t.text);
    p.text(c + vec2(0.0, 136.0 * f.s), Align2::CENTER_CENTER, body, w::font(21.0 * f.s), f.t.text_dim);
    let (icon2, label) = action?;
    let wdt = paint::pill_width(ui, true, label, f.s);
    let (_, hit) = button(bp, ui, f, Id::unique(("bp-empty", title)), c + vec2(-wdt / 2.0, 186.0 * f.s), wdt, Some(icon2), label, Kind::Primary, None);
    Some(hit.activated)
}

fn open_game(bp: &mut BigPicture, f: &Frame, id: AppId) {
    let owned = f.app.state.apps.as_ref().is_some_and(|a| a.contains_key(&id)) || f.app.local.games.iter().any(|g| g.app_id == id);
    bp.open(if owned { Page::Game(id) } else { Page::StoreApp(id) }, f.now);
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The main button for a game, as label, icon, look and progress.
fn main_button(a: MainAction) -> (String, &'static str, Kind, Option<f32>) {
    match a {
        MainAction::Play => (t!("Play").into(), icons::PLAY, Kind::Play, None),
        MainAction::Running => (t!("Stop").into(), icons::STOP, Kind::Danger, None),
        MainAction::Update => (t!("Update").into(), icons::DOWNLOAD, Kind::Primary, None),
        MainAction::Install => (t!("Install").into(), icons::DOWNLOAD, Kind::Primary, None),
        MainAction::Syncing => (t!("Syncing Steam Cloud…").into(), icons::CLOUD_CHECK, Kind::Muted, None),
        MainAction::Queued => (t!("Queued").into(), icons::CLOCK, Kind::Muted, None),
        MainAction::Uninstalling => (t!("Uninstalling…").into(), icons::TRASH, Kind::Muted, None),
        MainAction::Busy { fraction, verifying, paused } => {
            let pct = (fraction * 100.0) as u32;
            // Says what A does: resume or pause (verifying can only be cancelled).
            let label = if paused {
                t!("Resume · {pct}%", pct = pct)
            } else if verifying {
                t!("Verifying… {pct}%", pct = pct)
            } else {
                t!("Pause · {pct}%", pct = pct)
            };
            (label, if paused { icons::PLAY } else { icons::DOWNLOAD }, Kind::Secondary, Some(fraction))
        }
        MainAction::StorePage => (t!("Store page").into(), icons::STORE, Kind::Primary, None),
    }
}

/// Recently played games across the account, newest first: (id, name, when).
fn recent_games(app: &SteamApp) -> Vec<(AppId, String, u64)> {
    let mut list: Vec<(AppId, String, u64)> = match app.state.player_games.get(&app.state.steam_id) {
        Some(Ok(list)) if !list.is_empty() => list.iter().filter_map(|g| Some((g.app_id, g.name.clone(), g.last_played?))).collect(),
        _ => app.local.games.iter().filter_map(|g| Some((g.app_id, g.name.clone(), g.last_played?))).collect(),
    };
    list.sort_by_key(|g| std::cmp::Reverse(g.2));
    list
}

// ---------- Home ----------

fn home(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32) -> f32 {
    let s = f.s;
    let mut y = top + 16.0 * s;
    let recent = recent_games(f.app);
    // The hero: the installed game played last (or any installed game).
    let mut installed: Vec<&model::InstalledGame> = f.app.local.games.iter().collect();
    installed.sort_by_key(|g| std::cmp::Reverse(g.last_played.unwrap_or(0)));
    let hero = installed.first().map(|g| (g.app_id, g.name.clone(), g.playtime_minutes, g.last_played));
    match hero.clone() {
        Some((id, name, minutes, last)) => {
            bp.bg_want = Some(id);
            let rect = Rect::from_min_size(pos2(f.view.left() + f.pad, y), vec2(f.view.width() - 2.0 * f.pad, 440.0 * s));
            let (k, dy) = appear(ui, bp, 0);
            let rect = rect.translate(vec2(0.0, dy * s));
            let mut c = paint::faded(ui, rect.expand(60.0 * s), k);
            let r = 18.0 * s;
            paint::shadow(c.painter(), rect, r, s, 0.8, Color32::from_black_alpha(220));
            // The art slowly settles in (a gentle zoom) when the page opens.
            let zoom = 1.0 + 0.06 * (1.0 - since(ui, bp.page_since, 1.6));
            let art_rect = paint::grow(rect, zoom);
            let clip = c.clip_rect();
            c.set_clip_rect(rect.intersect(clip));
            if !w::paint_cover_first(&c, art_rect, &[&art::hero(id), &art::header(id)], paint::radius(r)) {
                w::skeleton(&c, &f.t, rect, paint::radius(r));
            }
            c.set_clip_rect(clip);
            w::fade_right(&c, rect, Color32::from_black_alpha(220), 0.7);
            w::fade_down(&c, rect, Color32::from_black_alpha(170), 0.45);
            let inner = rect.shrink(44.0 * s);
            let p = c.painter().clone();
            p.text(inner.left_top(), Align2::LEFT_TOP, t!("CONTINUE PLAYING"), w::font_bold(18.0 * s), f.t.accent);
            let logo = Rect::from_min_max(inner.left_top() + vec2(0.0, 40.0 * s), pos2(inner.left() + 560.0 * s, inner.bottom() - 150.0 * s));
            if !w::paint_contain(&c, logo, &art::logo(id), Align2::LEFT_BOTTOM) {
                paint::wrapped(&p, pos2(logo.left(), logo.bottom() - 120.0 * s), &name, w::font_bold(52.0 * s), Color32::WHITE, 760.0 * s, 2);
            }
            let mut facts = Vec::new();
            if let Some(m) = minutes.filter(|m| *m > 0) {
                facts.push(t!("{time} played", time = ui::playtime(m)));
            }
            if let Some(l) = last {
                facts.push(t!("Last played {when}", when = ui::ago(l)));
            }
            p.text(pos2(inner.left(), inner.bottom() - 102.0 * s), Align2::LEFT_BOTTOM, facts.join("   ·   "), w::font_medium(20.0 * s), Color32::from_white_alpha(225));
            let action = with_library(f.app, f.actions, |lib, cx| lib.main_action(cx, id)).unwrap_or(MainAction::Play);
            let (label, icon, kind, progress) = main_button(action);
            let main_id = Id::unique(("bp-home-main", id));
            let (b, main) = button(bp, &c, f, main_id, pos2(inner.left(), inner.bottom() - 72.0 * s), 260.0 * s, Some(icon), &label, kind, progress);
            let (_, details) = button(bp, &c, f, Id::unique(("bp-home-details", id)), pos2(b.right() + 20.0 * s, b.top()), 0.0, Some(icons::INFO), t!("Game details"), Kind::Secondary, None);
            bp.preferred = Some(main_id);
            if main.activated {
                if action == MainAction::StorePage {
                    bp.open(Page::StoreApp(id), f.now);
                } else {
                    with_library(f.app, f.actions, |lib, cx| lib.run_main_action(cx, id));
                }
            }
            if details.activated {
                bp.open(Page::Game(id), f.now);
            }
            y += 440.0 * s + 56.0 * s;
        }
        None => {
            if let Some(go) = empty(bp, ui, f, y, icons::BOOKS, t!("Nothing installed yet"), t!("Install a game from your library to see it here."), Some((icons::BOOKS, t!("Go to your library")))) {
                if go {
                    bp.switch_tab(Tab::Library, f.now, true);
                }
            }
            y += 480.0 * s;
        }
    }

    // Recently played.
    let first = hero.as_ref().map(|h| h.0);
    let shelf_games: Vec<(AppId, String, u64)> = recent.iter().filter(|g| Some(g.0) != first).take(20).cloned().collect();
    if !shelf_games.is_empty() {
        heading(ui, f, &mut y, t!("Recently played"));
        let tile = vec2(200.0 * s, 300.0 * s);
        let tiles = shelf(bp, ui, f, &mut y, "recent", shelf_games.len(), tile, |i| Id::unique(("bp-recent", i)));
        for (i, r, hit) in tiles {
            let (id, name, when) = &shelf_games[i];
            let dim = !f.app.local.games.iter().any(|g| g.app_id == *id);
            capsule(bp, ui, f, r, &hit, i + 1, *id, name, &ui::ago(*when), dim);
            if hit.activated {
                open_game(bp, f, *id);
            }
        }
        y += 90.0 * s;
    }

    // What's new in the games you play (the hero's first).
    let mut ids: Vec<AppId> = first.into_iter().collect();
    ids.extend(recent.iter().map(|g| g.0).filter(|id| Some(*id) != first));
    let (posts, loading) = with_library(f.app, f.actions, |lib, _| lib.whats_new_posts(&ids)).unwrap_or_default();
    if !posts.is_empty() || loading && !ids.is_empty() {
        heading(ui, f, &mut y, t!("What's new"));
        let tile = vec2(440.0 * s, 206.0 * s);
        if posts.is_empty() {
            for i in 0..4 {
                let r = Rect::from_min_size(pos2(f.view.left() + f.pad + i as f32 * (tile.x + 26.0 * s), y), tile);
                w::skeleton(ui, &f.t, r, paint::radius(10.0 * s));
            }
            y += tile.y;
        } else {
            let posts: Vec<_> = posts.into_iter().take(12).collect();
            let tiles = shelf(bp, ui, f, &mut y, "news", posts.len(), tile, |i| Id::unique(("bp-news", i)));
            for (i, r, hit) in tiles {
                let (id, _, e) = &posts[i];
                let game = f.app.state.app_name(*id).unwrap_or_default().to_string();
                let header = art::header(*id);
                let urls: Vec<&str> = e.capsule.as_deref().into_iter().chain(e.header.as_deref()).chain(std::iter::once(header.as_str())).collect();
                let age = now_unix().saturating_sub(e.time) / 86_400;
                let when = match age {
                    0 => t!("Today").to_string(),
                    1 => t!("Yesterday").to_string(),
                    _ => ui::date(e.time),
                };
                wide(bp, ui, f, r, &hit, i + 2, &urls, &e.title, &format!("{game} · {when}"), f.t.text_faint);
                if hit.focused {
                    bp.bg_want = Some(*id);
                }
                if hit.activated {
                    open_game(bp, f, *id);
                }
            }
        }
        y += 90.0 * s;
    }
    y - top + 40.0 * s
}

// ---------- Library ----------

fn library_tab(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32) -> f32 {
    let s = f.s;
    let mut y = top + 16.0 * s;
    let mut games: Vec<(AppId, String, bool)> = crate::builtin::entries(&f.app.state, &f.app.local).iter().map(|e| (e.app_id, e.name.to_string(), e.installed.is_some())).collect();
    let ids: Vec<AppId> = games.iter().map(|g| g.0).collect();
    let hidden: std::collections::HashSet<AppId> = with_library(f.app, f.actions, |lib, _| ids.iter().copied().filter(|id| lib.is_hidden(*id)).collect()).unwrap_or_default();
    games.retain(|g| !hidden.contains(&g.0));
    games.sort_by_key(|g| g.1.to_lowercase());
    let n_installed = games.iter().filter(|g| g.2).count();
    let installed_only = *bp.installed_only.get_or_insert(n_installed > 0);
    if f.x {
        bp.installed_only = Some(!installed_only);
    }

    // Filters.
    let chips = [(true, t!("Installed ({n})", n = n_installed)), (false, t!("All games ({n})", n = games.len()))];
    let mut x = f.view.left() + f.pad;
    for (i, (value, label)) in chips.iter().enumerate() {
        let id = Id::unique(("bp-filter", i));
        let on = installed_only == *value;
        let (r, hit) = button(bp, ui, f, id, pos2(x, y), 0.0, None, label, if on { Kind::Primary } else { Kind::Secondary }, None);
        if hit.activated {
            bp.installed_only = Some(*value);
        }
        x = r.right() + 18.0 * s;
    }
    y += 72.0 * s + 40.0 * s;

    let list: Vec<&(AppId, String, bool)> = games.iter().filter(|g| g.2 || !installed_only).collect();
    if list.is_empty() {
        let loading = f.app.state.apps.is_none();
        let (title, body) = if loading { (t!("Loading your library…"), "") } else { (t!("No games here"), t!("Switch to All games to see everything you own.")) };
        empty(bp, ui, f, y - 60.0 * s, icons::BOOKS, title, body, None);
        return y - top + 400.0 * s;
    }
    let tile = vec2(190.0 * s, 285.0 * s);
    let gap = 28.0 * s;
    let row_h = tile.y + 76.0 * s;
    let width = f.view.width() - 2.0 * f.pad;
    let cols = (((width + gap) / (tile.x + gap)).floor() as usize).max(1);
    let clip = ui.clip_rect();
    for (k, (id, name, installed)) in list.iter().enumerate() {
        let (row, col) = (k / cols, k % cols);
        let r = Rect::from_min_size(pos2(f.view.left() + f.pad + col as f32 * (tile.x + gap), y + row as f32 * row_h), tile);
        let hit = bp.focus.item(ui, Id::unique(("bp-lib", *id)), r, clip, Place::Page);
        if r.bottom() + row_h > clip.top() && r.top() - row_h < clip.bottom() {
            let caption = if *installed { String::new() } else { t!("Not installed").to_string() };
            capsule(bp, ui, f, r, &hit, k, *id, name, &caption, !installed);
        }
        if hit.activated {
            bp.open(Page::Game(*id), f.now);
        }
    }
    let rows = list.len().div_ceil(cols);
    y += rows as f32 * row_h;
    y - top + 30.0 * s
}

// ---------- Game page ----------

fn game(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32, id: AppId) -> f32 {
    let s = f.s;
    bp.bg_want = Some(id);
    let installed = f.app.local.games.iter().find(|g| g.app_id == id).cloned();
    let name = f.app.state.app_name(id).map(String::from).or_else(|| installed.as_ref().map(|g| g.name.clone())).unwrap_or_default();

    // Hero art under the bar, settling in with a slow zoom.
    let screen = ui.max_rect();
    let hero = Rect::from_min_size(pos2(screen.left(), top - 104.0 * s), vec2(screen.width(), 600.0 * s));
    let zoom = 1.0 + 0.08 * (1.0 - since(ui, bp.page_since, 1.8));
    // The art runs up under the tab bar.
    let clip = ui.clip_rect();
    ui.set_clip_rect(Rect::from_min_max(screen.min, clip.max));
    if !w::paint_cover_first(ui, paint::grow(hero, zoom), &[&art::hero(id), &art::header(id)], egui::CornerRadius::ZERO) {
        w::fade_down(ui, hero, f.t.surface, 0.0);
    }
    w::fade_down(ui, hero, f.t.bg, 0.35);
    w::fade_right(ui, hero, f.t.bg.gamma_multiply(0.7), 0.55);
    ui.set_clip_rect(clip);
    let (k, dy) = appear(ui, bp, 0);
    let logo = Rect::from_min_size(pos2(f.view.left() + f.pad, top + 70.0 * s + dy * s), vec2(620.0 * s, 230.0 * s));
    let c = paint::faded(ui, logo.expand(20.0 * s), k);
    if !w::paint_contain(&c, logo, &art::logo(id), Align2::LEFT_BOTTOM) {
        paint::wrapped(c.painter(), pos2(logo.left(), logo.bottom() - 130.0 * s), &name, w::font_bold(58.0 * s), Color32::WHITE, 900.0 * s, 2);
    }

    // Buttons.
    let mut y = top + 360.0 * s;
    let action = with_library(f.app, f.actions, |lib, cx| lib.main_action(cx, id)).unwrap_or(MainAction::StorePage);
    let (label, icon, kind, progress) = main_button(action);
    let main_id = Id::unique(("bp-game-main", id));
    bp.preferred = Some(main_id);
    let (b, main) = button(bp, ui, f, main_id, pos2(f.view.left() + f.pad, y), 300.0 * s, Some(icon), &label, kind, progress);
    if main.activated {
        if action == MainAction::StorePage {
            bp.open(Page::StoreApp(id), f.now);
        } else {
            with_library(f.app, f.actions, |lib, cx| lib.run_main_action(cx, id));
        }
    }
    let mut x = b.right() + 20.0 * s;
    if action != MainAction::StorePage {
        let (r, hit) = button(bp, ui, f, Id::unique(("bp-game-store", id)), pos2(x, y), 0.0, Some(icons::STORE), t!("Store page"), Kind::Secondary, None);
        if hit.activated {
            bp.open(Page::StoreApp(id), f.now);
        }
        x = r.right() + 20.0 * s;
    }
    if let MainAction::Busy { verifying, .. } = action {
        let (_, hit) = button(bp, ui, f, Id::unique(("bp-game-cancel", id)), pos2(x, y), 0.0, Some(icons::X), t!("Cancel"), Kind::Secondary, None);
        if hit.activated {
            f.app.client.send(if verifying { Command::CancelVerify { app_id: id } } else { Command::CancelDownload { app_id: id } });
        }
    }
    y += 72.0 * s + 50.0 * s;

    // Facts.
    let played = installed.as_ref().and_then(|g| g.playtime_minutes).or_else(|| match f.app.state.player_games.get(&f.app.state.steam_id) {
        Some(Ok(list)) => list.iter().find(|g| g.app_id == id).map(|g| g.playtime_minutes),
        _ => None,
    });
    let mut facts: Vec<(String, String)> = Vec::new();
    facts.push((t!("Play time").into(), played.filter(|m| *m > 0).map(ui::playtime).unwrap_or_else(|| t!("None yet").into())));
    let last = installed.as_ref().and_then(|g| g.last_played);
    facts.push((t!("Last played").into(), last.map(ui::ago).unwrap_or_else(|| t!("Never").into())));
    if let Some(g) = &installed {
        facts.push((t!("Size").into(), ui::bytes(g.size_on_disk)));
    }
    let achievements = f.app.state.achievements.get(&id).copied().or_else(|| match f.app.state.achievement_list.get(&id) {
        Some(Ok(list)) if !list.is_empty() => Some((list.iter().filter(|a| a.unlocked).count() as u32, list.len() as u32)),
        _ => None,
    });
    if let Some((got, total)) = achievements.filter(|a| a.1 > 0) {
        facts.push((t!("Achievements").into(), format!("{got} / {total}")));
    }
    let p = ui.painter().clone();
    let mut x = f.view.left() + f.pad;
    for (label, value) in &facts {
        p.text(pos2(x, y), Align2::LEFT_TOP, label.to_uppercase(), w::font_bold(15.0 * s), f.t.text_faint);
        let r = p.text(pos2(x, y + 26.0 * s), Align2::LEFT_TOP, value, w::font_semibold(28.0 * s), f.t.text);
        x = r.right().max(x + 180.0 * s) + 60.0 * s;
    }
    if let Some((got, total)) = achievements.filter(|a| a.1 > 0) {
        let r = Rect::from_min_size(pos2(x - 40.0 * s, y + 38.0 * s), vec2(220.0 * s, 8.0 * s));
        paint::bar(ui, &f.t, r, got as f32 / total as f32, false);
    }
    y += 100.0 * s;

    // Achievements, when Steam already sent them.
    let list = match f.app.state.achievement_list.get(&id) {
        Some(Ok(list)) => list.clone(),
        _ => Vec::new(),
    };
    if f.app.state.achievement_list.get(&id).is_none() && !f.app.demo && f.app.state.online() && bp.asked.insert(id) {
        f.app.client.send(Command::LoadAchievements { app_id: id, language: i18n::language().steam_name().into() });
    }
    if !list.is_empty() {
        y += 10.0 * s;
        heading(ui, f, &mut y, t!("Achievements"));
        let mut list = list;
        // Unlocked first, newest first; then the rest, most common first.
        list.sort_by(|a, b| b.unlocked.cmp(&a.unlocked).then(b.unlock_time.cmp(&a.unlock_time)).then(b.global_percent.total_cmp(&a.global_percent)));
        let tile = vec2(120.0 * s, 120.0 * s);
        let tiles = shelf(bp, ui, f, &mut y, "achievements", list.len(), tile, |i| Id::unique(("bp-ach", i)));
        let mut focused = None;
        for (i, r, hit) in tiles {
            let a = &list[i];
            let shown = paint::grow(r, 1.0 + 0.08 * hit.glow);
            paint::shadow(ui.painter(), shown, 12.0 * s, s, 0.4 + 0.6 * hit.glow, Color32::from_black_alpha(200));
            let url = if a.unlocked { &a.icon } else { &a.icon_gray };
            if !w::paint_cover(ui, shown, url, paint::radius(12.0 * s)) {
                if url.is_empty() || w::image_state(ui, url) == w::ImageState::Missing {
                    ui.painter().rect_filled(shown, paint::radius(12.0 * s), f.t.raised);
                    ui.painter().text(shown.center(), Align2::CENTER_CENTER, icons::TROPHY, w::font(48.0 * s), if a.unlocked { f.t.away } else { f.t.text_faint });
                } else {
                    w::skeleton(ui, &f.t, shown, paint::radius(12.0 * s));
                }
            }
            if !a.unlocked {
                ui.painter().rect_filled(shown, paint::radius(12.0 * s), Color32::from_black_alpha(90));
            }
            if hit.focused {
                bp.focus.ring_around(shown, 12.0 * s);
                focused = Some(a.clone());
            }
        }
        y += 22.0 * s;
        if let Some(a) = focused {
            let hidden = a.hidden && !a.unlocked;
            let desc = if hidden { t!("Hidden until you unlock it").to_string() } else { a.description.clone() };
            let when = if a.unlocked && a.unlock_time > 0 { t!("Unlocked {date}", date = ui::date(a.unlock_time)) } else { t!("{pct}% of players have this", pct = format!("{:.1}", a.global_percent)) };
            p.text(pos2(f.view.left() + f.pad, y), Align2::LEFT_TOP, &a.name, w::font_bold(24.0 * s), f.t.text);
            paint::line(&p, pos2(f.view.left() + f.pad, y + 34.0 * s), Align2::LEFT_TOP, &format!("{desc}   ·   {when}"), w::font(19.0 * s), f.t.text_dim, f.view.width() - 2.0 * f.pad);
        } else {
            let unlocked = list.iter().filter(|a| a.unlocked).count();
            p.text(pos2(f.view.left() + f.pad, y), Align2::LEFT_TOP, t!("{n} of {total} unlocked", n = unlocked, total = list.len()), w::font(19.0 * s), f.t.text_dim);
        }
        y += 80.0 * s;
    }
    y - top + 40.0 * s
}

// ---------- Store ----------

fn store_tab(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32) -> f32 {
    let s = f.s;
    let mut y = top + 16.0 * s;
    let search_id = Id::unique("bp-store-search");
    let (_, hit) = button(bp, ui, f, search_id, pos2(f.view.left() + f.pad, y), 520.0 * s, Some(icons::SEARCH), t!("Search the store"), Kind::Secondary, None);
    if hit.activated {
        bp.open(Page::Search, f.now);
    }
    if !f.app.demo && f.app.state.wishlist.is_none() && f.app.state.online() && bp.asked.insert(0) {
        f.app.client.send(Command::LoadWishlist);
    }
    y += 72.0 * s + 44.0 * s;
    let front = with_store(f.app, f.actions, |st, cx| st.front_for(cx)).flatten();
    match front {
        None => {
            heading(ui, f, &mut y, t!("Featured"));
            for i in 0..3 {
                let r = Rect::from_min_size(pos2(f.view.left() + f.pad + i as f32 * 586.0 * s, y), vec2(560.0 * s, 320.0 * s));
                w::skeleton(ui, &f.t, r, paint::radius(10.0 * s));
            }
            y += 400.0 * s;
        }
        Some(Err(e)) => {
            empty(bp, ui, f, y, icons::WARNING, t!("The store couldn't load"), &i18n::tr_dyn(&e), None);
            y += 400.0 * s;
        }
        Some(Ok((featured, sections))) => {
            let ids: Vec<AppId> = featured.iter().chain(sections.iter().flat_map(|s| s.items.iter())).map(|i| i.app_id).collect();
            let shown: std::collections::HashMap<AppId, Option<bool>> = with_store(f.app, f.actions, |st, _| ids.iter().map(|id| (*id, st.shown(*id))).collect()).unwrap_or_default();
            let ok = |id: &AppId| shown.get(id).copied().flatten() != Some(false);
            let featured: Vec<&store::Item> = featured.iter().filter(|i| ok(&i.app_id)).collect();
            let mut order = 0;
            if !featured.is_empty() {
                heading(ui, f, &mut y, t!("Featured"));
                let tiles = shelf(bp, ui, f, &mut y, "featured", featured.len(), vec2(560.0 * s, 320.0 * s), |i| Id::unique(("bp-feat", i)));
                for (i, r, hit) in tiles {
                    store_tile(bp, ui, f, r, &hit, order + i, featured[i], shown.get(&featured[i].app_id).copied().flatten(), true);
                }
                order += 2;
                y += 90.0 * s;
            }
            for sec in sections.iter().filter(|s| !s.items.is_empty()).take(6) {
                let items: Vec<&store::Item> = sec.items.iter().filter(|i| ok(&i.app_id)).collect();
                if items.is_empty() {
                    continue;
                }
                heading(ui, f, &mut y, &sec.title);
                let key = format!("sec-{}", sec.id);
                let tiles = shelf(bp, ui, f, &mut y, &key, items.len(), vec2(420.0 * s, 196.0 * s), |i| Id::unique(("bp-sec", &sec.id, i)));
                for (i, r, hit) in tiles {
                    store_tile(bp, ui, f, r, &hit, order + i, items[i], shown.get(&items[i].app_id).copied().flatten(), false);
                }
                order += 1;
                y += 90.0 * s;
            }
        }
    }
    y - top + 20.0 * s
}

#[allow(clippy::too_many_arguments)]
fn store_tile(bp: &mut BigPicture, ui: &mut Ui, f: &Frame, r: Rect, hit: &Hit, order: usize, item: &store::Item, shown: Option<bool>, large: bool) {
    let owned = f.app.state.apps.as_ref().is_some_and(|a| a.contains_key(&item.app_id));
    let price = if owned { t!("In your library").to_string() } else { item.price.label() };
    let color = if owned { f.t.online } else { f.t.text_dim };
    // Not checked against the content filter yet: a placeholder.
    let urls: Vec<&str> = if shown.is_none() { vec![] } else if large { item.large.as_deref().into_iter().chain(std::iter::once(item.header.as_str())).collect() } else { vec![item.header.as_str()] };
    let drawn = wide(bp, ui, f, r, hit, order, &urls, &item.name, &price, color);
    if item.price.discount > 0 && !owned {
        let badge = Rect::from_min_size(drawn.right_bottom() - vec2(84.0 * f.s, 40.0 * f.s), vec2(76.0 * f.s, 32.0 * f.s));
        ui.painter().rect_filled(badge, paint::radius(8.0 * f.s), f.t.discount);
        ui.painter().text(badge.center(), Align2::CENTER_CENTER, format!("-{}%", item.price.discount), w::font_bold(19.0 * f.s), Color32::from_rgb(0xBE, 0xEE, 0x11));
    }
    if hit.focused {
        bp.bg_want = Some(item.app_id);
    }
    if hit.activated {
        bp.open(Page::StoreApp(item.app_id), f.now);
    }
}

fn store_app(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32, id: AppId) -> f32 {
    let s = f.s;
    bp.bg_want = Some(id);
    let details = with_store(f.app, f.actions, |st, cx| st.details_for(cx, id)).flatten();
    let mut y = top + 20.0 * s;
    let left = f.view.left() + f.pad;
    let header = Rect::from_min_size(pos2(left, y), vec2(640.0 * s, 300.0 * s));
    let (k, dy) = appear(ui, bp, 0);
    let c = paint::faded(ui, header.expand(40.0 * s), k);
    let header_url = details.as_ref().map(|d| d.header.clone()).unwrap_or_else(|| art::header(id));
    paint::shadow(c.painter(), header, 14.0 * s, s, 0.8, Color32::from_black_alpha(220));
    if !w::paint_cover(&c, header.translate(vec2(0.0, dy * s)), &header_url, paint::radius(14.0 * s)) {
        w::skeleton(&c, &f.t, header, paint::radius(14.0 * s));
    }
    let tx = header.right() + 48.0 * s;
    let tw = f.view.right() - f.pad - tx;
    let p = ui.painter().clone();
    let owned = f.app.state.apps.as_ref().is_some_and(|a| a.contains_key(&id)) || f.app.local.games.iter().any(|g| g.app_id == id);
    match &details {
        Some(d) => {
            let r = paint::wrapped(&p, pos2(tx, y), &d.name, w::font_bold(44.0 * s), f.t.text, tw, 2);
            let mut ty = r.bottom() + 16.0 * s;
            let meta: Vec<String> = [d.genres.iter().take(3).cloned().collect::<Vec<_>>().join(", "), d.release_date.clone(), d.developers.first().cloned().unwrap_or_default()].into_iter().filter(|x| !x.is_empty()).collect();
            paint::line(&p, pos2(tx, ty), Align2::LEFT_TOP, &meta.join("   ·   "), w::font_medium(19.0 * s), f.t.text_faint, tw);
            ty += 38.0 * s;
            paint::wrapped(&p, pos2(tx, ty), &d.short_description, w::font(22.0 * s), f.t.text_dim, tw, 5);
        }
        None => {
            let name = f.app.state.app_name(id).unwrap_or_default().to_string();
            p.text(pos2(tx, y), Align2::LEFT_TOP, name, w::font_bold(44.0 * s), f.t.text);
            w::skeleton(ui, &f.t, Rect::from_min_size(pos2(tx, y + 80.0 * s), vec2(tw.min(700.0 * s), 120.0 * s)), paint::radius(8.0 * s));
        }
    }
    y = header.bottom() + 44.0 * s;

    // Buttons: open it, get it, or remember it.
    let x = left;
    let main_id = Id::unique(("bp-sa-main", id));
    bp.preferred = Some(main_id);
    let free = details.as_ref().is_some_and(|d| d.is_free);
    if owned {
        let (_, hit) = button(bp, ui, f, main_id, pos2(x, y), 280.0 * s, Some(icons::BOOKS), t!("Open in library"), Kind::Primary, None);
        if hit.activated {
            bp.open(Page::Game(id), f.now);
        }
    } else if free {
        let (_, hit) = button(bp, ui, f, main_id, pos2(x, y), 280.0 * s, Some(icons::PLUS), t!("Add to library"), Kind::Play, None);
        if hit.activated {
            f.app.client.send(Command::RequestFreeLicense { app_id: id });
        }
    } else {
        let price = details.as_ref().and_then(|d| d.price.as_ref().map(|p| p.label()).or(d.price_text.clone())).unwrap_or_default();
        let label = if price.is_empty() { t!("Buy in the store window").to_string() } else { t!("Buy for {price}", price = price) };
        let (r, hit) = button(bp, ui, f, main_id, pos2(x, y), 280.0 * s, Some(icons::CART), &label, Kind::Primary, None);
        if hit.activated {
            // Paying happens in the normal window's store (Steam takes payment on its website).
            bp.leave(ui.ctx());
            f.actions.push(Action::Navigate { page: "store".into(), arg: Some(format!("app:{id}")) });
        }
        let x = r.right() + 20.0 * s;
        let wished = f.app.state.wishlist.as_ref().is_some_and(|w| w.iter().any(|(a, _)| *a == id));
        let (label, kind) = if wished { (t!("On your wishlist"), Kind::Secondary) } else { (t!("Add to wishlist"), Kind::Secondary) };
        let (_, hit) = button(bp, ui, f, Id::unique(("bp-sa-wish", id)), pos2(x, y), 0.0, Some(icons::HEART), label, kind, None);
        if hit.activated {
            f.app.client.send(if wished { Command::WishlistRemove { app_id: id } } else { Command::WishlistAdd { app_id: id } });
        }
    }
    y += 72.0 * s + 56.0 * s;

    // Screenshots.
    if let Some(d) = &details {
        if !d.screenshots.is_empty() {
            heading(ui, f, &mut y, t!("Screenshots"));
            let shots = d.screenshots.clone();
            let tiles = shelf(bp, ui, f, &mut y, "shots", shots.len(), vec2(480.0 * s, 270.0 * s), |i| Id::unique(("bp-shot", i)));
            for (i, r, hit) in tiles {
                let (k, dy) = appear(ui, bp, i + 1);
                let mut c = paint::faded(ui, r.expand(40.0 * s), k);
                let sheen = (f.now - bp.focus.changed_at) as f32 / 0.7;
                let shown = paint::art_tile(&mut c, &f.t, r.translate(vec2(0.0, dy * s)), &[&shots[i].thumb], "", &hit, s, sheen);
                if hit.focused {
                    bp.focus.ring_around(shown, 10.0 * s);
                }
                if hit.activated {
                    bp.open(Page::Shots(id, i), f.now);
                }
            }
            y += 60.0 * s;
        }
    }
    y - top + 20.0 * s
}

pub fn shot_count(app: &mut SteamApp, actions: &mut Vec<Action>, id: AppId) -> usize {
    with_store(app, actions, |st, cx| st.details_for(cx, id).map(|d| d.screenshots.len())).flatten().unwrap_or(0)
}

/// One screenshot over the whole page; left/right flip through them.
fn shots(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, id: AppId, i: usize) -> f32 {
    let s = f.s;
    bp.bg_want = Some(id);
    let list = with_store(f.app, f.actions, |st, cx| st.details_for(cx, id).map(|d| d.screenshots)).flatten().unwrap_or_default();
    let area = f.view.shrink2(vec2(f.pad, 20.0 * s));
    let hit = bp.focus.item(ui, Id::unique("bp-shot-view"), area, ui.clip_rect(), Place::Page);
    let _ = hit;
    let Some(shot) = list.get(i) else {
        w::skeleton(ui, &f.t, area, paint::radius(12.0 * s));
        return f.view.height();
    };
    let (at, way) = bp.shot_since;
    let k = since(ui, at, 0.3);
    let r = area.translate(vec2((1.0 - k) * 120.0 * s * way, 0.0));
    let c = paint::faded(ui, area.expand(200.0 * s), k);
    // The thumbnail first (it's already loaded), then the full picture over it.
    let fit = |c: &Ui, url: &str| w::paint_contain(c, r, url, Align2::CENTER_CENTER);
    if !fit(&c, &shot.full) && !fit(&c, &shot.thumb) {
        w::skeleton(ui, &f.t, area, paint::radius(12.0 * s));
    }
    let label = format!("{} / {}", i + 1, list.len());
    let pill = Rect::from_center_size(pos2(area.center().x, area.bottom() - 24.0 * s), vec2(130.0 * s, 44.0 * s));
    ui.painter().rect_filled(pill, paint::radius(22.0 * s), Color32::from_black_alpha(170));
    ui.painter().text(pill.center(), Align2::CENTER_CENTER, label, w::font_semibold(20.0 * s), f.t.text);
    for (dx, icon) in [(-1.0, icons::CARET_LEFT), (1.0, icons::CARET_RIGHT)] {
        let c = pos2(area.center().x + dx * (area.width() / 2.0 - 30.0 * s), area.center().y);
        ui.painter().circle_filled(c, 30.0 * s, Color32::from_black_alpha(140));
        ui.painter().text(c, Align2::CENTER_CENTER, icon, w::font(30.0 * s), f.t.text);
    }
    f.view.height()
}

// ---------- Search with the on-screen keyboard ----------

const KEYS_EN: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl'", "zxcvbnm-:."];
const KEYS_RU: [&str; 4] = ["1234567890-", "йцукенгшщзх", "фывапролджэ", "ячсмитьбюъё"];

fn search(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32) -> f32 {
    let s = f.s;
    let mut y = top + 16.0 * s;
    let left = f.view.left() + f.pad;
    // X deletes, Y types a space (on a controller).
    if f.x && bp.query.pop().is_some() {
        bp.query_changed = f.now;
    }
    if f.y {
        bp.query.push(' ');
        bp.query_changed = f.now;
    }
    // Search a moment after typing stops.
    let q = bp.query.trim().to_string();
    if q != bp.submitted && f.now - bp.query_changed > 0.6 && (q.chars().count() >= 2 || q.is_empty()) {
        bp.submitted = q;
    }

    // The text box.
    let field = Rect::from_min_size(pos2(left, y), vec2((f.view.width() - 2.0 * f.pad).min(1100.0 * s), 84.0 * s));
    let p = ui.painter().clone();
    p.rect_filled(field, paint::radius(42.0 * s), Color32::from_black_alpha(120));
    p.rect_stroke(field, paint::radius(42.0 * s), egui::Stroke::new(2.0 * s, f.t.accent.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    p.text(pos2(field.left() + 36.0 * s, field.center().y), Align2::LEFT_CENTER, icons::SEARCH, w::font(30.0 * s), f.t.text_dim);
    let tx = field.left() + 84.0 * s;
    let typed = if bp.query.is_empty() {
        p.text(pos2(tx, field.center().y), Align2::LEFT_CENTER, t!("Type a game's name"), w::font(30.0 * s), f.t.text_faint);
        Rect::from_min_size(pos2(tx, field.center().y), Vec2::ZERO)
    } else {
        paint::line(&p, pos2(tx, field.center().y), Align2::LEFT_CENTER, &bp.query, w::font_semibold(32.0 * s), f.t.text, field.right() - tx - 40.0 * s)
    };
    // A blinking caret.
    let blink = ((f.now * 3.0).sin() as f32 * 0.5 + 0.5).powf(0.6);
    p.vline(typed.right() + 4.0 * s, (field.center().y - 20.0 * s)..=(field.center().y + 20.0 * s), egui::Stroke::new(3.0 * s, f.t.accent.gamma_multiply(blink)));
    y += 84.0 * s + 36.0 * s;

    // The keyboard.
    match keyboard(bp, ui, f, &mut y, t!("Search")) {
        Some(Key::Char(ch)) => {
            bp.query.push(ch);
            bp.query_changed = f.now;
        }
        Some(Key::Del) => {
            bp.query.pop();
            bp.query_changed = f.now;
        }
        Some(Key::Go) => bp.submitted = bp.query.trim().to_string(),
        None => {}
    }
    y += 50.0 * s - 10.0 * s;
    bp.preferred = Some(Id::unique(("bp-key", 1, 0)));

    // Results.
    if bp.submitted.is_empty() {
        return y - top;
    }
    let query = bp.submitted.clone();
    heading(ui, f, &mut y, &t!("Results for “{q}”", q = query));
    match with_store(f.app, f.actions, |st, cx| st.search_for(cx, &query)).flatten() {
        None => {
            for i in 0..4 {
                let r = Rect::from_min_size(pos2(left + i as f32 * 446.0 * s, y), vec2(420.0 * s, 196.0 * s));
                w::skeleton(ui, &f.t, r, paint::radius(10.0 * s));
            }
            y += 196.0 * s;
        }
        Some(Err(e)) => {
            p.text(pos2(left, y), Align2::LEFT_TOP, i18n::tr_dyn(&e), w::font(22.0 * s), f.t.danger);
            y += 40.0 * s;
        }
        Some(Ok(items)) if items.is_empty() => {
            p.text(pos2(left, y), Align2::LEFT_TOP, t!("No games found"), w::font(22.0 * s), f.t.text_dim);
            y += 40.0 * s;
        }
        Some(Ok(items)) => {
            let ids: Vec<AppId> = items.iter().map(|i| i.app_id).collect();
            let shown: std::collections::HashMap<AppId, Option<bool>> = with_store(f.app, f.actions, |st, _| ids.iter().map(|id| (*id, st.shown(*id))).collect()).unwrap_or_default();
            let items: Vec<&store::Item> = items.iter().filter(|i| shown.get(&i.app_id).copied().flatten() != Some(false)).collect();
            let key = format!("results-{query}");
            let tiles = shelf(bp, ui, f, &mut y, &key, items.len(), vec2(420.0 * s, 196.0 * s), |i| Id::unique(("bp-result", i)));
            for (i, r, hit) in tiles {
                store_tile(bp, ui, f, r, &hit, i, items[i], shown.get(&items[i].app_id).copied().flatten(), false);
            }
            y += 90.0 * s;
        }
    }
    y - top + 20.0 * s
}

/// A key pressed on the on-screen keyboard.
enum Key {
    Char(char),
    Del,
    Go,
}

/// The on-screen keyboard (letters, Space, Delete, ABC/АБВ and `go`), from
/// `y` down; `y` ends below it. Switching the alphabet is handled here.
fn keyboard(bp: &mut BigPicture, ui: &mut Ui, f: &Frame, y: &mut f32, go: &str) -> Option<Key> {
    let s = f.s;
    let left = f.view.left() + f.pad;
    let rows = if bp.cyrillic { KEYS_RU } else { KEYS_EN };
    let key = 74.0 * s;
    let gap = 10.0 * s;
    let mut out = None;
    for (ri, row) in rows.iter().enumerate() {
        for (ci, ch) in row.chars().enumerate() {
            let r = Rect::from_min_size(pos2(left + ci as f32 * (key + gap), *y), vec2(key, key));
            if key_cap(bp, ui, f, Id::unique(("bp-key", ri, ci)), r, &ch.to_string(), false) {
                out = Some(Key::Char(ch));
            }
        }
        *y += key + gap;
    }
    let unit = key + gap;
    let specials = [(4.0, " ", t!("Space")), (2.0, "del", t!("Delete")), (2.0, "lang", if bp.cyrillic { "ABC" } else { "АБВ" }), (3.0, "go", go)];
    let mut x = left;
    for (i, (cols, what, label)) in specials.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, *y), vec2(cols * unit - gap, key));
        if key_cap(bp, ui, f, Id::unique(("bp-key-special", i)), r, label, *what == "go") {
            out = match *what {
                " " => Some(Key::Char(' ')),
                "del" => Some(Key::Del),
                "lang" => {
                    bp.cyrillic = !bp.cyrillic;
                    None
                }
                _ => Some(Key::Go),
            };
        }
        x += cols * unit;
    }
    *y += key + gap;
    out
}

/// One key of the on-screen keyboard; true when pressed. Keys pop when hit.
fn key_cap(bp: &mut BigPicture, ui: &Ui, f: &Frame, id: Id, r: Rect, label: &str, accent: bool) -> bool {
    let s = f.s;
    let hit = bp.focus.item(ui, id, r, ui.clip_rect(), Place::Page);
    if hit.activated {
        bp.key_pop = Some((id, f.now));
    }
    let pop = bp.key_pop.filter(|(k, _)| *k == id).map(|(_, at)| ((f.now - at) / 0.18) as f32).filter(|e| *e < 1.0);
    let bump = pop.map(|e| (e * std::f32::consts::PI).sin() * 0.1).unwrap_or(0.0);
    if pop.is_some() {
        ui.ctx().request_repaint();
    }
    let shown = paint::grow(r, 1.0 + 0.06 * hit.glow - bump);
    let fill = if accent { f.t.accent } else { w::mix(f.t.raised, f.t.accent, hit.glow * 0.85) };
    ui.painter().rect_filled(shown, paint::radius(12.0 * s), fill);
    let size = if label.chars().count() == 1 { 32.0 } else { 22.0 };
    ui.painter().text(shown.center(), Align2::CENTER_CENTER, label, w::font_semibold(size * s), f.t.text);
    if hit.focused {
        bp.focus.ring_around(shown, 12.0 * s);
    }
    hit.activated
}

// ---------- Downloads ----------

fn downloads(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32) -> f32 {
    let s = f.s;
    let mut y = top + 16.0 * s;
    let left = f.view.left() + f.pad;
    let width = f.view.width() - 2.0 * f.pad;
    let mut active: Vec<model::DownloadStatus> = f.app.state.downloads.values().filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).cloned().collect();
    active.sort_by_key(|d| d.app_id);
    let queue = f.app.state.download_queue.clone();
    if active.is_empty() && queue.is_empty() {
        if empty(bp, ui, f, y, icons::DOWNLOAD, t!("Nothing is downloading"), t!("Games you install or update show up here."), Some((icons::BOOKS, t!("Go to your library")))) == Some(true) {
            bp.switch_tab(Tab::Library, f.now, true);
        }
        return 500.0 * s;
    }
    let p = ui.painter().clone();
    for (n, d) in active.iter().enumerate() {
        let id = d.app_id;
        let card = Rect::from_min_size(pos2(left, y), vec2(width, 220.0 * s));
        let (k, dy) = appear(ui, bp, n);
        let card = card.translate(vec2(0.0, dy * s));
        let c = paint::faded(ui, card.expand(30.0 * s), k);
        c.painter().rect_filled(card, paint::radius(18.0 * s), Color32::from_black_alpha(110));
        let art_r = Rect::from_min_size(card.min + vec2(20.0 * s, 20.0 * s), vec2(386.0 * s, 180.0 * s));
        if !w::paint_cover(&c, art_r, &art::header(id), paint::radius(12.0 * s)) {
            w::skeleton(&c, &f.t, art_r, paint::radius(12.0 * s));
        }
        let tx = art_r.right() + 36.0 * s;
        let name = f.app.state.app_name(id).unwrap_or_default().to_string();
        paint::line(c.painter(), pos2(tx, art_r.top() + 4.0 * s), Align2::LEFT_TOP, &name, w::font_bold(30.0 * s), f.t.text, card.right() - tx - 480.0 * s);
        let status = if d.paused { t!("Paused").to_string() } else { i18n::tr_dyn(d.activity()) };
        c.painter().text(pos2(tx, art_r.top() + 52.0 * s), Align2::LEFT_TOP, status, w::font_medium(21.0 * s), if d.paused { f.t.away } else { f.t.accent });
        let (done, total) = d.shown();
        let bar = Rect::from_min_size(pos2(tx, art_r.top() + 100.0 * s), vec2(card.right() - tx - 480.0 * s, 12.0 * s));
        paint::bar(&c, &f.t, bar, d.fraction(), !d.paused);
        let mut line = format!("{} / {}", ui::bytes(done), ui::bytes(total));
        if !d.paused && d.speed > 0 {
            line = format!("{line}   ·   {}/s", ui::bytes(d.speed));
        }
        c.painter().text(pos2(tx, bar.bottom() + 16.0 * s), Align2::LEFT_TOP, line, w::font(19.0 * s), f.t.text_dim);
        // Buttons on the right.
        let mut bx = card.right() - 32.0 * s;
        let (r, cancel) = {
            let label = t!("Cancel");
            let wdt = paint::pill_width(ui, true, label, s);
            bx -= wdt;
            button(bp, &c, f, Id::unique(("bp-dl-cancel", id)), pos2(bx, card.center().y - 36.0 * s), wdt, Some(icons::X), label, Kind::Secondary, None)
        };
        if !d.verifying() {
            let (label, icon) = if d.paused { (t!("Resume"), icons::PLAY) } else { (t!("Pause"), icons::STOP) };
            let wdt = paint::pill_width(ui, true, label, s).max(190.0 * s);
            let at = pos2(r.left() - 18.0 * s - wdt, card.center().y - 36.0 * s);
            let (_, hit) = button(bp, &c, f, Id::unique(("bp-dl-pause", id)), at, wdt, Some(icon), label, Kind::Primary, None);
            if n == 0 {
                bp.preferred = Some(Id::unique(("bp-dl-pause", id)));
            }
            if hit.activated {
                f.app.client.send(if d.paused { Command::ResumeDownload { app_id: id } } else { Command::PauseDownload { app_id: id } });
            }
        }
        if cancel.activated {
            f.app.client.send(if d.verifying() { Command::CancelVerify { app_id: id } } else { Command::CancelDownload { app_id: id } });
        }
        bp.bg_want = bp.bg_want.or(Some(id));
        y += 220.0 * s + 24.0 * s;
    }
    if !queue.is_empty() {
        y += 20.0 * s;
        heading(ui, f, &mut y, t!("Up next"));
        for (n, id) in queue.iter().enumerate() {
            let row = Rect::from_min_size(pos2(left, y), vec2(width, 120.0 * s));
            p.rect_filled(row, paint::radius(14.0 * s), Color32::from_black_alpha(80));
            let art_r = Rect::from_min_size(row.min + vec2(14.0 * s, 12.0 * s), vec2(206.0 * s, 96.0 * s));
            if !w::paint_cover(ui, art_r, &art::header(*id), paint::radius(10.0 * s)) {
                w::skeleton(ui, &f.t, art_r, paint::radius(10.0 * s));
            }
            let name = f.app.state.app_name(*id).unwrap_or_default().to_string();
            paint::line(&p, pos2(art_r.right() + 28.0 * s, row.center().y - 4.0 * s), Align2::LEFT_BOTTOM, &name, w::font_semibold(25.0 * s), f.t.text, width - 700.0 * s);
            p.text(pos2(art_r.right() + 28.0 * s, row.center().y + 4.0 * s), Align2::LEFT_TOP, t!("Queued · position {n}", n = n + 1), w::font(19.0 * s), f.t.text_dim);
            let label = t!("Remove");
            let wdt = paint::pill_width(ui, true, label, s);
            let (_, hit) = button(bp, ui, f, Id::unique(("bp-q-remove", *id)), pos2(row.right() - 24.0 * s - wdt, row.center().y - 36.0 * s), wdt, Some(icons::TRASH), label, Kind::Secondary, None);
            if hit.activated {
                f.app.client.send(Command::CancelDownload { app_id: *id });
            }
            y += 120.0 * s + 14.0 * s;
        }
    }
    y - top + 30.0 * s
}

// ---------- Friends ----------

fn friends(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32) -> f32 {
    let s = f.s;
    let mut y = top + 16.0 * s;
    let left = f.view.left() + f.pad;
    let mut list = f.app.state.friend_list();
    let rank = |p: &model::Persona| {
        if p.in_game() {
            0
        } else {
            match p.state {
                PersonaState::Offline | PersonaState::Invisible => 3,
                PersonaState::Away | PersonaState::Snooze | PersonaState::Busy => 2,
                _ => 1,
            }
        }
    };
    list.sort_by_key(|p| (rank(p), p.display_name().to_lowercase()));
    let online = list.iter().filter(|p| rank(p) < 3).count();
    let p = ui.painter().clone();
    p.text(pos2(left, y), Align2::LEFT_TOP, t!("{n} of {total} online", n = online, total = list.len()), w::font_medium(22.0 * s), f.t.text_dim);
    y += 56.0 * s;
    if list.is_empty() {
        empty(bp, ui, f, y, icons::USERS, t!("No friends yet"), t!("Friends you add on Steam show up here."), None);
        return 500.0 * s;
    }
    let width = f.view.width() - 2.0 * f.pad;
    let gap = 24.0 * s;
    let cols = ((width + gap) / (520.0 * s + gap)).floor().max(1.0) as usize;
    let card_w = (width - gap * (cols - 1) as f32) / cols as f32;
    let card_h = 132.0 * s;
    let clip = ui.clip_rect();
    for (k, fr) in list.iter().enumerate() {
        let (row, col) = (k / cols, k % cols);
        let r = Rect::from_min_size(pos2(left + col as f32 * (card_w + gap), y + row as f32 * (card_h + gap)), vec2(card_w, card_h));
        let hit = bp.focus.item(ui, Id::unique(("bp-friend", fr.steam_id)), r, clip, Place::Page);
        if r.bottom() < clip.top() || r.top() > clip.bottom() {
            continue;
        }
        let (a, dy) = appear(ui, bp, k);
        let r = r.translate(vec2(0.0, dy * s));
        let c = paint::faded(ui, r.expand(30.0 * s), a);
        let shown = paint::grow(r, 1.0 + 0.03 * hit.glow);
        let fill = w::mix(Color32::from_black_alpha(100), Color32::from_white_alpha(26), hit.glow);
        c.painter().rect_filled(shown, paint::radius(16.0 * s), fill);
        let color = crate::shell::status_color(&f.t, fr);
        let av = Rect::from_center_size(pos2(shown.left() + 70.0 * s, shown.center().y), vec2(84.0 * s, 84.0 * s));
        if !w::paint_cover(&c, av, &art::avatar_full(fr.avatar_hash.as_deref()), paint::radius(42.0 * s)) {
            c.painter().circle_filled(av.center(), 42.0 * s, f.t.raised);
        }
        c.painter().circle_stroke(av.center(), 45.0 * s, egui::Stroke::new(3.0 * s, color));
        let tx = av.right() + 26.0 * s;
        let tw = shown.right() - tx - 20.0 * s;
        paint::line(c.painter(), pos2(tx, shown.center().y - 4.0 * s), Align2::LEFT_BOTTOM, &fr.display_name(), w::font_semibold(26.0 * s), if rank(fr) == 3 { f.t.text_dim } else { f.t.text }, tw);
        paint::line(c.painter(), pos2(tx, shown.center().y + 4.0 * s), Align2::LEFT_TOP, &crate::shell::status_text(&f.app.state, fr), w::font(19.0 * s), color, tw);
        if hit.focused {
            bp.focus.ring_around(shown, 16.0 * s);
            if let Some(g) = fr.game_app_id {
                bp.bg_want = Some(g);
            }
        }
        // A on a friend: their page (message, join, invite, profile).
        if hit.activated {
            bp.open(Page::Friend(fr.steam_id), f.now);
        }
    }
    let rows = list.len().div_ceil(cols);
    y += rows as f32 * (card_h + gap);
    y - top + 20.0 * s
}

/// A friend: who they are, what they play, and what you can do together.
fn friend_page(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32, id: model::SteamId) -> f32 {
    let s = f.s;
    let left = f.view.left() + f.pad;
    let mut y = top + 24.0 * s;
    let fr = f.app.state.persona(id);
    let color = crate::shell::status_color(&f.t, &fr);
    let p = ui.painter().clone();
    let av = Rect::from_min_size(pos2(left, y), vec2(184.0 * s, 184.0 * s));
    if !w::paint_cover(ui, av, &art::avatar_full(fr.avatar_hash.as_deref()), paint::radius(92.0 * s)) {
        p.circle_filled(av.center(), 92.0 * s, f.t.raised);
    }
    p.circle_stroke(av.center(), 96.0 * s, egui::Stroke::new(4.0 * s, color));
    let tx = av.right() + 44.0 * s;
    let tw = f.view.right() - f.pad - tx;
    paint::line(&p, pos2(tx, av.center().y - 10.0 * s), Align2::LEFT_BOTTOM, &fr.display_name(), w::font_bold(52.0 * s), f.t.text, tw);
    paint::line(&p, pos2(tx, av.center().y + 6.0 * s), Align2::LEFT_TOP, &crate::shell::status_text(&f.app.state, &fr), w::font_medium(26.0 * s), color, tw);
    if let Some(g) = fr.game_app_id.filter(|_| fr.in_game()) {
        bp.bg_want = Some(g);
    }
    y = av.bottom() + 56.0 * s;

    // What you can do, best first; the first is where focus starts.
    let mut x = left;
    let mut next = |bp: &mut BigPicture, ui: &mut Ui, f: &Frame, key: &str, icon: &str, label: &str, kind: Kind| {
        let (r, hit) = button(bp, ui, f, Id::unique(("bp-friend-act", key)), pos2(x, y), 240.0 * s, Some(icon), label, kind, None);
        x = r.right() + 20.0 * s;
        hit.activated
    };
    let game = fr.game_app_id.filter(|_| fr.in_game());
    let game_name = game.map(|g| f.app.state.app_name(g).map(String::from).unwrap_or_else(|| t!("the game").to_string()));
    if let (Some(g), Some(args)) = (game, fr.join_args()) {
        if next(bp, ui, f, "join", icons::PLAY, t!("Join game"), Kind::Play) {
            let name = game_name.clone().unwrap_or_default();
            match f.app.local.games.iter().find(|x| x.app_id == g) {
                Some(installed) => {
                    f.app.client.send(Command::Launch { app_id: g, install_dir: installed.install_dir.clone(), args });
                    f.actions.push(Action::Toast(t!("Joining {name} in {game}…", name = fr.display_name(), game = name)));
                }
                None => f.actions.push(Action::Toast(t!("Install {game} to join {name}.", game = name, name = fr.display_name()))),
            }
        }
    }
    if next(bp, ui, f, "chat", icons::CHAT, t!("Send a message"), if game.is_some() { Kind::Secondary } else { Kind::Primary }) {
        bp.open(Page::Chat(id), f.now);
    }
    for (i, (invite, label)) in crate::shell::invite_options(&f.app.state).into_iter().enumerate() {
        if next(bp, ui, f, &format!("invite{i}"), icons::USER_PLUS, &label, Kind::Secondary) {
            crate::shell::send_invite(f.app, &fr, invite, f.actions);
        }
    }
    if let Some(g) = game {
        if next(bp, ui, f, "game", icons::CONTROLLER, t!("Game details"), Kind::Secondary) {
            open_game(bp, f, g);
        }
    }
    if next(bp, ui, f, "profile", icons::USER_CIRCLE, t!("View profile"), Kind::Secondary) {
        // Profiles live in the normal window.
        bp.leave(ui.ctx());
        f.actions.push(Action::Navigate { page: "profile".into(), arg: Some(id.to_string()) });
    }
    y += 72.0 * s + 24.0 * s;
    p.text(pos2(left, y), Align2::LEFT_TOP, t!("View profile opens it in the normal window."), w::font(19.0 * s), f.t.text_faint);
    y += 40.0 * s;
    y - top
}

/// A conversation: the latest messages, and a box typed with the on-screen
/// keyboard (or a real one; Enter sends).
fn chat_page(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32, id: model::SteamId) -> f32 {
    let s = f.s;
    let left = f.view.left() + f.pad;
    let width = (f.view.width() - 2.0 * f.pad).min(1180.0 * s);
    let mut y = top + 16.0 * s;
    if bp.chats_loaded.insert(id) {
        f.app.client.send(Command::LoadHistory { friend: id });
    }
    if f.app.state.unread.get(&id).is_some_and(|n| *n > 0) {
        f.actions.push(Action::MarkRead(id));
    }
    let fr = f.app.state.persona(id);
    let p = ui.painter().clone();
    let typing = if f.app.state.is_typing(id) { format!("  ·  {}", t!("typing…")) } else { String::new() };
    p.text(pos2(left, y), Align2::LEFT_TOP, format!("{}{typing}", fr.display_name()), w::font_bold(34.0 * s), f.t.text);
    y += 54.0 * s;

    // The last messages, oldest at the top.
    let me = f.app.state.steam_id;
    let messages: Vec<model::ChatMessage> = f.app.state.chats.get(&id).map(|c| c.iter().rev().take(4).rev().cloned().collect()).unwrap_or_default();
    // Short enough that the whole keyboard fits on a 1080p screen.
    let area = Rect::from_min_size(pos2(left, y), vec2(width, 200.0 * s));
    p.rect_filled(area, paint::radius(18.0 * s), Color32::from_black_alpha(90));
    if messages.is_empty() {
        p.text(area.center(), Align2::CENTER_CENTER, t!("No messages yet"), w::font(22.0 * s), f.t.text_faint);
    }
    let mut by = area.bottom() - 18.0 * s;
    let font = w::font(22.0 * s);
    for m in messages.iter().rev() {
        let mine = m.from == me;
        let wrap = width * 0.68;
        let job = egui::text::LayoutJob::simple(m.text.clone(), font.clone(), f.t.text, wrap);
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let size = galley.size() + vec2(36.0 * s, 24.0 * s);
        let bx = if mine { area.right() - 18.0 * s - size.x } else { area.left() + 18.0 * s };
        let bubble = Rect::from_min_size(pos2(bx, by - size.y), size);
        if bubble.top() < area.top() + 10.0 * s {
            break;
        }
        p.rect_filled(bubble, paint::radius(16.0 * s), if mine { f.t.accent.gamma_multiply(0.55) } else { f.t.raised });
        p.galley(bubble.min + vec2(18.0 * s, 12.0 * s), galley, f.t.text);
        by = bubble.top() - 12.0 * s;
    }
    y = area.bottom() + 18.0 * s;

    // The message box.
    if f.x {
        bp.message.pop();
    }
    if f.y {
        bp.message.push(' ');
    }
    let field = Rect::from_min_size(pos2(left, y), vec2(width, 64.0 * s));
    p.rect_filled(field, paint::radius(32.0 * s), Color32::from_black_alpha(120));
    p.rect_stroke(field, paint::radius(32.0 * s), egui::Stroke::new(2.0 * s, f.t.accent.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    let tx = field.left() + 32.0 * s;
    let typed = if bp.message.is_empty() {
        p.text(pos2(tx, field.center().y), Align2::LEFT_CENTER, t!("Message {name}", name = fr.display_name()), w::font(26.0 * s), f.t.text_faint);
        Rect::from_min_size(pos2(tx, field.center().y), Vec2::ZERO)
    } else {
        paint::line(&p, pos2(tx, field.center().y), Align2::LEFT_CENTER, &bp.message, w::font_semibold(28.0 * s), f.t.text, field.right() - tx - 40.0 * s)
    };
    let blink = ((f.now * 3.0).sin() as f32 * 0.5 + 0.5).powf(0.6);
    p.vline(typed.right() + 4.0 * s, (field.center().y - 18.0 * s)..=(field.center().y + 18.0 * s), egui::Stroke::new(3.0 * s, f.t.accent.gamma_multiply(blink)));
    y = field.bottom() + 18.0 * s;
    let mut send = std::mem::take(&mut bp.send_now);
    match keyboard(bp, ui, f, &mut y, t!("Send")) {
        Some(Key::Char(ch)) => bp.message.push(ch),
        Some(Key::Del) => {
            bp.message.pop();
        }
        Some(Key::Go) => send = true,
        None => {}
    }
    bp.preferred = Some(Id::unique(("bp-key", 1, 0)));
    let text = bp.message.trim().to_string();
    if send && !text.is_empty() {
        f.app.client.send(Command::SendMessage { to: id, text });
        bp.message.clear();
    }
    y - top + 30.0 * s
}

// ---------- Settings ----------

const SIZES: [f32; 4] = [0.85, 1.0, 1.15, 1.3];

fn settings(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32) -> f32 {
    let s = f.s;
    let mut y = top + 16.0 * s;
    let left = f.view.left() + f.pad;
    let p = ui.painter().clone();
    p.text(pos2(left, y), Align2::LEFT_TOP, t!("Big Picture settings"), w::font_bold(40.0 * s), f.t.text);
    y += 80.0 * s;
    let lang = i18n::language();
    // Selecting the row moves to the next language.
    let all = i18n::Lang::ALL;
    let other = all[(all.iter().position(|l| *l == lang).unwrap_or(0) + 1) % all.len()];
    let rows = [
        (icons::MONITOR, t!("Interface size"), format!("{}%", (f.app.prefs.bigpicture_scale * 100.0).round() as u32)),
        (icons::DESKTOP, t!("Start SteamInRust in Big Picture"), if f.app.prefs.bigpicture_start { t!("On") } else { t!("Off") }.to_string()),
        (icons::GLOBE, t!("Language"), lang.native_name().to_string()),
        (icons::GEAR, t!("All settings (leaves Big Picture)"), String::new()),
    ];
    let width = (f.view.width() - 2.0 * f.pad).min(1100.0 * s);
    for (i, (icon, label, value)) in rows.iter().enumerate() {
        let r = Rect::from_min_size(pos2(left, y), vec2(width, 88.0 * s));
        let hit = bp.focus.item(ui, Id::unique(("bp-set", i)), r, ui.clip_rect(), Place::Page);
        let (k, dy) = appear(ui, bp, i);
        let r = r.translate(vec2(0.0, dy * s));
        let c = paint::faded(ui, r.expand(20.0 * s), k);
        let shown = paint::grow(r, 1.0 + 0.02 * hit.glow);
        c.painter().rect_filled(shown, paint::radius(16.0 * s), w::mix(Color32::from_black_alpha(100), Color32::from_white_alpha(26), hit.glow));
        c.painter().text(pos2(shown.left() + 30.0 * s, shown.center().y), Align2::LEFT_CENTER, *icon, w::font(30.0 * s), f.t.text_dim);
        c.painter().text(pos2(shown.left() + 84.0 * s, shown.center().y), Align2::LEFT_CENTER, *label, w::font_semibold(25.0 * s), f.t.text);
        if !value.is_empty() {
            c.painter().text(pos2(shown.right() - 30.0 * s, shown.center().y), Align2::RIGHT_CENTER, format!("{value}   {}", icons::CARET_RIGHT), w::font_medium(23.0 * s), f.t.accent);
        }
        if hit.focused {
            bp.focus.ring_around(shown, 16.0 * s);
        }
        if hit.activated {
            match i {
                0 => {
                    let cur = SIZES.iter().position(|x| (*x - f.app.prefs.bigpicture_scale).abs() < 0.01).unwrap_or(1);
                    f.app.prefs.bigpicture_scale = SIZES[(cur + 1) % SIZES.len()];
                }
                1 => f.app.prefs.bigpicture_start = !f.app.prefs.bigpicture_start,
                2 => f.actions.push(Action::SetLanguage(other.code().into())),
                _ => {
                    bp.leave(ui.ctx());
                    f.actions.push(Action::Navigate { page: "settings".into(), arg: None });
                }
            }
        }
        y += 88.0 * s + 16.0 * s;
    }
    y - top + 20.0 * s
}

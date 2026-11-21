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
    // Leaving a trailer stops it.
    if bp.video_on && !matches!(bp.page(), Page::Video(..)) {
        bp.video_on = false;
        with_store(f.app, f.actions, |st, _| st.stop_trailer());
    }
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
        Page::Video(id, i) => video_page(bp, ui, f, id, i),
        Page::Install(id) => install_page(bp, ui, f, top, id),
    }
}

/// What X does on this page, for the hints.
pub fn x_hint(bp: &BigPicture) -> Option<String> {
    match bp.page() {
        Page::Tab(Tab::Library) => Some(t!("Change filter").into()),
        Page::Install(_) => Some(t!("Desktop shortcut").into()),
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
    // A sideways wheel or Shift+wheel over the row scrolls it (the focus
    // only follows the D-pad and keys).
    let row = Rect::from_min_max(pos2(f.view.left(), *y), pos2(f.view.right(), *y + tile.y));
    if ui.rect_contains_pointer(row) {
        let dx = ui.input(|i| i.smooth_scroll_delta.x);
        if dx != 0.0 {
            sc.target -= dx;
        }
    }
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
        MainAction::FirstTimeSetup => (t!("Performing first-time setup…").into(), icons::DOWNLOAD, Kind::Muted, None),
        MainAction::Queued => (t!("Queued").into(), icons::CLOCK, Kind::Muted, None),
        MainAction::Uninstalling => (t!("Uninstalling…").into(), icons::TRASH, Kind::Muted, None),
        MainAction::Moving => (t!("Moving…").into(), icons::DRIVES, Kind::Muted, None),
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
    let entries = crate::builtin::entries(&f.app.state, &f.app.local);
    // Games that can run here (a Mac lists Windows-only games too).
    let runs: std::collections::HashSet<AppId> = entries.iter().filter(|e| e.installed.is_some() || e.owned.is_none_or(|o| o.runs_on_this_os())).map(|e| e.app_id).collect();
    let mut games: Vec<(AppId, String, bool)> = entries.iter().map(|e| (e.app_id, e.name.to_string(), e.installed.is_some())).collect();
    let ids: Vec<AppId> = games.iter().map(|g| g.0).collect();
    let hidden: std::collections::HashSet<AppId> = with_library(f.app, f.actions, |lib, _| ids.iter().copied().filter(|id| lib.is_hidden(*id)).collect()).unwrap_or_default();
    games.retain(|g| !hidden.contains(&g.0));
    games.sort_by_key(|g| g.1.to_lowercase());
    let n_installed = games.iter().filter(|g| g.2).count();
    let n_runs = games.iter().filter(|g| runs.contains(&g.0)).count();
    let installed_only = *bp.installed_only.get_or_insert(n_installed > 0);
    // Installed, runs on this computer (only when some don't), or all.
    let mut chips: Vec<(u8, String)> = vec![(0, t!("Installed ({n})", n = n_installed))];
    if n_runs < games.len() {
        chips.push((1, t!("Runs on this computer ({n})", n = n_runs)));
    }
    chips.push((2, t!("All games ({n})", n = games.len())));
    let current = if installed_only { 0 } else if bp.this_os_only && n_runs < games.len() { 1 } else { 2 };
    if f.x {
        // X steps through the filters.
        let next = chips[(chips.iter().position(|c| c.0 == current).unwrap_or(0) + 1) % chips.len()].0;
        bp.installed_only = Some(next == 0);
        bp.this_os_only = next == 1;
    }
    let mut x = f.view.left() + f.pad;
    for (i, (value, label)) in chips.iter().enumerate() {
        let id = Id::unique(("bp-filter", i));
        let on = current == *value;
        let icon = (*value == 1).then_some(if cfg!(target_os = "macos") { icons::APPLE } else if cfg!(windows) { icons::WINDOWS } else { icons::LINUX });
        let (r, hit) = button(bp, ui, f, id, pos2(x, y), 0.0, icon, label, if on { Kind::Primary } else { Kind::Secondary }, None);
        if hit.activated {
            bp.installed_only = Some(*value == 0);
            bp.this_os_only = *value == 1;
        }
        x = r.right() + 18.0 * s;
    }
    y += 72.0 * s + 40.0 * s;

    let list: Vec<&(AppId, String, bool)> = games.iter().filter(|g| match current {
        0 => g.2,
        1 => runs.contains(&g.0),
        _ => true,
    }).collect();
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
            let caption = if *installed {
                String::new()
            } else if !runs.contains(id) {
                t!("Doesn't run on this computer").to_string()
            } else {
                t!("Not installed").to_string()
            };
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
        } else if action == MainAction::Install {
            // Like Steam: size, where, and a shortcut, before anything starts.
            bp.install = None;
            bp.open(Page::Install(id), f.now);
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

    // Trailers: play over the page (A pauses, left/right seek, B closes).
    let trailers = with_store(f.app, f.actions, |st, _| st.trailers_for(id)).unwrap_or_default();
    if !trailers.is_empty() {
        heading(ui, f, &mut y, t!("Trailers"));
        let tiles = shelf(bp, ui, f, &mut y, "trailers", trailers.len(), vec2(480.0 * s, 270.0 * s), |i| Id::unique(("bp-trailer", i)));
        for (i, r, hit) in tiles {
            let (k, dy) = appear(ui, bp, i + 1);
            let mut c = paint::faded(ui, r.expand(40.0 * s), k);
            let sheen = (f.now - bp.focus.changed_at) as f32 / 0.7;
            let shown = paint::art_tile(&mut c, &f.t, r.translate(vec2(0.0, dy * s)), &[&trailers[i].thumbnail], "", &hit, s, sheen);
            c.painter().circle_filled(shown.center(), 38.0 * s, Color32::from_black_alpha(160));
            w::icon_centered(c.painter(), shown.center(), icons::PLAY, 34.0 * s, Color32::WHITE);
            paint::line(c.painter(), shown.left_bottom() + vec2(16.0 * s, -16.0 * s), Align2::LEFT_BOTTOM, &trailers[i].name, w::font_semibold(19.0 * s), Color32::WHITE, shown.width() - 32.0 * s);
            if hit.focused {
                bp.focus.ring_around(shown, 10.0 * s);
            }
            if hit.activated {
                bp.open(Page::Video(id, i), f.now);
            }
        }
        y += 60.0 * s;
    }

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

    // Reviews: the share of thumbs up, then the most helpful ones.
    let (recent, all) = with_store(f.app, f.actions, |st, _| st.ratings_for(id)).unwrap_or((None, None));
    let reviews = with_store(f.app, f.actions, |st, _| st.top_reviews_for(id)).flatten().unwrap_or_default();
    if all.is_some() || !reviews.is_empty() {
        heading(ui, f, &mut y, t!("Reviews"));
        let p = ui.painter().clone();
        let mut x = left;
        let bars = [(t!("PAST MONTH"), recent.map(|(u, d)| ((u * 100 / (u + d)) as u32, u + d))), (t!("ALL TIME"), all)];
        for (label, value) in bars {
            let Some((pct, count)) = value else { continue };
            p.text(pos2(x, y), Align2::LEFT_TOP, label, w::font_bold(15.0 * s), f.t.text_faint);
            p.text(pos2(x, y + 24.0 * s), Align2::LEFT_TOP, t!("{pct}% positive", pct = pct), w::font_semibold(28.0 * s), if pct >= 70 { Color32::from_rgb(0x66, 0xC0, 0xF4) } else if pct >= 40 { f.t.away } else { f.t.danger });
            let bar = Rect::from_min_size(pos2(x, y + 66.0 * s), vec2(300.0 * s, 8.0 * s));
            paint::bar(ui, &f.t, bar, pct as f32 / 100.0, false);
            p.text(pos2(x, y + 82.0 * s), Align2::LEFT_TOP, t!("{count} reviews", count = crate::builtin::store::thousands(count)), w::font(17.0 * s), f.t.text_dim);
            x += 380.0 * s;
        }
        y += 130.0 * s;
        if !reviews.is_empty() {
            let list: Vec<_> = reviews.into_iter().take(10).collect();
            let tiles = shelf(bp, ui, f, &mut y, "reviews", list.len(), vec2(560.0 * s, 230.0 * s), |i| Id::unique(("bp-review", i)));
            let mut focused = None;
            for (i, r, hit) in tiles {
                let rv = &list[i];
                let shown = paint::grow(r, 1.0 + 0.03 * hit.glow);
                paint::shadow(ui.painter(), shown, 12.0 * s, s, 0.4 + 0.6 * hit.glow, Color32::from_black_alpha(200));
                ui.painter().rect_filled(shown, paint::radius(12.0 * s), f.t.surface);
                let (icon, color, verdict) = if rv.voted_up { (icons::THUMBS_UP, Color32::from_rgb(0x66, 0xC0, 0xF4), t!("Recommended|review")) } else { (icons::THUMBS_DOWN, f.t.danger, t!("Not recommended|review")) };
                let pad = 20.0 * s;
                let q = ui.painter();
                q.text(shown.left_top() + vec2(pad, pad), Align2::LEFT_TOP, format!("{icon}  {verdict}"), w::font_bold(20.0 * s), color);
                paint::line(q, shown.left_top() + vec2(pad, pad + 32.0 * s), Align2::LEFT_TOP, &format!("{}   ·   {}", rv.author, rv.hours), w::font(16.0 * s), f.t.text_faint, shown.width() - 2.0 * pad);
                paint::wrapped(q, shown.left_top() + vec2(pad, pad + 66.0 * s), &rv.text.replace('\n', " "), w::font(18.0 * s), f.t.text_dim, shown.width() - 2.0 * pad, 5);
                if hit.focused {
                    bp.focus.ring_around(shown, 12.0 * s);
                    focused = Some(rv.clone());
                }
            }
            y += 22.0 * s;
            // The focused review in full, under the row.
            if let Some(rv) = focused {
                let r = paint::wrapped(&p, pos2(left, y), &rv.text, w::font(20.0 * s), f.t.text, f.view.width() - 2.0 * f.pad, 14);
                y = r.bottom() + 20.0 * s;
            }
            y += 40.0 * s;
        }
    }
    y - top + 20.0 * s
}

/// A trailer over the whole page.
fn video_page(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, id: AppId, i: usize) -> f32 {
    let s = f.s;
    bp.bg_want = Some(id);
    bp.video_on = true;
    let area = f.view.shrink2(vec2(f.pad, 10.0 * s));
    let _ = bp.focus.item(ui, Id::unique("bp-video"), area, ui.clip_rect(), Place::Page);
    bp.focus.hide_ring();
    // 16:9 inside the area.
    let w_ = area.width().min(area.height() * 16.0 / 9.0);
    let rect = Rect::from_center_size(area.center(), vec2(w_, w_ * 9.0 / 16.0));
    let t = f.t.clone();
    let ended = with_store(f.app, f.actions, |st, _| st.play_trailer(ui, &t, rect, id, i)).unwrap_or(false);
    if ended {
        bp.back(f.now);
    }
    f.view.height()
}

/// Before an install: the size, where it goes (with free space) and a
/// desktop shortcut, like Steam's install dialog.
fn install_page(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32, id: AppId) -> f32 {
    let s = f.s;
    bp.bg_want = Some(id);
    let Some(plan) = with_library(f.app, f.actions, |lib, cx| lib.install_plan(cx, id)) else { return 0.0 };
    if bp.install.as_ref().is_none_or(|(g, _, _)| *g != id) {
        bp.install = Some((id, plan.default.clone(), true));
    }
    if f.x {
        if let Some(choice) = bp.install.as_mut() {
            choice.2 = !choice.2;
        }
    }
    let (_, chosen, shortcut) = bp.install.clone().expect("just set");
    let name = f.app.state.app_name(id).unwrap_or_default().to_string();
    let left = f.view.left() + f.pad;
    let mut y = top + 20.0 * s;
    let header = Rect::from_min_size(pos2(left, y), vec2(560.0 * s, 262.0 * s));
    paint::shadow(ui.painter(), header, 14.0 * s, s, 0.8, Color32::from_black_alpha(220));
    if !w::paint_cover(ui, header, &art::header(id), paint::radius(14.0 * s)) {
        w::skeleton(ui, &f.t, header, paint::radius(14.0 * s));
    }
    let p = ui.painter().clone();
    let tx = header.right() + 48.0 * s;
    let tw = f.view.right() - f.pad - tx;
    paint::wrapped(&p, pos2(tx, y), &t!("Install {game}", game = name), w::font_bold(40.0 * s), f.t.text, tw, 2);
    let free = plan.libraries.iter().find(|(l, _)| *l == chosen).map(|(_, f)| *f).unwrap_or(0);
    let enough = plan.size == 0 || free == 0 || free >= plan.size;
    let facts = [
        (t!("SIZE ON DISK"), if plan.size > 0 { ui::bytes(plan.size) } else { t!("Unknown").to_string() }),
        (t!("FREE SPACE THERE"), if free > 0 { ui::bytes(free) } else { "—".to_string() }),
    ];
    let mut fx = tx;
    for (label, value) in facts {
        p.text(pos2(fx, y + 120.0 * s), Align2::LEFT_TOP, label, w::font_bold(15.0 * s), f.t.text_faint);
        let r = p.text(pos2(fx, y + 146.0 * s), Align2::LEFT_TOP, value, w::font_semibold(28.0 * s), f.t.text);
        fx = r.right().max(fx + 200.0 * s) + 60.0 * s;
    }
    if !enough {
        p.text(pos2(tx, y + 200.0 * s), Align2::LEFT_TOP, t!("There isn't enough free space on this drive."), w::font_semibold(19.0 * s), f.t.danger);
    }
    y = header.bottom() + 50.0 * s;

    heading(ui, f, &mut y, t!("Install location"));
    for (k, (lib, free)) in plan.libraries.iter().enumerate() {
        let label = format!("{}   ·   {}", lib.display(), t!("{free} free", free = ui::bytes(*free)));
        let on = *lib == chosen;
        let (_, hit) = button(bp, ui, f, Id::unique(("bp-install-lib", k)), pos2(left, y), 900.0 * s, Some(if on { icons::CHECK } else { icons::DRIVES }), &label, if on { Kind::Primary } else { Kind::Secondary }, None);
        if hit.activated {
            if let Some(choice) = bp.install.as_mut() {
                choice.1 = lib.clone();
            }
        }
        y += 72.0 * s + 14.0 * s;
    }
    y += 20.0 * s;
    let label = if shortcut { t!("Desktop shortcut: on") } else { t!("Desktop shortcut: off") };
    let (_, hit) = button(bp, ui, f, Id::unique("bp-install-shortcut"), pos2(left, y), 0.0, Some(icons::DESKTOP), label, Kind::Secondary, None);
    if hit.activated {
        if let Some(choice) = bp.install.as_mut() {
            choice.2 = !choice.2;
        }
    }
    y += 72.0 * s + 40.0 * s;
    let go_id = Id::unique(("bp-install-go", id));
    bp.preferred = Some(go_id);
    let (r, go) = button(bp, ui, f, go_id, pos2(left, y), 280.0 * s, Some(icons::DOWNLOAD), t!("Install"), if enough { Kind::Play } else { Kind::Muted }, None);
    if go.activated && enough {
        with_library(f.app, f.actions, |lib, cx| lib.install_to(cx, id, chosen.clone(), shortcut));
        bp.install = None;
        bp.back(f.now);
    }
    let (_, cancel) = button(bp, ui, f, Id::unique(("bp-install-cancel", id)), pos2(r.right() + 20.0 * s, y), 0.0, Some(icons::X), t!("Cancel"), Kind::Secondary, None);
    if cancel.activated {
        bp.install = None;
        bp.back(f.now);
    }
    y += 72.0 * s;
    y - top + 40.0 * s
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
    // The whole page is the viewer: a focus box around it only gets in the way.
    bp.focus.hide_ring();
    let n = list.len();
    // The mouse flips too: the wheel (either way), the arrows, or a click
    // on the picture (like A) for the next one.
    let mut step = 0i32;
    if hit.activated {
        step = 1;
    }
    if ui.rect_contains_pointer(area) {
        let d = ui.input(|i| i.smooth_scroll_delta);
        bp.shot_wheel += d.y + d.x;
    }
    if f.now - bp.shot_since.0 < 0.3 {
        bp.shot_wheel = 0.0;
    } else if bp.shot_wheel.abs() > 30.0 {
        step = if bp.shot_wheel < 0.0 { 1 } else { -1 };
        bp.shot_wheel = 0.0;
    }
    for (way, icon) in [(-1i32, icons::CARET_LEFT), (1, icons::CARET_RIGHT)] {
        let c = pos2(area.center().x + way as f32 * (area.width() / 2.0 - 30.0 * s), area.center().y);
        let r = Rect::from_center_size(c, vec2(60.0 * s, 60.0 * s));
        let resp = ui.interact(r, Id::unique(("bp-shot-arrow", way)), egui::Sense::click());
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
        ui.painter().circle_filled(c, 30.0 * s * (1.0 + 0.12 * hover), Color32::from_black_alpha((140.0 + 80.0 * hover) as u8));
        ui.painter().text(c, Align2::CENTER_CENTER, icon, w::font(30.0 * s), f.t.text);
        if resp.clicked() {
            step = way;
        }
    }
    if step != 0 && n > 0 {
        let next = (i as i32 + step).rem_euclid(n as i32) as usize;
        if let Some(Page::Shots(_, at)) = bp.stack.last_mut() {
            *at = next;
        }
        bp.shot_since = (f.now, step as f32);
        bp.shot_wheel = 0.0;
    }
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
        // Long text: the end stays in view (the start slides out to the left).
        let room = field.right() - tx - 40.0 * s;
        let galley = p.layout_no_wrap(bp.query.clone(), w::font_semibold(32.0 * s), f.t.text);
        let x = tx - (galley.size().x - room).max(0.0);
        let rect = Align2::LEFT_CENTER.anchor_size(pos2(x, field.center().y), galley.size());
        p.with_clip_rect(Rect::from_min_max(pos2(tx, field.top()), pos2(tx + room + 8.0 * s, field.bottom()))).galley(rect.min, galley, f.t.text);
        rect
    };
    // A blinking caret.
    let blink = ((f.now * 3.0).sin() as f32 * 0.5 + 0.5).powf(0.6);
    p.vline(typed.right() + 4.0 * s, (field.center().y - 20.0 * s)..=(field.center().y + 20.0 * s), egui::Stroke::new(3.0 * s, f.t.accent.gamma_multiply(blink)));
    y += 84.0 * s + 36.0 * s;

    // The keyboard.
    match keyboard(bp, ui, f, f.view.left() + f.pad, &mut y, t!("Search")) {
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
fn keyboard(bp: &mut BigPicture, ui: &mut Ui, f: &Frame, left: f32, y: &mut f32, go: &str) -> Option<Key> {
    let s = f.s;
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
                    f.app.client.send(Command::Launch { app_id: g, install_dir: installed.install_dir.clone(), args: client::LaunchArgs::connect(args) });
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

/// The chat's message history (scrolls with up/down while focused).
pub fn chat_history_id() -> Id {
    Id::unique("bp-chat-history")
}

/// The chat's message box (selecting it brings up the keyboard).
fn chat_input_id() -> Id {
    Id::unique("bp-chat-input")
}

/// A key of the on-screen keyboard (see `keyboard`).
fn is_key(id: Option<Id>) -> bool {
    let Some(id) = id else { return false };
    (0..4).any(|r| (0..11).any(|c| id == Id::unique(("bp-key", r, c)))) || (0..4).any(|i| id == Id::unique(("bp-key-special", i)))
}

/// A conversation over the whole page: the friend at the top, the history
/// filling the middle (it scrolls), the message box at the bottom. The
/// on-screen keyboard slides up while the message box is selected, like
/// Steam's; a real keyboard types any time (Enter sends).
fn chat_page(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, _top: f32, id: model::SteamId) -> f32 {
    let s = f.s;
    let left = f.view.left() + f.pad;
    let right = f.view.right() - f.pad;
    if bp.chats_loaded.insert(id) {
        f.app.client.send(Command::LoadHistory { friend: id });
    }
    if f.app.state.unread.get(&id).is_some_and(|n| *n > 0) {
        f.actions.push(Action::MarkRead(id));
    }
    if bp.chat_for != Some(id) {
        bp.chat_for = Some(id);
        bp.chat_follow = true;
    }
    let fr = f.app.state.persona(id);
    let p = ui.painter().clone();

    // Who you're talking to.
    let head_y = f.view.top() + 6.0 * s;
    let av = Rect::from_min_size(pos2(left, head_y), vec2(64.0 * s, 64.0 * s));
    if !w::paint_cover(ui, av, &art::avatar_full(fr.avatar_hash.as_deref()), paint::radius(32.0 * s)) {
        p.circle_filled(av.center(), 32.0 * s, f.t.raised);
    }
    let color = crate::shell::status_color(&f.t, &fr);
    p.circle_stroke(av.center(), 34.0 * s, egui::Stroke::new(3.0 * s, color));
    let tx = av.right() + 22.0 * s;
    paint::line(&p, pos2(tx, av.center().y - 2.0 * s), Align2::LEFT_BOTTOM, &fr.display_name(), w::font_bold(32.0 * s), f.t.text, right - tx);
    let status = if f.app.state.is_typing(id) { t!("typing…").to_string() } else { crate::shell::status_text(&f.app.state, &fr) };
    paint::line(&p, pos2(tx, av.center().y + 4.0 * s), Align2::LEFT_TOP, &status, w::font_medium(20.0 * s), color, right - tx);
    let head_bottom = av.bottom() + 18.0 * s;

    // The keyboard slides up while the message box or one of its keys is focused.
    let input_id = chat_input_id();
    let want_keys = bp.focus.current == Some(input_id) || is_key(bp.focus.current);
    bp.chat_keys = want_keys;
    let k = super::focus::ease(ui.ctx().animate_bool_with_time(Id::unique(("bp-chat-keys", id)), want_keys, 0.25));
    let key = 74.0 * s;
    let unit = key + 10.0 * s;
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
        match keyboard(bp, ui, f, f.view.center().x - keys_w / 2.0, &mut ky, t!("Send")) {
            Some(Key::Char(ch)) => bp.message.push(ch),
            Some(Key::Del) => {
                bp.message.pop();
            }
            Some(Key::Go) => send = true,
            None => {}
        }
    }

    // The message box: grows to four lines, then shows the end of the text.
    if f.x {
        bp.message.pop();
    }
    if f.y {
        bp.message.push(' ');
    }
    let send_label = t!("Send");
    let send_w = paint::pill_width(ui, true, send_label, s);
    let field_w = right - left - send_w - 18.0 * s;
    let font = w::font_semibold(26.0 * s);
    let text_w = field_w - 64.0 * s;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(egui::text::LayoutJob::simple(bp.message.clone(), font.clone(), f.t.text, text_w)));
    let line_h = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let lines = (galley.size().y / line_h).round().clamp(1.0, 4.0);
    let field_h = (lines * line_h + 34.0 * s).max(72.0 * s);
    let bottom = panel_top - 20.0 * s;
    let field = Rect::from_min_max(pos2(left, bottom - field_h), pos2(left + field_w, bottom));
    let hit = bp.focus.item(ui, input_id, field, ui.clip_rect(), Place::Page);
    let r = (36.0 * s).min(field.height() / 2.0);
    p.rect_filled(field, paint::radius(r), Color32::from_black_alpha(130));
    let stroke = w::mix(f.t.accent.gamma_multiply(0.45), f.t.accent, hit.glow);
    p.rect_stroke(field, paint::radius(r), egui::Stroke::new((2.0 + hit.glow) * s, stroke), egui::StrokeKind::Inside);
    if hit.focused {
        bp.focus.ring_around(field, r);
    }
    let inner = field.shrink2(vec2(32.0 * s, 17.0 * s));
    let caret = if bp.message.is_empty() {
        p.text(pos2(inner.left(), inner.bottom() - line_h / 2.0), Align2::LEFT_CENTER, t!("Message {name}", name = fr.display_name()), w::font(26.0 * s), f.t.text_faint);
        Rect::from_min_size(pos2(inner.left(), inner.bottom() - line_h), vec2(0.0, line_h))
    } else {
        // The last lines stay in view (the start scrolls up out of the box).
        let at = pos2(inner.left(), inner.bottom() - galley.size().y);
        let end = galley.pos_from_cursor(egui::text::CCursor::new(bp.message.chars().count()));
        p.with_clip_rect(inner.expand2(vec2(4.0 * s, 0.0))).galley(at, galley, f.t.text);
        end.translate(at.to_vec2())
    };
    let blink = ((f.now * 3.0).sin() as f32 * 0.5 + 0.5).powf(0.6);
    p.vline(caret.right() + 3.0 * s, (caret.top() + 3.0 * s)..=(caret.bottom() - 3.0 * s), egui::Stroke::new(3.0 * s, f.t.accent.gamma_multiply(blink)));
    // A on the box: straight to the letters.
    if hit.activated {
        bp.focus.current = Some(Id::unique(("bp-key", 1, 0)));
    }
    let empty_box = bp.message.trim().is_empty();
    let (_, send_hit) = button(bp, ui, f, Id::unique(("bp-chat-send", id)), pos2(field.right() + 18.0 * s, bottom - 72.0 * s), send_w, Some(icons::SEND), send_label, if empty_box { Kind::Secondary } else { Kind::Primary }, None);
    if send_hit.activated {
        send = true;
    }
    bp.preferred = Some(input_id);
    let text = bp.message.trim().to_string();
    if send && !text.is_empty() {
        f.app.client.send(Command::SendMessage { to: id, text });
        bp.message.clear();
        bp.chat_follow = true;
    }

    // The history fills the rest and scrolls; newest at the bottom.
    let area = Rect::from_min_max(pos2(left, head_bottom), pos2(right, field.top() - 18.0 * s));
    if area.height() < 40.0 * s {
        return f.view.height();
    }
    let me = f.app.state.steam_id;
    let messages: Vec<model::ChatMessage> = f.app.state.chats.get(&id).cloned().unwrap_or_default();
    let hist = bp.focus.item(ui, chat_history_id(), area, ui.clip_rect(), Place::Page);
    p.rect_filled(area, paint::radius(18.0 * s), Color32::from_black_alpha(90));
    if hist.focused {
        bp.focus.ring_around(area, 18.0 * s);
    }
    let pad = 22.0 * s;
    let wrap = (area.width() * 0.68).max(200.0 * s);
    let body = w::font(22.0 * s);
    let mut laid = Vec::with_capacity(messages.len());
    let mut total = pad;
    for m in &messages {
        let job = egui::text::LayoutJob::simple(m.text.clone(), body.clone(), f.t.text, wrap);
        let g = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let size = g.size() + vec2(36.0 * s, 24.0 * s);
        laid.push((total, size, m.from == me, g, m.time));
        total += size.y + 30.0 * s;
    }
    total += pad - 8.0 * s;
    let view_h = area.height();
    let max = (total - view_h).max(0.0);
    let key = format!("chat-{id}");
    // The mouse wheel; up/down while the history is focused (see mod.rs).
    let wheel = if ui.rect_contains_pointer(area) { ui.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
    let nudge = std::mem::take(&mut bp.chat_nudge);
    let mut follow = bp.chat_follow;
    let sc = bp.scroller(&key);
    if wheel != 0.0 {
        sc.target -= wheel;
        follow = false;
    }
    if nudge != 0.0 {
        sc.target += nudge * 180.0 * s;
        follow = false;
    }
    if follow {
        sc.target = max;
    }
    sc.clamp(total, view_h);
    // Back at the bottom: new messages keep it there again.
    if sc.target >= max - 1.0 {
        follow = true;
    }
    sc.step(ui);
    let pos = sc.pos;
    let edges = (sc.target <= 0.5, sc.target >= max - 0.5);
    bp.chat_follow = follow;
    bp.chat_edges = edges;
    let clip = p.with_clip_rect(area.shrink(2.0));
    if messages.is_empty() {
        p.text(area.center(), Align2::CENTER_CENTER, t!("No messages yet"), w::font(22.0 * s), f.t.text_faint);
    }
    for (at, size, mine, g, time) in laid {
        let y = area.top() + at - pos;
        if y > area.bottom() || y + size.y + 30.0 * s < area.top() {
            continue;
        }
        let bx = if mine { area.right() - pad - size.x } else { area.left() + pad };
        let bubble = Rect::from_min_size(pos2(bx, y), size);
        clip.rect_filled(bubble, paint::radius(16.0 * s), if mine { f.t.accent.gamma_multiply(0.55) } else { f.t.raised });
        clip.galley(bubble.min + vec2(18.0 * s, 12.0 * s), g, f.t.text);
        let (anchor, align) = if mine { (bubble.right_bottom() + vec2(-4.0 * s, 4.0 * s), Align2::RIGHT_TOP) } else { (bubble.left_bottom() + vec2(4.0 * s, 4.0 * s), Align2::LEFT_TOP) };
        clip.text(anchor, align, ui::ago(time), w::font(15.0 * s), f.t.text_faint);
    }
    // Where you are in a long conversation.
    if max > 0.0 {
        let track = Rect::from_min_max(pos2(area.right() - 10.0 * s, area.top() + 14.0 * s), pos2(area.right() - 5.0 * s, area.bottom() - 14.0 * s));
        let h = (track.height() * view_h / total).max(30.0 * s);
        let y = track.top() + (track.height() - h) * (pos / max).clamp(0.0, 1.0);
        p.rect_filled(Rect::from_min_size(pos2(track.left(), y), vec2(track.width(), h)), paint::radius(3.0 * s), Color32::from_white_alpha(70));
    }
    f.view.height()
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

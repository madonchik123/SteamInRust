//! Shift+Tab overlay, laid out like Steam's: the game is dimmed, a bar at the
//! top shows the game, session time, the window buttons and the clock, and
//! Game Overview, Achievements, Friends, Chat and Notes open as windows you
//! can move, resize and pin. A pinned window stays over the game after the
//! overlay closes, at the opacity you pick, and lets clicks through to it.
//!
//! It is a separate always-on-top window, *not* injected into the game:
//! injection is how cheats work and anti-cheat (VAC, EAC, BattlEye) treats it
//! that way. So it shows over windowed and borderless-fullscreen games, not
//! exclusive fullscreen.

use i18n::t;
use crate::app::SteamApp;
use client::Command;
use egui::{Align, Align2, Color32, CornerRadius, Key, Layout, Margin, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use extensions::Action;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use model::{AppId, Persona, PersonaState, SteamId, art};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use ui::{Theme, icons, widgets as w};

/// The overlay's windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Win {
    Overview,
    Achievements,
    Friends,
    Chat,
    Notes,
    /// Community pages: hub, guides, discussions, Workshop items.
    Community,
    /// The game's store page.
    Store,
}

impl Win {
    const ALL: [Win; 7] = [Win::Overview, Win::Achievements, Win::Friends, Win::Chat, Win::Notes, Win::Community, Win::Store];
}

#[derive(Clone, Copy)]
struct WinState {
    open: bool,
    pinned: bool,
    /// How see-through the window is while pinned over the game.
    opacity: f32,
}

pub struct Overlay {
    pub open: bool,
    manager: Option<GlobalHotKeyManager>,
    hotkey: HotKey,
    registered: bool,
    pressed: Arc<AtomicBool>,
    game: Option<(AppId, Instant)>,
    chat_with: Option<SteamId>,
    input: String,
    notes: String,
    search: String,
    show_offline: bool,
    wins: [WinState; 7],
    /// Focus was given to the overlay since it opened.
    focused: bool,
    /// The window that had focus when the overlay opened (the game), so it
    /// gets focus back when the overlay closes.
    game_window: isize,
    /// Clicks currently pass through the overlay window.
    passthrough: Option<bool>,
    /// Data asked for this session.
    asked: bool,
    /// What the Community window shows (with its own back history).
    community: crate::builtin::hub::HubView,
    /// The Achievements window (compare with a friend inside it).
    achievements: crate::builtin::hub::HubView,
    /// Game whose store page the Store window shows, and whether the store
    /// page was pointed at it yet.
    store_app: Option<(AppId, bool)>,
    /// `--overlay-open=guides|discussions|hub|store|achievements`: a window to
    /// open for previews.
    preview: Option<String>,
}

impl Overlay {
    pub fn new(ctx: &egui::Context) -> Self {
        let pressed = Arc::new(AtomicBool::new(false));
        let manager = GlobalHotKeyManager::new().map_err(|e| tracing::warn!("global hotkeys unavailable: {e}")).ok();
        let (flag, repaint) = (pressed.clone(), ctx.clone());
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                flag.store(true, Ordering::Relaxed);
                repaint.request_repaint();
            }
        }));
        let closed = WinState { open: false, pinned: false, opacity: 0.85 };
        let mut wins = [closed; 7];
        // Like Steam: the game overview and friends open by default.
        wins[Win::Overview as usize].open = true;
        wins[Win::Friends as usize].open = true;
        Self {
            open: false,
            manager,
            hotkey: HotKey::new(Some(Modifiers::SHIFT), Code::Tab),
            registered: false,
            pressed,
            game: None,
            chat_with: None,
            input: String::new(),
            notes: String::new(),
            search: String::new(),
            show_offline: false,
            wins,
            focused: false,
            game_window: 0,
            passthrough: None,
            asked: false,
            community: Default::default(),
            achievements: Default::default(),
            store_app: None,
            preview: std::env::args().find_map(|a| a.strip_prefix("--overlay-open=").map(String::from)),
        }
    }

    /// Shift+Tab is only grabbed while a game we launched is running, so it
    /// keeps working normally in every other app.
    pub fn game_started(&mut self, app_id: AppId, enabled: bool) {
        self.game = Some((app_id, Instant::now()));
        self.asked = false;
        if enabled && !self.registered {
            if let Some(m) = &self.manager {
                self.registered = m.register(self.hotkey).map_err(|e| tracing::warn!("Shift+Tab: {e}")).is_ok();
            }
        }
    }

    pub fn game_stopped(&mut self) {
        self.game = None;
        self.open = false;
        for w in &mut self.wins {
            w.pinned = false;
        }
        if self.registered {
            if let Some(m) = &self.manager {
                let _ = m.unregister(self.hotkey);
            }
            self.registered = false;
        }
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open = true;
            self.focused = false;
            self.game_window = foreground_window();
        }
    }

    fn close(&mut self) {
        self.open = false;
        // Give the keyboard back to the game.
        give_focus(self.game_window);
    }

    fn win(&self, w: Win) -> WinState {
        self.wins[w as usize]
    }

    fn win_mut(&mut self, w: Win) -> &mut WinState {
        &mut self.wins[w as usize]
    }

    fn any_pinned(&self) -> bool {
        Win::ALL.iter().any(|w| {
            let s = self.win(*w);
            s.open && s.pinned && (*w != Win::Chat || self.chat_with.is_some())
        })
    }
}

#[cfg(windows)]
fn foreground_window() -> isize {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as isize }
}
#[cfg(not(windows))]
fn foreground_window() -> isize {
    0
}

#[cfg(windows)]
fn give_focus(window: isize) {
    if window != 0 {
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(window as _) };
    }
}
#[cfg(not(windows))]
fn give_focus(_: isize) {}

pub fn show(app: &mut SteamApp, ctx: &egui::Context, actions: &mut Vec<Action>) {
    if app.overlay.pressed.swap(false, Ordering::Relaxed) {
        app.overlay.toggle();
    }
    let active = app.overlay.open;
    if !active && !app.overlay.any_pinned() {
        app.overlay.passthrough = None;
        return;
    }
    // What the overview and achievements need, once per game session.
    if active && !app.overlay.asked {
        app.overlay.asked = true;
        if let Some((id, _)) = app.overlay.game {
            if !app.state.achievement_list.contains_key(&id) {
                app.client.send(Command::LoadAchievements { app_id: id, language: i18n::language().steam_name().into() });
            }
            let me = app.state.steam_id;
            if me != 0 && !app.state.player_games.contains_key(&me) {
                app.client.send(Command::LoadPlayerGames { steam_id: me, language: i18n::language().steam_name().into() });
            }
        }
    }
    if let (Some(what), Some((id, _))) = (app.overlay.preview.take(), app.overlay.game) {
        use crate::builtin::hub::{HubTab, View};
        match what.as_str() {
            "store" => open_store(app, id),
            "achievements" => app.overlay.win_mut(Win::Achievements).open = true,
            "guides" => open_community(app, View::Guides(id), true),
            "discussions" => open_community(app, View::Forum { app: id, forum: String::new() }, true),
            _ => open_community(app, View::Hub { app: id, tab: HubTab::All }, true),
        }
    }
    let t = app.prefs.theme.clone();
    let viewport = egui::ViewportId::from_hash_of("steam-rs-overlay");
    // Not true fullscreen: Windows turns a window that exactly covers the
    // screen into exclusive fullscreen, which drops transparency (the game
    // went black). One pixel short keeps it a normal see-through window.
    let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1920.0, 1080.0));
    let builder = egui::ViewportBuilder::default()
        .with_title("SteamInRust Overlay")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_position(egui::pos2(0.0, 0.0))
        .with_inner_size(monitor - egui::vec2(0.0, 1.0))
        .with_mouse_passthrough(!active)
        .with_taskbar(false);
    let opened_now = active && !app.overlay.focused;
    ctx.show_viewport_immediate(viewport, builder, |ui, _class| {
        // Pinned windows over the game let clicks through; the open overlay takes them.
        if app.overlay.passthrough != Some(!active) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::MousePassthrough(!active));
            app.overlay.passthrough = Some(!active);
        }
        // Take keyboard and mouse focus so input goes to the overlay, not the game.
        if opened_now {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            app.overlay.focused = true;
        }
        let close = active && ui.input(|i| i.key_pressed(Key::Escape) || (i.modifiers.shift && i.key_pressed(Key::Tab)) || i.viewport().close_requested());
        // Dim the game, like Steam does (nothing but pinned windows when closed).
        let fill = if active { Color32::from_black_alpha(205) } else { Color32::TRANSPARENT };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(fill)).show(ui, |ui| {
            if active {
                top_strip(app, ui, &t);
            }
        });
        let ctx = ui.ctx().clone();
        overview_window(app, &ctx, &t);
        achievements_window(app, &ctx, &t);
        friends_window(app, &ctx, &t, actions);
        chat_window(app, &ctx, &t, actions);
        notes_window(app, &ctx, &t);
        community_window(app, &ctx, &t, actions);
        store_window(app, &ctx, &t, actions);
        if close {
            app.overlay.close();
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    });
}

/// Local time as HH:MM.
fn clock() -> String {
    #[cfg(windows)]
    {
        let mut st = unsafe { std::mem::zeroed::<windows_sys::Win32::Foundation::SYSTEMTIME>() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
        format!("{:02}:{:02}", st.wHour, st.wMinute)
    }
    #[cfg(not(windows))]
    {
        let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) % 86_400;
        format!("{:02}:{:02}", s / 3600, (s / 60) % 60)
    }
}

const STRIP: f32 = 76.0;

fn top_strip(app: &mut SteamApp, ui: &mut Ui, t: &Theme) {
    let full = ui.max_rect();
    let strip = Rect::from_min_size(full.min, vec2(full.width(), STRIP));
    ui.painter().rect_filled(strip, CornerRadius::ZERO, t.chrome);
    ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, t.border));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(24.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
    // The game: its art opens the overview.
    match app.overlay.game {
        Some((id, since)) => {
            let (r, resp) = c.allocate_exact_size(vec2(118.0, 55.0), Sense::click());
            let name = app.state.app_name(id).unwrap_or("").to_string();
            w::image_or_title(&c, t, r, &art::header(id), &name, CornerRadius::same(4));
            if resp.on_hover_text(t!("Game overview")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                let s = app.overlay.win_mut(Win::Overview);
                s.open = !s.open;
            }
            c.add_space(10.0);
            c.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(w::text_bold(&name, 18.0, t.text));
                ui.label(w::text(format!("{}  {}", icons::CLOCK, session_text(since)), 12.5, t.text_dim));
            });
        }
        None => {
            w::logo(&mut c, t, 26.0);
            c.label(w::text_bold(t!("Overlay preview"), 18.0, t.text));
        }
    }
    // Window buttons in the middle, like Steam's toolbar.
    let game = app.overlay.game.map(|g| g.0);
    let mid = Rect::from_center_size(strip.center(), vec2(640.0, STRIP));
    let mut m = ui.new_child(egui::UiBuilder::new().max_rect(mid).layout(Layout::left_to_right(Align::Center)));
    m.spacing_mut().item_spacing.x = 6.0;
    let unread = app.state.total_unread();
    let mut toggle = |m: &mut Ui, win: Win, icon: &str, label: &str, badge: usize| {
        let on = app.overlay.win(win).open;
        if strip_button(m, t, icon, label, on, badge).clicked() {
            let s = app.overlay.win_mut(win);
            s.open = !s.open;
        }
    };
    if game.is_some() {
        toggle(&mut m, Win::Overview, icons::CONTROLLER, t!("Overview"), 0);
        toggle(&mut m, Win::Achievements, icons::TROPHY, t!("Achievements"), 0);
    }
    toggle(&mut m, Win::Friends, icons::USERS, t!("Friends"), unread);
    toggle(&mut m, Win::Notes, icons::NOTE, t!("Notes"), 0);
    if let Some(id) = game {
        use crate::builtin::hub::View;
        let now = if app.overlay.win(Win::Community).open { app.overlay.community.current().cloned() } else { None };
        let links = [
            (icons::BOOKS, t!("Guides"), View::Guides(id), matches!(now, Some(View::Guides(_)))),
            (icons::CHATS, t!("Discussions"), View::Forum { app: id, forum: String::new() }, matches!(now, Some(View::Forum { .. } | View::Thread(_)))),
        ];
        for (icon, label, view, on) in links {
            if strip_button(&mut m, t, icon, label, on, 0).clicked() {
                if on {
                    app.overlay.win_mut(Win::Community).open = false;
                } else {
                    open_community(app, view, true);
                }
            }
        }
    }
    // Clock and close on the right.
    let right = Rect::from_min_max(pos2(strip.right() - 420.0, strip.top()), pos2(strip.right() - 24.0, strip.bottom()));
    let mut r = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)));
    if w::icon_button(&mut r, t, icons::X, t!("Close (Shift+Tab)"), false, 0).clicked() {
        app.overlay.close();
    }
    r.add_space(10.0);
    r.vertical(|ui| {
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(14.0);
            ui.label(w::text_bold(clock(), 22.0, t.text));
            let hint = if app.overlay.game.is_some() { t!("Shift+Tab or Esc to return to the game") } else { t!("Esc to close") };
            ui.label(w::text(hint, 11.5, t.text_faint));
        });
    });
}

fn session_text(since: Instant) -> String {
    let mins = since.elapsed().as_secs() / 60;
    if mins < 60 { t!("{n} min this session", n = mins) } else { t!("{h} h {m} min this session", h = mins / 60, m = mins % 60) }
}

fn hours(minutes: u64) -> String {
    if minutes < 60 { t!("{n} min", n = minutes) } else { t!("{h} h", h = format!("{:.1}", minutes as f32 / 60.0)) }
}

fn strip_button(ui: &mut Ui, t: &Theme, icon: &str, label: &str, on: bool, badge: usize) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), w::font_medium(11.5), t.text);
    let width = galley.size().x.max(44.0) + 20.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 58.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let p = ui.painter();
    if on || hover > 0.0 {
        p.rect_filled(rect, CornerRadius::same(6), if on { t.accent.gamma_multiply(0.18) } else { t.surface_hover.gamma_multiply(hover) });
    }
    let color = if on { t.accent } else { w::mix(t.text_dim, t.text, hover) };
    p.text(pos2(rect.center().x, rect.top() + 21.0), Align2::CENTER_CENTER, icon, w::font(20.0), color);
    p.galley(pos2(rect.center().x - galley.size().x / 2.0, rect.top() + 36.0), galley, color);
    if badge > 0 {
        let c = pos2(rect.center().x + 13.0, rect.top() + 11.0);
        p.circle_filled(c, 8.0, t.danger);
        p.text(c, Align2::CENTER_CENTER, badge.min(99).to_string(), w::font_bold(10.0), Color32::WHITE);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Solid window look shared by the overlay's windows.
fn window_frame(t: &Theme, opacity: f32) -> egui::Frame {
    egui::Frame::new()
        .fill(t.chrome.gamma_multiply(opacity))
        .stroke(Stroke::new(1.0, t.border.gamma_multiply(opacity)))
        .corner_radius(t.corner())
        .shadow(if opacity < 1.0 { egui::Shadow::NONE } else { egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(160) } })
        .inner_margin(Margin::same(14))
}

/// One overlay window, Steam style: a title row with pin and close. While the
/// overlay is closed only pinned windows show, see-through and click-through.
fn ov_window(app: &mut SteamApp, ctx: &egui::Context, t: &Theme, win: Win, title: &str, pos: Pos2, size: Vec2, body: impl FnOnce(&mut SteamApp, &mut Ui)) {
    let s = app.overlay.win(win);
    let active = app.overlay.open;
    if !s.open || (!active && !s.pinned) {
        return;
    }
    let opacity = if active { 1.0 } else { s.opacity };
    let (mut close, mut pin, mut level) = (false, false, s.opacity);
    egui::Window::new(title)
        .id(egui::Id::unique(("ov-window", win as u8)))
        .title_bar(false)
        .frame(window_frame(t, opacity))
        .default_pos(pos)
        .default_size(size)
        .min_size(vec2(260.0, 160.0))
        .resizable(true)
        .movable(active)
        .interactable(active)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.multiply_opacity(opacity.max(0.35));
            if active {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(w::text_bold(title, 15.0, t.text)).truncate().selectable(false));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        close = w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked();
                        let tip = if s.pinned { t!("Unpin: hide this window when the overlay closes") } else { t!("Pin: keep this window over the game") };
                        pin = w::icon_button(ui, t, if s.pinned { icons::PUSH_PIN_SLASH } else { icons::PUSH_PIN }, tip, s.pinned, 0).clicked();
                        if s.pinned {
                            ui.add(egui::Slider::new(&mut level, 0.2..=1.0).show_value(false)).on_hover_text(t!("Opacity over the game"));
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }
            body(app, ui);
        });
    let s = app.overlay.win_mut(win);
    s.opacity = level;
    if pin {
        s.pinned = !s.pinned;
    }
    if close {
        s.open = false;
        s.pinned = false;
        if win == Win::Chat {
            app.overlay.chat_with = None;
        }
    }
}

fn overview_window(app: &mut SteamApp, ctx: &egui::Context, t: &Theme) {
    let Some((id, since)) = app.overlay.game else { return };
    let name = app.state.app_name(id).unwrap_or("").to_string();
    let pos = pos2(40.0, STRIP + 40.0);
    ov_window(app, ctx, t, Win::Overview, &t!("Game overview"), pos, vec2(460.0, 600.0), |app, ui| {
        egui::ScrollArea::vertical().id_salt("ov-overview").auto_shrink([false, false]).show(ui, |ui| {
            let wdt = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(wdt, wdt * 215.0 / 460.0), Sense::hover());
            w::image_or_title(ui, t, r, &art::header(id), &name, t.corner_small());
            ui.add_space(8.0);
            ui.label(w::text_bold(&name, 18.0, t.text));
            ui.add_space(4.0);
            // Playtime.
            let me = app.state.steam_id;
            let total = match app.state.player_games.get(&me) {
                Some(Ok(list)) => list.iter().find(|g| g.app_id == id).map(|g| (g.playtime_minutes, g.playtime_2weeks)),
                _ => None,
            };
            egui::Grid::new("ov-playtime").num_columns(2).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                ui.label(w::text_bold(t!("THIS SESSION"), 10.5, t.text_faint));
                ui.label(w::text(session_text(since), 13.0, t.text));
                ui.end_row();
                if let Some((all, recent)) = total {
                    ui.label(w::text_bold(t!("PLAYED IN TOTAL"), 10.5, t.text_faint));
                    ui.label(w::text(hours(all), 13.0, t.text));
                    ui.end_row();
                    ui.label(w::text_bold(t!("LAST TWO WEEKS"), 10.5, t.text_faint));
                    ui.label(w::text(hours(recent), 13.0, t.text));
                    ui.end_row();
                }
            });
            // Achievements.
            ui.add_space(10.0);
            match app.state.achievements.get(&id).copied() {
                Some((done, all)) if all > 0 => {
                    ui.horizontal(|ui| {
                        ui.label(w::text_bold(t!("ACHIEVEMENTS"), 11.0, t.text_faint));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(w::text(t!("{done} of {all}", done = done, all = all), 12.5, t.text_dim));
                        });
                    });
                    w::progress(ui, t, done as f32 / all as f32, ui.available_width());
                    ui.add_space(6.0);
                    if w::button(ui, t, Some(icons::TROPHY), t!("View achievements"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        app.overlay.win_mut(Win::Achievements).open = true;
                    }
                }
                Some(_) => {
                    ui.label(w::text(t!("This game has no achievements."), 12.5, t.text_faint));
                }
                None => {
                    ui.label(w::text(t!("Loading achievements…"), 12.5, t.text_faint));
                }
            }
            // Links, like the overview's shortcuts in Steam.
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                use crate::builtin::hub::{HubTab, View};
                if w::button(ui, t, Some(icons::STOREFRONT), t!("Store page"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    open_store(app, id);
                }
                for (icon, label, view) in [
                    (icons::USERS, t!("Community hub"), View::Hub { app: id, tab: HubTab::All }),
                    (icons::BOOKS, t!("Guides"), View::Guides(id)),
                    (icons::CHATS, t!("Discussions"), View::Forum { app: id, forum: String::new() }),
                ] {
                    if w::button(ui, t, Some(icon), label, w::Kind::Ghost, w::Size::Sm).clicked() {
                        open_community(app, view, true);
                    }
                }
            });
        });
    });
}

fn achievements_window(app: &mut SteamApp, ctx: &egui::Context, t: &Theme) {
    use crate::builtin::hub::View;
    let Some((id, _)) = app.overlay.game else { return };
    let pos = pos2(520.0, STRIP + 40.0);
    // The full achievements page, with "compare with a friend" inside the window.
    if !matches!(app.overlay.achievements.current(), Some(View::Achievements { app, .. }) if *app == id) {
        app.overlay.achievements.open(View::Achievements { app: id, friend: None }, true);
    }
    let mut toasts = Vec::new();
    ov_window(app, ctx, t, Win::Achievements, &t!("Achievements"), pos, vec2(600.0, 620.0), |app, ui| {
        let (cmds, acts, open) = {
            let mut io = crate::builtin::hub::Io::new(&app.state, &app.local, true);
            app.overlay.achievements.show(ui, t, &mut io);
            (io.cmds, io.actions, io.open)
        };
        for c in cmds {
            app.client.send(c);
        }
        toasts.extend(acts);
        for v in open {
            match v {
                View::Achievements { .. } => app.overlay.achievements.open(v, true),
                other => open_community(app, other, true),
            }
        }
    });
    for a in toasts {
        if let Action::Toast(s) = a {
            let now = ctx.input(|i| i.time);
            app.toast(now, s, "");
        }
    }
}

/// Show a page in the overlay's Community window (or the Store window).
/// `fresh` starts a new back history, for the overlay's own buttons.
fn open_community(app: &mut SteamApp, view: crate::builtin::hub::View, fresh: bool) {
    use crate::builtin::hub::View;
    match view {
        View::Page(page, arg) if page == "store" => {
            if let Some(id) = arg.as_deref().and_then(|a| a.strip_prefix("app:")).and_then(|a| a.parse().ok()) {
                open_store(app, id);
            }
        }
        View::Page(..) => {}
        v => {
            app.overlay.community.open(v, fresh);
            app.overlay.win_mut(Win::Community).open = true;
        }
    }
}

fn open_store(app: &mut SteamApp, id: AppId) {
    app.overlay.store_app = Some((id, false));
    app.overlay.win_mut(Win::Store).open = true;
}

/// Guides, discussions, the community hub and Workshop items, in a window.
fn community_window(app: &mut SteamApp, ctx: &egui::Context, t: &Theme, actions: &mut Vec<Action>) {
    use crate::builtin::hub::View;
    let Some(title) = app.overlay.community.current().map(|v| v.title(&app.state)) else { return };
    let pos = pos2(300.0, STRIP + 30.0);
    let size = vec2(980.0, (ctx.content_rect().height() - STRIP - 80.0).clamp(400.0, 820.0));
    ov_window(app, ctx, t, Win::Community, &title, pos, size, |app, ui| {
        let (cmds, acts, open) = {
            let mut io = crate::builtin::hub::Io::new(&app.state, &app.local, true);
            app.overlay.community.show(ui, t, &mut io);
            (io.cmds, io.actions, io.open)
        };
        for c in cmds {
            app.client.send(c);
        }
        actions.extend(acts);
        for v in open {
            match v {
                // Pages the overlay doesn't have (profiles, the Workshop browser) open in the main window.
                View::Page(page, arg) if page != "store" => actions.push(Action::Navigate { page, arg }),
                v => open_community(app, v, false),
            }
        }
    });
}

/// The game's store page (the app's native store) in a window.
fn store_window(app: &mut SteamApp, ctx: &egui::Context, t: &Theme, actions: &mut Vec<Action>) {
    let Some((id, sent)) = app.overlay.store_app else { return };
    if !app.overlay.win(Win::Store).open {
        return;
    }
    if !sent {
        app.registry.navigate("store", Some(&format!("app:{id}")));
        app.overlay.store_app = Some((id, true));
    }
    let title = app.state.app_name(id).map(|n| format!("{n} · {}", t!("Store page"))).unwrap_or_else(|| t!("Store page").to_string());
    let pos = pos2(260.0, STRIP + 50.0);
    let size = vec2(1000.0, (ctx.content_rect().height() - STRIP - 100.0).clamp(400.0, 800.0));
    ov_window(app, ctx, t, Win::Store, &title, pos, size, |app, ui| {
        let env = extensions::Env { state: &app.state, local: &app.local, theme: &app.prefs.theme, zoom: app.prefs.zoom, client: &app.client, actions: &mut *actions };
        if !app.registry.show_page("store", ui, env) {
            ui.label(w::text(t!("The store isn't available."), 13.0, t.text_faint));
        }
    });
}

fn friends_window(app: &mut SteamApp, ctx: &egui::Context, t: &Theme, actions: &mut Vec<Action>) {
    let screen = ctx.content_rect();
    let pos = pos2(screen.right() - 370.0, STRIP + 40.0);
    ov_window(app, ctx, t, Win::Friends, &t!("Friends"), pos, vec2(330.0, 600.0), |app, ui| {
        w::search(ui, t, &mut app.overlay.search, &t!("Search friends"), ui.available_width());
        ui.add_space(8.0);
        let q = app.overlay.search.trim().to_lowercase();
        let all: Vec<Persona> = app.state.friend_list().into_iter().filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q)).collect();
        let in_game: Vec<_> = all.iter().filter(|p| p.in_game()).cloned().collect();
        let online: Vec<_> = all.iter().filter(|p| !p.in_game() && p.state != PersonaState::Offline).cloned().collect();
        let offline: Vec<_> = all.iter().filter(|p| !p.in_game() && p.state == PersonaState::Offline).cloned().collect();
        egui::ScrollArea::vertical().id_salt("ov-friends-list").auto_shrink([false, false]).show(ui, |ui| {
            for (title, list, collapsible) in [(t!("In game"), &in_game, false), (t!("Online"), &online, false), (t!("Offline"), &offline, true)] {
                if list.is_empty() {
                    continue;
                }
                let open_section = !collapsible || app.overlay.show_offline || !q.is_empty();
                let head = format!("{}  {} — {}", if open_section { icons::CARET_DOWN } else { icons::CARET_RIGHT }, title.to_uppercase(), list.len());
                let r = ui.add(egui::Label::new(w::text_bold(head, 11.0, t.text_faint)).sense(Sense::click()));
                if collapsible && r.clicked() {
                    app.overlay.show_offline = !app.overlay.show_offline;
                }
                ui.add_space(4.0);
                if open_section {
                    for p in list {
                        friend_row(app, ui, t, p, actions);
                    }
                }
                ui.add_space(8.0);
            }
            if all.is_empty() {
                ui.label(w::text(t!("No friends match."), 13.0, t.text_faint));
            }
        });
    });
}

fn friend_row(app: &mut SteamApp, ui: &mut Ui, t: &Theme, p: &Persona, actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    let hover = resp.hovered() || resp.context_menu_opened();
    if hover || app.overlay.chat_with == Some(p.steam_id) {
        ui.painter().rect_filled(rect, t.corner_small(), t.surface_hover);
    }
    let av = Rect::from_min_size(rect.min + vec2(4.0, 7.0), vec2(32.0, 32.0));
    let offline = p.state == PersonaState::Offline;
    if !w::paint_cover(ui, av, &art::avatar(p.avatar_hash.as_deref()), CornerRadius::same(4)) {
        w::skeleton(ui, t, av, CornerRadius::same(4));
    }
    if offline {
        ui.painter().rect_filled(av, CornerRadius::same(4), Color32::from_black_alpha(110));
    }
    let color = crate::shell::status_color(t, p);
    let name_color = if p.in_game() { t.in_game } else if offline { t.text_dim } else { t.text };
    ui.painter().text(pos2(av.right() + 10.0, rect.top() + 15.0), Align2::LEFT_CENTER, &p.name, w::font_semibold(13.5), name_color);
    ui.painter().text(pos2(av.right() + 10.0, rect.top() + 32.0), Align2::LEFT_CENTER, crate::shell::status_text(&app.state, p), w::font(11.5), if offline { t.text_faint } else { color });
    let unread = app.state.unread.get(&p.steam_id).copied().unwrap_or(0);
    if unread > 0 {
        let c = pos2(rect.right() - 14.0, rect.center().y);
        ui.painter().circle_filled(c, 9.0, t.accent);
        ui.painter().text(c, Align2::CENTER_CENTER, unread.to_string(), w::font_bold(10.5), Color32::WHITE);
    }
    // Quick actions on hover, like Steam's overlay.
    let game = app.overlay.game.map(|g| g.0);
    let mut invite = false;
    if resp.hovered() && unread == 0 {
        let mut x = rect.right() - 6.0;
        let mut button = |ui: &mut Ui, icon: &str, tip: &str| -> bool {
            let r = Rect::from_center_size(pos2(x - 14.0, rect.center().y), vec2(28.0, 28.0));
            x -= 32.0;
            let b = ui.interact(r, egui::Id::unique(("ov-act", p.steam_id, icon)), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::same(6), if b.hovered() { t.accent } else { t.raised });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, w::font(14.0), if b.hovered() { t.on_accent } else { t.text });
            b.on_hover_text(tip).clicked()
        };
        if game.is_some() && button(ui, icons::USER_PLUS, t!("Invite to play")) {
            invite = true;
        }
        if button(ui, icons::CHAT, t!("Send a message")) {
            open_chat(app, p.steam_id, actions);
        }
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        open_chat(app, p.steam_id, actions);
    }
    resp.context_menu(|ui| {
        if ui.button(format!("{}  {}", icons::CHAT, t!("Send a message"))).clicked() {
            open_chat(app, p.steam_id, actions);
            ui.close();
        }
        if game.is_some() && ui.button(format!("{}  {}", icons::USER_PLUS, t!("Invite to play"))).clicked() {
            invite = true;
            ui.close();
        }
        if ui.button(format!("{}  {}", icons::USER_CIRCLE, t!("View profile"))).clicked() {
            actions.push(Action::Navigate { page: "profile".into(), arg: Some(p.steam_id.to_string()) });
            ui.close();
        }
    });
    if invite {
        if let Some(id) = game {
            // Steam's own invites come from inside the game (its lobby); this
            // sends a link that starts the game for them.
            let name = app.state.app_name(id).unwrap_or("").to_string();
            let text = t!("Play {game} with me! steam://run/{id}", game = name, id = id);
            app.client.send(Command::SendMessage { to: p.steam_id, text });
            actions.push(Action::Toast(if offline { t!("Invite sent. {name} will see it when they come online.", name = p.name) } else { t!("Invite sent to {name}", name = p.name) }));
        }
    }
}

fn open_chat(app: &mut SteamApp, id: SteamId, actions: &mut Vec<Action>) {
    app.overlay.chat_with = Some(id);
    app.overlay.win_mut(Win::Chat).open = true;
    app.client.send(Command::LoadHistory { friend: id });
    actions.push(Action::MarkRead(id));
}

fn chat_window(app: &mut SteamApp, ctx: &egui::Context, t: &Theme, actions: &mut Vec<Action>) {
    let Some(id) = app.overlay.chat_with else { return };
    let p = app.state.persona(id);
    let screen = ctx.content_rect();
    let pos = pos2(screen.right() - 830.0, STRIP + 40.0);
    ov_window(app, ctx, t, Win::Chat, &p.name, pos, vec2(440.0, 520.0), |app, ui| {
        ui.horizontal(|ui| {
            w::avatar(ui, t, &art::avatar(p.avatar_hash.as_deref()), 30.0, Some(crate::shell::status_color(t, &p)));
            ui.label(w::text(crate::shell::status_text(&app.state, &p), 12.5, t.text_dim));
        });
        ui.add_space(6.0);
        // Exactly the space the window has, so it never grows by itself.
        let h = ui.available_height();
        crate::builtin::community::chat_body(ui, t, &app.state, &app.client, id, &mut app.overlay.input, h);
    });
    if !app.overlay.win(Win::Chat).open {
        app.overlay.chat_with = None;
    }
    let _ = actions;
}

fn notes_window(app: &mut SteamApp, ctx: &egui::Context, t: &Theme) {
    let screen = ctx.content_rect();
    let pos = pos2(520.0, (screen.bottom() - 460.0).max(STRIP + 80.0));
    ov_window(app, ctx, t, Win::Notes, &t!("Notes"), pos, vec2(360.0, 420.0), |app, ui| {
        ui.label(w::text(t!("Jot things down while you play. Kept until you close the app."), 12.0, t.text_dim));
        ui.add_space(6.0);
        let size = ui.available_size();
        ui.add_sized(size, egui::TextEdit::multiline(&mut app.overlay.notes).font(w::font(14.0)));
    });
}

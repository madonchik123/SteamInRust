//! Big Picture: a fullscreen mode for the TV, driven by a controller or the
//! keyboard. It shows the same library, store, downloads and friends as the
//! normal window, and uses the Library's and Store's own logic for playing,
//! installing and loading, so the two never disagree.
//!
//! Controls: D-pad/stick or arrows move, A/Enter selects, B/Esc goes back,
//! LB/RB (Q/E, PageUp/PageDown, Tab) switch tabs, Start (M/F1) opens the
//! menu, Guide opens the menu and, pressed again there, leaves Big Picture.

mod focus;
mod input;
mod menu;
mod paint;
mod screens;

use crate::app::SteamApp;
use egui::{Align2, Color32, Id, Rect, Ui, pos2, vec2};
use extensions::Action;
use focus::{Dir, Focus, Place, Scroller, since};
use i18n::t;
use input::{Device, PadEvent, Press};
use model::{AppId, SteamId};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Home,
    Library,
    Store,
    Downloads,
    Friends,
}

const TABS: [Tab; 5] = [Tab::Home, Tab::Library, Tab::Store, Tab::Downloads, Tab::Friends];

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Home => t!("Home"),
            Tab::Library => t!("Library"),
            Tab::Store => t!("Store"),
            Tab::Downloads => t!("Downloads"),
            Tab::Friends => t!("Friends"),
        }
    }
    fn index(self) -> usize {
        TABS.iter().position(|x| *x == self).unwrap_or(0)
    }
}

/// A screen: a tab, or something opened on top of one.
#[derive(Clone, PartialEq, Debug)]
pub enum Page {
    Tab(Tab),
    Game(AppId),
    StoreApp(AppId),
    Search,
    /// Screenshots of a store page, at one of them.
    Shots(AppId, usize),
    Settings,
    /// A friend: message, join, invite, profile.
    Friend(SteamId),
    /// A conversation with a friend.
    Chat(SteamId),
    /// A store trailer, playing over the page.
    Video(AppId, usize),
    /// Before installing: size, where, shortcut.
    Install(AppId),
}

impl Page {
    fn key(&self) -> String {
        format!("{self:?}")
    }
}

#[derive(Default)]
pub struct BigPicture {
    pub active: bool,
    /// `--bigpicture=window`: stay in a window (previews, testing).
    windowed: bool,
    fullscreen_sent: bool,
    fullscreen_asked: f64,
    /// The normal window was maximized when Big Picture opened: it's
    /// un-maximized first (Windows won't resize a maximized window to full
    /// screen, leaving a taskbar-high strip undrawn) and maximized again on leaving.
    was_maximized: bool,
    entered_at: f64,
    tab: Tab,
    stack: Vec<Page>,
    focus: Focus,
    /// What was focused on each page, to come back to.
    memory: HashMap<String, Id>,
    /// Where focus goes first on the page being drawn (its main button).
    preferred: Option<Id>,
    scroll: HashMap<String, Scroller>,
    /// When the page changed, and from which side it slides in (-1, 0, 1).
    page_since: f64,
    slide: f32,
    menu: Option<menu::Menu>,
    pad: Option<mpsc::Receiver<PadEvent>>,
    /// Controller presses waiting for the next frame.
    queued: Vec<Press>,
    pads: usize,
    device: Device,
    /// The last hint pressed, for a little flash.
    flash: Option<(Press, f64)>,
    /// Library filter: installed only (None = decide from the library).
    installed_only: Option<bool>,
    /// Library filter: only games that run on this computer.
    this_os_only: bool,
    /// The install screen's choices: (game, library folder, desktop shortcut).
    install: Option<(AppId, std::path::PathBuf, bool)>,
    /// A trailer was playing (stopped when its page closes).
    video_on: bool,
    // Search (on-screen keyboard).
    query: String,
    query_changed: f64,
    submitted: String,
    cyrillic: bool,
    /// Chat: the message being typed, Enter pressed on a real keyboard, and
    /// conversations whose history was asked for.
    message: String,
    send_now: bool,
    chats_loaded: HashSet<SteamId>,
    /// Background art: games and when each started fading in.
    bg: Vec<(AppId, f64)>,
    /// The game whose art the background should show this frame.
    bg_want: Option<AppId>,
    /// Achievement lists asked for this session.
    asked: HashSet<AppId>,
    /// Sliding highlight under the selected tab.
    tab_pill: Option<Rect>,
    /// When the screenshot viewer last changed picture, and which way.
    shot_since: (f64, f32),
    /// The on-screen key pressed last, and when (it pops).
    key_pop: Option<(Id, f64)>,
    /// Mouse wheel turned in the screenshot viewer, not yet a flip.
    shot_wheel: f32,
    /// Chat: whose conversation is open, whether it sticks to the newest
    /// message, a pending up/down scroll, whether the keyboard is up, and
    /// whether the history is at its (top, bottom).
    chat_for: Option<SteamId>,
    chat_follow: bool,
    chat_nudge: f32,
    chat_keys: bool,
    chat_edges: (bool, bool),
}

impl BigPicture {
    pub fn new(ctx: &egui::Context) -> Self {
        let flag = std::env::args().find(|a| a == "--bigpicture" || a.starts_with("--bigpicture="));
        let mut bp = Self {
            active: flag.is_some(),
            // An off-screen preview never goes fullscreen (it would land on
            // the real screen).
            windowed: flag.as_deref() == Some("--bigpicture=window") || crate::app::offscreen(),
            pad: Some(input::spawn(ctx.clone())),
            slide: 0.0,
            ..Default::default()
        };
        // `--bp-page=store:<app>|game:<app>|install:<app>|shots:<app>|
        // chat:<steam id>|library` opens a page right away (previews and checks).
        if let Some(arg) = std::env::args().find_map(|a| a.strip_prefix("--bp-page=").map(String::from)) {
            let (kind, id) = arg.split_once(':').unwrap_or((arg.as_str(), ""));
            let wide: u64 = id.parse().unwrap_or(0);
            let id = id.parse().unwrap_or(0);
            match kind {
                "library" => bp.tab = Tab::Library,
                "store" => bp.stack.push(Page::StoreApp(id)),
                "shots" => bp.stack.extend([Page::StoreApp(id), Page::Shots(id, 0)]),
                "chat" => {
                    bp.tab = Tab::Friends;
                    bp.stack.extend([Page::Friend(wide), Page::Chat(wide)]);
                }
                "game" => bp.stack.push(Page::Game(id)),
                "install" => bp.stack.push(Page::Install(id)),
                _ => {}
            }
        }
        bp
    }

    pub fn enter(&mut self, ctx: &egui::Context) {
        if self.active {
            return;
        }
        self.active = true;
        self.fullscreen_sent = false;
        self.entered_at = ctx.input(|i| i.time);
        self.tab = Tab::Home;
        self.stack.clear();
        self.menu = None;
        self.focus.current = None;
        self.focus.reset_ring();
    }

    pub fn leave(&mut self, ctx: &egui::Context) {
        self.active = false;
        self.menu = None;
        if !self.windowed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
    }

    /// Running in a window (`--bigpicture=window`), not full screen.
    pub fn is_windowed(&self) -> bool {
        self.windowed
    }

    /// Whether the normal window was maximized before Big Picture opened (asked
    /// once, after leaving, when the window gets its normal size back).
    pub fn take_was_maximized(&mut self) -> bool {
        std::mem::take(&mut self.was_maximized)
    }

    fn page(&self) -> Page {
        self.stack.last().cloned().unwrap_or(Page::Tab(self.tab))
    }

    fn remember(&mut self) {
        // Only page items: a tab remembered for a page would switch tabs on return.
        if let Some(c) = self.focus.current.filter(|c| !TABS.iter().any(|t| tab_id(*t) == *c)) {
            self.memory.insert(self.page().key(), c);
        }
    }

    fn arrive(&mut self, now: f64, slide: f32) {
        self.focus.current = self.memory.get(&self.page().key()).copied();
        self.page_since = now;
        self.slide = slide;
    }

    /// Open a page on top of the current one.
    fn open(&mut self, page: Page, now: f64) {
        self.remember();
        // A fresh visit starts at the top.
        self.memory.remove(&page.key());
        self.scroll.retain(|k, _| !k.starts_with(&page.key()));
        self.stack.push(page);
        self.arrive(now, 1.0);
    }

    fn back(&mut self, now: f64) {
        if !self.stack.is_empty() {
            self.remember();
            self.stack.pop();
            self.arrive(now, -1.0);
        } else if self.tab != Tab::Home {
            self.switch_tab(Tab::Home, now, true);
        } else {
            self.menu = Some(menu::Menu::new(now));
        }
    }

    /// Show another tab. `take_focus`: move focus into it (LB/RB, B), or
    /// leave it on the tab bar (moving along the tabs).
    fn switch_tab(&mut self, tab: Tab, now: f64, take_focus: bool) {
        if tab == self.tab && self.stack.is_empty() {
            return;
        }
        let keep = self.focus.current;
        self.remember();
        let slide = if tab.index() >= self.tab.index() { 1.0 } else { -1.0 };
        self.stack.clear();
        self.tab = tab;
        self.arrive(now, slide);
        if !take_focus {
            self.focus.current = keep;
        }
    }

    fn scroller(&mut self, key: &str) -> &mut Scroller {
        let k = format!("{}/{key}", self.page().key());
        self.scroll.entry(k).or_default()
    }
}

/// Read controllers every frame. Outside Big Picture, Guide opens it.
pub fn poll(app: &mut SteamApp, ctx: &egui::Context) {
    let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
    let mut events = Vec::new();
    if let Some(rx) = &app.bigpicture.pad {
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
    }
    let bp = &mut app.bigpicture;
    for e in events {
        match e {
            PadEvent::Connected(n) => bp.pads = n,
            // Presses only count while our window is in front (not in a game).
            PadEvent::Press(p) if focused => {
                if bp.active {
                    bp.queued.push(p);
                } else if p == Press::Guide && app.state.phase == client::state::Phase::LoggedIn {
                    bp.enter(ctx);
                }
            }
            PadEvent::Press(_) => {}
        }
    }
}

/// Size everything for the screen: 1.0 on a 1080p TV, bigger on 4K, smaller in a window.
fn scale(screen: Rect, user: f32) -> f32 {
    ((screen.width() / 1920.0).min(screen.height() / 1080.0) * user).clamp(0.45, 3.0)
}

/// Draw Big Picture over the whole window.
pub fn show(app: &mut SteamApp, ui: &mut Ui, actions: &mut Vec<Action>) {
    let mut bp = std::mem::take(&mut app.bigpicture);
    bp.frame(app, ui, actions);
    // Keep anything that arrived meanwhile (it's empty unless Guide re-entered).
    bp.queued.append(&mut app.bigpicture.queued);
    app.bigpicture = bp;
}

/// What a page is drawn with this frame.
pub struct Frame<'a> {
    pub app: &'a mut SteamApp,
    pub actions: &'a mut Vec<Action>,
    pub t: Theme,
    pub s: f32,
    /// The page's visible area and its padding at the sides.
    pub view: Rect,
    pub pad: f32,
    pub now: f64,
    /// X/Y pressed this frame (each page decides what they do).
    pub x: bool,
    pub y: bool,
}

impl BigPicture {
    fn frame(&mut self, app: &mut SteamApp, ui: &mut Ui, actions: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        let t = app.prefs.theme.clone();
        let screen = ui.max_rect();
        let s = scale(screen, app.prefs.bigpicture_scale);
        let now = ctx.input(|i| i.time);
        if !self.fullscreen_sent {
            self.fullscreen_sent = true;
            if self.entered_at == 0.0 {
                self.entered_at = now;
            }
        }
        // Fullscreen, and again if something resized the window meanwhile
        // (signing in sets the normal window size).
        if !self.windowed && ctx.input(|i| i.viewport().fullscreen) != Some(true) && now - self.fullscreen_asked > 0.5 {
            self.fullscreen_asked = now;
            if ctx.input(|i| i.viewport().maximized) == Some(true) {
                self.was_maximized = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }

        // ---- input ----
        let page = self.page();
        let chat = matches!(page, Page::Chat(_));
        let typing = (page == Page::Search || chat) && self.menu.is_none();
        let mut submit = false;
        if typing {
            let (text, back) = ctx.input_mut(|i| {
                let text: String = i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect();
                (text, i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
            });
            let field = if chat { &mut self.message } else { &mut self.query };
            if !text.is_empty() {
                field.push_str(&text);
                self.query_changed = now;
                self.device = Device::Keyboard;
            }
            if back {
                if field.pop().is_none() {
                    self.queued.push(Press::B);
                } else {
                    self.query_changed = now;
                }
            }
        }
        let keys = input::keyboard(&ctx, typing);
        if !keys.is_empty() {
            self.device = Device::Keyboard;
        } else if !self.queued.is_empty() {
            self.device = Device::Pad;
        }
        let mut presses: Vec<(Press, bool)> = keys.into_iter().map(|p| (p, true)).collect();
        presses.extend(self.queued.drain(..).map(|p| (p, false)));
        if let Some((p, _)) = presses.last() {
            self.flash = Some((*p, now));
        }

        let mut dir = None;
        let mut press = false;
        let (mut x, mut y) = (false, false);
        if let Some(m) = &mut self.menu {
            if m.input(&presses, now) == menu::Outcome::LeaveBigPicture {
                self.leave(&ctx);
                ctx.request_repaint();
                return;
            }
        } else {
            for (p, from_keys) in presses {
                match p {
                    Press::Move(d) => dir = Some(d),
                    // Typing on a real keyboard: Enter searches what was typed
                    // (and then selects, like everywhere else).
                    Press::A if typing && from_keys && chat && !self.message.trim().is_empty() => self.send_now = true,
                    Press::A if typing && from_keys && !chat && self.query.trim() != self.submitted => submit = true,
                    Press::A => press = true,
                    // Chat with the keyboard up: B puts it away first.
                    Press::B if chat && self.chat_keys => self.focus.current = Some(screens::chat_history_id()),
                    Press::B => self.back(now),
                    Press::X => x = true,
                    Press::Y => y = true,
                    Press::Lb => self.switch_tab(TABS[(self.tab.index() + TABS.len() - 1) % TABS.len()], now, true),
                    Press::Rb => self.switch_tab(TABS[(self.tab.index() + 1) % TABS.len()], now, true),
                    Press::Start | Press::Guide => {
                        self.menu = Some(menu::Menu::new(now));
                    }
                }
            }
        }
        if submit {
            self.submitted = self.query.trim().to_string();
        }
        // The page changed this frame: a move meant for the old one is dropped.
        if self.page() != page {
            dir = None;
            press = false;
        }
        // A trailer: A pauses, left/right move 10 seconds.
        if let Page::Video(..) = self.page() {
            let seconds = match dir {
                Some(Dir::Left) => -10.0,
                Some(Dir::Right) => 10.0,
                _ => 0.0,
            };
            if press || seconds != 0.0 {
                screens::with_store(app, actions, |st, _| st.trailer_control(press, seconds));
            }
            dir = None;
            press = false;
        }
        // Chat: up/down on the history scroll it; past its ends they move on.
        if let (Page::Chat(_), Some(d @ (Dir::Up | Dir::Down))) = (self.page(), dir) {
            let (at_top, at_bottom) = self.chat_edges;
            let leave = (d == Dir::Up && at_top) || (d == Dir::Down && at_bottom);
            if self.focus.current == Some(screens::chat_history_id()) && !leave {
                self.chat_nudge += if d == Dir::Up { -1.0 } else { 1.0 };
                dir = None;
            }
        }
        // The screenshot viewer pages with left/right.
        if let (Page::Shots(app_id, i), Some(d @ (Dir::Left | Dir::Right))) = (self.page(), dir) {
            let n = screens::shot_count(app, actions, app_id);
            if n > 0 {
                let step = if d == Dir::Left { n - 1 } else { 1 };
                *self.stack.last_mut().unwrap() = Page::Shots(app_id, (i + step) % n);
                self.shot_since = (now, if d == Dir::Left { -1.0 } else { 1.0 });
            }
            dir = None;
        }
        let menu_open = self.menu.is_some();
        let before = self.focus.current;
        self.focus.begin(ui, if menu_open { None } else { dir }, press && !menu_open);
        // Up from a page into the tab bar lands on this page's tab (the
        // nearest one could be another tab, which would switch pages).
        let is_tab = |id: Id| TABS.iter().any(|t| tab_id(*t) == id);
        if self.focus.moved && before.is_some_and(|b| !is_tab(b)) && self.focus.current.is_some_and(is_tab) {
            self.focus.current = Some(tab_id(self.tab));
        }

        // ---- draw ----
        let top_h = 104.0 * s;
        let hint_h = 78.0 * s;
        let view = Rect::from_min_max(pos2(screen.left(), screen.top() + top_h), pos2(screen.right(), screen.bottom() - hint_h));
        self.preferred = None;

        // Pages slide and fade in.
        let e = since(ui, self.page_since, 0.35);
        let dx = (1.0 - e) * 90.0 * s * self.slide;
        let mut content = paint::faded(ui, screen, e.max(0.0));
        content.set_clip_rect(Rect::from_min_max(pos2(screen.left(), view.top() - 30.0 * s), pos2(screen.right(), view.bottom())));
        let page_view = view.translate(vec2(dx, 0.0));
        let pad = 80.0 * s;
        let mut f = Frame { app, actions, t: t.clone(), s, view: page_view, pad, now, x, y };

        self.background(ui, &f);
        let key = format!("{}/page", self.page().key());
        let pos = self.scroll.get(&key).map(|sc| sc.pos).unwrap_or(0.0);
        let height = screens::draw(self, &mut content, &mut f, pos);
        // Scroll the page: follow the focus, or the mouse wheel.
        {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            let moved = self.focus.moved;
            let focused = self.focus.focused();
            let sc = self.scroll.entry(key).or_default();
            if wheel != 0.0 && ui.rect_contains_pointer(view) {
                sc.target -= wheel;
            }
            if let (true, Some((r, Place::Page))) = (moved, focused) {
                let top = r.top() - (view.top() - sc.pos);
                sc.reveal(top, top + r.height(), view.height(), view.height() * 0.2);
                // Near the start: show the very top (the page title, the hero).
                if sc.target < view.height() * 0.35 {
                    sc.target = 0.0;
                }
            }
            sc.clamp(height, view.height());
            sc.step(ui);
        }

        self.top_bar(ui, &mut f, screen, top_h);
        self.hint_bar(ui, &mut f, screen, hint_h);
        let first: Vec<Id> = self.preferred.into_iter().collect();
        self.focus.end(&first, tab_id(self.tab));
        if !menu_open {
            self.focus.paint_ring(ui, &t, s, 12.0 * s);
        }
        // The library may need to ask something (a Steam Cloud conflict).
        screens::with_library(f.app, f.actions, |lib, cx| lib.background_dialogs(ui, cx));

        if self.menu.is_some() {
            menu::show(self, ui, &mut f, screen);
        }
        if !self.active {
            return;
        }
        self.intro(ui, &t, screen, s, now);
        // Gentle constant motion in the background.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }

    /// The focused game's art, slowly drifting, cross-faded when it changes;
    /// soft colored light behind everything.
    fn background(&mut self, ui: &mut Ui, f: &Frame) {
        let (t, s, now) = (&f.t, f.s, f.now);
        let screen = ui.max_rect();
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, t.bg);
        // Two slow lights in the theme's accent colors.
        let time = now as f32;
        let a = screen.center() + vec2((time * 0.07).sin() * screen.width() * 0.35, (time * 0.05).cos() * screen.height() * 0.3);
        let b = screen.center() + vec2((time * 0.06 + 2.0).cos() * screen.width() * 0.38, (time * 0.08 + 1.0).sin() * screen.height() * 0.32);
        paint::glow(&p, a, screen.width() * 0.45, t.accent.gamma_multiply(0.10));
        paint::glow(&p, b, screen.width() * 0.4, t.accent_2.gamma_multiply(0.09));

        // Pages say which game is focused while drawing: that shows next frame.
        if let Some(want) = self.bg_want.take().or(self.bg.last().map(|b| b.0)) {
            if self.bg.last().map(|b| b.0) != Some(want) {
                self.bg.push((want, now));
            }
        }
        // Drop pictures fully covered by a newer one.
        if let Some(i) = self.bg.iter().rposition(|(_, at)| now - at > 0.8) {
            self.bg.drain(..i);
        }
        let drift = vec2((time * 0.04).sin() * 24.0 * s, (time * 0.03).cos() * 14.0 * s);
        let big = paint::grow(screen, 1.1).translate(drift);
        for (id, at) in self.bg.clone() {
            let e = focus::ease(((now - at) / 0.8) as f32);
            let c = paint::faded(ui, screen, e * 0.42);
            w::paint_cover_first(&c, big, &[&model::art::hero(id), &model::art::header(id)], egui::CornerRadius::ZERO);
        }
        // Darken toward the bottom and left so text stays readable.
        w::gradient(&p, screen, t.bg.gamma_multiply(0.55), t.bg.gamma_multiply(0.97), false);
        w::fade_right(ui, screen, t.bg.gamma_multiply(0.75), 0.6);
    }

    fn top_bar(&mut self, ui: &mut Ui, f: &mut Frame, screen: Rect, h: f32) {
        let (t, s) = (f.t.clone(), f.s);
        let bar = Rect::from_min_size(screen.min, vec2(screen.width(), h));
        let p = ui.painter().clone();
        w::gradient(&p, bar, t.bg.gamma_multiply(0.96), t.bg.gamma_multiply(0.0), false);
        let pad = 80.0 * s;
        // Logo and name.
        let mut logo = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(pos2(bar.left() + pad + 20.0 * s, bar.center().y), vec2(40.0 * s, 40.0 * s))));
        w::logo(&mut logo, &t, 40.0 * s);

        // Tabs in the middle.
        let font = w::font_bold(25.0 * s);
        let gap = 14.0 * s;
        let widths: Vec<f32> = TABS.iter().map(|tab| paint::text_width(&p, &tab_label(f, *tab), font.clone()) + 44.0 * s).collect();
        let total: f32 = widths.iter().sum::<f32>() + gap * (TABS.len() - 1) as f32;
        let mut x = bar.center().x - total / 2.0;
        let tab_h = 54.0 * s;
        let mut selected_rect = None;
        for (i, tab) in TABS.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x, bar.center().y - tab_h / 2.0), vec2(widths[i], tab_h));
            x += widths[i] + gap;
            let hit = self.focus.item(ui, tab_id(*tab), r, screen, Place::Bar);
            if hit.focused {
                self.focus.ring_around(r, tab_h / 2.0);
            }
            let selected = *tab == self.tab;
            if selected {
                selected_rect = Some(r);
            }
            if hit.focused && !selected && !self.focus.by_pointer {
                // Moving along the tabs (D-pad, keys) shows each one; the
                // mouse only highlights a tab until it's clicked.
                self.switch_tab(*tab, f.now, false);
            }
            if hit.activated {
                // A on a tab goes into its page.
                self.focus.current = self.memory.get(&Page::Tab(*tab).key()).copied().or(Some(Id::NULL));
                self.focus.moved = true;
            }
            let color = if selected { t.text } else { w::mix(t.text_dim, t.text, hit.glow) };
            p.text(r.center(), Align2::CENTER_CENTER, tab_label(f, *tab), font.clone(), color);
        }
        // The highlight slides from tab to tab.
        if let Some(target) = selected_rect {
            let dt = ui.input(|i| i.stable_dt.min(0.1));
            let k = 1.0 - (-dt * 14.0).exp();
            let cur = self.tab_pill.map(|c| Rect::from_min_max(c.min + (target.min - c.min) * k, c.max + (target.max - c.max) * k)).unwrap_or(target);
            if (cur.min - target.min).length() > 0.5 {
                ui.ctx().request_repaint();
            }
            self.tab_pill = Some(cur);
            let under = ui.painter().clone();
            // Behind the label: a soft pill and a bright line under it.
            under.rect_filled(cur, paint::radius(cur.height() / 2.0), Color32::from_white_alpha(18));
            let line = Rect::from_center_size(pos2(cur.center().x, cur.bottom() + 6.0 * s), vec2(cur.width() * 0.4, 4.0 * s));
            w::gradient(&under.with_clip_rect(line), line, t.accent, t.accent_2, true);
            // Labels again on top of the pill.
            let mut x = bar.center().x - total / 2.0;
            for (i, tab) in TABS.iter().enumerate() {
                let r = Rect::from_min_size(pos2(x, bar.center().y - tab_h / 2.0), vec2(widths[i], tab_h));
                x += widths[i] + gap;
                if *tab == self.tab {
                    p.text(r.center(), Align2::CENTER_CENTER, tab_label(f, *tab), font.clone(), t.text);
                }
            }
        }

        // Right: controller and account.
        let right = bar.right() - pad;
        let me = f.app.state.me().cloned();
        let name = f.app.state.persona_name.clone();
        let av = Rect::from_center_size(pos2(right - 24.0 * s, bar.center().y), vec2(48.0 * s, 48.0 * s));
        let url = model::art::avatar_full(me.as_ref().and_then(|m| m.avatar_hash.as_deref()));
        if !w::paint_cover(ui, av, &url, paint::radius(24.0 * s)) {
            p.circle_filled(av.center(), 24.0 * s, t.raised);
        }
        if let Some(m) = &me {
            p.circle_stroke(av.center(), 25.5 * s, egui::Stroke::new(2.5 * s, crate::shell::status_color(&t, m)));
        }
        let name_rect = paint::line(&p, pos2(av.left() - 14.0 * s, bar.center().y), Align2::RIGHT_CENTER, &name, w::font_semibold(21.0 * s), t.text, 260.0 * s);
        if self.pads > 0 {
            p.text(pos2(name_rect.left() - 20.0 * s, bar.center().y), Align2::RIGHT_CENTER, icons::CONTROLLER, w::font(28.0 * s), t.text_dim);
        }
    }

    fn hint_bar(&mut self, ui: &mut Ui, f: &mut Frame, screen: Rect, h: f32) {
        let (t, s) = (f.t.clone(), f.s);
        let bar = Rect::from_min_max(pos2(screen.left(), screen.bottom() - h), screen.max);
        let p = ui.painter().clone();
        // Content fades out just above the bar.
        let fade = Rect::from_min_max(pos2(bar.left(), bar.top() - 40.0 * s), pos2(bar.right(), bar.top()));
        w::gradient(&p, fade, t.bg.gamma_multiply(0.0), t.bg.gamma_multiply(0.85), false);
        w::gradient(&p, bar, t.bg.gamma_multiply(0.85), t.bg.gamma_multiply(0.97), false);
        p.hline(bar.x_range().shrink(80.0 * s), bar.top() + 6.0 * s, egui::Stroke::new(1.0, Color32::from_white_alpha(18)));
        let pad = 80.0 * s;
        let pad_mode = self.device == Device::Pad;
        // Left: where you are.
        let left = match self.page() {
            Page::Tab(_) => format!("{}  {}", if pad_mode { "LB / RB" } else { "Q / E" }, t!("Switch tabs")),
            _ => String::new(),
        };
        if !left.is_empty() {
            p.text(pos2(bar.left() + pad, bar.center().y), Align2::LEFT_CENTER, left, w::font_medium(18.0 * s), t.text_faint);
        }
        let menu_open = self.menu.is_some();
        let mut hints: Vec<(Press, String)> = vec![(Press::A, t!("Select").into()), (Press::B, if menu_open { t!("Close") } else { t!("Back") }.into())];
        if !menu_open {
            if let Some(x) = screens::x_hint(self) {
                hints.push((Press::X, x));
            }
            hints.push((Press::Start, t!("Menu").into()));
        }
        let mut x = bar.right() - pad;
        let flash = self.flash.filter(|(_, at)| f.now - at < 0.3);
        for (press, label) in hints.iter().rev() {
            let lw = paint::text_width(&p, label, w::font_medium(19.0 * s));
            x -= lw;
            p.text(pos2(x, bar.center().y), Align2::LEFT_CENTER, label, w::font_medium(19.0 * s), t.text_dim);
            let (glyph, round) = match (press, pad_mode, self.page()) {
                // Typing: X is a letter, Backspace deletes.
                (Press::X, false, Page::Search | Page::Chat(_)) => ("Backspace", false),
                _ => glyph_for(*press, pad_mode),
            };
            let fl = flash.filter(|(fp, _)| fp == press).map(|(_, at)| 1.0 - ((f.now - at) / 0.3) as f32).unwrap_or(0.0);
            let gw = paint::text_width(&p, glyph, w::font_bold(17.0 * s)) + 22.0 * s;
            let gw = if round { 34.0 * s } else { gw.max(34.0 * s) };
            x -= gw + 10.0 * s;
            let r = paint::glyph(&p, &t, pos2(x, bar.center().y), glyph, round, s, fl);
            // Hints are buttons for the mouse too.
            let resp = ui.interact(r.union(Rect::from_min_size(r.right_top(), vec2(lw + 10.0 * s, r.height()))), Id::unique(("bp-hint", label)), egui::Sense::click());
            if resp.clicked() {
                self.queued.push(*press);
            }
            x -= 30.0 * s;
        }
        if flash.is_some() {
            ui.ctx().request_repaint();
        }
    }

    /// Entering: the logo blooms and fades as the screen appears.
    fn intro(&self, ui: &mut Ui, t: &Theme, screen: Rect, s: f32, now: f64) {
        let e = ((now - self.entered_at) / 0.9) as f32;
        if !(0.0..1.0).contains(&e) {
            return;
        }
        ui.ctx().request_repaint();
        let fade = 1.0 - focus::ease(((e - 0.35) / 0.65).max(0.0));
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, t.bg.gamma_multiply(fade));
        paint::glow(&p, screen.center(), screen.height() * (0.25 + 0.5 * e), t.accent.gamma_multiply(0.35 * fade));
        let size = 120.0 * s * (0.8 + 0.35 * focus::ease(e));
        let mut c = paint::faded(ui, Rect::from_center_size(screen.center(), vec2(size, size)), fade);
        w::logo_as(&mut c, ui::brand::Logo::OutlineWhite, size);
        p.text(screen.center() + vec2(0.0, size * 0.75 + 20.0 * s), Align2::CENTER_TOP, t!("Big Picture"), w::font_bold(30.0 * s), t.text.gamma_multiply(fade));
    }
}

pub(crate) fn tab_id(tab: Tab) -> Id {
    Id::unique(("bp-tab", tab.index()))
}

/// Tab names, with a count where it helps (downloads, friends online).
fn tab_label(f: &Frame, tab: Tab) -> String {
    match tab {
        Tab::Downloads => {
            let n = f.app.state.downloads.values().filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).count() + f.app.state.download_queue.len();
            if n > 0 { format!("{} · {n}", tab.label()) } else { tab.label().to_string() }
        }
        _ => tab.label().to_string(),
    }
}

/// The glyph for a press: controller buttons, or keys.
fn glyph_for(p: Press, pad: bool) -> (&'static str, bool) {
    match (p, pad) {
        (Press::A, true) => ("A", true),
        (Press::B, true) => ("B", true),
        (Press::X, true) => ("X", true),
        (Press::Y, true) => ("Y", true),
        (Press::Start, true) => (icons::LIST, true),
        (Press::A, false) => ("Enter", false),
        (Press::B, false) => ("Esc", false),
        (Press::X, false) => ("X", false),
        (Press::Y, false) => ("Y", false),
        (Press::Start, false) => ("M", false),
        (_, true) => ("LB", false),
        (_, false) => ("Q", false),
    }
}

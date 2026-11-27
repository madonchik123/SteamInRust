//! Plugins in Big Picture: the Plugins tab (pages whose manifest says
//! `bigpicture = true`), each page's widgets drawn for a controller, the
//! on-screen keyboard for their text fields, and plugin buttons on game and
//! friend pages.
//!
//! Every widget gets a Big Picture form: buttons, switches, choices, tabs,
//! links and games are focusable; text fields open the keyboard; a slider is
//! picked with A and moved with left and right; table rows can be stepped
//! through, and a page with nothing to focus scrolls with up and down.

use super::focus::{Dir, Place};
use super::paint::{self, Kind};
use super::screens::{Key, button, keyboard};
use super::{BigPicture, Frame, Page, Tab};
use crate::app::SteamApp;
use crate::builtin::plugins::{BigPicturePage, PageEvent, PluginItem, Plugins, image_allowed, paint_chart, tone_color};
use egui::{Align2, Color32, Id, Rect, Ui, pos2, vec2};
use extensions::Action;
use i18n::t;
use model::{AppId, SteamId, art};
use plugins::{ChartKind, Widget};
use std::collections::HashMap;
use ui::{icons, widgets as w};

/// What Big Picture remembers about plugin pages.
#[derive(Default)]
pub struct PluginUi {
    /// Pages for the Plugins tab (refreshed every second).
    pages: Vec<BigPicturePage>,
    checked: f64,
    /// The text being typed on a plugin's text page, and Enter was pressed.
    pub text: String,
    pub submit: bool,
    /// A slider picked with A: its focus id, the plugin's slider id, and
    /// its value while moving.
    pub adjusting: Option<(Id, String, f64)>,
    /// Left/right presses while a slider is picked (handled on draw).
    pub nudge: i32,
}

impl PluginUi {
    pub fn has_pages(&self) -> bool {
        !self.pages.is_empty()
    }
}

fn with_plugins<R>(app: &mut SteamApp, actions: &mut Vec<Action>, f: impl FnOnce(&mut Plugins, &mut extensions::Ctx) -> R) -> Option<R> {
    let env = extensions::Env { state: &app.state, local: &app.local, theme: &app.prefs.theme, zoom: app.prefs.zoom, client: &app.client, actions };
    app.registry.with_ext::<Plugins, R>("plugins", env, f)
}

/// Keep the list of plugin pages up to date (once a second).
pub fn refresh(bp: &mut BigPicture, app: &mut SteamApp, actions: &mut Vec<Action>, now: f64) {
    if now - bp.plugins.checked < 1.0 && bp.plugins.checked > 0.0 {
        return;
    }
    bp.plugins.checked = now;
    bp.plugins.pages = with_plugins(app, actions, |p, _| p.bigpicture_pages()).unwrap_or_default();
}

/// A plugin asked to open a page while in Big Picture: open the Big Picture
/// one instead of changing the hidden normal window.
fn route(bp: &mut BigPicture, f: &mut Frame, from: usize) {
    let mut pages = Vec::new();
    let mut i = from;
    while i < f.actions.len() {
        if let Action::Navigate { page, arg } = &f.actions[i] {
            pages.push((page.clone(), arg.clone()));
            f.actions.remove(i);
        } else {
            i += 1;
        }
    }
    for (page, arg) in pages {
        open_target(bp, f, &page, arg);
    }
}

fn open_target(bp: &mut BigPicture, f: &Frame, page: &str, arg: Option<String>) {
    let id: Option<AppId> = arg.as_deref().and_then(|a| a.parse().ok());
    match (page, id) {
        ("store", Some(id)) => bp.open(Page::StoreApp(id), f.now),
        ("store", None) => bp.switch_tab(Tab::Store, f.now, true),
        ("library", Some(id)) => bp.open(Page::Game(id), f.now),
        ("library", None) => bp.switch_tab(Tab::Library, f.now, true),
        ("downloads", _) => bp.switch_tab(Tab::Downloads, f.now, true),
        ("settings", _) => bp.open(Page::Settings, f.now),
        (p, _) if p.starts_with("plugin:") => {
            let id = p.trim_start_matches("plugin:");
            if bp.plugins.pages.iter().any(|x| x.id == id) {
                bp.open(Page::Plugin(id.to_string()), f.now);
            } else {
                bp.open(Page::Embedded(p.to_string(), arg), f.now);
            }
        }
        (p, _) => bp.open(Page::Embedded(p.to_string(), arg), f.now),
    }
}

// ---------- The Plugins tab ----------

pub fn tab(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32) -> f32 {
    let s = f.s;
    let left = f.view.left() + f.pad;
    let mut y = top + 16.0 * s;
    let p = ui.painter().clone();
    p.text(pos2(left, y), Align2::LEFT_TOP, t!("Plugins"), w::font_bold(40.0 * s), f.t.text);
    y += 60.0 * s;
    paint::line(&p, pos2(left, y), Align2::LEFT_TOP, t!("Plugins whose page works with a controller. Add and turn on plugins in the normal window's settings."), w::font(20.0 * s), f.t.text_dim, f.view.width() - 2.0 * f.pad);
    y += 56.0 * s;
    let pages = bp.plugins.pages.iter().map(|x| (x.id.clone(), x.title.clone(), x.icon.clone(), x.name.clone(), x.description.clone())).collect::<Vec<_>>();
    let tile = vec2(560.0 * s, 150.0 * s);
    let gap = 26.0 * s;
    let cols = (((f.view.width() - 2.0 * f.pad + gap) / (tile.x + gap)).floor() as usize).max(1);
    for (i, (id, title, icon, name, description)) in pages.iter().enumerate() {
        let r = Rect::from_min_size(pos2(left + (i % cols) as f32 * (tile.x + gap), y + (i / cols) as f32 * (tile.y + gap)), tile);
        let hit = bp.focus.item(ui, Id::unique(("bp-plugin-tile", id)), r, ui.clip_rect(), Place::Page);
        let shown = paint::grow(r, 1.0 + 0.03 * hit.glow);
        paint::shadow(&p, shown, 16.0 * s, s, 0.3 + 0.6 * hit.glow, Color32::from_black_alpha(180));
        p.rect_filled(shown, paint::radius(16.0 * s), w::mix(f.t.surface.gamma_multiply(0.9), f.t.surface_hover, hit.glow));
        let ic = pos2(shown.left() + 70.0 * s, shown.center().y);
        p.circle_filled(ic, 44.0 * s, f.t.accent.gamma_multiply(0.2 + 0.2 * hit.glow));
        let glyph = if icon.is_empty() { icons::PLUGS } else { icons::by_name(icon) };
        w::icon_centered(&p, ic, glyph, 40.0 * s, f.t.accent);
        let tx = shown.left() + 138.0 * s;
        let room = shown.right() - tx - 24.0 * s;
        paint::line(&p, pos2(tx, shown.top() + 30.0 * s), Align2::LEFT_TOP, title, w::font_bold(27.0 * s), f.t.text, room);
        let sub = if description.is_empty() { name.clone() } else { format!("{name} · {description}") };
        paint::wrapped(&p, pos2(tx, shown.top() + 72.0 * s), &sub, w::font(19.0 * s), f.t.text_dim, room, 2);
        if hit.focused {
            bp.focus.ring_around(shown, 16.0 * s);
        }
        if hit.activated {
            bp.open(Page::Plugin(id.clone()), f.now);
        }
    }
    let rows = pages.len().div_ceil(cols);
    y += rows as f32 * (tile.y + gap);
    y - top + 40.0 * s
}

// ---------- A plugin's page ----------

pub fn page(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32, id: &str) -> f32 {
    let s = f.s;
    let start = f.actions.len();
    let Some((header, widgets)) = with_plugins(f.app, f.actions, |p, cx| p.bigpicture_page(id, cx)).flatten() else {
        // Turned off or gone: back to the list.
        let p = ui.painter();
        p.text(pos2(f.view.center().x, top + 200.0 * s), Align2::CENTER_CENTER, t!("This plugin isn't running"), w::font_bold(32.0 * s), f.t.text);
        return 400.0 * s;
    };
    route(bp, f, start);
    let left = f.view.left() + f.pad;
    let width = (f.view.width() - 2.0 * f.pad).min(1500.0 * s);
    let p = ui.painter().clone();
    let mut y = top + 16.0 * s;
    p.text(pos2(left, y), Align2::LEFT_TOP, &header.title, w::font_bold(40.0 * s), f.t.text);
    y += 58.0 * s;
    let by = if header.author.is_empty() { t!("an unknown author").to_string() } else { header.author.clone() };
    let mut line = t!("{name} {version} by {author}", name = header.name, version = header.version, author = by).to_string();
    if !header.permissions.is_empty() {
        line = format!("{line}  ·  {}", header.permissions.join(", "));
    }
    paint::line(&p, pos2(left, y), Align2::LEFT_TOP, &line, w::font(18.0 * s), f.t.text_faint, width);
    y += 34.0 * s;
    if let Some(e) = &header.error {
        let r = paint::wrapped(&p, pos2(left, y), &format!("{}  {e}", icons::WARNING), w::font(19.0 * s), f.t.danger, width, 3);
        y = r.bottom() + 10.0 * s;
    }
    y += 16.0 * s;

    // A picked slider: left/right move it.
    let nudge = std::mem::take(&mut bp.plugins.nudge);
    let mut d = Draw { bp, ui, f, plugin: id, hosts: &header.hosts, events: Vec::new(), seen: HashMap::new() };
    d.nudge(nudge, &widgets);
    let h = d.list(&widgets, left, y, width, 0);
    let events = std::mem::take(&mut d.events);
    y += h;

    // Nothing to focus in that direction: scroll instead (a page of text).
    if matches!(bp.focus.last_dir, Some(Dir::Up | Dir::Down)) && !bp.focus.moved {
        let step = if bp.focus.last_dir == Some(Dir::Up) { -220.0 } else { 220.0 } * s;
        bp.scroller("page").target += step;
    }
    if !events.is_empty() {
        let start = f.actions.len();
        let id = id.to_string();
        with_plugins(f.app, f.actions, |p, cx| {
            for e in events {
                p.bigpicture_event(&id, e, cx);
            }
        });
        route(bp, f, start);
    }
    y - top + 60.0 * s
}

/// Draws widgets, registering focusable ones and noting what was done.
struct Draw<'a, 'f> {
    bp: &'a mut BigPicture,
    ui: &'a Ui,
    f: &'a Frame<'f>,
    plugin: &'a str,
    hosts: &'a [String],
    events: Vec<PageEvent>,
    /// How often each widget key was used (two buttons with one id).
    seen: HashMap<String, usize>,
}

/// A widget that sits in a line with others at its own width.
fn inline(w: &Widget) -> bool {
    matches!(w, Widget::Button { .. } | Widget::Pill(_) | Widget::Badge { .. } | Widget::Link { .. } | Widget::Avatar { .. } | Widget::Sparkline(_) | Widget::Loading(_))
        || matches!(w, Widget::Tooltip { item, .. } if inline(item))
}

impl Draw<'_, '_> {
    fn s(&self) -> f32 {
        self.f.s
    }

    fn id(&mut self, kind: &str, key: &str) -> Id {
        let k = format!("{kind}:{key}");
        let n = self.seen.entry(k.clone()).or_insert(0);
        *n += 1;
        Id::unique(("bp-plugin", self.plugin.to_string(), k, *n))
    }

    /// Left/right on a picked slider: its value moves by a step each.
    fn nudge(&mut self, n: i32, widgets: &[Widget]) {
        let Some((_, slider, value)) = self.bp.plugins.adjusting.clone() else { return };
        if n == 0 {
            return;
        }
        fn find<'w>(ws: &'w [Widget], id: &str) -> Option<&'w Widget> {
            ws.iter().find_map(|w| match w {
                Widget::Slider { id: i, .. } if i == id => Some(w),
                _ => w.children().and_then(|c| find(c, id)),
            })
        }
        if let Some(Widget::Slider { min, max, step, .. }) = find(widgets, &slider) {
            let v = (value + *step * n as f64).clamp(*min, *max);
            if let Some(a) = &mut self.bp.plugins.adjusting {
                a.2 = v;
            }
        }
    }

    /// Widgets one under another; returns the height used.
    fn list(&mut self, ws: &[Widget], x: f32, y: f32, width: f32, depth: usize) -> f32 {
        let s = self.s();
        let mut h = 0.0;
        let mut i = 0;
        while i < ws.len() {
            // Inline widgets next to each other share a line.
            if inline(&ws[i]) {
                let j = ws[i..].iter().position(|w| !inline(w)).map_or(ws.len(), |k| i + k);
                h += self.flow(&ws[i..j], x, y + h, width, depth) + 18.0 * s;
                i = j;
            } else {
                h += self.block(&ws[i], x, y + h, width, depth) + 18.0 * s;
                i += 1;
            }
        }
        (h - 18.0 * s).max(0.0)
    }

    /// Inline widgets left to right, wrapping.
    fn flow(&mut self, ws: &[Widget], x: f32, y: f32, width: f32, depth: usize) -> f32 {
        let s = self.s();
        let (mut cx, mut cy, mut line_h) = (x, y, 0.0f32);
        for w in ws {
            let wd = self.natural(w);
            if cx > x && cx + wd > x + width {
                cx = x;
                cy += line_h + 14.0 * s;
                line_h = 0.0;
            }
            let h = self.block(w, cx, cy, wd.min(width), depth);
            line_h = line_h.max(h);
            cx += wd + 16.0 * s;
        }
        cy + line_h - y
    }

    fn natural(&self, w: &Widget) -> f32 {
        let s = self.s();
        let p = self.ui.painter();
        match w {
            Widget::Button { text, .. } => paint::pill_width(self.ui, false, text, s),
            Widget::Pill(text) | Widget::Badge { text, .. } => paint::text_width(p, text, w::font_semibold(19.0 * s)) + 36.0 * s,
            Widget::Link { text, .. } => paint::text_width(p, text, w::font_semibold(22.0 * s)) + 40.0 * s,
            Widget::Avatar { size, .. } => size * s * 1.25,
            Widget::Sparkline(_) => 180.0 * s,
            Widget::Loading(text) => paint::text_width(p, text, w::font(20.0 * s)) + 60.0 * s,
            Widget::Tooltip { item, .. } => self.natural(item),
            _ => 400.0 * s,
        }
    }

    /// A rounded card behind what's drawn after `slot`.
    fn backdrop(&self, slot: egui::layers::ShapeIdx, r: Rect, focused: f32) {
        let fill = w::mix(self.f.t.surface.gamma_multiply(0.85), self.f.t.surface_hover, focused);
        self.ui.painter().set(slot, egui::Shape::rect_filled(r, paint::radius(16.0 * self.s()), fill));
    }

    /// A focusable row (switch, choice, text field...): a card that glows.
    fn row(&mut self, id: Id, r: Rect) -> super::focus::Hit {
        let hit = self.bp.focus.item(self.ui, id, r, self.ui.clip_rect(), Place::Page);
        let shown = paint::grow(r, 1.0 + 0.01 * hit.glow);
        self.ui.painter().rect_filled(shown, paint::radius(14.0 * self.s()), w::mix(self.f.t.surface.gamma_multiply(0.85), self.f.t.surface_hover, hit.glow));
        if hit.focused {
            self.bp.focus.ring_around(shown, 14.0 * self.s());
        }
        hit
    }

    /// One widget at `x, y`, `width` wide; returns its height.
    fn block(&mut self, wd: &Widget, x: f32, y: f32, width: f32, depth: usize) -> f32 {
        let (s, t) = (self.s(), self.f.t.clone());
        let p = self.ui.painter().clone();
        let at = pos2(x, y);
        match wd {
            Widget::Heading(text) => paint::wrapped(&p, at, text, w::font_bold(32.0 * s), t.text, width, 3).height(),
            Widget::Text(text) => paint::wrapped(&p, at, text, w::font(22.0 * s), t.text, width, 30).height(),
            Widget::Muted(text) => paint::wrapped(&p, at, text, w::font(20.0 * s), t.text_dim, width, 30).height(),
            Widget::Separator => {
                p.hline(x..=x + width, y + 10.0 * s, egui::Stroke::new(1.5 * s, Color32::from_white_alpha(28)));
                20.0 * s
            }
            Widget::Button { id, text } => {
                let fid = self.id("button", id);
                let (r, hit) = button(self.bp, self.ui, self.f, fid, at, 0.0, None, text, Kind::Secondary, None);
                if hit.activated {
                    self.events.push(PageEvent::Click(id.clone()));
                }
                r.height()
            }
            Widget::Stat { label, value } => self.metric(x, y, width.min(380.0 * s), label, value, None, t.text_dim, ""),
            Widget::Metric { label, value, change, tone, hint } => self.metric(x, y, width.min(420.0 * s), label, value, change.as_deref(), tone_color(&t, *tone), hint),
            Widget::Progress(v) => {
                let r = Rect::from_min_size(pos2(x, y + 4.0 * s), vec2(width.min(900.0 * s), 12.0 * s));
                paint::bar(self.ui, &t, r, *v, false);
                20.0 * s
            }
            Widget::Game(app) => {
                let fid = self.id("game", &app.to_string());
                let r = Rect::from_min_size(at, vec2(460.0 * s, 215.0 * s).min(vec2(width, 215.0 * s)));
                let hit = self.bp.focus.item(self.ui, fid, r, self.ui.clip_rect(), Place::Page);
                let name = self.f.app.state.app_name(*app).map(String::from).unwrap_or_else(|| t!("App {id}", id = app).to_string());
                let shown = paint::grow(r, 1.0 + 0.05 * hit.glow);
                paint::shadow(&p, shown, 10.0 * s, s, 0.3 + 0.6 * hit.glow, Color32::from_black_alpha(200));
                if !w::paint_cover_first(self.ui, shown, &[&art::header(*app), &art::capsule(*app)], paint::radius(10.0 * s)) {
                    paint::title_card(&p, &t, shown, &name, s);
                }
                if hit.focused {
                    self.bp.focus.ring_around(shown, 10.0 * s);
                    self.bp.bg_want = Some(*app);
                }
                if hit.activated {
                    self.bp.open(Page::Game(*app), self.f.now);
                }
                r.height()
            }
            Widget::Input { id, value, hint } => {
                let fid = self.id("input", id);
                let r = Rect::from_min_size(at, vec2(width.min(1000.0 * s), 76.0 * s));
                let hit = self.row(fid, r);
                p.text(pos2(r.left() + 24.0 * s, r.center().y), Align2::LEFT_CENTER, icons::NOTE, w::font(24.0 * s), t.text_dim);
                let (shown, color) = if value.is_empty() { (hint.as_str(), t.text_faint) } else { (value.as_str(), t.text) };
                paint::line(&p, pos2(r.left() + 64.0 * s, r.center().y), Align2::LEFT_CENTER, shown, w::font(23.0 * s), color, r.width() - 90.0 * s);
                if hit.activated {
                    self.bp.plugins.text = value.clone();
                    self.bp.plugins.submit = false;
                    self.bp.open(Page::PluginText { plugin: self.plugin.to_string(), field: id.clone(), hint: hint.clone() }, self.f.now);
                }
                r.height()
            }
            Widget::Toggle { id, text, value } => {
                let fid = self.id("toggle", id);
                let r = Rect::from_min_size(at, vec2(width.min(1000.0 * s), 76.0 * s));
                let hit = self.row(fid, r);
                let k = self.ui.ctx().animate_bool_with_time(fid.with("switch"), *value, 0.14);
                let track = Rect::from_center_size(pos2(r.right() - 60.0 * s, r.center().y), vec2(72.0 * s, 38.0 * s));
                p.rect_filled(track, paint::radius(19.0 * s), w::mix(t.raised, t.accent, k));
                p.circle_filled(pos2(track.left() + 19.0 * s + k * 34.0 * s, track.center().y), 14.0 * s, Color32::WHITE);
                paint::line(&p, pos2(r.left() + 26.0 * s, r.center().y), Align2::LEFT_CENTER, text, w::font_semibold(23.0 * s), t.text, track.left() - r.left() - 50.0 * s);
                if hit.activated {
                    self.events.push(PageEvent::Toggle(id.clone(), !*value));
                }
                r.height()
            }
            Widget::Select { id, label, options, value } => {
                let fid = self.id("select", id);
                let r = Rect::from_min_size(at, vec2(width.min(1000.0 * s), 76.0 * s));
                let hit = self.row(fid, r);
                let shown = options.iter().find(|(v, _)| v == value).map(|o| o.1.as_str()).unwrap_or("");
                let vr = paint::line(&p, pos2(r.right() - 52.0 * s, r.center().y), Align2::RIGHT_CENTER, shown, w::font_medium(23.0 * s), t.accent, r.width() * 0.45);
                w::icon_centered(&p, pos2(r.right() - 30.0 * s, r.center().y), icons::CARET_RIGHT, 20.0 * s, t.accent);
                paint::line(&p, pos2(r.left() + 26.0 * s, r.center().y), Align2::LEFT_CENTER, label, w::font_semibold(23.0 * s), t.text, vr.left() - r.left() - 46.0 * s);
                // A picks the next option, like Big Picture's settings.
                if hit.activated && !options.is_empty() {
                    let i = options.iter().position(|(v, _)| v == value).map_or(0, |i| (i + 1) % options.len());
                    self.events.push(PageEvent::Select(id.clone(), options[i].0.clone()));
                }
                r.height()
            }
            Widget::Slider { id, label, min, max, value, .. } => {
                let fid = self.id("slider", id);
                let r = Rect::from_min_size(at, vec2(width.min(1000.0 * s), 96.0 * s));
                let hit = self.row(fid, r);
                let picked = self.bp.plugins.adjusting.as_ref().filter(|a| a.0 == fid).map(|a| a.2);
                let v = picked.unwrap_or(*value);
                paint::line(&p, pos2(r.left() + 26.0 * s, r.top() + 30.0 * s), Align2::LEFT_CENTER, label, w::font_semibold(23.0 * s), t.text, r.width() * 0.6);
                let note = if picked.is_some() { format!("◀  {}  ▶", plugins::format_number(v)) } else { plugins::format_number(v) };
                p.text(pos2(r.right() - 26.0 * s, r.top() + 30.0 * s), Align2::RIGHT_CENTER, note, w::font_medium(23.0 * s), t.accent);
                let track = Rect::from_min_size(pos2(r.left() + 26.0 * s, r.bottom() - 30.0 * s), vec2(r.width() - 52.0 * s, 10.0 * s));
                let frac = if max > min { ((v - min) / (max - min)) as f32 } else { 0.0 };
                paint::bar(self.ui, &t, track, frac, false);
                p.circle_filled(pos2(track.left() + track.width() * frac, track.center().y), if picked.is_some() { 14.0 } else { 10.0 } * s, Color32::WHITE);
                if hit.activated {
                    match self.bp.plugins.adjusting.take() {
                        // A again: done, tell the plugin.
                        Some((f_id, slider, v)) if f_id == fid => {
                            if v != *value {
                                self.events.push(PageEvent::Slider(slider, v));
                            }
                        }
                        _ => self.bp.plugins.adjusting = Some((fid, id.clone(), *value)),
                    }
                } else if picked.is_some() && !hit.focused {
                    // Moved away: keep what was set.
                    if let Some((_, slider, v)) = self.bp.plugins.adjusting.take() {
                        if v != *value {
                            self.events.push(PageEvent::Slider(slider, v));
                        }
                    }
                }
                r.height()
            }
            Widget::Tabs { id, items, value } => {
                let mut cx = x;
                let h = 62.0 * s;
                for (v, label) in items {
                    let fid = self.id("tab", &format!("{id}/{v}"));
                    let wd = paint::text_width(&p, label, w::font_bold(22.0 * s)) + 48.0 * s;
                    let r = Rect::from_min_size(pos2(cx, y), vec2(wd, h));
                    cx += wd + 12.0 * s;
                    let hit = self.bp.focus.item(self.ui, fid, r, self.ui.clip_rect(), Place::Page);
                    let on = v == value;
                    let fill = if on { t.accent } else { w::mix(Color32::from_white_alpha(14), Color32::from_white_alpha(34), hit.glow) };
                    p.rect_filled(r, paint::radius(h / 2.0), fill);
                    p.text(r.center(), Align2::CENTER_CENTER, label, w::font_bold(22.0 * s), if on { t.on_accent } else { t.text });
                    if hit.focused {
                        self.bp.focus.ring_around(r, h / 2.0);
                    }
                    if hit.activated && !on {
                        self.events.push(PageEvent::Select(id.clone(), v.clone()));
                    }
                }
                h
            }
            Widget::Link { text, page, arg } => {
                let fid = self.id("link", &format!("{page}/{}", arg.clone().unwrap_or_default()));
                let wd = self.natural(wd);
                let r = Rect::from_min_size(at, vec2(wd, 56.0 * s));
                let hit = self.bp.focus.item(self.ui, fid, r, self.ui.clip_rect(), Place::Page);
                p.rect_filled(r, paint::radius(12.0 * s), Color32::from_white_alpha((28.0 * hit.glow) as u8));
                let tr = p.text(r.center(), Align2::CENTER_CENTER, text, w::font_semibold(22.0 * s), t.accent);
                p.hline(tr.x_range(), tr.bottom() + 2.0 * s, egui::Stroke::new(1.5 * s, t.accent.gamma_multiply(0.6)));
                if hit.focused {
                    self.bp.focus.ring_around(r, 12.0 * s);
                }
                let allowed = ["store", "library", "community", "profile", "downloads", "settings"];
                if hit.activated && (allowed.contains(&page.as_str()) || page.starts_with("plugin:")) {
                    open_target(self.bp, self.f, page, arg.clone());
                }
                r.height()
            }
            Widget::Pill(text) => self.badge(x, y, text, t.text_dim, t.raised),
            Widget::Badge { text, tone } => {
                let c = tone_color(&t, *tone);
                self.badge(x, y, text, c, c.gamma_multiply(0.18))
            }
            Widget::Table { columns, rows } => {
                let pad = 24.0 * s;
                let row_h = 54.0 * s;
                let n = columns.len().max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
                let col = (width - 2.0 * pad) / n as f32;
                let mut cy = y + 12.0 * s;
                let slot = p.add(egui::Shape::Noop);
                if !columns.is_empty() {
                    for (i, c) in columns.iter().enumerate() {
                        paint::line(&p, pos2(x + pad + i as f32 * col, cy + row_h / 2.0), Align2::LEFT_CENTER, &c.to_uppercase(), w::font_bold(16.0 * s), t.text_faint, col - 12.0 * s);
                    }
                    cy += row_h;
                }
                for (ri, row) in rows.iter().enumerate() {
                    // Each row takes focus, so a controller can step down a long table.
                    let fid = self.id("row", &ri.to_string());
                    let rr = Rect::from_min_size(pos2(x + 8.0 * s, cy), vec2(width - 16.0 * s, row_h));
                    let hit = self.bp.focus.item(self.ui, fid, rr, self.ui.clip_rect(), Place::Page);
                    if ri % 2 == 1 || hit.glow > 0.0 {
                        p.rect_filled(rr, paint::radius(10.0 * s), w::mix(Color32::from_white_alpha(8), Color32::from_white_alpha(30), hit.glow));
                    }
                    if hit.focused {
                        self.bp.focus.ring_around(rr, 10.0 * s);
                    }
                    for (i, cell) in row.iter().enumerate().take(n) {
                        paint::line(&p, pos2(x + pad + i as f32 * col, cy + row_h / 2.0), Align2::LEFT_CENTER, cell, w::font(21.0 * s), t.text, col - 12.0 * s);
                    }
                    cy += row_h;
                }
                let r = Rect::from_min_max(at, pos2(x + width, cy + 12.0 * s));
                self.backdrop(slot, r, 0.0);
                r.height()
            }
            Widget::Image { url, width: iw, height: ih } => {
                let size = vec2((iw * s).min(width), ih * s);
                let r = Rect::from_min_size(at, size);
                if image_allowed(url, self.hosts) {
                    w::image_or_skeleton(self.ui, &t, r, url, paint::radius(10.0 * s));
                } else {
                    p.rect_filled(r, paint::radius(10.0 * s), t.raised);
                    p.text(r.center(), Align2::CENTER_CENTER, t!("Image not allowed"), w::font(18.0 * s), t.text_faint);
                }
                r.height()
            }
            Widget::Row(items) => {
                if depth + 1 >= plugins::MAX_WIDGET_DEPTH {
                    return 0.0;
                }
                if items.iter().all(inline) {
                    self.flow(items, x, y, width, depth + 1)
                } else {
                    self.columns(items, x, y, width, items.len().max(1), depth + 1)
                }
            }
            Widget::Grid { columns, items } => self.columns(items, x, y, width, *columns, depth + 1),
            Widget::Card { title, items } => {
                let pad = 28.0 * s;
                let slot = p.add(egui::Shape::Noop);
                let mut h = pad;
                if !title.is_empty() {
                    h += paint::wrapped(&p, pos2(x + pad, y + h), title, w::font_bold(26.0 * s), t.text, width - 2.0 * pad, 2).height() + 16.0 * s;
                }
                if depth + 1 < plugins::MAX_WIDGET_DEPTH {
                    h += self.list(items, x + pad, y + h, width - 2.0 * pad, depth + 1);
                }
                h += pad;
                self.backdrop(slot, Rect::from_min_size(at, vec2(width, h)), 0.0);
                h
            }
            Widget::Section { title, items } => {
                let mut h = 0.0;
                if !title.is_empty() {
                    h += paint::wrapped(&p, at, title, w::font_bold(28.0 * s), t.text, width, 2).height() + 14.0 * s;
                }
                if depth + 1 < plugins::MAX_WIDGET_DEPTH {
                    h += self.list(items, x, y + h, width, depth + 1);
                }
                h
            }
            Widget::Avatar { steam_id, size } => {
                let d = size * s * 1.25;
                let r = Rect::from_min_size(at, vec2(d, d));
                let hash = self.f.app.state.personas.get(steam_id).and_then(|x| x.avatar_hash.clone());
                if !w::paint_cover(self.ui, r, &art::avatar_full(hash.as_deref()), paint::radius(d / 2.0)) {
                    p.circle_filled(r.center(), d / 2.0, t.raised);
                }
                d
            }
            Widget::Empty { icon, title, text } => {
                let c = pos2(x + width / 2.0, y);
                let glyph = if icon.is_empty() { icons::PLUGS } else { icons::by_name(icon) };
                p.text(c + vec2(0.0, 60.0 * s), Align2::CENTER_CENTER, glyph, w::font(80.0 * s), t.text_faint);
                p.text(c + vec2(0.0, 140.0 * s), Align2::CENTER_CENTER, title, w::font_bold(30.0 * s), t.text);
                let mut job = egui::text::LayoutJob::simple(text.clone(), w::font(21.0 * s), t.text_dim, width * 0.7);
                job.halign = egui::Align::Center;
                let galley = p.layout_job(job);
                let h = galley.size().y;
                p.galley(pos2(x + width / 2.0, y + 172.0 * s), galley, t.text_dim);
                172.0 * s + h + 20.0 * s
            }
            Widget::Loading(text) => {
                let c = pos2(x + 20.0 * s, y + 24.0 * s);
                let time = self.f.now as f32;
                for i in 0..8 {
                    let a = i as f32 / 8.0 * std::f32::consts::TAU + time * 4.0;
                    let alpha = ((i as f32 / 8.0) * 255.0) as u8;
                    p.circle_filled(c + vec2(a.cos(), a.sin()) * 14.0 * s, 3.5 * s, t.accent.gamma_multiply(alpha as f32 / 255.0));
                }
                p.text(pos2(x + 52.0 * s, c.y), Align2::LEFT_CENTER, text, w::font(20.0 * s), t.text_dim);
                48.0 * s
            }
            Widget::Chart { kind, values, labels, height } => {
                let pad = 24.0 * s;
                let h = height * s;
                let slot = p.add(egui::Shape::Noop);
                let plot = Rect::from_min_size(pos2(x + pad, y + pad), vec2(width.min(1300.0 * s) - 2.0 * pad, h));
                paint_chart(&p, plot, *kind, values, t.accent, Color32::from_white_alpha(30), 3.0 * s);
                let mut bottom = plot.bottom() + pad;
                if let (Some(first), Some(last)) = (labels.first(), labels.last()) {
                    p.text(pos2(plot.left(), plot.bottom() + 10.0 * s), Align2::LEFT_TOP, first, w::font(17.0 * s), t.text_faint);
                    if labels.len() > 1 {
                        p.text(pos2(plot.right(), plot.bottom() + 10.0 * s), Align2::RIGHT_TOP, last, w::font(17.0 * s), t.text_faint);
                    }
                    bottom += 30.0 * s;
                }
                if *kind == ChartKind::Line {
                    if let Some(v) = values.last() {
                        p.text(pos2(plot.right(), plot.top()), Align2::RIGHT_TOP, plugins::format_number(*v), w::font_bold(20.0 * s), t.text);
                    }
                }
                let r = Rect::from_min_max(at, pos2(plot.right() + pad, bottom));
                self.backdrop(slot, r, 0.0);
                r.height()
            }
            Widget::Sparkline(values) => {
                let r = Rect::from_min_size(at, vec2(180.0 * s, 48.0 * s));
                paint_chart(&p, r.shrink(4.0 * s), ChartKind::Line, values, t.accent, Color32::TRANSPARENT, 2.5 * s);
                r.height()
            }
            Widget::Tooltip { text, item } => {
                let h = self.block(item, x, y, width, depth + 1);
                // Its text shows under the widget while that's focused.
                let area = Rect::from_min_size(at, vec2(width, h));
                if !text.is_empty() && self.bp.focus.focused().is_some_and(|(r, _)| area.contains(r.center())) {
                    let slot = p.add(egui::Shape::Noop);
                    let tip = paint::wrapped(&p, pos2(x + 12.0 * s, y + h + 14.0 * s), text, w::font(18.0 * s), t.text, 700.0 * s, 3).expand(10.0 * s);
                    p.set(slot, egui::Shape::rect_filled(tip, paint::radius(8.0 * s), t.chrome));
                }
                h
            }
        }
    }

    fn metric(&mut self, x: f32, y: f32, width: f32, label: &str, value: &str, change: Option<&str>, tone: Color32, hint: &str) -> f32 {
        let (s, t) = (self.s(), self.f.t.clone());
        let p = self.ui.painter().clone();
        let h = if hint.is_empty() { 132.0 } else { 162.0 } * s;
        let r = Rect::from_min_size(pos2(x, y), vec2(width, h));
        p.rect_filled(r, paint::radius(16.0 * s), t.surface.gamma_multiply(0.85));
        paint::line(&p, pos2(r.left() + 26.0 * s, r.top() + 24.0 * s), Align2::LEFT_TOP, &label.to_uppercase(), w::font_bold(16.0 * s), t.text_faint, width - 52.0 * s);
        let vr = paint::line(&p, pos2(r.left() + 26.0 * s, r.top() + 54.0 * s), Align2::LEFT_TOP, value, w::font_bold(40.0 * s), t.text, width - 52.0 * s);
        if let Some(c) = change {
            p.text(pos2(vr.right() + 16.0 * s, vr.center().y), Align2::LEFT_CENTER, c, w::font_semibold(22.0 * s), tone);
        }
        if !hint.is_empty() {
            paint::line(&p, pos2(r.left() + 26.0 * s, r.bottom() - 40.0 * s), Align2::LEFT_TOP, hint, w::font(18.0 * s), t.text_dim, width - 52.0 * s);
        }
        h
    }

    fn badge(&self, x: f32, y: f32, text: &str, fg: Color32, bg: Color32) -> f32 {
        let s = self.s();
        let p = self.ui.painter();
        let wd = paint::text_width(p, text, w::font_semibold(19.0 * s)) + 36.0 * s;
        let r = Rect::from_min_size(pos2(x, y), vec2(wd, 40.0 * s));
        p.rect_filled(r, paint::radius(20.0 * s), bg);
        p.text(r.center(), Align2::CENTER_CENTER, text, w::font_semibold(19.0 * s), fg);
        r.height()
    }

    /// Widgets in `n` equal columns, wrapping into rows.
    fn columns(&mut self, items: &[Widget], x: f32, y: f32, width: f32, n: usize, depth: usize) -> f32 {
        if depth >= plugins::MAX_WIDGET_DEPTH {
            return 0.0;
        }
        let s = self.s();
        let n = n.clamp(1, 6);
        let gap = 22.0 * s;
        let col = (width - gap * (n - 1) as f32) / n as f32;
        let mut h = 0.0;
        for chunk in items.chunks(n) {
            let mut line = 0.0f32;
            for (i, item) in chunk.iter().enumerate() {
                line = line.max(self.list(std::slice::from_ref(item), x + i as f32 * (col + gap), y + h, col, depth));
            }
            h += line + gap;
        }
        (h - gap).max(0.0)
    }
}

// ---------- Typing into a plugin's text field ----------

pub fn text_page(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, top: f32, plugin: &str, field: &str, hint: &str) -> f32 {
    let s = f.s;
    let left = f.view.left() + f.pad;
    let mut y = top + 16.0 * s;
    // X deletes, Y types a space (on a controller).
    if f.x {
        bp.plugins.text.pop();
    }
    if f.y {
        bp.plugins.text.push(' ');
    }
    let p = ui.painter().clone();
    let field_r = Rect::from_min_size(pos2(left, y), vec2((f.view.width() - 2.0 * f.pad).min(1100.0 * s), 84.0 * s));
    p.rect_filled(field_r, paint::radius(20.0 * s), Color32::from_black_alpha(120));
    p.rect_stroke(field_r, paint::radius(20.0 * s), egui::Stroke::new(2.0 * s, f.t.accent.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    let tx = field_r.left() + 30.0 * s;
    let room = field_r.right() - tx - 40.0 * s;
    let end = if bp.plugins.text.is_empty() {
        p.text(pos2(tx, field_r.center().y), Align2::LEFT_CENTER, if hint.is_empty() { t!("Type here") } else { hint }, w::font(30.0 * s), f.t.text_faint);
        tx
    } else {
        let galley = p.layout_no_wrap(bp.plugins.text.clone(), w::font_semibold(32.0 * s), f.t.text);
        let gx = tx - (galley.size().x - room).max(0.0);
        let rect = Align2::LEFT_CENTER.anchor_size(pos2(gx, field_r.center().y), galley.size());
        p.with_clip_rect(Rect::from_min_max(pos2(tx, field_r.top()), pos2(tx + room + 8.0 * s, field_r.bottom()))).galley(rect.min, galley, f.t.text);
        rect.right()
    };
    let blink = ((f.now * 3.0).sin() as f32 * 0.5 + 0.5).powf(0.6);
    p.vline(end + 4.0 * s, (field_r.center().y - 20.0 * s)..=(field_r.center().y + 20.0 * s), egui::Stroke::new(3.0 * s, f.t.accent.gamma_multiply(blink)));
    y += 84.0 * s + 36.0 * s;
    let mut done = std::mem::take(&mut bp.plugins.submit);
    match keyboard(bp, ui, f, left, &mut y, t!("Done")) {
        Some(Key::Char(ch)) => bp.plugins.text.push(ch),
        Some(Key::Del) => {
            bp.plugins.text.pop();
        }
        Some(Key::Go) => done = true,
        None => {}
    }
    bp.preferred = Some(Id::unique(("bp-key", 1, 0)));
    if done {
        let text: String = bp.plugins.text.chars().take(2000).collect();
        let (plugin, field) = (plugin.to_string(), field.to_string());
        let start = f.actions.len();
        with_plugins(f.app, f.actions, |p, cx| p.bigpicture_event(&plugin, PageEvent::Input(field, text), cx));
        bp.back(f.now);
        route(bp, f, start);
    }
    y - top + 40.0 * s
}

// ---------- Plugin buttons on game and friend pages ----------

/// Plugin buttons for a game, in a row from `(x, y)`. Returns the row's
/// height (0 without any).
pub fn game_buttons(bp: &mut BigPicture, ui: &Ui, f: &mut Frame, x: f32, y: f32, app_id: AppId) -> f32 {
    let items = with_plugins(f.app, f.actions, |p, cx| p.game_items(app_id, cx)).unwrap_or_default();
    let clicked = item_row(bp, ui, f, x, y, &items, "game");
    if let Some(item) = clicked {
        let start = f.actions.len();
        with_plugins(f.app, f.actions, |p, cx| p.game_item_clicked(&item.plugin, &item.id, app_id, cx));
        route(bp, f, start);
    }
    if items.is_empty() { 0.0 } else { 72.0 * f.s }
}

/// Plugin items for a friend (their right-click menu in the normal window).
pub fn friend_buttons(bp: &mut BigPicture, ui: &Ui, f: &mut Frame, x: f32, y: f32, steam_id: SteamId) -> f32 {
    let items = with_plugins(f.app, f.actions, |p, cx| p.friend_items(steam_id, cx)).unwrap_or_default();
    let clicked = item_row(bp, ui, f, x, y, &items, "friend");
    if let Some(item) = clicked {
        let start = f.actions.len();
        with_plugins(f.app, f.actions, |p, cx| p.friend_item_clicked(&item.plugin, &item.id, steam_id, cx));
        route(bp, f, start);
    }
    if items.is_empty() { 0.0 } else { 72.0 * f.s }
}

fn item_row<'a>(bp: &mut BigPicture, ui: &Ui, f: &Frame, x: f32, y: f32, items: &'a [PluginItem], salt: &str) -> Option<&'a PluginItem> {
    let s = f.s;
    let mut cx = x;
    let mut out = None;
    let right = f.view.right() - f.pad;
    let mut cy = y;
    for item in items {
        let wd = paint::pill_width(ui, true, &item.text, s);
        if cx > x && cx + wd > right {
            cx = x;
            cy += 72.0 * s + 16.0 * s;
        }
        let (r, hit) = button(bp, ui, f, Id::unique(("bp-plugin-item", salt, item.plugin.as_str(), item.id.as_str())), pos2(cx, cy), 0.0, Some(icons::PLUGS), &item.text, Kind::Secondary, None);
        cx = r.right() + 20.0 * s;
        if hit.focused {
            // Whose button it is.
            paint::line(ui.painter(), pos2(r.left() + 8.0 * s, r.bottom() + 8.0 * s), Align2::LEFT_TOP, &t!("From the {name} plugin", name = item.plugin_name), w::font(16.0 * s), f.t.text_faint, 600.0 * s);
        }
        if hit.activated {
            out = Some(item);
        }
    }
    out
}

//! System tray icon. Right-click opens our own menu window (styled like the
//! app) instead of the plain Windows menu.
//!
//! The menu window is created once and then kept alive hidden, so it opens
//! instantly; it repaints on its own (it doesn't wait for the main window),
//! and it sizes itself to its contents so every item, Exit included, shows
//! at any UI zoom.

use crate::app::SteamApp;
use egui::{Align2, CornerRadius, Pos2, Sense, Stroke, Ui, ViewportBuilder, ViewportId, pos2, vec2};
use i18n::t;
use model::{Persona, PersonaState};
use std::sync::{Arc, Mutex, mpsc};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use ui::{Theme, icons, widgets as w};

#[derive(Debug, Clone, Copy)]
pub enum TrayCmd {
    Show,
    /// Right-click at this screen position (physical pixels).
    Menu(f64, f64),
    Page(&'static str),
    Friends,
    Status(PersonaState),
    Exit,
}

const MENU_W: f32 = 270.0;
const MENU_ID: &str = "tray-menu";

/// What the menu shows, copied from the app each frame.
#[derive(Clone, Default)]
struct MenuData {
    signed_in: bool,
    name: String,
    me: Persona,
    online: usize,
    download: Option<String>,
    theme: Theme,
}

/// Shared between the app and the menu window (which paints on its own).
#[derive(Default)]
struct MenuState {
    open: bool,
    /// Bottom-right corner, at the cursor (logical points).
    anchor: Pos2,
    had_focus: bool,
    /// Measured content height.
    height: f32,
    data: MenuData,
}

pub struct Tray {
    _icon: Option<TrayIcon>,
    rx: mpsc::Receiver<TrayCmd>,
    tx: Arc<Mutex<mpsc::Sender<TrayCmd>>>,
    menu: Arc<Mutex<MenuState>>,
    /// The menu window exists (created on first use, then kept hidden).
    created: bool,
}

/// `logo` drawn at the size the tray shows it at, so it isn't blurry.
fn tray_picture(logo: ui::brand::Logo) -> Option<tray_icon::Icon> {
    let size = crate::window_icon::tray_size();
    tray_icon::Icon::from_rgba(logo.rgba(size), size, size).ok()
}

impl Tray {
    pub fn new(ctx: &egui::Context, logo: ui::brand::Logo) -> Self {
        let (tx, rx) = mpsc::channel();
        let icon = TrayIconBuilder::new()
            .with_tooltip("SteamInRust")
            .with_icon(tray_picture(logo).expect("valid icon"))
            .build()
            .map_err(|e| tracing::warn!("tray icon unavailable: {e}"))
            .ok();
        let (events_tx, repaint) = (tx.clone(), ctx.clone());
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            let cmd = match e {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => Some(TrayCmd::Show),
                TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => Some(TrayCmd::Show),
                TrayIconEvent::Click { button: MouseButton::Right, button_state: MouseButtonState::Up, position, .. } => Some(TrayCmd::Menu(position.x, position.y)),
                _ => None,
            };
            if let Some(cmd) = cmd {
                let _ = events_tx.send(cmd);
                repaint.request_repaint();
            }
        }));
        let menu = MenuState { height: 400.0, ..Default::default() };
        Self { _icon: icon, rx, tx: Arc::new(Mutex::new(tx)), menu: Arc::new(Mutex::new(menu)), created: false }
    }

    /// Show another version of the logo (picked in Settings → Appearance).
    pub fn set_icon(&self, logo: ui::brand::Logo) {
        let (Some(tray), Some(icon)) = (&self._icon, tray_picture(logo)) else {
            tracing::warn!("tray icon not updated: no tray icon");
            return;
        };
        match tray.set_icon(Some(icon)) {
            Ok(()) => tracing::info!("tray icon: {}", logo.key()),
            Err(e) => tracing::warn!("tray icon not updated: {e}"),
        }
    }

    /// Open the menu with its bottom-right corner at `at` (for previews).
    pub fn open_menu(&self, at: Pos2) {
        let mut m = self.menu.lock().unwrap();
        m.anchor = at;
        m.open = true;
        m.had_focus = false;
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<TrayCmd> {
        let mut out = Vec::new();
        for cmd in self.rx.try_iter() {
            match cmd {
                TrayCmd::Menu(x, y) => {
                    // The cursor is in screen pixels; window positions are in
                    // points times the app's zoom (the theme's size included).
                    let ppp = ctx.native_pixels_per_point().unwrap_or(1.0) * ctx.zoom_factor();
                    let mut m = self.menu.lock().unwrap();
                    m.anchor = pos2(x as f32 / ppp, y as f32 / ppp);
                    m.open = true;
                    m.had_focus = false;
                    ctx.request_repaint_of(ViewportId::from_hash_of(MENU_ID));
                }
                other => out.push(other),
            }
        }
        out
    }
}

/// Keep the menu window alive (hidden when closed) and feed it fresh data.
pub fn show(app: &mut SteamApp, ctx: &egui::Context) {
    let open = {
        let mut m = app.tray.menu.lock().unwrap();
        m.data = snapshot(app);
        m.open
    };
    // Created hidden right away, so even the first right-click is instant.
    app.tray.created = true;
    let (pos, size) = {
        let m = app.tray.menu.lock().unwrap();
        let size = vec2(MENU_W, m.height);
        (pos2((m.anchor.x - size.x).max(0.0), (m.anchor.y - size.y).max(0.0)), size)
    };
    let builder = ViewportBuilder::default()
        .with_title("SteamInRust")
        .with_decorations(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_visible(open)
        .with_position(pos)
        .with_inner_size(size);
    let (menu, tx) = (app.tray.menu.clone(), app.tray.tx.clone());
    ctx.show_viewport_deferred(ViewportId::from_hash_of(MENU_ID), builder, move |ui, _| {
        let mut m = menu.lock().unwrap();
        if !m.open {
            return;
        }
        let focused = ui.input(|i| i.viewport().focused);
        if !m.had_focus {
            if focused == Some(true) {
                m.had_focus = true;
            } else {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                ui.ctx().request_repaint();
            }
        }
        let mut close = (m.had_focus && focused == Some(false)) || ui.input(|i| i.key_pressed(egui::Key::Escape));
        let data = m.data.clone();
        let t = data.theme.clone();
        let mut picked = None;
        let mut used = 0.0;
        // Fill the whole window (Windows rounds its corners itself); a rounded
        // shape inside a see-through window left light edges at the corners.
        // The theme's menu style: its fill, border and padding.
        let menu_c = &t.components.menu;
        let look = t.look(menu_c, ui::theme::Interact::default());
        let pad = t.padding_of(menu_c);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(w::solid_fill(&t, menu_c))).show(ui, |ui| {
            let rect = ui.max_rect();
            let border = if look.border.a() > 0 && menu_c.border > 0.0 { look.border } else { t.border };
            ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(menu_c.border.max(1.0), border), egui::StrokeKind::Inside);
            let inner = rect.shrink2(vec2(pad.x.max(6.0), pad.y.max(6.0)));
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
            picked = contents(&data, &mut c, &t);
            used = c.min_rect().height() + pad.y.max(6.0) * 2.0 + 4.0;
        });
        // Fit the window to what was drawn (UI zoom, languages…).
        if (used - m.height).abs() > 1.0 {
            m.height = used;
            ui.ctx().request_repaint_of(ViewportId::ROOT);
        }
        if let Some(cmd) = picked {
            let _ = tx.lock().unwrap().send(cmd);
            close = true;
        }
        if close {
            m.open = false;
            ui.ctx().request_repaint_of(ViewportId::ROOT);
        }
    });
}

fn snapshot(app: &SteamApp) -> MenuData {
    let me = app.state.persona(app.state.steam_id);
    MenuData {
        signed_in: app.state.steam_id != 0,
        name: if app.state.persona_name.is_empty() { me.name.clone() } else { app.state.persona_name.clone() },
        me,
        online: app.state.friend_list().iter().filter(|p| p.state != PersonaState::Offline).count(),
        download: app.state.downloads.values().find(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).map(|d| {
            let pct = d.disk_done as f64 * 100.0 / d.disk_total.max(1) as f64;
            format!("{pct:.0}%")
        }),
        theme: app.prefs.theme.clone(),
    }
}

fn contents(d: &MenuData, ui: &mut Ui, t: &Theme) -> Option<TrayCmd> {
    let mut picked = None;
    let item = &t.components.menu_item;
    let item_pad = t.padding_of(item).x;
    // Who's signed in.
    ui.horizontal(|ui| {
        ui.add_space(item_pad * 0.5);
        if d.signed_in {
            w::avatar(ui, t, &model::art::avatar(d.me.avatar_hash.as_deref()), 34.0, Some(crate::shell::status_color(t, &d.me)));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(egui::RichText::new(&d.name).font(t.font_of(item)).color(t.text).strong());
                ui.label(w::body_s(t, i18n::tr_dyn(d.me.state.label()), crate::shell::status_color(t, &d.me)));
            });
        } else {
            w::logo(ui, t, 26.0);
            ui.label(egui::RichText::new("SteamInRust").font(t.font_of(item)).color(t.text).strong());
        }
    });
    if d.signed_in {
        ui.add_space(t.spacing.sm);
        // Statuses in a 2×2 grid, in the theme's chip style.
        let chip = &t.components.chip;
        let states = [(PersonaState::Online, t.online), (PersonaState::Away, t.away), (PersonaState::Busy, t.danger), (PersonaState::Invisible, t.text_faint)];
        for pair in states.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.xs;
                let cell = ((ui.available_width() - t.spacing.xs) / 2.0).floor();
                for (s, color) in pair {
                    let on = d.me.state == *s;
                    let (r, resp) = ui.allocate_exact_size(vec2(cell, t.height_of(chip, 28.0).min(30.0)), Sense::click());
                    let look = t.look(chip, w::interact(ui, &resp, on));
                    w::paint_box(ui, t, r, chip, &look, if on { 1.0 } else { 0.0 });
                    crate::shell::status_mark(ui.painter(), pos2(r.left() + 12.0, r.center().y), 4.0, *s, *color);
                    let label = i18n::tr_dyn(s.label());
                    let font = t.font_of(chip);
                    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), font, look.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 30.0).max(10.0));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(r.left() + 22.0, r.center().y - g.size().y / 2.0), g, look.text);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        picked = Some(TrayCmd::Status(*s));
                    }
                }
            });
        }
    }
    ui.add_space(t.spacing.sm);
    separator(ui, t);
    let items: [(&str, &str, TrayCmd); 7] = [
        (icons::STORE, t!("Store"), TrayCmd::Page("store")),
        (icons::BOOKS, t!("Library"), TrayCmd::Page("library")),
        (icons::CHATS, t!("Community"), TrayCmd::Page("community")),
        (icons::USERS, t!("Friends"), TrayCmd::Friends),
        (icons::USER_CIRCLE, t!("Profile"), TrayCmd::Page("profile")),
        (icons::DOWNLOAD, t!("Downloads"), TrayCmd::Page("downloads")),
        (icons::GEAR, t!("Settings"), TrayCmd::Page("settings")),
    ];
    for (icon, label, cmd) in items {
        let extra = match cmd {
            TrayCmd::Friends => (d.online > 0).then(|| t!("{n} online", n = d.online)),
            TrayCmd::Page("downloads") => d.download.clone(),
            _ => None,
        };
        let resp = w::menu_item(ui, t, icon, label);
        if let Some(extra) = extra {
            let look = t.look(item, ui::theme::Interact::default());
            ui.painter().text(pos2(resp.rect.right() - item_pad, resp.rect.center().y), Align2::RIGHT_CENTER, extra, t.font_of(&t.components.chip), look.icon);
        }
        if resp.clicked() {
            picked = Some(cmd);
        }
    }
    separator(ui, t);
    if w::menu_item_tinted(ui, t, icons::POWER, t!("Exit SteamInRust"), Some(t.danger)).clicked() {
        picked = Some(TrayCmd::Exit);
    }
    picked
}

fn separator(ui: &mut Ui, t: &Theme) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.spacing.sm * 2.0 + 1.0), Sense::hover());
    ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0, t.border));
}

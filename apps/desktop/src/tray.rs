//! System tray icon. Right-click opens our own menu window (styled like the
//! app) instead of the plain Windows menu.
//!
//! The menu window is created once and then kept alive hidden, so it opens
//! instantly; it repaints on its own (it doesn't wait for the main window),
//! and it sizes itself to its contents so every item, Exit included, shows
//! at any UI zoom.

use crate::app::SteamApp;
use egui::{
    Align2, Color32, CornerRadius, Pos2, Sense, Stroke, Ui, Vec2, ViewportBuilder, ViewportId,
    pos2, vec2,
};
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

impl Tray {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let icon = TrayIconBuilder::new()
            .with_tooltip("SteamInRust")
            .with_icon(
                tray_icon::Icon::from_rgba(ui::brand::icon_rgba(32), 32, 32).expect("valid icon"),
            )
            .build()
            .map_err(|e| tracing::warn!("tray icon unavailable: {e}"))
            .ok();
        let (events_tx, repaint) = (tx.clone(), ctx.clone());
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            let cmd = match e {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => Some(TrayCmd::Show),
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => Some(TrayCmd::Show),
                TrayIconEvent::Click {
                    button: MouseButton::Right,
                    button_state: MouseButtonState::Up,
                    position,
                    ..
                } => Some(TrayCmd::Menu(position.x, position.y)),
                _ => None,
            };
            if let Some(cmd) = cmd {
                let _ = events_tx.send(cmd);
                repaint.request_repaint();
            }
        }));
        let menu = MenuState {
            height: 400.0,
            ..Default::default()
        };
        Self {
            _icon: icon,
            rx,
            tx: Arc::new(Mutex::new(tx)),
            menu: Arc::new(Mutex::new(menu)),
            created: false,
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
                    let ppp = ctx.native_pixels_per_point().unwrap_or(1.0);
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
        (
            pos2(
                (m.anchor.x - size.x).max(0.0),
                (m.anchor.y - size.y).max(0.0),
            ),
            size,
        )
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
        let mut close = (m.had_focus && focused == Some(false))
            || ui.input(|i| i.key_pressed(egui::Key::Escape));
        let data = m.data.clone();
        let t = data.theme.clone();
        let mut picked = None;
        let mut used = 0.0;
        // Fill the whole window (Windows rounds its corners itself); a rounded
        // shape inside a see-through window left light edges at the corners.
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.chrome))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, t.border),
                    egui::StrokeKind::Inside,
                );
                let mut c = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink(8.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                picked = contents(&data, &mut c, &t);
                used = c.min_rect().height() + 20.0;
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
        name: if app.state.persona_name.is_empty() {
            me.name.clone()
        } else {
            app.state.persona_name.clone()
        },
        me,
        online: app
            .state
            .friend_list()
            .iter()
            .filter(|p| p.state != PersonaState::Offline)
            .count(),
        download: app
            .state
            .downloads
            .values()
            .find(|d| {
                !matches!(
                    d.phase,
                    model::DownloadPhase::Done | model::DownloadPhase::Failed
                )
            })
            .map(|d| {
                let pct = d.disk_done as f64 * 100.0 / d.disk_total.max(1) as f64;
                format!("{pct:.0}%")
            }),
        theme: app.prefs.theme.clone(),
    }
}

fn contents(d: &MenuData, ui: &mut Ui, t: &Theme) -> Option<TrayCmd> {
    let mut picked = None;
    // Who's signed in, and status dots.
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        if d.signed_in {
            w::avatar(
                ui,
                t,
                &model::art::avatar(d.me.avatar_hash.as_deref()),
                34.0,
                Some(crate::shell::status_color(t, &d.me)),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(w::text_semibold(&d.name, 14.0, t.text));
                ui.label(w::text(
                    i18n::tr_dyn(d.me.state.label()),
                    12.0,
                    crate::shell::status_color(t, &d.me),
                ));
            });
        } else {
            w::logo(ui, t, 26.0);
            ui.label(w::text_semibold("SteamInRust", 14.0, t.text));
        }
    });
    if d.signed_in {
        ui.add_space(6.0);
        // Statuses in a 2×2 grid, with their full names.
        let states = [
            (PersonaState::Online, t.online),
            (PersonaState::Away, t.away),
            (PersonaState::Busy, t.danger),
            (PersonaState::Invisible, t.text_faint),
        ];
        for pair in states.chunks(2) {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.x = 4.0;
                let cell = (ui.available_width() - 8.0) / 2.0;
                for (s, color) in pair {
                    let on = d.me.state == *s;
                    let (r, resp) = ui.allocate_exact_size(vec2(cell, 26.0), Sense::click());
                    let hover = resp.hovered();
                    ui.painter().rect_filled(
                        r,
                        CornerRadius::same(13),
                        if on {
                            color.gamma_multiply(0.2)
                        } else if hover {
                            t.surface_hover
                        } else {
                            t.raised
                        },
                    );
                    ui.painter()
                        .circle_filled(pos2(r.left() + 12.0, r.center().y), 4.0, *color);
                    let label = i18n::tr_dyn(s.label());
                    ui.painter().text(
                        pos2(r.left() + 22.0, r.center().y),
                        Align2::LEFT_CENTER,
                        &label,
                        w::font_medium(11.5),
                        if on { t.text } else { t.text_dim },
                    );
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        picked = Some(TrayCmd::Status(*s));
                    }
                }
            });
        }
    }
    ui.add_space(6.0);
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
        if row(ui, t, icon, label, extra.as_deref(), t.text) {
            picked = Some(cmd);
        }
    }
    separator(ui, t);
    if row(ui, t, icons::POWER, t!("Exit SteamInRust"), None, t.danger) {
        picked = Some(TrayCmd::Exit);
    }
    picked
}

fn separator(ui: &mut Ui, t: &Theme) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        r.x_range().shrink(6.0),
        r.center().y,
        Stroke::new(1.0, t.border),
    );
}

fn row(
    ui: &mut Ui,
    t: &Theme,
    icon: &str,
    label: &str,
    extra: Option<&str>,
    color: Color32,
) -> bool {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.08);
    if hover > 0.0 {
        ui.painter().rect_filled(
            r,
            CornerRadius::same(6),
            t.surface_hover.gamma_multiply(hover),
        );
    }
    let p = ui.painter();
    p.text(
        pos2(r.left() + 16.0, r.center().y),
        Align2::CENTER_CENTER,
        icon,
        w::font(15.0),
        if color == t.text { t.accent } else { color },
    );
    p.text(
        pos2(r.left() + 34.0, r.center().y),
        Align2::LEFT_CENTER,
        label,
        w::font_medium(13.5),
        color,
    );
    if let Some(extra) = extra {
        p.text(
            pos2(r.right() - 10.0, r.center().y),
            Align2::RIGHT_CENTER,
            extra,
            w::font(12.0),
            t.text_dim,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

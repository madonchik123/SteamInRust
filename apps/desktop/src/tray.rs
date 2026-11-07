//! System tray icon. Right-click opens our own menu window (styled like the
//! app) instead of the plain Windows menu.

use crate::app::SteamApp;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, ViewportBuilder, ViewportId, pos2, vec2};
use i18n::t;
use model::PersonaState;
use std::sync::mpsc;
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

pub struct Tray {
    _icon: Option<TrayIcon>,
    rx: mpsc::Receiver<TrayCmd>,
    tx: mpsc::Sender<TrayCmd>,
    /// Open menu: where (logical points) and whether it has had focus yet.
    menu: Option<(egui::Pos2, bool)>,
}

const MENU_W: f32 = 270.0;
const MENU_H: f32 = 404.0;

impl Tray {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let icon = TrayIconBuilder::new()
            .with_tooltip("SteamInRust")
            .with_icon(tray_icon::Icon::from_rgba(ui::brand::icon_rgba(32), 32, 32).expect("valid icon"))
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
        Self { _icon: icon, rx, tx, menu: None }
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<TrayCmd> {
        let mut out = Vec::new();
        for cmd in self.rx.try_iter() {
            match cmd {
                TrayCmd::Menu(x, y) => {
                    // Place the menu so its bottom-right corner sits at the cursor.
                    let ppp = ctx.native_pixels_per_point().unwrap_or(1.0);
                    let cursor = pos2(x as f32 / ppp, y as f32 / ppp);
                    self.menu = Some((pos2((cursor.x - MENU_W).max(0.0), (cursor.y - MENU_H).max(0.0)), false));
                }
                other => out.push(other),
            }
        }
        out
    }
}

/// Draw the tray menu window while it's open.
pub fn show(app: &mut SteamApp, ctx: &egui::Context) {
    let Some((pos, had_focus)) = app.tray.menu else { return };
    let t = app.prefs.theme.clone();
    let builder = ViewportBuilder::default()
        .with_title("SteamInRust")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_position(pos)
        .with_inner_size([MENU_W, MENU_H]);
    let mut close = false;
    let mut picked: Option<TrayCmd> = None;
    ctx.show_viewport_immediate(ViewportId::from_hash_of("tray-menu"), builder, |ui, _| {
        let focused = ui.input(|i| i.viewport().focused);
        if focused == Some(true) && !had_focus {
            if let Some(m) = &mut app.tray.menu {
                m.1 = true;
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        } else if !had_focus {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        // Clicking anywhere else closes it, like any menu.
        if (had_focus && focused == Some(false)) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::TRANSPARENT)).show(ui, |ui| {
            let rect = ui.max_rect().shrink(2.0);
            ui.painter().rect_filled(rect, CornerRadius::same(10), t.chrome);
            ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)).layout(egui::Layout::top_down(egui::Align::Min)));
            picked = contents(app, &mut c, &t);
        });
    });
    if let Some(cmd) = picked {
        let _ = app.tray.tx.send(cmd);
        ctx.request_repaint();
        close = true;
    }
    if close {
        app.tray.menu = None;
    }
}

fn contents(app: &SteamApp, ui: &mut Ui, t: &Theme) -> Option<TrayCmd> {
    let mut picked = None;
    // Who's signed in, and status dots.
    let me = app.state.persona(app.state.steam_id);
    let signed_in = app.state.steam_id != 0;
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        if signed_in {
            w::avatar(ui, t, &model::art::avatar(me.avatar_hash.as_deref()), 34.0, Some(crate::shell::status_color(t, &me)));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let name = if app.state.persona_name.is_empty() { me.name.clone() } else { app.state.persona_name.clone() };
                ui.label(w::text_semibold(name, 14.0, t.text));
                ui.label(w::text(i18n::tr_dyn(me.state.label()), 12.0, crate::shell::status_color(t, &me)));
            });
        } else {
            w::logo(ui, t, 26.0);
            ui.label(w::text_semibold("SteamInRust", 14.0, t.text));
        }
    });
    if signed_in {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 4.0;
            for (s, color) in [(PersonaState::Online, t.online), (PersonaState::Away, t.away), (PersonaState::Busy, t.danger), (PersonaState::Invisible, t.text_faint)] {
                let on = me.state == s;
                let (r, resp) = ui.allocate_exact_size(vec2(58.0, 26.0), Sense::click());
                let hover = resp.hovered();
                ui.painter().rect_filled(r, CornerRadius::same(13), if on { color.gamma_multiply(0.2) } else if hover { t.surface_hover } else { t.raised });
                ui.painter().circle_filled(pos2(r.left() + 12.0, r.center().y), 4.0, color);
                let label = i18n::tr_dyn(s.label());
                let short: String = label.chars().take(5).collect();
                ui.painter().text(pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, short, w::font_medium(11.0), if on { t.text } else { t.text_dim });
                if resp.on_hover_text(label).clicked() {
                    picked = Some(TrayCmd::Status(s));
                }
            }
        });
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
            TrayCmd::Friends => {
                let online = app.state.friend_list().iter().filter(|p| p.state != PersonaState::Offline).count();
                (online > 0).then(|| t!("{n} online", n = online))
            }
            TrayCmd::Page("downloads") => app.state.downloads.values().find(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).map(|d| {
                let pct = d.disk_done as f64 * 100.0 / d.disk_total.max(1) as f64;
                format!("{pct:.0}%")
            }),
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
    ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0, t.border));
}

fn row(ui: &mut Ui, t: &Theme, icon: &str, label: &str, extra: Option<&str>, color: Color32) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.08);
    if hover > 0.0 {
        ui.painter().rect_filled(r, CornerRadius::same(6), t.surface_hover.gamma_multiply(hover));
    }
    let p = ui.painter();
    p.text(pos2(r.left() + 16.0, r.center().y), Align2::CENTER_CENTER, icon, w::font(15.0), if color == t.text { t.accent } else { color });
    p.text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, label, w::font_medium(13.5), color);
    if let Some(extra) = extra {
        p.text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, extra, w::font(12.0), t.text_dim);
    }
    let _ = Rect::NOTHING;
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

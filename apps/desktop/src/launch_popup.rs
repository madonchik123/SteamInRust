//! The small "Launching <game>…" window Steam shows when a game is started
//! from outside the main window (desktop shortcut, steam:// link, tray).

use crate::app::SteamApp;
use client::Command;
use egui::{Align, Color32, Layout, Margin, Sense, Stroke, vec2};
use i18n::t;
use model::{AppId, art};
use ui::{icons, widgets as w};

pub enum Stage {
    Launching,
    Running(f64),
    Failed(String),
}

pub struct LaunchPopup {
    pub app_id: AppId,
    pub name: String,
    pub since: f64,
    pub stage: Stage,
}

impl LaunchPopup {
    pub fn new(app_id: AppId, name: String, now: f64) -> Self {
        Self { app_id, name, since: now, stage: Stage::Launching }
    }
}

/// What the window shows and what was clicked, shared with its own paint
/// (a deferred viewport, so it can open while the main window is hidden).
#[derive(Default)]
struct View {
    app_id: AppId,
    name: String,
    stage: Option<ShownStage>,
    theme: ui::Theme,
    close: bool,
    stop: bool,
}

#[derive(Clone)]
enum ShownStage {
    Launching,
    Running,
    Failed(String),
}

static VIEW: std::sync::LazyLock<std::sync::Arc<std::sync::Mutex<View>>> = std::sync::LazyLock::new(Default::default);

pub fn show(app: &mut SteamApp, ctx: &egui::Context) {
    let Some(p) = &app.launch_popup else { return };
    let now = ctx.input(|i| i.time);
    // Close a moment after the game is up, or if nothing happened for a while.
    let done = match &p.stage {
        Stage::Running(at) => now - at > 2.5,
        Stage::Launching => now - p.since > 45.0,
        Stage::Failed(_) => false,
    };
    if done {
        app.launch_popup = None;
        return;
    }
    let (app_id, name) = (p.app_id, p.name.clone());
    let stage = match &p.stage {
        Stage::Launching => ShownStage::Launching,
        Stage::Running(_) => ShownStage::Running,
        Stage::Failed(e) => ShownStage::Failed(e.clone()),
    };
    let (close, stop) = {
        let mut v = VIEW.lock().unwrap();
        if v.app_id != app_id {
            *v = View::default();
        }
        v.app_id = app_id;
        v.name = name.clone();
        v.stage = Some(stage);
        v.theme = app.prefs.theme.clone();
        (std::mem::take(&mut v.close), std::mem::take(&mut v.stop))
    };
    if stop {
        app.client.send(Command::StopGame { app_id });
    }
    if close {
        app.launch_popup = None;
        return;
    }
    let builder = egui::ViewportBuilder::default()
        .with_title(t!("Launching {name}", name = name))
        .with_inner_size([440.0, 170.0])
        .with_resizable(false)
        .with_decorations(false)
        .with_always_on_top();
    let view = VIEW.clone();
    ctx.show_viewport_deferred(egui::ViewportId::from_hash_of("launch-popup"), builder, move |ui, _| {
        let mut v = view.lock().unwrap();
        let t = v.theme.clone();
        let (app_id, name, stage) = (v.app_id, v.name.clone(), v.stage.clone());
        let (mut close, mut stop) = (false, false);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.chrome).stroke(Stroke::new(1.0, t.border)).inner_margin(Margin::same(18)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(160.0, 75.0), Sense::hover());
                    w::image_or_skeleton(ui, &t, r, &art::header(app_id), t.corner_small());
                    ui.vertical(|ui| {
                        ui.label(w::text_bold(&name, 17.0, t.text));
                        ui.add_space(4.0);
                        match &stage {
                            Some(ShownStage::Launching) => {
                                ui.horizontal(|ui| {
                                    ui.add(egui::Spinner::new().color(t.accent));
                                    ui.label(w::text(t!("Preparing to launch…"), 13.5, t.text_dim));
                                });
                            }
                            Some(ShownStage::Running) => {
                                ui.label(w::text(format!("{}  {}", icons::CHECK, t!("Running")), 13.5, t.play));
                            }
                            Some(ShownStage::Failed(e)) => {
                                ui.add(egui::Label::new(w::text(format!("{}  {}", icons::WARNING, i18n::tr_dyn(e.as_str())), 13.0, t.danger)).wrap());
                            }
                            None => {}
                        }
                    });
                });
                ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
                    ui.horizontal(|ui| {
                        if w::button(ui, &t, None, t!("Close"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            close = true;
                        }
                        let launching = matches!(stage, Some(ShownStage::Launching | ShownStage::Running));
                        if launching && w::button(ui, &t, Some(icons::STOP), t!("Cancel"), w::Kind::Danger, w::Size::Sm).clicked() {
                            stop = true;
                            close = true;
                        }
                    });
                });
            });
        v.close |= close;
        v.stop |= stop;
        let _ = Color32::TRANSPARENT;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    });
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
}

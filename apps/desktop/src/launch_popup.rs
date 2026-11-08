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
        Self {
            app_id,
            name,
            since: now,
            stage: Stage::Launching,
        }
    }
}

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
    let t = app.prefs.theme.clone();
    let (app_id, name) = (p.app_id, p.name.clone());
    let builder = egui::ViewportBuilder::default()
        .with_title(t!("Launching {name}", name = name))
        .with_inner_size([440.0, 170.0])
        .with_resizable(false)
        .with_decorations(false)
        .with_always_on_top();
    let mut close = false;
    let mut stop = false;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("launch-popup"),
        builder,
        |ui, _| {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(t.chrome)
                        .stroke(Stroke::new(1.0, t.border))
                        .inner_margin(Margin::same(18)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(160.0, 75.0), Sense::hover());
                        w::image_or_skeleton(ui, &t, r, &art::header(app_id), t.corner_small());
                        ui.vertical(|ui| {
                            ui.label(w::text_bold(&name, 17.0, t.text));
                            ui.add_space(4.0);
                            match &app.launch_popup.as_ref().map(|p| &p.stage) {
                                Some(Stage::Launching) => {
                                    ui.horizontal(|ui| {
                                        ui.add(egui::Spinner::new().color(t.accent));
                                        ui.label(w::text(
                                            t!("Preparing to launch…"),
                                            13.5,
                                            t.text_dim,
                                        ));
                                    });
                                }
                                Some(Stage::Running(_)) => {
                                    ui.label(w::text(
                                        format!("{}  {}", icons::CHECK, t!("Running")),
                                        13.5,
                                        t.play,
                                    ));
                                }
                                Some(Stage::Failed(e)) => {
                                    ui.add(
                                        egui::Label::new(w::text(
                                            format!(
                                                "{}  {}",
                                                icons::WARNING,
                                                i18n::tr_dyn(e.as_str())
                                            ),
                                            13.0,
                                            t.danger,
                                        ))
                                        .wrap(),
                                    );
                                }
                                None => {}
                            }
                        });
                    });
                    ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
                        ui.horizontal(|ui| {
                            if w::button(ui, &t, None, t!("Close"), w::Kind::Ghost, w::Size::Sm)
                                .clicked()
                            {
                                close = true;
                            }
                            let launching = matches!(
                                app.launch_popup.as_ref().map(|p| &p.stage),
                                Some(Stage::Launching | Stage::Running(_))
                            );
                            if launching
                                && w::button(
                                    ui,
                                    &t,
                                    Some(icons::STOP),
                                    t!("Cancel"),
                                    w::Kind::Danger,
                                    w::Size::Sm,
                                )
                                .clicked()
                            {
                                stop = true;
                                close = true;
                            }
                        });
                    });
                });
            let _ = Color32::TRANSPARENT;
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
    if stop {
        app.client.send(Command::StopGame { app_id });
    }
    if close {
        app.launch_popup = None;
    }
}

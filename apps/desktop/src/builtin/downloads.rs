//! Downloads: update queue, recently updated games, and download settings.
//! The transfer engine itself (depot downloader) plugs in here next; until
//! then the queue shows what Steam has marked as needing an update.

use egui::{Align, Layout, Rect, Sense, Ui, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{InstallState, art};
use serde::{Deserialize, Serialize};
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadSettings {
    pub region: String,
    pub limit_bandwidth: bool,
    /// KB/s
    pub limit_kbps: u32,
    pub while_playing: bool,
    pub schedule: bool,
    pub schedule_from: u8,
    pub schedule_to: u8,
    pub show_bits: bool,
    /// Parallel chunk downloads.
    pub connections: u32,
}

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            region: "auto".into(),
            limit_bandwidth: false,
            limit_kbps: 10_000,
            while_playing: false,
            schedule: false,
            schedule_from: 1,
            schedule_to: 7,
            show_bits: false,
            connections: 16,
        }
    }
}

pub struct Downloads {
    flags: super::settings::SharedFlags,
    /// Regions measured from this PC, fastest first.
    regions: Option<Vec<model::DownloadRegion>>,
    probing: bool,
    region_sent: bool,
    settings: DownloadSettings,
    /// Last limits sent to the engine.
    sent: Option<(u64, u32)>,
    /// Downloads we paused because a game started.
    auto_paused: Vec<model::AppId>,
}

impl Downloads {
    pub fn new(flags: super::settings::SharedFlags) -> Self {
        Self {
            flags,
            regions: None,
            probing: false,
            region_sent: false,
            settings: Default::default(),
            sent: None,
            auto_paused: Vec::new(),
        }
    }
}

impl Extension for Downloads {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "downloads",
            name: "Downloads",
            author: "SteamInRust",
            description: "Download queue and settings",
            core: true,
            ..Manifest::DEFAULT
        }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![
            PageSpec::new("downloads", "Downloads", icons::DOWNLOAD)
                .hidden()
                .order(900),
        ]
    }

    fn on_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        match event {
            client::Event::LoggedIn { .. } => {
                self.sent = None;
                self.region_sent = false;
            }
            client::Event::Regions(list) => {
                self.regions = Some(list.clone());
                self.probing = false;
            }
            client::Event::GameStarted { .. } if !self.settings.while_playing => {
                for (id, d) in &ctx.state.downloads {
                    if matches!(
                        d.phase,
                        model::DownloadPhase::Downloading | model::DownloadPhase::Checking
                    ) && !d.paused
                    {
                        ctx.send(client::Command::PauseDownload { app_id: *id });
                        self.auto_paused.push(*id);
                    }
                }
                if !self.auto_paused.is_empty() {
                    ctx.toast(t!("Downloads paused while you play"));
                }
            }
            client::Event::GameStopped { .. } => {
                for id in self.auto_paused.drain(..) {
                    ctx.send(client::Command::ResumeDownload { app_id: id });
                }
            }
            _ => {}
        }
        self.sync_limits(ctx);
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        self.sync_limits(ctx);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(26.0);
                w::centered(ui, 1100.0, 32.0, |ui| {
                    w::page_title(ui, &t, t!("Downloads"), None);
                    self.speed_card(ui, &t, ctx);
                    ui.add_space(20.0);
                    self.active(ui, &t, ctx);

                    let mut queue: Vec<_> = ctx
                        .local
                        .games
                        .iter()
                        .filter(|g| {
                            matches!(
                                g.state,
                                InstallState::UpdateRequired | InstallState::Downloading
                            )
                        })
                        .collect();
                    queue.sort_by_key(|g| g.name.to_lowercase());
                    w::section(ui, &t, &t!("Up next ({n})", n = queue.len()), None);
                    if queue.is_empty() {
                        w::card(&t).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(w::text(icons::CHECK, 22.0, t.play));
                                ui.vertical(|ui| {
                                    ui.label(w::text_semibold(
                                        t!("Everything is up to date"),
                                        15.0,
                                        t.text,
                                    ));
                                    ui.label(w::text(
                                        t!("Updates show up here when a game needs one."),
                                        13.0,
                                        t.text_dim,
                                    ));
                                });
                            });
                        });
                    }
                    for g in &queue {
                        row(
                            ui,
                            &t,
                            ctx,
                            g.app_id,
                            &g.name,
                            &if g.state == InstallState::Downloading {
                                t!("Downloading").to_string()
                            } else {
                                t!("Update required").to_string()
                            },
                            &ui::bytes(g.size_on_disk),
                            true,
                        );
                    }
                    ui.add_space(20.0);

                    let mut recent: Vec<_> = ctx
                        .local
                        .games
                        .iter()
                        .filter(|g| g.last_updated.is_some())
                        .collect();
                    recent.sort_by_key(|g| std::cmp::Reverse(g.last_updated));
                    w::section(ui, &t, t!("Recently updated"), None);
                    if recent.is_empty() {
                        ui.label(w::text(
                            t!("No games installed on this PC yet."),
                            13.5,
                            t.text_faint,
                        ));
                    }
                    for g in recent.iter().take(12) {
                        let when = g.last_updated.map(ui::ago).unwrap_or_default();
                        let mut detail = t!("Updated {when}", when = when);
                        if let Some(b) = g.last_update_bytes {
                            detail = format!(
                                "{detail}  ·  {}",
                                t!("{size} downloaded", size = ui::bytes(b))
                            );
                        }
                        row(
                            ui,
                            &t,
                            ctx,
                            g.app_id,
                            &g.name,
                            &detail,
                            &t!("{size} on disk", size = ui::bytes(g.size_on_disk)),
                            false,
                        );
                    }
                    ui.add_space(24.0);
                    self.settings_card(ui, &t, ctx);
                    ui.add_space(40.0);
                });
            });
    }

    fn save(&self) -> Option<String> {
        serde_json::to_string(&self.settings).ok()
    }

    fn load(&mut self, data: &str) {
        self.settings = serde_json::from_str(data).unwrap_or_default();
    }
}

impl Downloads {
    fn sync_limits(&mut self, ctx: &Ctx) {
        let want = (
            if self.settings.limit_bandwidth {
                self.settings.limit_kbps as u64
            } else {
                0
            },
            self.settings.connections,
        );
        if self.sent != Some(want) {
            ctx.send(client::Command::DownloadLimits {
                limit_kbps: want.0,
                concurrency: want.1,
            });
            self.sent = Some(want);
        }
    }

    fn rate(&self, bytes_per_sec: u64) -> String {
        if self.settings.show_bits {
            format!("{:.1} Mbps", bytes_per_sec as f64 * 8.0 / 1_000_000.0)
        } else {
            format!("{}/s", ui::bytes(bytes_per_sec))
        }
    }

    /// The running download, the queue, and what finished this session.
    fn active(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let running: Vec<model::DownloadStatus> = ctx
            .state
            .downloads
            .values()
            .filter(|d| {
                !matches!(
                    d.phase,
                    model::DownloadPhase::Done | model::DownloadPhase::Failed
                )
            })
            .cloned()
            .collect();
        if !running.is_empty() || !ctx.state.download_queue.is_empty() {
            w::section(ui, t, t!("Downloading"), None);
        }
        for d in &running {
            let name = ctx.state.app_name(d.app_id).unwrap_or_default().to_string();
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(184.0, 86.0), Sense::hover());
                    w::image_or_skeleton(ui, t, r, &art::header(d.app_id), t.corner_small());
                    ui.vertical(|ui| {
                        ui.label(w::text_bold(&name, 17.0, t.text));
                        // Downloading / Writing to disk / Checking existing files…
                        ui.label(w::text_semibold(i18n::tr_dyn(d.activity()), 13.5, t.accent));
                        let width = (ui.available_width() - 120.0).max(80.0);
                        w::progress(
                            ui,
                            t,
                            d.disk_done as f32 / d.disk_total.max(1) as f32,
                            width,
                        );
                        let left = d.download_total.saturating_sub(d.downloaded);
                        let eta = if d.speed > 0 && !d.paused {
                            ui::playtime(left / d.speed / 60 + 1)
                        } else {
                            "—".into()
                        };
                        ui.label(w::text(
                            format!(
                                "{} / {}   ·   {}   ·   {}   ·   {}",
                                ui::bytes(d.disk_done),
                                ui::bytes(d.disk_total),
                                t!("network {rate}", rate = self.rate(d.speed)),
                                // Disk in MB/s like Steam; network in the unit picked in settings.
                                t!(
                                    "disk {rate}",
                                    rate = format!("{}/s", ui::bytes(d.disk_speed))
                                ),
                                t!("{time} left", time = eta)
                            ),
                            12.5,
                            t.text_dim,
                        ));
                        notes(ui, t, d);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::icon_button(ui, t, ui::icons::X, t!("Cancel"), false, 0).clicked() {
                            ctx.send(client::Command::CancelDownload { app_id: d.app_id });
                        }
                        if d.paused {
                            if w::icon_button(ui, t, ui::icons::PLAY, t!("Resume"), false, 0)
                                .clicked()
                            {
                                ctx.send(client::Command::ResumeDownload { app_id: d.app_id });
                            }
                        } else if w::icon_button(ui, t, "\u{E39E}", t!("Pause"), false, 0).clicked()
                        {
                            ctx.send(client::Command::PauseDownload { app_id: d.app_id });
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
        // Queued games; the one downloading is never listed twice.
        for id in ctx
            .state
            .download_queue
            .clone()
            .into_iter()
            .filter(|id| !running.iter().any(|d| d.app_id == *id))
        {
            let name = ctx.state.app_name(id).unwrap_or_default().to_string();
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(92.0, 43.0), Sense::hover());
                w::image_or_skeleton(ui, t, r, &art::header(id), t.corner_small());
                ui.label(w::text_medium(name, 14.0, t.text));
                ui.label(w::text(t!("Queued"), 12.5, t.text_faint));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, ui::icons::X, t!("Remove from queue"), false, 0)
                        .clicked()
                    {
                        ctx.send(client::Command::CancelDownload { app_id: id });
                    }
                });
            });
            ui.add_space(4.0);
        }
        let finished: Vec<model::DownloadStatus> = ctx
            .state
            .downloads
            .values()
            .filter(|d| {
                matches!(
                    d.phase,
                    model::DownloadPhase::Done | model::DownloadPhase::Failed
                )
            })
            .cloned()
            .collect();
        if !finished.is_empty() {
            ui.add_space(12.0);
            w::section(ui, t, t!("Finished this session"), None);
            for d in finished {
                let name = ctx.state.app_name(d.app_id).unwrap_or_default().to_string();
                ui.horizontal(|ui| {
                    let (icon, color) = if d.error.is_some() {
                        (ui::icons::WARNING, t.danger)
                    } else {
                        (ui::icons::CHECK, t.play)
                    };
                    ui.label(w::text(icon, 16.0, color));
                    ui.label(w::text_medium(name, 14.0, t.text));
                    if let Some(e) = &d.error {
                        ui.label(w::text(i18n::tr_dyn(e), 12.5, t.danger));
                    } else {
                        ui.label(w::text(ui::bytes(d.disk_total), 12.5, t.text_faint));
                    }
                });
                notes(ui, t, &d);
            }
        }
        ui.add_space(20.0);
    }

    fn speed_card(&self, ui: &mut Ui, t: &Theme, ctx: &Ctx) {
        let total: u64 = ctx.local.games.iter().map(|g| g.size_on_disk).sum();
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                w::stat(
                    ui,
                    t,
                    t!("Current speed"),
                    &if self.settings.show_bits {
                        "0 Mbps".to_string()
                    } else {
                        "0 MB/s".to_string()
                    },
                );
                ui.add_space(28.0);
                w::stat(
                    ui,
                    t,
                    t!("Installed"),
                    &t!(
                        "{n} games · {size}",
                        n = ctx.local.games.len(),
                        size = ui::bytes(total)
                    ),
                );
                ui.add_space(28.0);
                w::stat(
                    ui,
                    t,
                    t!("Libraries"),
                    &ctx.local.libraries.len().to_string(),
                );
            });
            ui.add_space(8.0);
            // Network graph area (fills in once transfers run).
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 70.0), Sense::hover());
            ui.painter().rect_filled(rect, t.corner_small(), t.raised);
            for i in 1..4 {
                let y = rect.top() + rect.height() * i as f32 / 4.0;
                ui.painter()
                    .hline(rect.x_range(), y, egui::Stroke::new(1.0, t.border));
            }
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                t!("No active downloads"),
                w::font(12.5),
                t.text_faint,
            );
        });
    }

    fn settings_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if !self.region_sent && ctx.state.steam_id != 0 {
            self.region_sent = true;
            ctx.send(region_command(&self.settings.region));
        }
        if self.regions.is_none() && !self.probing {
            self.probing = true;
            ctx.send(client::Command::ProbeRegions);
        }
        w::section(ui, t, t!("Download settings"), None);
        // Master auto-update switch, like Steam's.
        {
            let mut f = *self.flags.lock().unwrap();
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(w::text_semibold(t!("Automatic updates"), 14.5, t.text));
                        ui.label(w::text(t!("Keep installed games up to date. Turn off to update only when you choose (or when you launch a game)."), 12.5, t.text_dim));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(super::settings::toggle(&mut f.auto_updates));
                    });
                });
            });
            *self.flags.lock().unwrap() = f;
        }
        ui.add_space(10.0);
        self.region_card(ui, t, ctx);
        ui.add_space(10.0);
        let s = &mut self.settings;
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("dl-settings").num_columns(2).spacing(vec2(32.0, 14.0)).show(ui, |ui| {

                ui.label(w::text_medium(t!("Limit bandwidth"), 14.0, t.text));
                ui.horizontal(|ui| {
                    ui.add(super::settings::toggle(&mut s.limit_bandwidth));
                    ui.add_enabled(s.limit_bandwidth, egui::Slider::new(&mut s.limit_kbps, 256..=100_000).logarithmic(true).suffix(" KB/s"));
                });
                ui.end_row();

                ui.label(w::text_medium(t!("Allow downloads while playing"), 14.0, t.text));
                ui.add(super::settings::toggle(&mut s.while_playing));
                ui.end_row();

                ui.label(w::text_medium(t!("Only auto-update games between"), 14.0, t.text));
                ui.horizontal(|ui| {
                    ui.add(super::settings::toggle(&mut s.schedule));
                    ui.add_enabled_ui(s.schedule, |ui| {
                        ui.add(egui::DragValue::new(&mut s.schedule_from).range(0..=23).suffix(":00"));
                        ui.label("—");
                        ui.add(egui::DragValue::new(&mut s.schedule_to).range(0..=23).suffix(":00"));
                    });
                });
                ui.end_row();

                ui.label(w::text_medium(t!("Show speeds in bits per second"), 14.0, t.text));
                ui.add(super::settings::toggle(&mut s.show_bits));
                ui.end_row();

                ui.label(w::text_medium(t!("Parallel connections"), 14.0, t.text));
                ui.add(egui::Slider::new(&mut s.connections, 4..=48));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(w::text(t!("SteamInRust measures Steam's content servers and uses the fastest one, reuses unchanged parts of games when updating, and checks every piece as it arrives."), 12.5, t.text_faint));
        });
    }
}

impl Downloads {
    /// Every download region, measured from this PC, fastest first.
    fn region_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(w::text_semibold(t!("Download region"), 14.5, t.text));
                    ui.label(w::text(
                        t!("Measured from this PC: the closest servers usually download fastest."),
                        12.5,
                        t.text_dim,
                    ));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(!self.probing, |ui| {
                        if w::button(
                            ui,
                            t,
                            Some(icons::REFRESH),
                            if self.probing {
                                t!("Measuring…")
                            } else {
                                t!("Measure again")
                            },
                            w::Kind::Ghost,
                            w::Size::Sm,
                        )
                        .clicked()
                        {
                            self.probing = true;
                            ctx.send(client::Command::ProbeRegions);
                        }
                    });
                });
            });
            ui.add_space(8.0);
            let mut pick: Option<String> = None;
            let current = self.settings.region.clone();
            let mut option =
                |ui: &mut Ui, id: String, name: String, ping: Option<u32>, tier: Option<u8>| {
                    let on = current == id;
                    let (r, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    if on || resp.hovered() {
                        ui.painter().rect_filled(
                            r,
                            t.corner_small(),
                            if on {
                                t.accent.gamma_multiply(0.16)
                            } else {
                                t.surface_hover
                            },
                        );
                    }
                    let dot = egui::pos2(r.left() + 14.0, r.center().y);
                    ui.painter().circle_stroke(
                        dot,
                        6.5,
                        egui::Stroke::new(1.5, if on { t.accent } else { t.text_faint }),
                    );
                    if on {
                        ui.painter().circle_filled(dot, 3.5, t.accent);
                    }
                    ui.painter().text(
                        egui::pos2(r.left() + 30.0, r.center().y),
                        egui::Align2::LEFT_CENTER,
                        &name,
                        w::font_medium(13.5),
                        t.text,
                    );
                    if let Some(tier) = tier {
                        let (label, color) = match tier {
                            3 => (t!("Fastest"), t.online),
                            2 => (t!("Fast"), t.online),
                            1 => (t!("Average"), t.away),
                            _ => (t!("Slow"), t.text_faint),
                        };
                        // Speed bars + label, then ping.
                        let x = r.right() - 200.0;
                        for i in 0..4u8 {
                            let bar = Rect::from_min_size(
                                egui::pos2(
                                    x + i as f32 * 6.0,
                                    r.center().y + 5.0 - (i as f32 + 1.0) * 3.0,
                                ),
                                vec2(4.0, (i as f32 + 1.0) * 3.0),
                            );
                            ui.painter().rect_filled(
                                bar,
                                1.0,
                                if i <= tier { color } else { t.raised },
                            );
                        }
                        ui.painter().text(
                            egui::pos2(x + 30.0, r.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            w::font(12.5),
                            color,
                        );
                    }
                    let ping_text = match ping {
                        Some(ms) => format!("{ms} ms"),
                        None if tier.is_some() => t!("unreachable").to_string(),
                        None => String::new(),
                    };
                    ui.painter().text(
                        egui::pos2(r.right() - 12.0, r.center().y),
                        egui::Align2::RIGHT_CENTER,
                        ping_text,
                        w::font(12.5),
                        t.text_dim,
                    );
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        pick = Some(id);
                    }
                };
            option(
                ui,
                "auto".into(),
                t!("Automatic (recommended)").to_string(),
                None,
                None,
            );
            match &self.regions {
                None => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(t.accent));
                        ui.label(w::text(t!("Measuring every region…"), 12.5, t.text_faint));
                    });
                }
                Some(list) => {
                    // The whole list, fastest first (no tiny scroll box).
                    for r in list {
                        let id = if r.cell_id == 0 {
                            format!("host:{}", r.host)
                        } else {
                            r.cell_id.to_string()
                        };
                        option(ui, id, r.name.clone(), r.ping_ms, Some(r.speed_tier()));
                    }
                }
            }
            if let Some(id) = pick {
                self.settings.region = id;
                ctx.send(region_command(&self.settings.region));
            }
        });
    }
}

/// "auto", a Steam cell id, or "host:<name>" for a partner CDN.
fn region_command(region: &str) -> client::Command {
    match region.strip_prefix("host:") {
        Some(host) => client::Command::DownloadRegion {
            cell_id: None,
            host: Some(host.to_string()),
        },
        None => client::Command::DownloadRegion {
            cell_id: region.parse().ok(),
            host: None,
        },
    }
}

fn row(
    ui: &mut Ui,
    t: &Theme,
    ctx: &mut Ctx,
    app_id: u32,
    name: &str,
    status: &str,
    size: &str,
    pending: bool,
) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 74.0), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter()
        .rect_filled(rect, t.corner(), w::mix(t.surface, t.surface_hover, hover));
    // Click opens the game; right-click has Steam's usual actions.
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        ctx.navigate("library", Some(app_id.to_string()));
    }
    resp.context_menu(|ui| {
        ui.set_min_width(220.0);
        let mut go = |ui: &mut Ui, icon: &str, label: &str, arg: String| {
            if ui.button(format!("{icon}   {label}")).clicked() {
                ctx.navigate("library", Some(arg));
                ui.close();
            }
        };
        go(ui, icons::BOOKS, t!("Open game page"), app_id.to_string());
        go(
            ui,
            icons::NOTE,
            t!("See what's new"),
            format!("news:{app_id}"),
        );
        go(
            ui,
            icons::SHIELD,
            t!("Verify game files"),
            format!("verify:{app_id}"),
        );
        go(
            ui,
            icons::SLIDERS,
            t!("Properties…"),
            format!("props:{app_id}"),
        );
        if ui
            .button(format!("{}   {}", icons::FOLDER, t!("Browse local files")))
            .clicked()
        {
            if let Some(g) = ctx.local.games.iter().find(|g| g.app_id == app_id) {
                local::open_folder(&g.install_dir);
            }
            ui.close();
        }
    });
    let img = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(116.0, 54.0));
    w::image_or_skeleton(ui, t, img, &art::header(app_id), t.corner_small());
    let mut c = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                egui::pos2(img.right() + 16.0, rect.top()),
                rect.max - vec2(14.0, 0.0),
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    c.vertical(|ui| {
        ui.add_space(14.0);
        ui.label(w::text_semibold(name, 15.0, t.text));
        ui.label(w::text(
            format!("{status}  ·  {size}"),
            12.5,
            if pending { t.away } else { t.text_dim },
        ));
    });
    c.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if pending {
            if w::button(
                ui,
                t,
                Some(icons::DOWNLOAD),
                t!("Update"),
                w::Kind::Primary,
                w::Size::Sm,
            )
            .clicked()
            {
                ctx.navigate("library", Some(format!("update:{app_id}")));
            }
        } else if w::button(
            ui,
            t,
            Some(icons::NOTE),
            t!("What's new"),
            w::Kind::Ghost,
            w::Size::Sm,
        )
        .clicked()
        {
            ctx.navigate("library", Some(format!("news:{app_id}")));
        }
    });
    ui.add_space(8.0);
}

/// Things that didn't stop the download but are worth knowing, like a part
/// of the game the account's license doesn't include (Steam skips those too).
fn notes(ui: &mut Ui, t: &Theme, d: &model::DownloadStatus) {
    if d.notes.is_empty() {
        return;
    }
    let text = if d.notes.len() == 1 {
        t!("1 part of this game was skipped: {why}", why = d.notes[0])
    } else {
        t!(
            "{n} parts of this game were skipped (your license doesn't include them)",
            n = d.notes.len()
        )
    };
    ui.label(w::text(
        format!("{}  {text}", ui::icons::INFO),
        12.0,
        t.text_faint,
    ))
    .on_hover_text(d.notes.join("\n"));
}

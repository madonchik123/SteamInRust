//! Downloads (Figma "download" `375:5665`): every download, queued game,
//! pending update and finished download as a row, recently updated games,
//! and the download settings.

use egui::{Align, Align2, Layout, Margin, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{AppId, InstallState, art};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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
    /// Speed over the last minute for the graph: (seconds since start,
    /// network bytes/s, disk bytes/s), at most two samples a second.
    history: std::collections::VecDeque<(f64, u64, u64)>,
    started: Option<std::time::Instant>,
    /// Rows ticked with their check box.
    selected: BTreeSet<AppId>,
    /// Finished downloads taken off the list with "Clear Completed".
    cleared: BTreeSet<AppId>,
    /// The gear was clicked: scroll down to the settings.
    show_settings: bool,
}

/// How much speed history the graph shows.
const GRAPH_SECS: f64 = 60.0;

/// What a row of the list is.
enum Kind {
    /// Downloading, paused, or checking files.
    Active(model::DownloadStatus),
    /// Waiting its turn; how far it got if it was set aside part-way.
    Queued { set_aside: Option<f32> },
    /// Installed, with an update waiting.
    Update { size: u64 },
    /// Finished (or failed) this session.
    Done(model::DownloadStatus),
}

struct Row {
    id: AppId,
    name: String,
    kind: Kind,
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
            history: Default::default(),
            started: None,
            selected: BTreeSet::new(),
            cleared: BTreeSet::new(),
            show_settings: false,
        }
    }
}

impl Extension for Downloads {
    fn manifest(&self) -> Manifest {
        Manifest { id: "downloads", name: "Downloads", author: "SteamInRust", description: "Download queue and settings", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("downloads", "Downloads", icons::DOWNLOAD).hidden().order(900)]
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
                    if matches!(d.phase, model::DownloadPhase::Downloading | model::DownloadPhase::Checking) && !d.paused {
                        ctx.send(client::Command::PauseDownload { app_id: *id });
                        self.auto_paused.push(*id);
                    }
                }
                if !self.auto_paused.is_empty() {
                    ctx.toast(t!("Downloads paused while you play"));
                }
            }
            client::Event::Download(_) => self.sample(ctx),
            client::Event::GameStopped { .. } => {
                for id in self.auto_paused.drain(..) {
                    ctx.send(client::Command::ResumeDownload { app_id: id });
                }
            }
            // A game downloaded again shows up again once it finishes.
            client::Event::DownloadFinished { app_id, .. } => {
                self.cleared.remove(app_id);
            }
            _ => {}
        }
        self.sync_limits(ctx);
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        self.sync_limits(ctx);
        let rows = self.rows(ctx);
        self.selected.retain(|id| rows.iter().any(|r| r.id == *id));
        let side = (t.spacing.xl + 1.0) as i8;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: side, right: side, top: (t.spacing.xl + 4.0) as i8, bottom: (t.spacing.xl * 2.0) as i8 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.header(ui, &t, ctx, &rows);
                ui.add_space(t.spacing.lg + 3.0);
                if rows.is_empty() {
                    w::card(&t).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(w::text(icons::CHECK, t.typography.heading_m.size, t.play));
                            ui.vertical(|ui| {
                                ui.label(w::body_l(&t, t!("Everything is up to date"), t.text));
                                ui.label(w::body_m(&t, t!("Updates show up here when a game needs one."), t.text_dim));
                            });
                        });
                    });
                }
                let queued: Vec<AppId> = rows.iter().filter(|r| matches!(r.kind, Kind::Queued { .. })).map(|r| r.id).collect();
                for row in &rows {
                    self.row(ui, &t, ctx, row, &queued);
                    ui.add_space(t.spacing.lg - 2.0 - ui.spacing().item_spacing.y);
                }
                ui.add_space(t.spacing.xl);

                let mut recent: Vec<_> = ctx.local.games.iter().filter(|g| g.last_updated.is_some()).collect();
                recent.sort_by_key(|g| std::cmp::Reverse(g.last_updated));
                ui.label(w::heading_s(&t, t!("Recently updated"), t.text));
                ui.add_space(t.spacing.sm);
                if recent.is_empty() {
                    ui.label(w::body_m(&t, t!("No games installed on this PC yet."), t.text_faint));
                }
                let recent: Vec<(AppId, String, String, String)> = recent
                    .iter()
                    .take(12)
                    .map(|g| {
                        let when = g.last_updated.map(ui::ago).unwrap_or_default();
                        let mut detail = t!("Updated {when}", when = when);
                        if let Some(b) = g.last_update_bytes {
                            detail = format!("{detail}  ·  {}", t!("{size} downloaded", size = ui::bytes(b)));
                        }
                        (g.app_id, g.name.clone(), detail, t!("{size} on disk", size = ui::bytes(g.size_on_disk)).to_string())
                    })
                    .collect();
                for (id, name, detail, size) in &recent {
                    small_row(ui, &t, ctx, *id, name, detail, size);
                }
                ui.add_space(t.spacing.xl);
                let r = self.settings_card(ui, &t, ctx);
                if std::mem::take(&mut self.show_settings) {
                    ui.scroll_to_rect(r, Some(Align::Min));
                }
            });
        });
        if Self::running(ctx).count() > 0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
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
        let want = (if self.settings.limit_bandwidth { self.settings.limit_kbps as u64 } else { 0 }, self.settings.connections);
        if self.sent != Some(want) {
            ctx.send(client::Command::DownloadLimits { limit_kbps: want.0, concurrency: want.1 });
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

    /// The list, in order: what's downloading, the queue, updates waiting,
    /// then what finished this session.
    fn rows(&self, ctx: &Ctx) -> Vec<Row> {
        let name = |id: AppId| ctx.state.app_name(id).map(String::from).or_else(|| ctx.local.games.iter().find(|g| g.app_id == id).map(|g| g.name.clone())).unwrap_or_default();
        let mut rows: Vec<Row> = Vec::new();
        let mut active: Vec<&model::DownloadStatus> = ctx.state.downloads.values().filter(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)).collect();
        active.sort_by_key(|d| (d.paused, ctx.state.download_queue.iter().position(|q| *q == d.app_id), d.app_id));
        rows.extend(active.iter().map(|d| Row { id: d.app_id, name: name(d.app_id), kind: Kind::Active((*d).clone()) }));
        // Queued games, in the order they'll download; the one downloading
        // is never listed twice.
        for &id in &ctx.state.download_queue {
            if !rows.iter().any(|r| r.id == id) {
                rows.push(Row { id, name: name(id), kind: Kind::Queued { set_aside: ctx.state.set_aside.get(&id).copied().filter(|f| *f > 0.0) } });
            }
        }
        // Games waiting for an update.
        let mut updates: Vec<&model::InstalledGame> = ctx.local.games.iter().filter(|g| matches!(g.state, InstallState::UpdateRequired | InstallState::Downloading) && !rows.iter().any(|r| r.id == g.app_id)).collect();
        updates.sort_by_key(|g| g.name.to_lowercase());
        rows.extend(updates.iter().map(|g| Row { id: g.app_id, name: g.name.clone(), kind: Kind::Update { size: g.size_on_disk } }));
        let mut done: Vec<&model::DownloadStatus> = ctx.state.downloads.values().filter(|d| matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed) && !self.cleared.contains(&d.app_id)).collect();
        done.sort_by_key(|d| d.app_id);
        rows.extend(done.iter().map(|d| Row { id: d.app_id, name: name(d.app_id), kind: Kind::Done((*d).clone()) }));
        rows
    }

    /// "Download Manager", the settings gear, and the buttons for the
    /// rows ticked (Figma "Frame 100" `379:8603`).
    fn header(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, rows: &[Row]) {
        let total: u64 = ctx.local.games.iter().map(|g| g.size_on_disk).sum();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.sm + 2.0;
            ui.label(w::heading_m(t, t!("Download Manager"), t.text));
            if w::icon_box(ui, t, icons::GEAR, t!("Download settings"), false).clicked() {
                self.show_settings = true;
            }
            ui.add_space(t.spacing.sm);
            let n = ctx.local.games.len();
            if n > 0 {
                ui.label(w::body_m(t, t!("{n} games · {size} installed · {libraries} libraries", n = n, size = ui::bytes(total), libraries = ctx.local.libraries.len()), t.text_dim));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.md - 1.0;
                ui.add_space(t.spacing.sm + 2.0);
                let all = !rows.is_empty() && rows.iter().all(|r| self.selected.contains(&r.id));
                let mut on = all;
                if w::checkbox(ui, t, &mut on, 30.0).on_hover_text(if all { t!("Untick all") } else { t!("Tick all") }).changed() {
                    self.selected = if on { rows.iter().map(|r| r.id).collect() } else { BTreeSet::new() };
                }
                let any_done = rows.iter().any(|r| matches!(r.kind, Kind::Done(_)));
                if ui.add_enabled_ui(any_done, |ui| w::button(ui, t, Some(icons::TRASH), t!("Clear Completed"), w::Kind::Pause, w::Size::Md)).inner.clicked() {
                    self.cleared.extend(rows.iter().filter(|r| matches!(r.kind, Kind::Done(_))).map(|r| r.id));
                }
                let pausable: Vec<AppId> = rows.iter().filter(|r| self.selected.contains(&r.id) && matches!(&r.kind, Kind::Active(d) if !d.paused && !d.verifying())).map(|r| r.id).collect();
                if ui.add_enabled_ui(!pausable.is_empty(), |ui| w::button(ui, t, Some(icons::PAUSE), t!("Pause Selected"), w::Kind::Pause, w::Size::Md)).inner.clicked() {
                    for id in pausable {
                        ctx.send(client::Command::PauseDownload { app_id: id });
                    }
                }
                let startable: Vec<&Row> = rows.iter().filter(|r| self.selected.contains(&r.id) && matches!(&r.kind, Kind::Active(d) if d.paused) || self.selected.contains(&r.id) && matches!(r.kind, Kind::Queued { .. })).collect();
                if ui.add_enabled_ui(!startable.is_empty(), |ui| w::button(ui, t, Some(icons::DOWNLOAD), t!("Download Selected"), w::Kind::Install, w::Size::Md)).inner.clicked() {
                    // Paused ones carry on; queued ones move to the front, in
                    // their order (the last one sent ends up first).
                    for r in startable.iter().rev() {
                        match r.kind {
                            Kind::Active(_) => ctx.send(client::Command::ResumeDownload { app_id: r.id }),
                            _ => ctx.send(client::Command::DownloadNow { app_id: r.id }),
                        }
                    }
                }
            });
        });
    }

    /// One game (Figma "Frame 421" `757:9589`): its place in the queue (drag
    /// the handle to move it), the picture and name, the speed graph and
    /// speeds, how far it got, its button and its check box.
    fn row(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, row: &Row, queued: &[AppId]) {
        let c = &t.components.download_row;
        let h = t.dense(126.0);
        let id = row.id;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let look = t.look(c, w::interact(ui, &resp, self.selected.contains(&id)));
        w::paint_box(ui, t, rect, c, &look, 0.0);
        let pad = t.padding_of(c);
        let inner = Rect::from_min_max(rect.min + pad, rect.max - pad);
        let queue_pos = queued.iter().position(|q| *q == id);

        // Dropping a dragged row here puts it at this place in the queue.
        if let (Some(to), Some(from)) = (queue_pos, resp.dnd_release_payload::<AppId>()) {
            if *from != id {
                ctx.send(client::Command::MoveDownload { app_id: *from, to });
            }
        }
        if let (Some(_), Some(_)) = (queue_pos, resp.dnd_hover_payload::<AppId>()) {
            ui.painter().rect_stroke(rect, t.radius_of(c), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }

        // The handle and the place in the queue.
        let col = Rect::from_min_size(inner.min, vec2(34.0, inner.height()));
        let place = match (&row.kind, queue_pos) {
            (Kind::Active(_), _) => Some(1),
            (_, Some(i)) => Some(i + 1 + usize::from(ctx.state.downloads.values().any(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed)))),
            _ => None,
        };
        let handle = Rect::from_center_size(pos2(col.center().x, col.top() + 23.0 + 12.0), Vec2::splat(24.0));
        if queue_pos.is_some() {
            let mut hu = ui.new_child(UiBuilder::new().max_rect(handle).layout(Layout::left_to_right(Align::Center)));
            hu.dnd_drag_source(egui::Id::unique(("dl-drag", id)), id, |ui| {
                ui.label(w::text(icons::LIST, t.typography.heading_s.size, t.text_dim));
            })
            .response
            .on_hover_text(t!("Drag to change the order"));
        } else {
            w::icon_centered(ui.painter(), handle.center(), icons::LIST, t.typography.heading_s.size, t.text_faint);
        }
        let num = Rect::from_min_size(pos2(col.left(), col.top() + 76.0), vec2(34.0, 29.0));
        ui.painter().rect_filled(num, t.corner_small(), t.chrome);
        ui.painter().rect_stroke(num, t.corner_small(), egui::Stroke::new(1.0, t.raised), egui::StrokeKind::Inside);
        let label = place.map(|p| p.to_string()).unwrap_or_else(|| "0".into());
        ui.painter().text(num.center(), Align2::CENTER_CENTER, label, t.typography.body_m.font(), t.text_dim);

        // Picture and name.
        let img = Rect::from_min_size(pos2(col.right() + t.spacing.lg + 3.0, inner.top()), vec2((inner.height() * 363.0 / 106.0).round(), inner.height()));
        w::image_or_title(ui, t, img, &art::header(id), &row.name, t.corner_small());
        let title_x = img.right() + t.spacing.xl - 4.0;

        // The right side, from the right: check box, button, progress, speeds, graph.
        let mut x = inner.right();
        let check = Rect::from_center_size(pos2(x - 15.0, inner.center().y), Vec2::splat(30.0));
        let mut on = self.selected.contains(&id);
        let mut cu = ui.new_child(UiBuilder::new().max_rect(check).layout(Layout::left_to_right(Align::Center)));
        if w::checkbox(&mut cu, t, &mut on, 30.0).changed() {
            if on {
                self.selected.insert(id);
            } else {
                self.selected.remove(&id);
            }
        }
        x = check.left() - t.spacing.sm - 2.0;
        let btn_h = (t.height_of(&t.components.button_play, 40.0) * 1.325).round();
        let btn = Rect::from_min_max(pos2(x - 151.0, inner.center().y - btn_h / 2.0), pos2(x, inner.center().y + btn_h / 2.0));
        let mut bu = ui.new_child(UiBuilder::new().max_rect(btn).layout(Layout::right_to_left(Align::Center)));
        self.row_button(&mut bu, t, ctx, row);
        x = bu.min_rect().left().min(btn.left()) - t.spacing.xl - 6.0;
        // Cancel (or take it out of the queue), next to the button.
        if matches!(row.kind, Kind::Active(_) | Kind::Queued { .. }) {
            let cx = Rect::from_center_size(pos2(x + 6.0, inner.center().y), Vec2::splat(t.height_of(&t.components.icon_button, 36.0)));
            let mut xu = ui.new_child(UiBuilder::new().max_rect(cx).layout(Layout::left_to_right(Align::Center)));
            let (tip, cmd) = match &row.kind {
                Kind::Active(d) if d.verifying() => (t!("Cancel"), client::Command::CancelVerify { app_id: id }),
                Kind::Active(_) => (t!("Cancel"), client::Command::CancelDownload { app_id: id }),
                _ => (t!("Remove from queue"), client::Command::CancelDownload { app_id: id }),
            };
            if w::icon_button(&mut xu, t, icons::X, tip, false, 0).clicked() {
                ctx.send(cmd);
            }
            x = cx.left() - t.spacing.lg;
        }
        let prog_w = 279.0f32.min((x - title_x - 200.0).max(120.0));
        let prog = Rect::from_min_max(pos2(x - prog_w, inner.top() + 10.0), pos2(x, inner.bottom() - 10.0));
        self.progress_block(ui, t, prog, row);
        x = prog.left() - t.spacing.xl * 2.0;

        // Speeds and the graph, while there's room for them.
        if let Kind::Active(d) = &row.kind {
            let peak = self.history.iter().map(|s| s.1).max().unwrap_or(0).max(d.speed);
            let speeds = [(t!("Current"), self.rate(if d.paused { 0 } else { d.speed })), (t!("Peak"), self.rate(peak)), (t!("Disk"), format!("{}/s", ui::bytes(if d.paused { 0 } else { d.disk_speed })))];
            let col_w = 120.0;
            let speeds_left = x - col_w * 3.0;
            if speeds_left > title_x + 160.0 {
                for (i, (label, value)) in speeds.iter().enumerate() {
                    let at = pos2(speeds_left + col_w * i as f32, inner.center().y - 20.0);
                    ui.painter().text(at, Align2::LEFT_TOP, label.to_uppercase(), t.typography.body_s.font(), t.text_dim);
                    ui.painter().text(at + vec2(0.0, 24.0), Align2::LEFT_TOP, value, t.typography.body_m.font(), t.text);
                }
                let graph = Rect::from_min_max(pos2(speeds_left - 63.0 - 98.0, inner.top() + 11.0), pos2(speeds_left - 63.0, inner.bottom() - 11.0));
                if graph.left() > title_x + 200.0 && !d.paused {
                    self.graph(ui, t, graph);
                }
            }
        } else {
            let (label, value) = match &row.kind {
                Kind::Update { size } => (t!("Size"), ui::bytes(*size)),
                Kind::Done(d) => (t!("Downloaded"), ui::bytes(if d.download_total > 0 { d.download_total } else { d.disk_total })),
                _ => ("", String::new()),
            };
            let at = pos2(x - 120.0, inner.center().y - 20.0);
            if !label.is_empty() && at.x > title_x + 160.0 {
                ui.painter().text(at, Align2::LEFT_TOP, label.to_uppercase(), t.typography.body_s.font(), t.text_dim);
                ui.painter().text(at + vec2(0.0, 24.0), Align2::LEFT_TOP, value, t.typography.body_m.font(), t.text);
            }
        }

        // The name (and notes about skipped parts) over what's left.
        let text = Rect::from_min_max(pos2(title_x, inner.top()), pos2((x - t.spacing.lg).max(title_x + 40.0), inner.bottom()));
        let mut tu = ui.new_child(UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
        tu.add_space(t.spacing.sm + 2.0);
        tu.add(egui::Label::new(w::heading_s(t, &row.name, t.text)).truncate());
        match &row.kind {
            Kind::Active(d) | Kind::Done(d) => {
                if let Some(e) = &d.error {
                    tu.add(egui::Label::new(w::body_s(t, i18n::tr_dyn(e), t.danger)).truncate());
                }
                notes(&mut tu, t, d);
            }
            _ => {}
        }

        // Click opens the game; right-click has everything else.
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            ctx.navigate("library", Some(id.to_string()));
        }
        resp.context_menu(|ui| self.row_menu(ui, t, ctx, row, queue_pos, queued.len()));
    }

    /// The row's main button: Pause, Resume, Download, Update or Play.
    fn row_button(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, row: &Row) {
        let id = row.id;
        let lg = w::Size::Lg;
        match &row.kind {
            Kind::Active(d) if d.verifying() => {
                ui.add_enabled_ui(false, |ui| w::button(ui, t, Some(icons::SHIELD), t!("Verifying…"), w::Kind::Pause, lg));
            }
            Kind::Active(d) if d.paused => {
                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Resume"), w::Kind::Install, lg).clicked() {
                    ctx.send(client::Command::ResumeDownload { app_id: id });
                }
            }
            Kind::Active(_) => {
                if w::pause_button(ui, t, t!("Pause")).clicked() {
                    ctx.send(client::Command::PauseDownload { app_id: id });
                }
            }
            Kind::Queued { .. } => {
                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Download"), w::Kind::Install, lg).on_hover_text(t!("Download now")).clicked() {
                    ctx.send(client::Command::DownloadNow { app_id: id });
                }
            }
            Kind::Update { .. } => {
                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Update"), w::Kind::Install, lg).clicked() {
                    ctx.navigate("library", Some(format!("update:{id}")));
                }
            }
            Kind::Done(d) if d.error.is_some() => {
                if w::button(ui, t, Some(icons::REFRESH), t!("Retry"), w::Kind::Install, lg).clicked() {
                    ctx.navigate("library", Some(format!("install:{id}")));
                }
            }
            Kind::Done(_) => {
                if w::play_button(ui, t, t!("Play")).clicked() {
                    ctx.navigate("library", Some(format!("play:{id}")));
                }
            }
        }
    }

    /// Time left and status over the bar, sizes under it (Figma "Frame 97" `755:9341`).
    fn progress_block(&self, ui: &mut Ui, t: &Theme, r: Rect, row: &Row) {
        let p = ui.painter().clone();
        let (status, eta, fraction, sizes): (String, Option<String>, f32, Option<((u64, u64), (u64, u64))>) = match &row.kind {
            Kind::Active(d) => {
                let pct = (d.fraction() * 100.0) as u32;
                let status = if d.paused { t!("Paused").to_uppercase() } else { format!("{} {pct}%", i18n::tr_dyn(d.activity()).to_uppercase()) };
                let left = d.download_total.saturating_sub(d.downloaded);
                let eta = (!d.paused && !d.verifying() && d.speed > 0).then(|| t!("{time} left", time = ui::playtime(left / d.speed / 60 + 1)).to_string());
                (status, eta, d.fraction(), Some((d.shown(), (d.disk_done, d.disk_total))))
            }
            Kind::Queued { set_aside } => {
                let status = match set_aside {
                    Some(f) => t!("Queued · {pct}% done", pct = (f * 100.0) as u32).to_uppercase(),
                    None => t!("Awaiting download").to_uppercase(),
                };
                (status, None, set_aside.unwrap_or(0.0), None)
            }
            Kind::Update { .. } => (t!("Update required").to_uppercase(), None, 0.0, None),
            Kind::Done(d) if d.error.is_some() => (t!("Failed").to_uppercase(), None, 0.0, None),
            Kind::Done(d) => (t!("Completed").to_uppercase(), None, 1.0, Some(((d.download_total, d.download_total), (d.disk_total, d.disk_total)))),
        };
        let top = r.top() + 12.0;
        let color = if matches!(row.kind, Kind::Update { .. }) { t.away } else { t.text_dim };
        p.text(pos2(r.right(), top), Align2::RIGHT_TOP, status, t.typography.body_m.font(), color);
        if let Some(eta) = eta {
            p.text(pos2(r.left(), top), Align2::LEFT_TOP, eta, t.typography.body_m.font(), t.text);
        }
        let bar = Rect::from_min_size(pos2(r.left(), top + 30.0), vec2(r.width(), t.height_of(&t.components.progress, 6.0)));
        let mut bu = ui.new_child(UiBuilder::new().max_rect(bar));
        w::progress(&mut bu, t, fraction, r.width());
        let Some(((done, total), (disk_done, disk_total))) = sizes else { return };
        // "↓ 1.5 GB / 6 GB   ▤ 3 GB / 12 GB", right-aligned under the bar.
        let y = bar.bottom() + 13.0;
        let mut right = r.right();
        for (icon, a, b) in [(icons::DRIVES, disk_done, disk_total), (icons::DOWNLOAD, done, total)] {
            if b == 0 {
                continue;
            }
            let dim = p.layout_no_wrap(format!(" / {}", ui::bytes(b)), t.typography.body_m.font(), t.text_dim);
            let main = p.layout_no_wrap(ui::bytes(a.min(b)), t.typography.body_m.font(), t.text);
            let x = right - dim.size().x;
            p.galley(pos2(x, y), dim, t.text_dim);
            let x2 = x - main.size().x;
            p.galley(pos2(x2, y), main, t.text);
            w::icon_centered(&p, pos2(x2 - 14.0, y + 9.0), icon, 16.0, t.text_dim);
            right = x2 - 28.0 - t.spacing.sm;
        }
    }

    /// The last minute's network speed as bars, the disk's as a line
    /// (Figma "Group 73" `755:9359`).
    fn graph(&self, ui: &Ui, t: &Theme, r: Rect) {
        let now = self.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        const BARS: usize = 10;
        let mut net = [0u64; BARS];
        let mut disk = [0u64; BARS];
        for s in &self.history {
            let age = now - s.0;
            if !(0.0..GRAPH_SECS).contains(&age) {
                continue;
            }
            let i = BARS - 1 - ((age / GRAPH_SECS) * BARS as f64) as usize;
            net[i] = net[i].max(s.1);
            disk[i] = disk[i].max(s.2);
        }
        let peak = net.iter().chain(disk.iter()).copied().max().unwrap_or(0).max(1) as f32;
        let p = ui.painter();
        let pitch = r.width() / BARS as f32;
        let mut line = Vec::new();
        for i in 0..BARS {
            let x = r.left() + pitch * i as f32;
            let hgt = (net[i] as f32 / peak * r.height()).max(2.0);
            p.rect_filled(Rect::from_min_max(pos2(x, r.bottom() - hgt), pos2(x + 4.0, r.bottom())), 1.0, t.accent_2);
            line.push(pos2(x + 2.0, r.bottom() - disk[i] as f32 / peak * r.height()));
        }
        p.add(egui::Shape::line(line, egui::Stroke::new(1.5, t.play)));
    }

    /// Right-click on a row: everything the row's buttons do and more.
    fn row_menu(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, row: &Row, queue_pos: Option<usize>, queued: usize) {
        ui.set_min_width(220.0);
        let id = row.id;
        let item = |ui: &mut Ui, icon: &str, label: &str| {
            let r = w::menu_item(ui, t, icon, label).clicked();
            if r {
                ui.close();
            }
            r
        };
        match &row.kind {
            Kind::Active(d) if !d.verifying() => {
                if d.paused {
                    if item(ui, icons::PLAY, t!("Resume download")) {
                        ctx.send(client::Command::ResumeDownload { app_id: id });
                    }
                } else if item(ui, icons::PAUSE, t!("Pause download")) {
                    ctx.send(client::Command::PauseDownload { app_id: id });
                }
                if item(ui, icons::X, t!("Cancel download")) {
                    ctx.send(client::Command::CancelDownload { app_id: id });
                }
                ui.separator();
            }
            Kind::Queued { .. } => {
                if item(ui, icons::DOWNLOAD, t!("Download now")) {
                    ctx.send(client::Command::DownloadNow { app_id: id });
                }
                if let Some(i) = queue_pos {
                    if i > 0 && item(ui, icons::CARET_UP, t!("Move up")) {
                        ctx.send(client::Command::MoveDownload { app_id: id, to: i - 1 });
                    }
                    if i + 1 < queued && item(ui, icons::CARET_DOWN, t!("Move down")) {
                        ctx.send(client::Command::MoveDownload { app_id: id, to: i + 1 });
                    }
                }
                if item(ui, icons::X, t!("Remove from queue")) {
                    ctx.send(client::Command::CancelDownload { app_id: id });
                }
                ui.separator();
            }
            _ => {}
        }
        if item(ui, icons::BOOKS, t!("Open game page")) {
            ctx.navigate("library", Some(id.to_string()));
        }
        if item(ui, icons::NOTE, t!("See what's new")) {
            ctx.navigate("library", Some(format!("news:{id}")));
        }
        if let Some(g) = ctx.local.games.iter().find(|g| g.app_id == id).cloned() {
            if item(ui, icons::SHIELD, t!("Verify game files")) {
                ctx.navigate("library", Some(format!("verify:{id}")));
            }
            if item(ui, icons::FOLDER, t!("Browse local files")) {
                local::open_folder(&g.install_dir);
            }
        }
        if item(ui, icons::SLIDERS, t!("Properties…")) {
            ctx.navigate("library", Some(format!("props:{id}")));
        }
    }

    /// Downloads running right now (not paused or finished).
    fn running<'a>(ctx: &'a Ctx<'_>) -> impl Iterator<Item = &'a model::DownloadStatus> + 'a {
        ctx.state.downloads.values().filter(|d| !d.paused && !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed))
    }

    /// Note the current speed for the graph.
    fn sample(&mut self, ctx: &Ctx) {
        let started = *self.started.get_or_insert_with(std::time::Instant::now);
        let now = started.elapsed().as_secs_f64();
        if self.history.back().is_some_and(|s| now - s.0 < 0.5) {
            return;
        }
        let (net, disk) = Self::running(ctx).fold((0, 0), |(n, d), x| (n + x.speed, d + x.disk_speed));
        self.history.push_back((now, net, disk));
        while self.history.front().is_some_and(|s| now - s.0 > GRAPH_SECS) {
            self.history.pop_front();
        }
    }

    /// The download settings; returns where they are (for the gear).
    fn settings_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) -> Rect {
        if !self.region_sent && ctx.state.steam_id != 0 {
            self.region_sent = true;
            ctx.send(region_command(&self.settings.region));
        }
        if self.regions.is_none() && !self.probing {
            self.probing = true;
            ctx.send(client::Command::ProbeRegions);
        }
        let title = ui.label(w::heading_s(t, t!("Download settings"), t.text)).rect;
        ui.add_space(t.spacing.sm);
        // Master auto-update switch, like Steam's.
        {
            let mut f = *self.flags.lock().unwrap();
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(w::body_l(t, t!("Automatic updates"), t.text));
                        ui.label(w::body_s(t, t!("Keep installed games up to date. Turn off to update only when you choose (or when you launch a game)."), t.text_dim));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        w::toggle(ui, t, &mut f.auto_updates);
                    });
                });
            });
            *self.flags.lock().unwrap() = f;
        }
        ui.add_space(t.spacing.sm + 2.0);
        self.region_card(ui, t, ctx);
        ui.add_space(t.spacing.sm + 2.0);
        let s = &mut self.settings;
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("dl-settings").num_columns(2).spacing(vec2(t.spacing.xl + t.spacing.sm, t.spacing.md + 2.0)).show(ui, |ui| {
                ui.label(w::body_m(t, t!("Limit bandwidth"), t.text));
                ui.horizontal(|ui| {
                    w::toggle(ui, t, &mut s.limit_bandwidth);
                    ui.add_enabled(s.limit_bandwidth, egui::Slider::new(&mut s.limit_kbps, 256..=100_000).logarithmic(true).suffix(" KB/s"));
                });
                ui.end_row();

                ui.label(w::body_m(t, t!("Allow downloads while playing"), t.text));
                w::toggle(ui, t, &mut s.while_playing);
                ui.end_row();

                ui.label(w::body_m(t, t!("Only auto-update games between"), t.text));
                ui.horizontal(|ui| {
                    w::toggle(ui, t, &mut s.schedule);
                    ui.add_enabled_ui(s.schedule, |ui| {
                        ui.add(egui::DragValue::new(&mut s.schedule_from).range(0..=23).suffix(":00"));
                        ui.label("—");
                        ui.add(egui::DragValue::new(&mut s.schedule_to).range(0..=23).suffix(":00"));
                    });
                });
                ui.end_row();

                ui.label(w::body_m(t, t!("Show speeds in bits per second"), t.text));
                w::toggle(ui, t, &mut s.show_bits);
                ui.end_row();

                ui.label(w::body_m(t, t!("Parallel connections"), t.text));
                ui.add(egui::Slider::new(&mut s.connections, 4..=48));
                ui.end_row();
            });
            ui.add_space(t.spacing.sm - 2.0);
            ui.label(w::body_s(t, t!("SteamInRust measures Steam's content servers and uses the fastest one, reuses unchanged parts of games when updating, and checks every piece as it arrives."), t.text_faint));
        });
        title
    }
}

impl Downloads {
    /// Every download region, measured from this PC, fastest first.
    fn region_card(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(w::body_l(t, t!("Download region"), t.text));
                    ui.label(w::body_s(t, t!("Measured from this PC: the closest servers usually download fastest."), t.text_dim));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(!self.probing, |ui| {
                        if w::button(ui, t, Some(icons::REFRESH), if self.probing { t!("Measuring…") } else { t!("Measure again") }, w::Kind::Ghost, w::Size::Sm).clicked() {
                            self.probing = true;
                            ctx.send(client::Command::ProbeRegions);
                        }
                    });
                });
            });
            ui.add_space(t.spacing.sm);
            let mut pick: Option<String> = None;
            let current = self.settings.region.clone();
            let mut option = |ui: &mut Ui, id: String, name: String, ping: Option<u32>, tier: Option<u8>| {
                let on = current == id;
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), t.height_of(&t.components.menu_item, 38.0) - 4.0), Sense::click());
                let mut ru = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(r.min + vec2(t.spacing.sm, 0.0), vec2(24.0, r.height()))).layout(Layout::left_to_right(Align::Center)));
                let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                if on || hover > 0.0 {
                    ui.painter().rect_filled(r, t.corner_small(), if on { t.surface_hover } else { t.raised.gamma_multiply(hover) });
                }
                w::radio(&mut ru, t, on, "");
                let font = t.typography.body_m.font();
                ui.painter().text(pos2(r.left() + t.spacing.xl + t.spacing.sm + 4.0, r.center().y), Align2::LEFT_CENTER, &name, font.clone(), t.text);
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
                        let bar = Rect::from_min_size(pos2(x + i as f32 * 6.0, r.center().y + 5.0 - (i as f32 + 1.0) * 3.0), vec2(4.0, (i as f32 + 1.0) * 3.0));
                        ui.painter().rect_filled(bar, 1.0, if i <= tier { color } else { t.raised });
                    }
                    ui.painter().text(pos2(x + 30.0, r.center().y), Align2::LEFT_CENTER, label, t.typography.body_s.font(), color);
                }
                let ping_text = match ping {
                    Some(ms) => format!("{ms} ms"),
                    None if tier.is_some() => t!("unreachable").to_string(),
                    None => String::new(),
                };
                ui.painter().text(pos2(r.right() - t.spacing.md, r.center().y), Align2::RIGHT_CENTER, ping_text, t.typography.body_s.font(), t.text_dim);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    pick = Some(id);
                }
            };
            option(ui, "auto".into(), t!("Automatic (recommended)").to_string(), None, None);
            match &self.regions {
                None => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(t.accent));
                        ui.label(w::body_s(t, t!("Measuring every region…"), t.text_faint));
                    });
                }
                Some(list) => {
                    // The whole list, fastest first (no tiny scroll box).
                    for r in list {
                        let id = if r.cell_id == 0 { format!("host:{}", r.host) } else { r.cell_id.to_string() };
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
        Some(host) => client::Command::DownloadRegion { cell_id: None, host: Some(host.to_string()) },
        None => client::Command::DownloadRegion { cell_id: region.parse().ok(), host: None },
    }
}

/// A recently updated game: its picture, name, when and how much, and
/// "What's new".
fn small_row(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app_id: AppId, name: &str, status: &str, size: &str) {
    let c = &t.components.download_row;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), t.dense(74.0)), Sense::click());
    let look = t.look(c, w::interact(ui, &resp, false));
    w::paint_box(ui, t, rect, c, &look, 0.0);
    // Click opens the game; right-click has Steam's usual actions.
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        ctx.navigate("library", Some(app_id.to_string()));
    }
    resp.context_menu(|ui| {
        ui.set_min_width(220.0);
        let mut go = |ui: &mut Ui, icon: &str, label: &str, arg: String| {
            if w::menu_item(ui, t, icon, label).clicked() {
                ctx.navigate("library", Some(arg));
                ui.close();
            }
        };
        go(ui, icons::BOOKS, t!("Open game page"), app_id.to_string());
        go(ui, icons::NOTE, t!("See what's new"), format!("news:{app_id}"));
        go(ui, icons::SHIELD, t!("Verify game files"), format!("verify:{app_id}"));
        go(ui, icons::SLIDERS, t!("Properties…"), format!("props:{app_id}"));
        if w::menu_item(ui, t, icons::FOLDER, t!("Browse local files")).clicked() {
            if let Some(g) = ctx.local.games.iter().find(|g| g.app_id == app_id) {
                local::open_folder(&g.install_dir);
            }
            ui.close();
        }
    });
    let pad = t.padding_of(c);
    let img = Rect::from_min_size(rect.min + pad, vec2(((rect.height() - pad.y * 2.0) * 363.0 / 106.0).round(), rect.height() - pad.y * 2.0));
    w::image_or_skeleton(ui, t, img, &art::header(app_id), t.corner_small());
    let mut c = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(pos2(img.right() + t.spacing.lg, rect.top()), rect.max - vec2(pad.x + t.spacing.sm, 0.0))).layout(Layout::left_to_right(Align::Center)));
    c.vertical(|ui| {
        ui.add_space(t.spacing.md);
        ui.label(w::body_l(t, name, t.text));
        ui.label(w::body_s(t, format!("{status}  ·  {size}"), t.text_dim));
    });
    c.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if w::button(ui, t, Some(icons::NOTE), t!("What's new"), w::Kind::Ghost, w::Size::Sm).clicked() {
            ctx.navigate("library", Some(format!("news:{app_id}")));
        }
    });
    ui.add_space(t.spacing.sm);
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
        t!("{n} parts of this game were skipped (your license doesn't include them)", n = d.notes.len())
    };
    ui.add(egui::Label::new(w::body_s(t, format!("{}  {text}", ui::icons::INFO), t.text_faint)).truncate()).on_hover_text(d.notes.join("\n"));
}

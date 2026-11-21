//! The game Properties window (a separate OS window, like Steam's).

use ui::widgets::EditMenu;
use super::{GameSettings, Library};
use client::Command;
use egui::{Align, Layout, Margin, Sense, Ui, vec2};
use extensions::{Action, Ctx};
use i18n::t;
use model::{AppId, InstalledGame, OwnedApp, art};
use super::shortcuts::{self, ArtKind, ArtSource, Pin, is_shortcut};
use std::sync::{Arc, Mutex};
use store::events::WorkshopItem as RemoteItem;
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PropTab {
    #[default]
    General,
    Updates,
    Files,
    Betas,
    Dlc,
    Controller,
    Workshop,
    Privacy,
    Customization,
    /// A non-Steam game's name, program, folder and options.
    Shortcut,
}

/// A non-Steam game's tabs, like Steam's.
const SHORTCUT_TABS: [(PropTab, &str); 3] = [(PropTab::Shortcut, "Shortcut"), (PropTab::Privacy, "Privacy"), (PropTab::Customization, "Customization")];

const TABS: [(PropTab, &str); 9] = [
    (PropTab::General, "General"),
    (PropTab::Updates, "Updates"),
    (PropTab::Files, "Installed files"),
    (PropTab::Betas, "Versions & betas"),
    (PropTab::Dlc, "DLC"),
    (PropTab::Controller, "Controller"),
    (PropTab::Workshop, "Workshop"),
    (PropTab::Privacy, "Privacy"),
    (PropTab::Customization, "Customization"),
];

#[derive(Default)]
pub struct Props {
    pub app: AppId,
    pub tab: PropTab,
    pub workshop: Option<Arc<Mutex<Option<Vec<RemoteItem>>>>>,
    /// Workshop tab: search, sort, and items picked for batch unsubscribe.
    pub ws_search: String,
    pub ws_sort: WsSort,
    /// 0 any size, 1 under 100 MB, 2 100 MB to 1 GB, 3 over 1 GB.
    pub ws_size: u8,
    pub ws_selecting: bool,
    pub ws_selected: std::collections::BTreeSet<u64>,
    pub ws_confirm: bool,
    pub dlc_search: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum WsSort {
    #[default]
    Updated,
    Name,
    Size,
}

impl Library {
    pub(super) fn properties_window(&mut self, ui: &Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(app_id) = self.props.as_ref().map(|p| p.app) else { return };
        let name = self.shortcut(app_id).map(|g| g.name.clone()).or_else(|| ctx.state.app_name(app_id).map(String::from)).or_else(|| ctx.local.games.iter().find(|g| g.app_id == app_id).map(|g| g.name.clone())).unwrap_or_default();
        let tabs: &[(PropTab, &str)] = if is_shortcut(app_id) { &SHORTCUT_TABS } else { &TABS };
        let installed = ctx.local.games.iter().find(|g| g.app_id == app_id).cloned();
        let owned = ctx.state.apps.as_ref().and_then(|a| a.get(&app_id)).cloned();
        let mut builder = egui::ViewportBuilder::default().with_title(&name).with_inner_size([860.0, 620.0]).with_min_inner_size([700.0, 480.0]);
        // A preview copy keeps it off the screen, shown once it's there;
        // `--props-size=WxH` sets its size (to check narrow windows).
        if crate::app::offscreen() {
            builder = builder.with_position([crate::app::OFFSCREEN_POS[0] + 6000.0, crate::app::OFFSCREEN_POS[1]]).with_visible(false);
            let size = std::env::args().find_map(|a| a.strip_prefix("--props-size=").and_then(|v| v.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))));
            if let Some(size) = size {
                builder = builder.with_inner_size(size).with_min_inner_size(size);
            }
        }
        let mut close = false;
        ui.ctx().show_viewport_immediate(egui::ViewportId::from_hash_of(("props", app_id)), builder, |ui, _| {
            if ui.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            if crate::app::offscreen() {
                match ui.input(|i| i.viewport().outer_rect) {
                    Some(r) if r.right() < -1000.0 => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true)),
                    _ => {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(crate::app::OFFSCREEN_POS[0] + 6000.0, crate::app::OFFSCREEN_POS[1])));
                        ui.ctx().request_repaint();
                    }
                }
            }
            let nav = (ui.available_width() * 0.26).clamp(150.0, 210.0);
            egui::Panel::left("props-nav")
                .exact_size(nav)
                .resizable(false)
                .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::same(14)))
                .show(ui, |ui| {
                    let current = self.props.as_ref().map(|p| p.tab).unwrap_or_default();
                    for &(tab, label) in tabs {
                        let r = ui.add(egui::Button::selectable(current == tab, w::body_m(t, i18n::tr_dyn(label), t.text)).min_size(vec2(ui.available_width(), 34.0)));
                        if r.clicked() {
                            if let Some(p) = &mut self.props {
                                p.tab = tab;
                            }
                        }
                    }
                });
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg).inner_margin(Margin::same(22))).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let tab = self.props.as_ref().map(|p| p.tab).unwrap_or_default();
                    ui.label(w::heading_m(t, i18n::tr_dyn(tabs.iter().find(|x| x.0 == tab).map(|x| x.1).unwrap_or("")), t.text));
                    ui.add_space(10.0);
                    match tab {
                        PropTab::General => {
                            self.tab_cloud(ui, t, ctx, app_id, &name, installed.as_ref(), owned.as_ref());
                            self.tab_general(ui, t, app_id, owned.as_ref());
                        }
                        PropTab::Updates => self.tab_updates(ui, t, ctx, app_id, installed.as_ref(), owned.as_ref()),
                        PropTab::Files => self.tab_files(ui, t, ctx, app_id, installed.as_ref()),
                        PropTab::Betas => self.tab_betas(ui, t, ctx, app_id, installed.as_ref(), owned.as_ref()),
                        PropTab::Controller => self.tab_controller(ui, t, app_id),
                        PropTab::Dlc => self.tab_dlc(ui, t, ctx, app_id, installed.as_ref(), owned.as_ref()),
                        PropTab::Workshop => self.tab_workshop(ui, t, ctx, app_id, installed.as_ref()),
                        PropTab::Privacy => self.tab_privacy(ui, t, app_id),
                        PropTab::Customization => self.tab_custom(ui, t, app_id),
                        PropTab::Shortcut => self.tab_shortcut(ui, t, ctx, app_id),
                    }
                });
            });
        });
        if close {
            self.props = None;
        }
    }

    fn settings_mut(&mut self, app: AppId) -> &mut GameSettings {
        self.games.entry(app).or_default()
    }

    /// Steam Cloud, as in Steam's Properties: toggle, usage, sync now.
    #[allow(clippy::too_many_arguments)]
    fn tab_cloud(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, name: &str, installed: Option<&InstalledGame>, owned: Option<&OwnedApp>) {
        let Some(cfg) = owned.and_then(|o| o.cloud.clone()) else { return };
        // Usage comes from a look-only check, once per window.
        if let Some(g) = installed {
            if !ctx.state.cloud.contains_key(&app) && !ctx.state.cloud_syncing.contains(&app) && self.cloud_checked.insert(app) {
                ctx.send(Command::CloudSync { app_id: app, install_dir: g.install_dir.clone(), mode: client::cloud::Mode::Check, keep_local: vec![], keep_cloud: vec![], then_launch: None });
            }
        }
        let report = ctx.state.cloud.get(&app).cloned();
        let syncing = ctx.state.cloud_syncing.contains(&app);
        let mut changed = false;
        ui.label(w::body_m(t, t!("Cloud saves"), t.text));
        ui.label(w::body_s(t, t!("Steam Cloud keeps your game data the same on every PC."), t.text_dim));
        ui.add_space(6.0);
        w::panel(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let s = self.games.entry(app).or_default();
                let mut on = !s.cloud_off;
                if w::toggle(ui, t, &mut on).changed() {
                    s.cloud_off = !on;
                    changed = true;
                }
                ui.with_layout(Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
                    ui.add(egui::Label::new(w::body_m(t, t!("Keep saves for “{name}” in sync with Steam Cloud", name = name), t.text)).wrap());
                });
            });
        });
        // Used / available, like Steam.
        let quota = report.as_ref().map(|r| r.quota_bytes).filter(|q| *q > 0).unwrap_or(cfg.quota);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(w::body_s(t, t!("Available: {size}", size = ui::bytes(quota)), t.accent));
                ui.label(w::body_s(t, "/", t.text_faint));
                let used = report.as_ref().map(|r| ui::bytes(r.cloud_bytes)).unwrap_or_else(|| "…".into());
                ui.label(w::body_s(t, t!("Used: {size}", size = used), t.text_dim));
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            if let Some(g) = installed {
                ui.add_enabled_ui(!syncing, |ui| {
                    if w::button(ui, t, Some(icons::REFRESH), if syncing { t!("Syncing…") } else { t!("Sync now") }, w::Kind::Secondary, w::Size::Sm).clicked() {
                        ctx.send(Command::CloudSync { app_id: app, install_dir: g.install_dir.clone(), mode: client::cloud::Mode::Sync, keep_local: vec![], keep_cloud: vec![], then_launch: None });
                    }
                });
            }
            let backups = client::cloud::backups_dir(app);
            if backups.exists() && w::button(ui, t, Some(icons::FOLDER), t!("Backups of replaced saves"), w::Kind::Ghost, w::Size::Sm).clicked() {
                local::open_folder(&backups);
            }
            if let Some(r) = &report {
                let text = if !r.conflicts.is_empty() {
                    t!("{n} files changed both here and in the cloud", n = r.conflicts.len())
                } else if r.uploaded + r.downloaded > 0 {
                    t!("Synced {when}: {up} up, {down} down", when = ui::ago(r.finished), up = r.uploaded, down = r.downloaded)
                } else {
                    t!("{n} files in the cloud · checked {when}", n = r.cloud_files, when = ui::ago(r.finished))
                };
                ui.label(w::body_s(t, text, if r.conflicts.is_empty() { t.text_faint } else { t.away }));
            }
        });
        if let (Some(r), Some(g)) = (&report, installed) {
            if !r.conflicts.is_empty() {
                ui.horizontal(|ui| {
                    let names: Vec<String> = r.conflicts.iter().map(|c| c.name.clone()).collect();
                    if w::button(ui, t, Some(icons::DOWNLOAD), t!("Use the cloud's saves"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        ctx.send(Command::CloudSync { app_id: app, install_dir: g.install_dir.clone(), mode: client::cloud::Mode::Sync, keep_local: vec![], keep_cloud: names.clone(), then_launch: None });
                    }
                    if w::button(ui, t, Some(icons::DESKTOP), t!("Keep this PC's saves"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        ctx.send(Command::CloudSync { app_id: app, install_dir: g.install_dir.clone(), mode: client::cloud::Mode::Sync, keep_local: names, keep_cloud: vec![], then_launch: None });
                    }
                });
            }
        }
        ui.add_space(14.0);
        if changed {
            self.send_cloud_settings(ctx);
        }
    }

    /// DLC, like Steam's tab: choose which to install, with size.
    fn tab_dlc(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, installed: Option<&InstalledGame>, owned: Option<&OwnedApp>) {
        let mut dlc: Vec<OwnedApp> = ctx.state.apps.as_ref().map(|a| a.values().filter(|d| d.is_dlc() && d.parent == Some(app)).cloned().collect()).unwrap_or_default();
        dlc.sort_by_key(|d| d.name.to_lowercase());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
            if w::button(ui, t, Some(icons::STORE), t!("More in store"), w::Kind::Secondary, w::Size::Md).clicked() {
                ctx.navigate("store", Some(format!("app:{app}")));
            }
            let width = ui.available_width().min(260.0);
            let width = if width < 140.0 { ui.max_rect().width().min(260.0) } else { width };
            let search = &mut self.props.as_mut().expect("open").dlc_search;
            w::search(ui, t, search, &t!("Search…"), width);
        });
        ui.add_space(10.0);
        if dlc.is_empty() {
            ui.label(w::body_m(t, t!("You don't own any DLC for this game."), t.text_faint));
            return;
        }
        let q = self.props.as_ref().map(|p| p.dlc_search.trim().to_lowercase()).unwrap_or_default();
        // Size of each DLC's own content, from the game's depots.
        let size_of = |d: AppId| -> u64 { owned.map(|o| o.depots.iter().filter(|x| x.dlc_app == Some(d)).filter_map(|x| x.manifest("public").map(|m| m.1)).sum()).unwrap_or(0) };
        let mut changed = false;
        // What's on disk now: a DLC with content counts as installed when its
        // depots are (an update keeps those; ticking adds, unticking removes).
        let on_disk: Vec<local::manage::InstalledDepot> = installed.map(local::manage::installed_depots).unwrap_or_default();
        let has_content = |d: AppId| owned.is_some_and(|o| o.depots.iter().any(|x| x.dlc_app == Some(d)));
        let is_installed = |d: AppId| on_disk.iter().any(|x| x.dlc_app == Some(d));
        egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.add_sized(vec2(70.0, 30.0), egui::Label::new(w::styled(t!("INSTALL"), t.typography.caption, t.text_faint)));
                ui.add_sized(vec2((ui.available_width() - 110.0).max(0.0), 30.0), egui::Label::new(w::styled(t!("NAME"), t.typography.caption, t.text_faint)));
                ui.label(w::styled(t!("SIZE"), t.typography.caption, t.text_faint));
            });
            for d in dlc.iter().filter(|d| q.is_empty() || d.name.to_lowercase().contains(&q)) {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add_space(30.0);
                    let s = self.games.entry(app).or_default();
                    let mut on = if installed.is_some() && has_content(d.app_id) {
                        (is_installed(d.app_id) && !s.disabled_dlc.contains(&d.app_id)) || s.enabled_dlc.contains(&d.app_id)
                    } else {
                        // Not installed yet: a new install gets owned DLC unless unticked.
                        !s.disabled_dlc.contains(&d.app_id)
                    };
                    if w::check(ui, t, &mut on, "").changed() {
                        if on {
                            s.disabled_dlc.remove(&d.app_id);
                            s.enabled_dlc.insert(d.app_id);
                        } else {
                            s.disabled_dlc.insert(d.app_id);
                            s.enabled_dlc.remove(&d.app_id);
                        }
                        changed = true;
                    }
                    ui.add_space(22.0);
                    // DLC without its own art shows the game's, with a DLC badge, like Steam.
                    let (r, _) = ui.allocate_exact_size(vec2(96.0, 45.0), Sense::hover());
                    let own = d.store_assets && w::paint_cover(ui, r, &art::header(d.app_id), t.corner_small());
                    if !own {
                        w::image_or_skeleton(ui, t, r, &art::header(app), t.corner_small());
                        let badge = egui::Rect::from_min_size(r.min, vec2(28.0, 14.0));
                        ui.painter().rect_filled(badge, egui::CornerRadius { nw: t.corner_small().nw, se: t.corner_small().se, ne: 0, sw: 0 }, t.accent_2);
                        ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, "DLC", t.typography.caption.font(), t.text_on(t.accent_2));
                    }
                    ui.add_space(8.0);
                    ui.add_sized(vec2((ui.available_width() - 100.0).max(0.0), 45.0), egui::Label::new(w::body_m(t, &d.name, t.text)).wrap());
                    let size = size_of(d.app_id);
                    ui.label(w::body_s(t, if size > 0 { ui::bytes(size) } else { "—".into() }, t.text_dim));
                });
            }
            ui.add_space(6.0);
        });
        if changed {
            ctx.toast(t!("DLC choice saved. Apply now to download or remove it, or it applies on the next update."));
        }
        if let Some(g) = installed {
            ui.add_space(10.0);
            let running = ctx.state.downloads.get(&app).is_some_and(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed) && !d.verifying());
            let hint = if running { t!("Restarts the running download with your DLC choice") } else { t!("Downloads ticked DLC and removes unticked DLC") };
            if w::button(ui, t, Some(icons::DOWNLOAD), t!("Apply now"), w::Kind::Primary, w::Size::Sm).on_hover_text(hint).clicked() {
                // A running download has its file list fixed: start it again.
                if running {
                    ctx.send(Command::CancelDownload { app_id: app });
                }
                self.queue_install(ctx, app, Some(g.library.clone()));
            }
        }
    }

    fn tab_general(&mut self, ui: &mut Ui, t: &Theme, app: AppId, owned: Option<&OwnedApp>) {
        let s = self.settings_mut(app);
        section(ui, t, t!("Launch options"), t!("Extra arguments added when the game starts."), |ui| {
            ui.add(egui::TextEdit::singleline(&mut s.args).hint_text("-novid -fullscreen").desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
        });
        section(ui, t, t!("In-game overlay"), t!("Open friends and chat with Shift+Tab while playing."), |ui| {
            w::toggle(ui, t, &mut s.overlay_on);
        });
        let langs: Vec<String> = {
            let mut v: Vec<String> = owned.map(|o| o.depots.iter().map(|d| d.language.clone()).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
            v.sort();
            v.dedup();
            v
        };
        if !langs.is_empty() {
            section(ui, t, t!("Language"), t!("Which language to download for this game."), |ui| {
                let current = if s.language.is_empty() { t!("Same as SteamInRust").to_string() } else { s.language.clone() };
                egui::ComboBox::from_id_salt(("game-lang", app)).selected_text(current).width(ui.available_width().min(240.0)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.language, String::new(), t!("Same as SteamInRust"));
                    for l in &langs {
                        ui.selectable_value(&mut s.language, l.clone(), l);
                    }
                });
            });
        }
    }

    fn tab_updates(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, installed: Option<&InstalledGame>, owned: Option<&OwnedApp>) {
        let branch = self.games.get(&app).map(|g| g.branch.clone()).filter(|b| !b.is_empty()).unwrap_or_else(|| "public".into());
        if let Some(g) = installed {
            let (build, _) = local::manage::installed_build(g);
            let latest = owned.and_then(|o| o.branches.iter().find(|b| b.name == branch)).map(|b| (b.build_id, b.updated));
            w::panel(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new("upd-grid").num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                    ui.label(w::styled(t!("INSTALLED BUILD"), t.typography.caption, t.text_faint));
                    ui.label(w::body_m(t, if build == 0 { "—".into() } else { build.to_string() }, t.text));
                    ui.end_row();
                    if let Some((b, when)) = latest {
                        ui.label(w::styled(t!("LATEST BUILD"), t.typography.caption, t.text_faint));
                        ui.label(w::body_m(t, format!("{b}  ·  {}", super::news::date(when)), t.text));
                        ui.end_row();
                    }
                    ui.label(w::styled(t!("LAST UPDATED"), t.typography.caption, t.text_faint));
                    ui.label(w::body_m(t, g.last_updated.map(super::news::date).unwrap_or_else(|| "—".into()), t.text));
                    ui.end_row();
                });
                ui.add_space(8.0);
                match latest {
                    Some((b, _)) if b != 0 && build != 0 && b != build => {
                        ui.label(w::body_m(t, format!("{}  {}", icons::DOWNLOAD, t!("An update is available.")), t.away));
                        if w::button(ui, t, Some(icons::DOWNLOAD), t!("Update now"), w::Kind::Primary, w::Size::Sm).clicked() {
                            self.queue_install(ctx, app, Some(g.library.clone()));
                        }
                    }
                    Some(_) => {
                        ui.label(w::body_m(t, format!("{}  {}", icons::CHECK, t!("Up to date")), t.play));
                    }
                    None => {}
                }
            });
            ui.add_space(12.0);
        }
        let s = self.settings_mut(app);
        section(ui, t, t!("Automatic updates"), t!("When this game should be updated."), |ui| {
            let opts = [(0u8, t!("Always keep this game updated")), (1, t!("Only update when I launch it")), (2, t!("High priority: update before other games"))];
            for (v, l) in opts {
                w::radio_value(ui, t, &mut s.auto_update, v, l);
            }
        });
        section(ui, t, t!("Background downloads"), t!("Whether updates for this game may download while you play something else."), |ui| {
            let opts = [(0u8, t!("Use the global setting")), (1, t!("Always allow")), (2, t!("Never allow"))];
            for (v, l) in opts {
                w::radio_value(ui, t, &mut s.background, v, l);
            }
        });
    }

    fn tab_files(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, installed: Option<&InstalledGame>) {
        // Being downloaded: its folder already exists, so it can be opened.
        match super::setup_state(ctx, app, installed.is_some()) {
            super::Setup::Downloading(d) => {
                w::panel(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // The same measure as the Downloads page (what this download fetches).
                    let pct = d.fraction() as f64 * 100.0;
                    let label = if d.paused { t!("Download paused at {pct}%", pct = format!("{pct:.0}")) } else { t!("Installing… {pct}%", pct = format!("{pct:.0}")) };
                    ui.label(w::body_m(t, label, t.accent));
                    ui.label(w::body_s(t, format!("{} / {}", ui::bytes(d.disk_done), ui::bytes(d.disk_total)), t.text_dim));
                    w::progress(ui, t, (pct / 100.0) as f32, ui.available_width());
                    if let Some(dir) = &d.install_dir {
                        ui.add_space(6.0);
                        let path = dir.display().to_string();
                        ui.add(egui::Label::new(w::body_s(t, &path, t.text_dim).monospace()).truncate()).on_hover_text(&path);
                    }
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                        if let Some(dir) = d.install_dir.as_ref().filter(|p| p.exists()) {
                            if w::button(ui, t, Some(icons::FOLDER), t!("Browse…"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                local::open_folder(dir);
                            }
                        }
                        let (icon, label, cmd) = if d.paused {
                            (icons::PLAY, t!("Resume download"), Command::ResumeDownload { app_id: app })
                        } else {
                            ("\u{E39E}", t!("Pause download"), Command::PauseDownload { app_id: app })
                        };
                        if w::button(ui, t, Some(icon), label, w::Kind::Secondary, w::Size::Sm).clicked() {
                            ctx.send(cmd);
                        }
                        if w::button(ui, t, Some(icons::X), t!("Cancel download"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            ctx.send(Command::CancelDownload { app_id: app });
                        }
                    });
                });
                return;
            }
            super::Setup::Queued if installed.is_none() => {
                ui.label(w::body_m(t, t!("Waiting in the download queue."), t.accent));
                if w::button(ui, t, Some(icons::X), t!("Remove from download queue"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    ctx.send(Command::CancelDownload { app_id: app });
                }
                return;
            }
            _ => {}
        }
        let Some(g) = installed else {
            ui.label(w::body_m(t, t!("This game isn't installed."), t.text_dim));
            if w::button(ui, t, Some(icons::DOWNLOAD), t!("Install"), w::Kind::Primary, w::Size::Md).clicked() {
                self.install_dialog = Some(super::InstallDialog::new(app, ctx));
            }
            return;
        };
        w::panel(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::body_m(t, t!("Size of installation: {size}", size = ui::bytes(g.size_on_disk)), t.text));
            let path = g.install_dir.display().to_string();
            ui.add(egui::Label::new(w::body_s(t, &path, t.text_dim).monospace()).truncate()).on_hover_text(&path);
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if w::button(ui, t, Some(icons::FOLDER), t!("Browse…"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    local::open_folder(&g.install_dir);
                }
                if w::button(ui, t, Some(icons::SHIELD), t!("Verify integrity of game files"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    self.start_verify(ctx, g);
                }
                if w::button(ui, t, Some(icons::DESKTOP), t!("Add desktop shortcut"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    self.add_shortcut(ctx, app, g);
                }
                let moving = local::relocate::is_moving(app);
                let label = if moving { t!("Moving…") } else { t!("Move install folder…") };
                if ui.add_enabled_ui(!moving, |ui| w::button(ui, t, Some(icons::DRIVES), label, w::Kind::Secondary, w::Size::Sm)).inner.clicked() {
                    self.open_move(ui.ctx(), app);
                }
            });
        });
        if let Some(d) = ctx.state.downloads.get(&app).filter(|d| d.verifying()) {
            ui.add_space(8.0);
            ui.label(w::body_m(t, t!("Verifying files…"), t.text_dim));
            w::progress(ui, t, d.fraction(), ui.available_width());
        }
        if let Some(r) = self.reports.get(&app) {
            ui.add_space(10.0);
            let (icon, color) = if r.healthy() { (icons::CHECK, t.play) } else { (icons::WARNING, t.away) };
            ui.label(w::body_m(t, format!("{icon}  {}", t!("Last check: {ok} of {total} files OK", ok = r.ok, total = r.files)), color));
            if !r.healthy() && w::button(ui, t, Some(icons::DOWNLOAD), t!("Repair now"), w::Kind::Primary, w::Size::Sm).clicked() {
                self.queue_install(ctx, app, Some(g.library.clone()));
            }
        }
        ui.add_space(14.0);
        if w::button(ui, t, Some(icons::TRASH), t!("Uninstall…"), w::Kind::Danger, w::Size::Sm).clicked() {
            self.dialog = Some(super::Dialog::Uninstall);
            self.selected = Some(app);
        }
    }

    fn tab_betas(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, installed: Option<&InstalledGame>, owned: Option<&OwnedApp>) {
        let branches = owned.map(|o| o.branches.clone()).unwrap_or_default();
        let installed_branch = installed.map(|g| local::manage::installed_build(g).1).unwrap_or_else(|| "public".into());
        ui.label(w::body_m(t, t!("Choose which version of the game to play. Older versions and betas are provided by the developer."), t.text_dim));
        ui.add_space(8.0);
        if branches.is_empty() {
            ui.label(w::body_m(t, t!("Version information appears once your owned games are loaded from Steam."), t.text_faint));
            return;
        }
        let mut chosen = self.games.get(&app).map(|g| g.branch.clone()).filter(|b| !b.is_empty()).unwrap_or(installed_branch.clone());
        for b in &branches {
            let label = if b.name == "public" { t!("None (default version)").to_string() } else { b.name.clone() };
            let r = w::panel(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.xs);
                    w::radio_value(ui, t, &mut chosen, b.name.clone(), w::body_m(t, label, t.text));
                    if b.password {
                        w::pill(ui, t!("Needs a code"), t.away, t.away.gamma_multiply(0.15));
                    }
                    if b.name == installed_branch {
                        w::pill(ui, t!("Installed"), t.play, t.play.gamma_multiply(0.15));
                    }
                    ui.label(w::body_s(t, format!("{} {}  ·  {}", t!("build"), b.build_id, super::news::date(b.updated)), t.text_faint));
                });
                if !b.description.is_empty() {
                    ui.label(w::body_s(t, &b.description, t.text_dim));
                }
            });
            let _ = r;
            ui.add_space(6.0);
        }
        let s = self.settings_mut(app);
        s.branch = chosen.clone();
        ui.add_space(8.0);
        section(ui, t, t!("Private beta code"), t!("Some betas need a code from the developer."), |ui| {
            ui.add(egui::TextEdit::singleline(&mut s.beta_code).desired_width(ui.available_width().min(260.0)).margin(vec2(10.0, 8.0))).with_edit_menu();
        });
        // Like Steam: an installed game downloads the other version on
        // request; one that isn't installed just remembers the choice.
        match installed {
            Some(g) if chosen != installed_branch => {
                ui.label(w::body_m(t, t!("Switching versions downloads the files for that version."), t.away));
                if w::button(ui, t, Some(icons::DOWNLOAD), t!("Switch now"), w::Kind::Primary, w::Size::Sm).clicked() {
                    self.queue_install(ctx, app, Some(g.library.clone()));
                }
            }
            None if chosen != "public" => {
                ui.label(w::body_m(t, t!("This version installs when you install the game."), t.text_dim));
            }
            _ => {}
        }
    }

    fn tab_controller(&mut self, ui: &mut Ui, t: &Theme, app: AppId) {
        let s = self.settings_mut(app);
        section(ui, t, t!("Steam Input"), t!("Override controller handling for this game."), |ui| {
            for (v, l) in [(0u8, t!("Use the default settings")), (1, t!("Enable Steam Input")), (2, t!("Disable Steam Input"))] {
                w::radio_value(ui, t, &mut s.controller, v, l);
            }
        });
    }

    fn tab_workshop(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId, installed: Option<&InstalledGame>) {
        let Some(g) = installed else {
            ui.label(w::body_m(t, t!("This game isn't installed."), t.text_dim));
            return;
        };
        let items = local::manage::workshop_items(g);
        let total: u64 = items.iter().map(|i| i.size).sum();
        let summary = t!("{n} items · {size}", n = items.len(), size = ui::bytes(total));
        let (browse, sync) = (t!("Browse the Workshop"), t!("Download subscribed items"));
        let needed = text_w(ui, w::font_semibold(t.typography.body_l.size - 1.0), &summary) + button_w(ui, t, browse) + button_w(ui, t, sync) + t.spacing.xl;
        let buttons = |ui: &mut Ui, ctx: &mut Ctx| {
            if w::button(ui, t, Some(icons::DOWNLOAD), sync, w::Kind::Secondary, w::Size::Sm)
                .on_hover_text(t!("Downloads every item you subscribe to that's missing or out of date. This also happens before the game starts."))
                .clicked()
            {
                ctx.send(Command::SyncWorkshop { app_id: app });
                ctx.toast(t!("Checking your Workshop subscriptions…"));
            }
            if w::button(ui, t, Some(icons::STORE), browse, w::Kind::Secondary, w::Size::Sm).clicked() {
                ctx.act(Action::Navigate { page: "workshop".into(), arg: Some(app.to_string()) });
            }
        };
        if ui.available_width() >= needed {
            ui.horizontal(|ui| {
                ui.label(w::body_m(t, &summary, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| buttons(ui, ctx));
                });
            });
        } else {
            ui.label(w::body_m(t, &summary, t.text));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
                buttons(ui, ctx)
            });
        }
        ui.add_space(8.0);
        if items.is_empty() {
            ui.label(w::body_m(t, t!("You're not subscribed to any Workshop items for this game."), t.text_faint));
            return;
        }
        // Titles and previews from Steam (one request for all items).
        let need_fetch = self.props.as_ref().is_some_and(|p| p.workshop.is_none());
        if need_fetch {
            let slot = Arc::new(Mutex::new(None));
            if let (Some(rt), Some(ectx)) = (&self.rt, &self.egui) {
                let ids: Vec<u64> = items.iter().map(|i| i.id).collect();
                let (out, ectx) = (slot.clone(), ectx.clone());
                rt.spawn(async move {
                    let r = store::StoreClient::default().workshop_details(&ids).await.unwrap_or_default();
                    *out.lock().unwrap() = Some(r);
                    ectx.request_repaint();
                });
            }
            if let Some(p) = &mut self.props {
                p.workshop = Some(slot);
            }
        }
        let remote = self.props.as_ref().and_then(|p| p.workshop.as_ref()).and_then(|s| s.lock().unwrap().clone()).unwrap_or_default();
        let Some(props) = self.props.as_mut() else { return };
        let title_of = |id: u64| remote.iter().find(|r| r.id == id).map(|r| r.title.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("#{id}"));

        // Toolbar: search, sort, select mode.
        let sorts = [(WsSort::Updated, t!("Recently updated")), (WsSort::Name, t!("Name")), (WsSort::Size, t!("Size"))];
        let sizes = [(0u8, t!("Any size")), (1, t!("Under 100 MB")), (2, t!("100 MB – 1 GB")), (3, t!("Over 1 GB"))];
        let gap = t.spacing.sm;
        let (search_w, select_w) = (220.0, button_w(ui, t, t!("Select")) + 20.0);
        let one_row = search_w + choices_w(ui, t, &sorts) + choices_w(ui, t, &sizes) + select_w + gap * 4.0 <= ui.available_width();
        let mut select = |ui: &mut Ui| {
            let label = if props.ws_selecting { t!("Done") } else { t!("Select") };
            if w::toggle_chip(ui, t, label, &mut props.ws_selecting).changed() && !props.ws_selecting {
                props.ws_selected.clear();
            }
        };
        if one_row {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                w::search(ui, t, &mut props.ws_search, &t!("Search your items"), search_w);
                w::segmented(ui, t, &mut props.ws_sort, &sorts);
                w::segmented(ui, t, &mut props.ws_size, &sizes);
                select(ui);
            });
        } else {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let w_ = (ui.available_width() - select_w - gap).clamp(120.0, search_w);
                w::search(ui, t, &mut props.ws_search, &t!("Search your items"), w_);
                select(ui);
            });
            ui.add_space(gap);
            choices(ui, t, &mut props.ws_sort, &sorts);
            ui.add_space(gap);
            choices(ui, t, &mut props.ws_size, &sizes);
        }
        let q = props.ws_search.trim().to_lowercase();
        const MB: u64 = 1024 * 1024;
        let size_ok = |n: u64| match props.ws_size {
            1 => n < 100 * MB,
            2 => (100 * MB..1024 * MB).contains(&n),
            3 => n >= 1024 * MB,
            _ => true,
        };
        let mut shown: Vec<&local::manage::WorkshopItem> = items
            .iter()
            .filter(|i| size_ok(i.size))
            .filter(|i| q.is_empty() || title_of(i.id).to_lowercase().contains(&q) || i.id.to_string().contains(&q))
            .collect();
        match props.ws_sort {
            WsSort::Updated => shown.sort_by_key(|i| std::cmp::Reverse(i.updated)),
            WsSort::Name => shown.sort_by_key(|i| title_of(i.id).to_lowercase()),
            WsSort::Size => shown.sort_by_key(|i| std::cmp::Reverse(i.size)),
        }
        // Batch bar.
        if props.ws_selecting {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
                let all = !shown.is_empty() && shown.iter().all(|i| props.ws_selected.contains(&i.id));
                let mut pick_all = all;
                if w::check(ui, t, &mut pick_all, t!("Select all")).changed() {
                    if pick_all {
                        props.ws_selected.extend(shown.iter().map(|i| i.id));
                    } else {
                        props.ws_selected.clear();
                    }
                }
                let n = props.ws_selected.len();
                let size: u64 = items.iter().filter(|i| props.ws_selected.contains(&i.id)).map(|i| i.size).sum();
                ui.label(w::body_m(t, t!("{n} selected · {size}", n = n, size = ui::bytes(size)), t.text_dim));
                ui.add_enabled_ui(n > 0, |ui| {
                    if w::button(ui, t, Some(icons::TRASH), &t!("Unsubscribe selected ({n})", n = n), w::Kind::Danger, w::Size::Sm).clicked() {
                        props.ws_confirm = true;
                    }
                });
            });
        }
        ui.add_space(8.0);
        let mut removed: Vec<u64> = Vec::new();
        for item in &shown {
            let info = remote.iter().find(|r| r.id == item.id);
            let picked = props.ws_selected.contains(&item.id);
            let frame = w::panel(t).stroke(egui::Stroke::new(1.0, if picked { t.accent } else { t.border }));
            let resp = frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if props.ws_selecting {
                        let mut on = picked;
                        if w::check(ui, t, &mut on, "").changed() {
                            if on {
                                props.ws_selected.insert(item.id);
                            } else {
                                props.ws_selected.remove(&item.id);
                            }
                        }
                    }
                    let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                    match info.and_then(|i| i.preview.as_deref()) {
                        Some(u) => w::image_or_skeleton(ui, t, r, u, t.corner_small()),
                        None => w::placeholder(ui, t, r, "", t.corner_small()),
                    }
                    // Room for the buttons first; "Unsubscribe" is an icon when it's tight.
                    let icon_w = t.height_of(&t.components.icon_button, 36.0) + ui.spacing().item_spacing.x;
                    let wide = ui.available_width() > 420.0;
                    let buttons_w = if props.ws_selecting { 0.0 } else { icon_w * 2.0 + if wide { button_w(ui, t, t!("Unsubscribe")) } else { icon_w } };
                    let text_w = (ui.available_width() - buttons_w - t.spacing.sm).max(40.0);
                    ui.allocate_ui_with_layout(vec2(text_w, 64.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(text_w);
                        ui.add(egui::Label::new(w::body_m(t, title_of(item.id), t.text)).truncate());
                        ui.add(egui::Label::new(w::body_s(t, format!("{}  ·  {}", ui::bytes(item.size), t!("updated {when}", when = super::news::date(item.updated))), t.text_faint)).truncate());
                    });
                    if !props.ws_selecting {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let tip = t!("Remove it from this game and delete its files");
                            let unsubscribe = if wide {
                                w::button(ui, t, Some(icons::TRASH), t!("Unsubscribe"), w::Kind::Danger, w::Size::Sm).on_hover_text(tip)
                            } else {
                                w::icon_button(ui, t, icons::TRASH, &format!("{}: {tip}", t!("Unsubscribe")), false, 0)
                            };
                            if unsubscribe.clicked() {
                                ctx.send(Command::UnsubscribeWorkshop { app_id: app, file_id: item.id });
                                let _ = local::manage::delete_workshop_files(item);
                                removed.push(item.id);
                            }
                            if w::icon_button(ui, t, icons::FOLDER, t!("Open folder"), false, 0).clicked() {
                                local::open_folder(&item.folder);
                            }
                            if w::icon_button(ui, t, icons::CUBE, t!("Workshop page"), false, 0).clicked() {
                                ctx.navigate("workshop_item", Some(item.id.to_string()));
                            }
                        });
                    }
                });
            });
            // In select mode the whole row toggles.
            if props.ws_selecting && resp.response.interact(Sense::click()).clicked() {
                if !props.ws_selected.remove(&item.id) {
                    props.ws_selected.insert(item.id);
                }
            }
            ui.add_space(6.0);
        }
        if shown.is_empty() {
            ui.label(w::body_m(t, t!("No items match your search."), t.text_faint));
        }
        // Confirm batch unsubscribe.
        if props.ws_confirm {
            let n = props.ws_selected.len();
            let mut go = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique(("ws-confirm", app))).show(ui.ctx(), |ui| {
                ui.set_width(380.0);
                ui.label(w::heading_s(t, t!("Unsubscribe from {n} items?", n = n), t.text));
                ui.add(egui::Label::new(w::body_m(t, t!("They'll be removed from this game and their files deleted. You can subscribe again later from the Workshop."), t.text_dim)).wrap());
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if w::button(ui, t, Some(icons::TRASH), t!("Unsubscribe"), w::Kind::Danger, w::Size::Md).clicked() {
                        go = true;
                    }
                    if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                        cancel = true;
                    }
                });
            });
            if go {
                for item in items.iter().filter(|i| props.ws_selected.contains(&i.id)) {
                    ctx.send(Command::UnsubscribeWorkshop { app_id: app, file_id: item.id });
                    let _ = local::manage::delete_workshop_files(item);
                    removed.push(item.id);
                }
                props.ws_selected.clear();
                props.ws_selecting = false;
            }
            if go || cancel {
                props.ws_confirm = false;
            }
        }
        match removed.len() {
            0 => {}
            1 => ctx.toast(t!("Unsubscribed from item {id}", id = removed[0])),
            n => ctx.toast(t!("Unsubscribed from {n} items", n = n)),
        }
    }

    fn tab_privacy(&mut self, ui: &mut Ui, t: &Theme, app: AppId) {
        let mut hidden = self.hidden.contains(&app);
        section(ui, t, t!("Hide in library"), t!("Hidden games only show when you turn on “Show hidden games”."), |ui| {
            if w::toggle(ui, t, &mut hidden).changed() {
                if hidden {
                    self.hidden.insert(app);
                } else {
                    self.hidden.remove(&app);
                }
            }
        });
        let s = self.settings_mut(app);
        section(ui, t, t!("Mark as private"), t!("Don't show this game as “now playing” in your status while you play."), |ui| {
            w::toggle(ui, t, &mut s.private);
        });
    }

    /// Custom artwork, like Steam's: each picture with Change and Clear, and
    /// where the logo sits on the background.
    fn tab_custom(&mut self, ui: &mut Ui, t: &Theme, app: AppId) {
        ui.label(w::body_m(t, t!("Pictures you pick are copied into SteamInRust's own folder. Pictures set in Steam show here too."), t.text_dim));
        ui.add_space(t.spacing.sm);
        for kind in ArtKind::ALL {
            let (title, body, size) = art_info(kind);
            w::panel(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let current = shortcuts::custom_art_from(app, kind);
                    let url = match kind {
                        ArtKind::Capsule => shortcuts::capsule(app),
                        ArtKind::Hero => shortcuts::hero(app),
                        ArtKind::Logo => shortcuts::logo(app),
                        ArtKind::Header => shortcuts::header(app),
                        ArtKind::Icon => current.as_ref().map(|c| c.0.clone()).unwrap_or_default(),
                    };
                    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                    let shown = if kind == ArtKind::Logo { w::paint_contain(ui, r, &url, egui::Align2::CENTER_CENTER) } else { w::paint_cover(ui, r, &url, t.corner_small()) };
                    if !shown {
                        w::placeholder(ui, t, r, "", t.corner_small());
                    }
                    ui.vertical(|ui| {
                        ui.label(w::body_l(t, title, t.text));
                        ui.add(egui::Label::new(w::body_s(t, body, t.text_dim)).wrap());
                        let from = match current.as_ref().map(|c| c.1) {
                            Some(ArtSource::Ours) => t!("Your picture"),
                            Some(ArtSource::Steam) => t!("Set in Steam"),
                            None if is_shortcut(app) => t!("None yet"),
                            None => t!("Steam's picture"),
                        };
                        ui.label(w::body_s(t, from, t.text_faint));
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
                            if w::button(ui, t, Some(icons::IMAGE), t!("Change…"), w::Kind::Secondary, w::Size::Sm).clicked() {
                                pick_art(ui.ctx(), app, kind);
                            }
                            if ui.add_enabled_ui(current.is_some(), |ui| w::button(ui, t, Some(icons::X), t!("Clear"), w::Kind::Ghost, w::Size::Sm)).inner.clicked() {
                                shortcuts::clear_art(app, kind);
                            }
                        });
                    });
                });
                if kind == ArtKind::Logo {
                    ui.add_space(t.spacing.sm);
                    logo_position_editor(ui, t, app);
                }
            });
            ui.add_space(t.spacing.sm);
        }
    }

    /// A non-Steam game's details, like Steam's shortcut properties: name,
    /// program ("Target"), the folder it starts in, launch options, icon.
    fn tab_shortcut(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, app: AppId) {
        let Some(i) = self.custom.iter().position(|g| g.id == app) else {
            ui.label(w::body_m(t, t!("This game isn't in your library any more."), t.text_dim));
            return;
        };
        let mut remove = false;
        {
            let g = &mut self.custom[i];
            ui.horizontal(|ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::click());
                let icon = shortcuts::icon(app, None, Some(&g.icon));
                if !icon.as_deref().is_some_and(|u| w::paint_cover(ui, r, u, t.corner_small())) {
                    ui.painter().rect_filled(r, t.corner_small(), t.surface_hover);
                    w::icon_centered(ui.painter(), r.center(), icons::CONTROLLER, 40.0, t.text_dim);
                }
                if resp.on_hover_text(t!("Change the icon")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    pick_art(ui.ctx(), app, ArtKind::Icon);
                }
                ui.vertical(|ui| {
                    ui.label(w::body_s(t, t!("Name"), t.text_dim));
                    ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(ui.available_width()).margin(vec2(10.0, 8.0))).with_edit_menu();
                });
            });
            ui.add_space(t.spacing.md);
            let field = |ui: &mut Ui, label: &str, value: &mut String, hint: &str, folder: bool| {
                ui.label(w::body_s(t, label, t.text_dim));
                ui.horizontal(|ui| {
                    let bw = button_w(ui, t, t!("Browse…")) + t.spacing.md;
                    ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width((ui.available_width() - bw).max(120.0)).margin(vec2(10.0, 8.0))).with_edit_menu();
                    if w::button(ui, t, Some(icons::FOLDER), t!("Browse…"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        let picked = if folder { rfd::FileDialog::new().pick_folder() } else { rfd::FileDialog::new().pick_file() };
                        if let Some(p) = picked {
                            *value = p.display().to_string();
                        }
                    }
                });
                ui.add_space(t.spacing.sm);
            };
            field(ui, t!("Target"), &mut g.exe, t!("Path to the game's .exe"), false);
            field(ui, t!("Start in"), &mut g.start_dir, t!("The program's own folder"), true);
            ui.label(w::body_s(t, t!("Launch options"), t.text_dim));
            ui.add(egui::TextEdit::singleline(&mut g.args).hint_text("-novid -fullscreen").desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
            ui.add_space(t.spacing.lg);
            if g.from_steam {
                ui.label(w::body_s(t, t!("Added in Steam. Changes here stay in SteamInRust; Steam's own list isn't changed."), t.text_faint));
                ui.add_space(t.spacing.sm);
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
                if let Some(dir) = g.work_dir().filter(|d| d.is_dir()) {
                    if w::button(ui, t, Some(icons::FOLDER), t!("Browse local files"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        local::open_folder(&dir);
                    }
                }
                if w::button(ui, t, Some(icons::TRASH), t!("Remove from library"), w::Kind::Danger, w::Size::Sm).clicked() {
                    remove = true;
                }
            });
        }
        if remove {
            self.remove_shortcut(ctx, app);
            self.props = None;
        }
    }
}

/// A picture kind's title, note and preview size in Customization.
fn art_info(kind: ArtKind) -> (&'static str, &'static str, egui::Vec2) {
    match kind {
        ArtKind::Capsule => (t!("Cover"), t!("Shown in the library grid and collections. Best at 600 × 900."), vec2(100.0, 150.0)),
        ArtKind::Hero => (t!("Background"), t!("Shown at the top of the game's page. Best at 3840 × 1240."), vec2(220.0, 71.0)),
        ArtKind::Logo => (t!("Logo"), t!("Shown over the background. Transparent PNG works best."), vec2(220.0, 80.0)),
        ArtKind::Header => (t!("Wide header"), t!("Shown in the list view and recent games. Best at 920 × 430."), vec2(184.0, 86.0)),
        ArtKind::Icon => (t!("Icon"), t!("Shown in the game list. A square picture, 256 × 256 or smaller."), vec2(48.0, 48.0)),
    }
}

/// Pick a picture file and use it.
pub(super) fn pick_art(ctx: &egui::Context, app: AppId, kind: ArtKind) {
    let Some(src) = rfd::FileDialog::new().add_filter(t!("Pictures"), &["png", "jpg", "jpeg", "webp", "ico"]).pick_file() else { return };
    if let Err(e) = shortcuts::set_art(app, kind, &src) {
        tracing::warn!("custom art for {app}: {e}");
    }
    ctx.request_repaint();
}

/// Where the logo sits: Steam's five spots and its size.
fn logo_position_editor(ui: &mut Ui, t: &Theme, app: AppId) {
    let saved = shortcuts::logo_position(app);
    let mut pos = saved.unwrap_or_default();
    ui.label(w::body_s(t, t!("Logo position"), t.text_dim));
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.spacing.xs + 2.0, t.spacing.xs + 2.0);
        for pin in Pin::ALL {
            let label = match pin {
                Pin::BottomLeft => t!("Bottom left"),
                Pin::UpperLeft => t!("Top left"),
                Pin::CenterCenter => t!("Center"),
                Pin::UpperCenter => t!("Top center"),
                Pin::BottomCenter => t!("Bottom center"),
            };
            if w::chip(ui, t, label, saved.is_some() && pos.pin == pin).clicked() {
                pos.pin = pin;
                changed = true;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(w::body_s(t, t!("Size"), t.text_dim));
        changed |= ui.add(egui::Slider::new(&mut pos.width_pct, 10.0..=80.0).suffix("%")).changed();
        if saved.is_some() && w::button(ui, t, None, t!("Reset"), w::Kind::Ghost, w::Size::Sm).clicked() {
            shortcuts::set_logo_position(app, None);
        }
    });
    if changed {
        pos.height_pct = (pos.width_pct * 1.7).min(90.0);
        shortcuts::set_logo_position(app, Some(pos));
    }
}

/// How wide a line of text is.
fn text_w(ui: &Ui, font: egui::FontId, s: &str) -> f32 {
    ui.painter().layout_no_wrap(s.to_string(), font, egui::Color32::PLACEHOLDER).size().x
}

/// About how wide a small button with an icon is.
fn button_w(ui: &Ui, t: &Theme, label: &str) -> f32 {
    let c = w::button_style(t, w::Kind::Secondary);
    let fid = t.font_of(c);
    let size = (fid.size - 2.0).max(10.0);
    text_w(ui, egui::FontId::new(size, fid.family), label) + size * 1.1 + (size * 0.55).round() + t.padding_of(c).x * 1.4 + ui.spacing().item_spacing.x
}

/// How wide [`w::segmented`] draws these choices.
fn choices_w<T>(ui: &Ui, t: &Theme, options: &[(T, &str)]) -> f32 {
    let font = egui::FontId::new(t.typography.body_m.size - 1.0, t.font_of(&t.components.sort).family);
    options.iter().map(|(_, l)| text_w(ui, font.clone(), l) + 22.0 + 2.0).sum::<f32>() + t.padding_of(&t.components.segmented).x * 2.0
}

/// Choices as one segmented block when the row has room, else as chips
/// that wrap onto more lines.
fn choices<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str)]) {
    if choices_w(ui, t, options) <= ui.available_width() {
        w::segmented(ui, t, value, options);
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.spacing.xs + 2.0, t.spacing.xs + 2.0);
        for (v, label) in options {
            if w::chip(ui, t, label, *value == *v).clicked() {
                *value = *v;
            }
        }
    });
}

/// Title + description + control block used throughout Properties.
fn section(ui: &mut Ui, t: &Theme, title: &str, body: &str, add: impl FnOnce(&mut Ui)) {
    w::panel(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(w::body_m(t, title, t.text));
        ui.label(w::body_s(t, body, t.text_dim));
        ui.add_space(6.0);
        add(ui);
    });
    ui.add_space(8.0);
}

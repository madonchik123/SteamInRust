//! The screenshot manager, like Steam's: your screenshots per game, on this
//! PC and uploaded, a large viewer with caption, spoiler tag and visibility,
//! upload, delete (with a question first), show on disk and copy.
//!
//! Opened from the View menu, a game's page and the in-game "Screenshot
//! saved" notice (`screenshots` page, with the game's id as its argument).

use client::screenshots::{CAPTION_MAX, Request, Response, Uploaded, clean_caption};
use client::{Command, Event};
use egui::{Align, Key, Layout, Margin, Rect, Sense, Ui, vec2};
use extensions::{Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use local::screenshots::{self as shots, Edit, Shot, Visibility};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use ui::{Theme, icons, widgets as w};

pub const PAGE: &str = "screenshots";

/// Picture addresses of this page: `sir-shot://<path>`.
const SCHEME: &str = "sir-shot://";

/// The account's `760/remote` folder: the only place the loader reads.
static ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Reads screenshots from disk: only pictures in the player's screenshot
/// folders, never any other file.
pub struct ShotLoader;

impl ShotLoader {
    pub fn install(ctx: &egui::Context) {
        ctx.add_bytes_loader(Arc::new(ShotLoader));
    }
}

fn allowed(path: &Path) -> bool {
    let picture = path.extension().is_some_and(|e| ["jpg", "jpeg", "png"].contains(&e.to_string_lossy().to_ascii_lowercase().as_str()));
    let in_folder = path.parent().and_then(Path::file_name).is_some_and(|n| n == "screenshots" || n == "thumbnails");
    let Some(root) = ROOT.read().unwrap_or_else(|e| e.into_inner()).clone() else { return false };
    let (Ok(real), Ok(root)) = (path.canonicalize(), root.canonicalize()) else { return false };
    picture && in_folder && real.starts_with(root)
}

impl egui::load::BytesLoader for ShotLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(ShotLoader)
    }

    fn load(&self, _ctx: &egui::Context, uri: &str) -> egui::load::BytesLoadResult {
        use egui::load::{BytesPoll, LoadError};
        let Some(path) = uri.strip_prefix(SCHEME) else { return Err(LoadError::NotSupported) };
        let path = Path::new(path);
        if !allowed(path) {
            return Err(LoadError::Loading(format!("{}not a screenshot", w::GONE)));
        }
        match std::fs::read(path) {
            Ok(bytes) => Ok(BytesPoll::Ready { size: None, bytes: egui::load::Bytes::Shared(bytes.into()), mime: None }),
            Err(e) => Err(LoadError::Loading(format!("{}{e}", w::GONE))),
        }
    }

    fn forget(&self, _uri: &str) {}
    fn forget_all(&self) {}
    fn byte_size(&self) -> usize {
        0
    }
}

fn uri(p: &Path) -> String {
    format!("{SCHEME}{}", p.display())
}

/// `--demo --demo-screenshots=<folder>`: a made-up Steam folder to test with.
fn demo_root() -> Option<PathBuf> {
    if !crate::app::demo() {
        return None;
    }
    std::env::args().find_map(|a| a.strip_prefix("--demo-screenshots=").map(PathBuf::from))
}

/// The Steam folder and account to work on.
fn place(ctx: &Ctx) -> Option<(PathBuf, u32)> {
    if let Some(root) = demo_root() {
        let account = std::fs::read_dir(root.join("userdata")).ok()?.flatten().find_map(|e| e.file_name().to_str()?.parse().ok())?;
        return Some((root, account));
    }
    if crate::app::demo() || ctx.local.steam_root.as_os_str().is_empty() || ctx.state.steam_id == 0 {
        return None;
    }
    Some((ctx.local.steam_root.clone(), model::account_id(ctx.state.steam_id)))
}

/// One screenshot: on this PC, on Steam, or both.
#[derive(Clone, PartialEq)]
struct Item {
    local: Option<Shot>,
    up: Option<Uploaded>,
}

impl Item {
    fn key(&self) -> String {
        match (&self.local, &self.up) {
            (Some(s), _) => format!("{}/{}", s.game, s.name),
            (None, Some(u)) => format!("up/{}", u.id),
            _ => String::new(),
        }
    }

    fn game(&self) -> u64 {
        self.local.as_ref().map(|s| s.game).or(self.up.as_ref().map(|u| u.app_id as u64)).unwrap_or(0)
    }

    fn created(&self) -> u64 {
        self.local.as_ref().map(|s| s.created).or(self.up.as_ref().map(|u| u.created)).unwrap_or(0)
    }

    fn caption(&self) -> &str {
        self.up.as_ref().map(|u| u.caption.as_str()).filter(|c| !c.is_empty()).or(self.local.as_ref().map(|s| s.caption.as_str())).unwrap_or("")
    }

    fn spoiler(&self) -> bool {
        self.up.as_ref().map(|u| u.spoiler).or(self.local.as_ref().map(|s| s.spoiler)).unwrap_or(false)
    }

    fn visibility(&self) -> Visibility {
        self.up.as_ref().map(|u| u.visibility).or(self.local.as_ref().map(|s| s.visibility)).unwrap_or_default()
    }

    /// Its id on Steam, from either side.
    fn published(&self) -> Option<u64> {
        self.up.as_ref().map(|u| u.id).or(self.local.as_ref().and_then(|s| s.published))
    }

    fn thumb(&self) -> Option<String> {
        if let Some(s) = &self.local {
            return Some(uri(s.thumbnail.as_ref().unwrap_or(&s.path)));
        }
        self.up.as_ref().and_then(|u| u.preview.clone().or(u.image.clone()))
    }

    fn full(&self) -> Option<String> {
        if let Some(s) = &self.local {
            return Some(uri(&s.path));
        }
        self.up.as_ref().and_then(|u| u.image.clone().or(u.preview.clone()))
    }
}

/// Screenshots on this PC joined with the uploaded list, newest first. An
/// upload recorded locally that Steam's list no longer has (deleted on the
/// web) shows as local only once the list has loaded.
fn merge(local: &[Shot], uploaded: &[Uploaded], list_complete: bool) -> Vec<Item> {
    let by_id: BTreeMap<u64, &Uploaded> = uploaded.iter().map(|u| (u.id, u)).collect();
    let mut used = BTreeSet::new();
    let mut out: Vec<Item> = local
        .iter()
        .map(|s| {
            let up = s.published.and_then(|id| by_id.get(&id).copied());
            if let Some(u) = up {
                used.insert(u.id);
            }
            let mut s = s.clone();
            if list_complete && up.is_none() {
                s.published = None;
                s.handle = None;
            }
            Item { local: Some(s), up: up.cloned() }
        })
        .collect();
    out.extend(uploaded.iter().filter(|u| !used.contains(&u.id)).map(|u| Item { local: None, up: Some(u.clone()) }));
    out.sort_by_key(|i| std::cmp::Reverse(i.created()));
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Show {
    #[default]
    All,
    OnPc,
    Uploaded,
}

/// A question before anything leaves or is deleted.
#[derive(Clone)]
enum Confirm {
    Upload(Vec<Shot>),
    DeleteLocal(Vec<Shot>),
    DeleteCloud(Vec<Item>),
}

type Slot<T> = Arc<Mutex<Option<T>>>;

#[derive(Default)]
pub struct Screenshots {
    game: Option<u64>,
    show: Show,
    local: Vec<Shot>,
    loading: Option<Slot<Vec<Shot>>>,
    stale: bool,
    uploaded: Vec<Uploaded>,
    up_total: u64,
    up_page: u32,
    up_loading: bool,
    up_error: Option<String>,
    /// Which account the lists are for.
    account: Option<u32>,
    selecting: bool,
    selected: BTreeSet<String>,
    /// The screenshot open in the viewer (by key), and its form.
    viewing: Option<String>,
    form: Edit,
    form_for: String,
    confirm: Option<Confirm>,
    /// Requests sent to Steam and not answered yet.
    pending: usize,
    copying: Option<Slot<Result<egui::ColorImage, String>>>,
}

impl Extension for Screenshots {
    fn manifest(&self) -> Manifest {
        Manifest { id: "screenshots", name: "Screenshots", author: "SteamInRust", description: "Your screenshots", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new(PAGE, "Screenshots", icons::CAMERA).hidden().order(60)]
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        // A game's id, or `view:<game>/<file>` for one screenshot.
        match arg.and_then(|a| a.strip_prefix("view:")) {
            Some(key) => {
                self.game = key.split('/').next().and_then(|g| g.parse().ok());
                self.viewing = Some(key.to_string());
            }
            None => {
                self.game = arg.and_then(|a| a.parse().ok());
                self.viewing = None;
            }
        }
        self.confirm = None;
        self.stale = true;
    }

    fn on_event(&mut self, event: &Event, ctx: &mut Ctx) {
        let Event::Screenshots(r) = event else { return };
        self.pending = self.pending.saturating_sub(1);
        match r {
            Response::List { page, result } => {
                self.up_loading = false;
                match result {
                    Ok((total, items)) => {
                        self.up_error = None;
                        self.up_total = *total;
                        self.up_page = *page;
                        if *page <= 1 {
                            self.uploaded.clear();
                        }
                        for u in items {
                            if !self.uploaded.iter().any(|x| x.id == u.id) {
                                self.uploaded.push(u.clone());
                            }
                        }
                    }
                    Err(e) => self.up_error = Some(e.clone()),
                }
            }
            Response::Uploaded { name, result } => match result {
                Ok(_) => {
                    ctx.toast(t!("Screenshot uploaded"));
                    self.stale = true;
                    self.refresh_uploaded(ctx);
                }
                Err(e) => ctx.toast(t!("Couldn't upload {name}: {error}", name = name, error = i18n::tr_dyn(e))),
            },
            Response::Updated { id, edit, result } => match result {
                Ok(()) => {
                    if let Some(u) = self.uploaded.iter_mut().find(|u| u.id == *id) {
                        u.caption = clean_caption(&edit.caption);
                        u.spoiler = edit.spoiler;
                        u.visibility = edit.visibility;
                    }
                    ctx.toast(t!("Saved"));
                }
                Err(e) => ctx.toast(t!("Couldn't save it on Steam: {error}", error = e)),
            },
            Response::Deleted { id, result } => match result {
                Ok(()) => {
                    self.uploaded.retain(|u| u.id != *id);
                    self.up_total = self.up_total.saturating_sub(1);
                    self.stale = true;
                    ctx.toast(t!("Deleted from Steam"));
                }
                Err(e) => ctx.toast(t!("Couldn't delete it from Steam: {error}", error = i18n::tr_dyn(e))),
            },
        }
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        self.load(ui.ctx(), ctx);
        self.finish_copy(ui.ctx(), ctx);
        let items = merge(&self.local, &self.uploaded, self.list_complete());
        let found = self.viewing.as_ref().is_some_and(|k| items.iter().any(|i| i.key() == *k));
        // Not there once the folder has been read: it was deleted.
        if self.viewing.is_some() && !found && self.loading.is_none() {
            self.viewing = None;
        }
        if found {
            self.viewer(ui, &t, ctx, &items);
        } else {
            self.browse(ui, &t, ctx, &items);
        }
        self.confirm_dialog(ui, &t, ctx);
    }

    fn as_any(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Steam can be asked: signed in, online, and not the demo.
fn online(ctx: &Ctx) -> bool {
    !crate::app::demo() && ctx.state.link == client::Link::Online && ctx.state.steam_id != 0
}

/// Why a Steam action can't run right now.
fn offline_note(ctx: &mut Ctx) {
    if crate::app::demo() {
        ctx.toast(t!("The demo account can't change anything on Steam."));
    } else {
        ctx.toast(t!("Go online to do this."));
    }
}

impl Screenshots {
    fn list_complete(&self) -> bool {
        self.up_page > 0 && self.uploaded.len() as u64 >= self.up_total && self.up_error.is_none()
    }

    fn refresh_uploaded(&mut self, ctx: &Ctx) {
        if online(ctx) {
            self.up_loading = true;
            self.pending += 1;
            ctx.send(Command::Screenshots(Request::List { page: 1 }));
        }
    }

    /// Read the folder when needed, off the UI thread.
    fn load(&mut self, egui_ctx: &egui::Context, ctx: &Ctx) {
        let place = place(ctx);
        let account = place.as_ref().map(|p| p.1);
        if account != self.account {
            self.account = account;
            self.local.clear();
            self.uploaded.clear();
            self.up_page = 0;
            self.up_total = 0;
            self.selected.clear();
            self.stale = true;
            *ROOT.write().unwrap_or_else(|e| e.into_inner()) = place.as_ref().map(|(root, a)| shots::game_folder(root, *a, 0).parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default());
            self.refresh_uploaded(ctx);
        } else if self.up_page == 0 && !self.up_loading && self.up_error.is_none() {
            self.refresh_uploaded(ctx);
        }
        if let Some(slot) = &self.loading {
            let done = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(list) = done {
                self.local = list;
                self.loading = None;
                let keys: BTreeSet<String> = merge(&self.local, &self.uploaded, false).iter().map(Item::key).collect();
                self.selected.retain(|k| keys.contains(k));
            }
        }
        if self.stale && self.loading.is_none() {
            self.stale = false;
            let Some((root, account)) = place else {
                self.local.clear();
                return;
            };
            let slot: Slot<Vec<Shot>> = Arc::new(Mutex::new(None));
            self.loading = Some(slot.clone());
            let repaint = egui_ctx.clone();
            std::thread::spawn(move || {
                let list = shots::list(&root, account);
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(list);
                repaint.request_repaint();
            });
        }
    }

    fn game_name(ctx: &Ctx, game: u64) -> String {
        u32::try_from(game).ok().and_then(|id| ctx.state.app_name(id).map(String::from).or_else(|| ctx.local.games.iter().find(|g| g.app_id == id).map(|g| g.name.clone()))).unwrap_or_else(|| game.to_string())
    }

    fn shown<'a>(&self, items: &'a [Item]) -> Vec<&'a Item> {
        items
            .iter()
            .filter(|i| self.game.is_none_or(|g| i.game() == g))
            .filter(|i| match self.show {
                Show::All => true,
                Show::OnPc => i.local.is_some(),
                Show::Uploaded => i.published().is_some(),
            })
            .collect()
    }

    /// The list of games and the grid.
    fn browse(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, items: &[Item]) {
        let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
        for i in items {
            *counts.entry(i.game()).or_default() += 1;
        }
        let mut games: Vec<(u64, String, usize)> = counts.into_iter().map(|(g, n)| (g, Self::game_name(ctx, g), n)).collect();
        games.sort_by_key(|g| g.1.to_lowercase());
        let side = (ui.available_width() * 0.2).clamp(210.0, 300.0);
        egui::Panel::left("shots-games").exact_size(side).resizable(false).frame(egui::Frame::new().fill(t.raised).inner_margin(Margin::same(t.spacing.md as i8))).show(ui, |ui| {
            ui.label(w::heading_s(t, t!("Screenshots"), t.text));
            ui.add_space(t.spacing.sm);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let row = |ui: &mut Ui, on: bool, name: &str, n: usize| {
                    ui.add(egui::Button::selectable(on, w::body_m(t, format!("{name}  ·  {n}"), t.text)).truncate().min_size(vec2(ui.available_width(), t.dense(32.0)))).clicked()
                };
                if row(ui, self.game.is_none(), &t!("All games"), items.len()) {
                    self.game = None;
                }
                for (g, name, n) in &games {
                    if row(ui, self.game == Some(*g), name, *n) {
                        self.game = Some(*g);
                    }
                }
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin::same(t.spacing.lg as i8))).show(ui, |ui| {
            self.toolbar(ui, t, ctx, items);
            ui.add_space(t.spacing.sm);
            let list = self.shown(items);
            if list.is_empty() {
                if self.loading.is_some() || self.up_loading {
                    ui.add_space(t.spacing.xl);
                    ui.vertical_centered(|ui| ui.spinner());
                } else {
                    w::empty(ui, t, icons::CAMERA, t!("No screenshots here yet"), t!("Press F12 in a game to take one."));
                }
                return;
            }
            let list: Vec<Item> = list.into_iter().cloned().collect();
            self.grid(ui, t, ctx, &list);
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, items: &[Item]) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
            for (v, label) in [(Show::All, t!("All")), (Show::OnPc, t!("On this PC")), (Show::Uploaded, t!("Uploaded"))] {
                if ui.add(egui::Button::selectable(self.show == v, w::body_m(t, label, t.text))).clicked() {
                    self.show = v;
                }
            }
            ui.add_space(t.spacing.md);
            let label = if self.selecting { t!("Done") } else { t!("Select") };
            if w::button(ui, t, Some(icons::CHECK), label, w::Kind::Secondary, w::Size::Sm).clicked() {
                self.selecting = !self.selecting;
                self.selected.clear();
            }
            if self.selecting {
                let chosen: Vec<Item> = items.iter().filter(|i| self.selected.contains(&i.key())).cloned().collect();
                ui.label(w::body_s(t, t!("{n} selected", n = chosen.len()), t.text_dim));
                self.bulk_buttons(ui, t, ctx, &chosen);
            }
            if self.up_loading {
                ui.spinner();
            }
        });
        if let Some(e) = &self.up_error {
            ui.label(w::body_s(t, t!("Couldn't load your uploaded screenshots: {error}", error = e), t.away));
        } else if crate::app::demo() {
            ui.label(w::body_s(t, t!("Demo: nothing here is sent to Steam."), t.text_faint));
        } else if !online(ctx) {
            ui.label(w::body_s(t, t!("Offline: showing the screenshots on this PC."), t.text_faint));
        }
        if self.up_page > 0 && (self.uploaded.len() as u64) < self.up_total && !self.up_loading {
            let more = t!("Load more uploaded screenshots ({n} of {total})", n = self.uploaded.len(), total = self.up_total);
            if w::button(ui, t, Some(icons::DOWNLOAD), &more, w::Kind::Ghost, w::Size::Sm).clicked() {
                self.up_loading = true;
                self.pending += 1;
                ctx.send(Command::Screenshots(Request::List { page: self.up_page + 1 }));
            }
        }
    }

    fn bulk_buttons(&mut self, ui: &mut Ui, t: &Theme, _ctx: &mut Ctx, chosen: &[Item]) {
        let can_upload: Vec<Shot> = chosen.iter().filter(|i| i.published().is_none()).filter_map(|i| i.local.clone()).filter(|s| s.game <= u32::MAX as u64).collect();
        let on_pc: Vec<Shot> = chosen.iter().filter_map(|i| i.local.clone()).collect();
        let on_steam: Vec<Item> = chosen.iter().filter(|i| i.published().is_some()).cloned().collect();
        if ui.add_enabled_ui(!can_upload.is_empty(), |ui| w::button(ui, t, Some(icons::CLOUD_CHECK), t!("Upload…"), w::Kind::Primary, w::Size::Sm)).inner.clicked() {
            self.confirm = Some(Confirm::Upload(can_upload));
        }
        if ui.add_enabled_ui(!on_pc.is_empty(), |ui| w::button(ui, t, Some(icons::TRASH), t!("Delete from this PC…"), w::Kind::Secondary, w::Size::Sm)).inner.clicked() {
            self.confirm = Some(Confirm::DeleteLocal(on_pc));
        }
        if ui.add_enabled_ui(!on_steam.is_empty(), |ui| w::button(ui, t, Some(icons::TRASH), t!("Delete from Steam…"), w::Kind::Danger, w::Size::Sm)).inner.clicked() {
            self.confirm = Some(Confirm::DeleteCloud(on_steam));
        }
    }

    /// Only the rows on screen are drawn, so their pictures alone load.
    fn grid(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, list: &[Item]) {
        let gap = t.spacing.md;
        let width = ui.available_width();
        let columns = ((width + gap) / (t.dense(260.0) + gap)).floor().max(1.0) as usize;
        let tile_w = ((width - gap * (columns as f32 - 1.0)) / columns as f32).max(1.0);
        let tile_h = tile_w * 9.0 / 16.0;
        let rows = list.len().div_ceil(columns);
        let mut open = None;
        egui::ScrollArea::vertical().id_salt("shots-grid").auto_shrink([false, false]).show_rows(ui, tile_h + gap, rows, |ui, range| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            for row in range {
                ui.horizontal(|ui| {
                    for item in list.iter().skip(row * columns).take(columns) {
                        let (rect, r) = ui.allocate_exact_size(vec2(tile_w, tile_h), Sense::click());
                        if !ui.is_rect_visible(rect) {
                            continue;
                        }
                        self.tile(ui, t, ctx, item, rect);
                        let key = item.key();
                        if r.clicked() {
                            if self.selecting {
                                if !self.selected.remove(&key) {
                                    self.selected.insert(key);
                                }
                            } else {
                                open = Some(key);
                            }
                        }
                    }
                });
            }
        });
        if let Some(key) = open {
            self.viewing = Some(key);
        }
    }

    fn tile(&self, ui: &mut Ui, t: &Theme, ctx: &Ctx, item: &Item, rect: Rect) {
        let radius = t.corner_small();
        match item.thumb() {
            Some(url) => w::image_or_skeleton(ui, t, rect, &url, radius),
            None => {
                ui.painter().rect_filled(rect, radius, t.surface);
            }
        }
        let p = ui.painter();
        if item.spoiler() {
            p.rect_filled(rect, radius, t.overlay_dark(3));
            p.text(rect.center(), egui::Align2::CENTER_CENTER, t!("Spoiler"), t.typography.body_m.font(), t.text);
        }
        // Where it is: on this PC, on Steam, or both.
        let mut x = rect.right() - t.spacing.xs;
        for (on, icon) in [(item.published().is_some(), icons::CLOUD_CHECK), (item.local.is_some(), icons::DESKTOP)] {
            if on {
                let c = egui::pos2(x - t.spacing.sm, rect.top() + t.spacing.md);
                p.circle_filled(c, t.spacing.sm + 2.0, t.overlay_dark(2));
                p.text(c, egui::Align2::CENTER_CENTER, icon, t.typography.caption.font(), t.text);
                x -= t.spacing.xl;
            }
        }
        if self.selecting {
            let on = self.selected.contains(&item.key());
            let c = egui::pos2(rect.left() + t.spacing.md + 2.0, rect.top() + t.spacing.md + 2.0);
            p.circle_filled(c, t.spacing.sm + 3.0, if on { t.accent } else { t.overlay_dark(3) });
            if on {
                p.text(c, egui::Align2::CENTER_CENTER, icons::CHECK, t.typography.caption.font(), t.text_on(t.accent));
            }
            if on {
                p.rect_stroke(rect, radius, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            }
        }
        let bar = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - t.dense(24.0)), rect.max);
        let caption = item.caption();
        let text = if caption.is_empty() { if self.game.is_none() { Self::game_name(ctx, item.game()) } else { ui::date(item.created()) } } else { caption.to_string() };
        p.rect_filled(bar, egui::CornerRadius { sw: radius.sw, se: radius.se, nw: 0, ne: 0 }, t.overlay_dark(2));
        let galley = p.layout_no_wrap(text, t.typography.caption.font(), t.text);
        let clip = bar.shrink2(vec2(t.spacing.sm, 0.0));
        p.with_clip_rect(clip).galley(egui::pos2(clip.left(), bar.center().y - galley.size().y / 2.0), galley, t.text);
    }

    /// One screenshot, large, with its details beside it.
    fn viewer(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, items: &[Item]) {
        let list: Vec<Item> = self.shown(items).into_iter().cloned().collect();
        let key = self.viewing.clone().unwrap_or_default();
        let Some(item) = items.iter().find(|i| i.key() == key).cloned() else {
            self.viewing = None;
            return;
        };
        if self.form_for != key {
            self.form_for = key.clone();
            self.form = Edit { caption: item.caption().to_string(), spoiler: item.spoiler(), visibility: item.visibility() };
        }
        let index = list.iter().position(|i| i.key() == key);
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (mut prev, mut next) = if typing || self.confirm.is_some() { (false, false) } else { ui.input(|i| (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight))) };
        if self.confirm.is_none() && ui.input(|i| i.key_pressed(Key::Escape)) {
            self.viewing = None;
            return;
        }
        let side = (ui.available_width() * 0.3).clamp(280.0, 380.0);
        egui::Panel::right("shot-details").exact_size(side).resizable(false).frame(egui::Frame::new().fill(t.raised).inner_margin(Margin::same(t.spacing.lg as i8))).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.details(ui, t, ctx, &item));
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg).inner_margin(Margin::same(t.spacing.md as i8))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Back"), w::Kind::Ghost, w::Size::Sm).clicked() {
                    self.viewing = None;
                }
                if let Some(i) = index {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        next |= ui.add_enabled_ui(i + 1 < list.len(), |ui| w::icon_button(ui, t, icons::CARET_RIGHT, t!("Next"), false, 0)).inner.clicked();
                        ui.label(w::body_s(t, format!("{} / {}", i + 1, list.len()), t.text_dim));
                        prev |= ui.add_enabled_ui(i > 0, |ui| w::icon_button(ui, t, icons::CARET_LEFT, t!("Previous"), false, 0)).inner.clicked();
                    });
                }
            });
            ui.add_space(t.spacing.sm);
            let area = ui.available_rect_before_wrap();
            ui.allocate_rect(area, Sense::hover());
            if let Some(url) = item.full() {
                // Fitted by the picture's real shape, once it's known.
                let image = egui::Image::new(url).show_loading_spinner(true);
                let natural = image.load_for_size(ui.ctx(), area.size()).ok().and_then(|p| p.size());
                let size = natural.filter(|s| s.x > 0.0 && s.y > 0.0).map_or(area.size(), |s| s * (area.width() / s.x).min(area.height() / s.y));
                image.paint_at(ui, Rect::from_center_size(area.center(), size));
            }
        });
        if let Some(i) = index {
            let target = if prev { i.checked_sub(1) } else if next { Some(i + 1) } else { None };
            if let Some(n) = target.and_then(|n| list.get(n)) {
                self.viewing = Some(n.key());
            }
        }
    }

    fn details(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, item: &Item) {
        ui.spacing_mut().item_spacing.y = t.spacing.sm;
        ui.add(egui::Label::new(w::heading_s(t, Self::game_name(ctx, item.game()), t.text)).wrap());
        let mut facts = vec![ui::date(item.created())];
        if let Some(s) = &item.local {
            if s.width > 0 {
                facts.push(format!("{} × {}", s.width, s.height));
            }
            facts.push(ui::bytes(s.bytes));
        }
        ui.label(w::body_s(t, facts.join("  ·  "), t.text_dim));
        ui.horizontal_wrapped(|ui| {
            if item.local.is_some() {
                w::pill(ui, t!("On this PC"), t.text_dim, t.surface);
            }
            if item.published().is_some() {
                w::pill(ui, t!("Uploaded"), t.play, t.play.gamma_multiply(0.15));
            }
        });
        ui.add_space(t.spacing.sm);
        ui.label(w::body_s(t, t!("Caption"), t.text_dim));
        ui.add(egui::TextEdit::multiline(&mut self.form.caption).char_limit(CAPTION_MAX).desired_rows(3).desired_width(f32::INFINITY));
        w::check(ui, t, &mut self.form.spoiler, t!("Mark as a spoiler"));
        ui.label(w::body_s(t, t!("Who can see it"), t.text_dim));
        for (v, label) in [(Visibility::Public, t!("Everyone")), (Visibility::Friends, t!("Friends only")), (Visibility::Unlisted, t!("Anyone with the link")), (Visibility::Private, t!("Only me"))] {
            w::radio_value(ui, t, &mut self.form.visibility, v, label);
        }
        let changed = self.form.caption != item.caption() || self.form.spoiler != item.spoiler() || self.form.visibility != item.visibility();
        if ui.add_enabled_ui(changed, |ui| w::button(ui, t, Some(icons::CHECK), t!("Save"), w::Kind::Primary, w::Size::Sm)).inner.clicked() {
            self.save(ctx, item);
        }
        ui.separator();
        let uploadable = item.local.as_ref().filter(|s| item.published().is_none() && s.game <= u32::MAX as u64).cloned();
        if let Some(s) = uploadable
            && w::button(ui, t, Some(icons::CLOUD_CHECK), t!("Upload…"), w::Kind::Primary, w::Size::Sm).clicked()
        {
            self.confirm = Some(Confirm::Upload(vec![s]));
        }
        if let Some(s) = &item.local {
            if w::button(ui, t, Some(icons::FOLDER), t!("Show on disk"), w::Kind::Secondary, w::Size::Sm).clicked() {
                show_on_disk(&s.path);
            }
            let copying = self.copying.is_some();
            if ui.add_enabled_ui(!copying, |ui| w::button(ui, t, Some(icons::COPY), t!("Copy picture"), w::Kind::Secondary, w::Size::Sm)).inner.clicked() {
                self.copy(ui.ctx(), s.path.clone());
            }
        }
        if let Some(id) = item.published()
            && w::button(ui, t, Some(icons::COPY), t!("Copy link"), w::Kind::Secondary, w::Size::Sm).clicked()
        {
            ui.ctx().copy_text(format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}"));
            ctx.toast(t!("Link copied"));
        }
        ui.separator();
        if let Some(s) = &item.local
            && w::button(ui, t, Some(icons::TRASH), t!("Delete from this PC…"), w::Kind::Secondary, w::Size::Sm).clicked()
        {
            self.confirm = Some(Confirm::DeleteLocal(vec![s.clone()]));
        }
        if item.published().is_some() && w::button(ui, t, Some(icons::TRASH), t!("Delete from Steam…"), w::Kind::Danger, w::Size::Sm).clicked() {
            self.confirm = Some(Confirm::DeleteCloud(vec![item.clone()]));
        }
    }

    fn save(&mut self, ctx: &mut Ctx, item: &Item) {
        let edit = Edit { caption: clean_caption(&self.form.caption), ..self.form.clone() };
        if let (Some(s), Some((root, account))) = (&item.local, place(ctx)) {
            match shots::save_edit(&root, account, s, &edit) {
                Ok(()) => {
                    self.stale = true;
                    if item.published().is_none() {
                        ctx.toast(t!("Saved"));
                    }
                }
                Err(e) => {
                    ctx.toast(t!("Couldn't save it: {error}", error = i18n::tr_dyn(&e)));
                    return;
                }
            }
        }
        if let Some(id) = item.published() {
            if online(ctx) {
                self.pending += 1;
                ctx.send(Command::Screenshots(Request::Update { id, edit }));
            } else {
                offline_note(ctx);
            }
        }
    }

    fn copy(&mut self, egui_ctx: &egui::Context, path: PathBuf) {
        let slot: Slot<Result<egui::ColorImage, String>> = Arc::new(Mutex::new(None));
        self.copying = Some(slot.clone());
        let repaint = egui_ctx.clone();
        std::thread::spawn(move || {
            let result = image::open(&path).map_err(|e| e.to_string()).map(|img| {
                let rgba = img.to_rgba8();
                egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
            });
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
            repaint.request_repaint();
        });
    }

    fn finish_copy(&mut self, egui_ctx: &egui::Context, ctx: &mut Ctx) {
        let Some(slot) = &self.copying else { return };
        let done = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(result) = done else { return };
        self.copying = None;
        match result {
            Ok(image) => {
                egui_ctx.copy_image(image);
                ctx.toast(t!("Picture copied"));
            }
            Err(e) => ctx.toast(t!("Couldn't copy the picture: {error}", error = e)),
        }
    }

    fn confirm_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(confirm) = self.confirm.clone() else { return };
        let (title, body, label, danger) = match &confirm {
            Confirm::Upload(l) if l.len() == 1 => (t!("Upload this screenshot?").to_string(), t!("It goes to your Steam profile with the visibility set for it. Everyone is the default.").to_string(), t!("Upload"), false),
            Confirm::Upload(l) => (t!("Upload {n} screenshots?", n = l.len()).to_string(), t!("They go to your Steam profile, each with the visibility set for it. Everyone is the default.").to_string(), t!("Upload"), false),
            Confirm::DeleteLocal(l) if l.len() == 1 => {
                let body = if l[0].published.is_some() { t!("The picture is deleted from this PC. It stays on your Steam profile.") } else { t!("It isn't uploaded, so this can't be undone.") };
                (t!("Delete this screenshot from this PC?").to_string(), body.to_string(), t!("Delete"), true)
            }
            Confirm::DeleteLocal(l) => {
                let up = l.iter().filter(|s| s.published.is_some()).count();
                let body = if up > 0 { t!("The pictures are deleted from this PC. {n} of them stay on your Steam profile.", n = up).to_string() } else { t!("They aren't uploaded, so this can't be undone.").to_string() };
                (t!("Delete {n} screenshots from this PC?", n = l.len()).to_string(), body, t!("Delete"), true)
            }
            Confirm::DeleteCloud(l) if l.len() == 1 => (t!("Delete this screenshot from Steam?").to_string(), t!("It's removed from your profile for everyone, with its comments and awards. A copy on this PC stays.").to_string(), t!("Delete"), true),
            Confirm::DeleteCloud(l) => (t!("Delete {n} screenshots from Steam?", n = l.len()).to_string(), t!("They're removed from your profile for everyone, with their comments and awards. Copies on this PC stay.").to_string(), t!("Delete"), true),
        };
        let mut close = false;
        let mut go = false;
        egui::Modal::new(egui::Id::unique("shots-confirm")).show(ui.ctx(), |ui| {
            ui.set_width(t.dense(440.0));
            ui.label(w::heading_s(t, title, t.text));
            ui.add_space(t.spacing.xs);
            ui.add(egui::Label::new(w::body_m(t, body, t.text_dim)).wrap());
            ui.add_space(t.spacing.md);
            ui.horizontal(|ui| {
                let kind = if danger { w::Kind::Danger } else { w::Kind::Primary };
                go = w::button(ui, t, None, label, kind, w::Size::Md).clicked();
                close = w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md).clicked();
            });
        });
        if go {
            self.run(ctx, confirm);
            close = true;
        }
        if close {
            self.confirm = None;
        }
    }

    fn run(&mut self, ctx: &mut Ctx, confirm: Confirm) {
        match confirm {
            Confirm::Upload(list) => {
                if !online(ctx) {
                    return offline_note(ctx);
                }
                let Some((root, account)) = place(ctx) else { return };
                for s in list {
                    let edit = Edit { caption: s.caption.clone(), spoiler: s.spoiler, visibility: s.visibility };
                    self.pending += 1;
                    ctx.send(Command::Screenshots(Request::Upload { steam_root: root.clone(), account, shot: Box::new(s), edit }));
                }
                ctx.toast(t!("Uploading…"));
            }
            Confirm::DeleteLocal(list) => {
                let Some((root, account)) = place(ctx) else { return };
                let mut failed = None;
                let mut done = 0;
                for s in &list {
                    match shots::delete_local(&root, account, s) {
                        Ok(()) => done += 1,
                        Err(e) => failed = Some(e),
                    }
                }
                self.stale = true;
                self.selected.clear();
                match failed {
                    Some(e) => ctx.toast(t!("Couldn't delete a screenshot: {error}", error = i18n::tr_dyn(&e))),
                    None => ctx.toast(t!("{n} deleted from this PC", n = done)),
                }
            }
            Confirm::DeleteCloud(list) => {
                if !online(ctx) {
                    return offline_note(ctx);
                }
                let place = place(ctx);
                for i in list {
                    let Some(id) = i.published() else { continue };
                    let handle = i.local.as_ref().and_then(|s| s.handle);
                    let local = match (&place, i.local) {
                        (Some((root, account)), Some(s)) => Some((root.clone(), *account, Box::new(s))),
                        _ => None,
                    };
                    self.pending += 1;
                    ctx.send(Command::Screenshots(Request::Delete { id, handle, local }));
                }
                self.selected.clear();
            }
        }
    }
}

/// Open the folder with the file selected, where the system can.
fn show_on_disk(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn().is_ok() {
            return;
        }
    }
    if let Some(dir) = path.parent() {
        local::open_folder(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(name: &str, created: u64, published: Option<u64>) -> Shot {
        Shot {
            game: 440,
            name: name.into(),
            path: PathBuf::from(name),
            thumbnail: None,
            created,
            width: 0,
            height: 0,
            bytes: 1,
            caption: "local".into(),
            spoiler: false,
            visibility: Visibility::Friends,
            published,
            handle: published,
        }
    }

    fn up(id: u64, created: u64) -> Uploaded {
        Uploaded { id, app_id: 440, caption: "steam".into(), created, ..Default::default() }
    }

    #[test]
    fn joins_local_and_uploaded() {
        let items = merge(&[shot("a.jpg", 10, Some(1)), shot("b.jpg", 30, None)], &[up(1, 10), up(2, 20)], true);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].key(), "440/b.jpg", "newest first");
        assert_eq!(items[1].key(), "up/2");
        assert!(items[2].local.is_some() && items[2].up.is_some());
        assert_eq!(items[2].caption(), "steam", "Steam's caption wins once uploaded");
        assert_eq!(items[0].caption(), "local");
    }

    #[test]
    fn an_upload_deleted_elsewhere_shows_as_local_once_the_list_is_complete() {
        let local = [shot("a.jpg", 10, Some(9))];
        assert_eq!(merge(&local, &[], false)[0].published(), Some(9), "list not loaded: trust the record");
        assert_eq!(merge(&local, &[], true)[0].published(), None);
    }

    #[test]
    fn the_loader_reads_only_screenshot_folders() {
        let base = std::env::temp_dir().join(format!("sir-shot-loader-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let remote = base.join("userdata").join("5").join("760").join("remote");
        let dir = remote.join("440").join("screenshots");
        std::fs::create_dir_all(dir.join("thumbnails")).unwrap();
        for p in [dir.join("a.jpg"), dir.join("thumbnails").join("a.jpg"), dir.join("notes.txt"), base.join("b.jpg")] {
            std::fs::write(p, b"x").unwrap();
        }
        *ROOT.write().unwrap() = Some(remote);
        assert!(allowed(&dir.join("a.jpg")));
        assert!(allowed(&dir.join("thumbnails").join("a.jpg")));
        assert!(!allowed(&dir.join("notes.txt")));
        assert!(!allowed(&base.join("b.jpg")));
        assert!(!allowed(&dir.join("..").join("..").join("..").join("..").join("..").join("..").join("b.jpg")));
        *ROOT.write().unwrap() = None;
        let _ = std::fs::remove_dir_all(&base);
    }
}

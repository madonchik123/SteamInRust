//! "Move install folder": pick games and a library folder, see the space
//! needed and free, then a progress bar with Cancel. The careful part (copy,
//! verify, switch over, delete) is [`local::relocate`]; this is the window.
//!
//! The window can be opened from Properties, a game's right-click menu and
//! Settings → Storage. It's drawn with the library's dialogs on every page,
//! and a move keeps going when the window is hidden.

use super::{Library, fs4_free};
use egui::{Align, Layout, Ui};
use extensions::Ctx;
use i18n::t;
use local::relocate::{self, MoveError, MoveProgress, Outcome, Phase, Recovered};
use model::{AppId, InstallState, InstalledGame};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use ui::{Theme, icons, widgets as w};

/// Asked for from a page without access to the library (Settings).
static REQUEST: Mutex<Option<Vec<AppId>>> = Mutex::new(None);

/// Open the move window, with these games ticked.
pub(crate) fn request_open(games: Vec<AppId>) {
    *REQUEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(games);
}

/// One run through the ticked games, on its own thread.
struct Run {
    games: Vec<(AppId, String)>,
    dest: PathBuf,
    current: Mutex<Option<(usize, Arc<MoveProgress>)>>,
    cancel: AtomicBool,
    results: Mutex<Vec<(String, Result<Outcome, MoveError>)>>,
    finished: AtomicBool,
}

impl Run {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some((_, p)) = &*self.current.lock().unwrap_or_else(|e| e.into_inner()) {
            p.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Default)]
struct Dialog {
    ticked: BTreeSet<AppId>,
    dest: Option<PathBuf>,
    /// Folders picked with "Choose another folder…".
    extra: Vec<PathBuf>,
    search: String,
    /// What the last run did, shown until the window is closed.
    report: Vec<(String, Result<Outcome, MoveError>)>,
}

/// Library state for moves.
#[derive(Default)]
pub(super) struct Moves {
    dialog: Option<Dialog>,
    run: Option<Arc<Run>>,
    recovery: Option<Arc<Mutex<Option<Vec<Recovered>>>>>,
    recovery_started: bool,
}

/// Why a game can't be ticked right now.
fn blocked(lib: &Library, ctx: &Ctx, g: &InstalledGame, dest: Option<&Path>) -> Option<String> {
    let id = g.app_id;
    if ctx.state.playing == Some(id) {
        return Some(t!("Running").to_string());
    }
    let downloading = ctx.state.downloads.get(&id).is_some_and(|d| !matches!(d.phase, model::DownloadPhase::Done | model::DownloadPhase::Failed));
    if downloading || ctx.state.download_queue.contains(&id) || matches!(g.state, InstallState::Downloading | InstallState::Unknown) {
        return Some(t!("Updating").to_string());
    }
    if lib.uninstalling.contains_key(&id) {
        return Some(t!("Uninstalling…").to_string());
    }
    if ctx.state.cloud_syncing.contains(&id) || ctx.state.first_time_setup.contains_key(&id) {
        return Some(t!("Busy").to_string());
    }
    if dest.is_some_and(|d| local::same_path(&g.library, d)) {
        return Some(t!("Already here").to_string());
    }
    None
}

/// The last part of a path: enough to name a file in a message.
fn short(path: &str) -> &str {
    path.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(path)
}

/// A long folder path with its middle left out, so it fits the window: the
/// drive and the last folders are what tell two libraries apart.
fn shorter(path: &str) -> String {
    const START: usize = 12;
    const END: usize = 40;
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= START + END + 1 {
        return path.to_string();
    }
    let head: String = chars[..START].iter().collect();
    let tail: String = chars[chars.len() - END..].iter().collect();
    format!("{head}…{tail}")
}

/// A move's failure in plain words.
fn reason(e: &MoveError) -> String {
    match e {
        MoveError::SameFolder => t!("The game is already in this folder.").to_string(),
        MoveError::NotReady => t!("The game is being updated. Try again when that finishes.").to_string(),
        MoveError::AlreadyMoving => t!("This game is already being moved.").to_string(),
        MoveError::InUse(p) => t!("Close the game first: {file} is in use.", file = short(p)).to_string(),
        MoveError::NoSpace { need, free } => t!("Not enough free space: needs {need}, {free} free.", need = ui::bytes(*need), free = ui::bytes(*free)).to_string(),
        MoveError::FolderExists(p) => t!("{folder} already exists there. Move or delete it first.", folder = p.display().to_string()).to_string(),
        MoveError::AlreadyThere => t!("The game is already installed in that folder.").to_string(),
        MoveError::Links(p) => t!("{file} is a link that would break if it moved.", file = short(p)).to_string(),
        MoveError::Changed(p) => t!("{file} changed during the move, so the game stayed where it was. Close the game and try again.", file = short(p)).to_string(),
        MoveError::Mismatch(p) => t!("The copy of {file} didn't match the original, so the game stayed where it was. The drive may be failing.", file = short(p)).to_string(),
        MoveError::Leftover => t!("An earlier move is still being cleaned up. Try again in a moment.").to_string(),
        MoveError::Cancelled => t!("Cancelled. The game stayed where it was.").to_string(),
        MoveError::Interrupted => t!("Something went wrong").to_string(),
        MoveError::Io(e) => t!("Couldn't move the game, it stayed where it was: {error}", error = e).to_string(),
    }
}

impl Library {
    /// Ask to move this game (the window shows over the main window).
    pub(super) fn open_move(&mut self, ui: &egui::Context, id: AppId) {
        request_open(vec![id]);
        ui.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        ui.request_repaint();
    }

    /// Cut-off moves are finished or undone once, when the app starts.
    fn recover_moves(&mut self, egui_ctx: &egui::Context, ctx: &mut Ctx) {
        if !self.moves.recovery_started && !ctx.local.libraries.is_empty() {
            self.moves.recovery_started = true;
            let slot = Arc::new(Mutex::new(None));
            self.moves.recovery = Some(slot.clone());
            let (libraries, repaint) = (ctx.local.libraries.clone(), egui_ctx.clone());
            std::thread::spawn(move || {
                let done = relocate::recover(&libraries);
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(done);
                repaint.request_repaint();
            });
        }
        let done = self.moves.recovery.as_ref().and_then(|s| s.lock().unwrap_or_else(|e| e.into_inner()).take());
        if let Some(done) = done {
            self.moves.recovery = None;
            if !done.is_empty() {
                ctx.act(extensions::Action::ReloadLocal);
            }
            for r in done {
                let name = ctx.state.app_name(r.app).map(String::from).unwrap_or_else(|| r.app.to_string());
                if r.moved {
                    ctx.toast(t!("Finished moving {name}", name = name));
                } else {
                    ctx.toast(t!("A move of {name} was interrupted. The game is still in {folder}.", name = name, folder = r.library.display().to_string()));
                }
            }
        }
    }

    /// A finished run: report it and refresh the installed games.
    fn finish_move_run(&mut self, ctx: &mut Ctx) {
        let done = self.moves.run.as_ref().is_some_and(|r| r.finished.load(Ordering::Relaxed));
        if !done {
            return;
        }
        let Some(run) = self.moves.run.take() else { return };
        let results = std::mem::take(&mut *run.results.lock().unwrap_or_else(|e| e.into_inner()));
        ctx.act(extensions::Action::ReloadLocal);
        for (name, r) in &results {
            match r {
                Ok(o) => {
                    ctx.toast(t!("{name} moved to {folder}", name = name, folder = run.dest.display().to_string()));
                    if let Some(p) = &o.leftovers {
                        tracing::warn!("old files of {name} remain in {}", p.display());
                    }
                }
                Err(MoveError::Cancelled) => {}
                Err(e) => ctx.toast(format!("{name}: {}", reason(e))),
            }
        }
        // All went well: nothing to read. Otherwise the window says what happened.
        let all_ok = results.iter().all(|(_, r)| matches!(r, Ok(o) if o.leftovers.is_none()));
        if all_ok {
            self.moves.dialog = None;
        } else if let Some(d) = &mut self.moves.dialog {
            d.report = results;
            d.ticked.clear();
        }
    }

    fn start_move(&mut self, egui_ctx: &egui::Context, ctx: &Ctx) {
        let Some(d) = &mut self.moves.dialog else { return };
        let Some(dest) = d.dest.clone() else { return };
        let games: Vec<InstalledGame> = ctx.local.games.iter().filter(|g| d.ticked.contains(&g.app_id)).cloned().collect();
        d.report.clear();
        // Remembered first: a game moved to a folder Steam doesn't list must
        // still be found if the app stops right after the switch.
        if !crate::demo::fake_library() {
            client::store::remember_library(&dest);
        }
        let run = Arc::new(Run {
            games: games.iter().map(|g| (g.app_id, g.name.clone())).collect(),
            dest: dest.clone(),
            current: Mutex::new(None),
            cancel: AtomicBool::new(false),
            results: Mutex::new(Vec::new()),
            finished: AtomicBool::new(false),
        });
        self.moves.run = Some(run.clone());
        let repaint = egui_ctx.clone();
        std::thread::spawn(move || {
            for (i, game) in games.iter().enumerate() {
                if run.cancel.load(Ordering::Relaxed) {
                    break;
                }
                let progress = Arc::new(MoveProgress::default());
                *run.current.lock().unwrap_or_else(|e| e.into_inner()) = Some((i, progress.clone()));
                if run.cancel.load(Ordering::Relaxed) {
                    progress.cancel.store(true, Ordering::Relaxed);
                }
                repaint.request_repaint();
                let result = relocate::plan(game, &dest).and_then(|plan| relocate::run(&plan, &progress));
                let stop = matches!(result, Err(MoveError::Cancelled));
                run.results.lock().unwrap_or_else(|e| e.into_inner()).push((game.name.clone(), result));
                if stop {
                    break;
                }
            }
            run.finished.store(true, Ordering::Relaxed);
            repaint.request_repaint();
        });
    }

    /// The move window, whatever page is open.
    pub(super) fn move_window(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        self.recover_moves(ui.ctx(), ctx);
        self.finish_move_run(ctx);
        if let Some(games) = REQUEST.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let d = self.moves.dialog.get_or_insert_with(Dialog::default);
            if self.moves.run.is_none() {
                d.ticked = games.into_iter().collect();
                d.report.clear();
            }
        }
        if self.moves.run.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
        if self.moves.dialog.is_none() {
            return;
        }
        let mut close = false;
        let mut start = false;
        egui::Modal::new(egui::Id::unique("move-games")).show(ui.ctx(), |ui| {
            ui.set_width(t.dense(620.0));
            ui.spacing_mut().item_spacing.y = t.spacing.sm;
            ui.horizontal(|ui| {
                ui.label(w::text(icons::DRIVES, t.typography.heading_s.size, t.accent));
                ui.label(w::heading_s(t, t!("Move install folder"), t.text));
            });
            if let Some(run) = self.moves.run.clone() {
                move_progress(ui, t, &run, &mut close);
            } else {
                self.move_form(ui, t, ctx, &mut start, &mut close);
            }
        });
        if start {
            self.start_move(ui.ctx(), ctx);
        }
        if close {
            self.moves.dialog = None;
        }
    }

    /// Choosing games and where they go.
    fn move_form(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, start: &mut bool, close: &mut bool) {
        let mut libraries = ctx.local.libraries.clone();
        if let Some(d) = &self.moves.dialog {
            for p in &d.extra {
                if !libraries.iter().any(|l| local::same_path(l, p)) {
                    libraries.push(p.clone());
                }
            }
        }
        // Start on the folder with the most room that isn't the game's own.
        if self.moves.dialog.as_ref().is_some_and(|d| d.dest.is_none()) {
            let ticked = self.moves.dialog.as_ref().map(|d| d.ticked.clone()).unwrap_or_default();
            let homes: Vec<&Path> = ctx.local.games.iter().filter(|g| ticked.contains(&g.app_id)).map(|g| g.library.as_path()).collect();
            let best = libraries.iter().filter(|l| !homes.iter().any(|h| local::same_path(h, l))).max_by_key(|l| fs4_free(l).unwrap_or(0)).cloned();
            if let Some(d) = &mut self.moves.dialog {
                d.dest = best;
            }
        }
        let dest = self.moves.dialog.as_ref().and_then(|d| d.dest.clone());

        if let Some(d) = &self.moves.dialog {
            for (name, r) in &d.report {
                let (icon, text, color) = match r {
                    Ok(Outcome { leftovers: Some(p) }) => (icons::WARNING, t!("{name} moved, but its old folder couldn't be fully deleted: {folder}. It's tried again at the next start.", name = name, folder = p.display().to_string()).to_string(), t.away),
                    Ok(_) => (icons::CHECK, t!("{name} moved to {folder}", name = name, folder = dest.as_deref().unwrap_or(Path::new("")).display().to_string()).to_string(), t.play),
                    Err(e) => (icons::WARNING, format!("{name}: {}", reason(e)), t.away),
                };
                ui.add(egui::Label::new(w::body_s(t, format!("{icon}  {text}"), color)).wrap());
            }
        }

        ui.label(w::body_s(t, t!("Games to move"), t.text_dim));
        if ctx.local.games.len() > 8 {
            let width = ui.available_width();
            if let Some(d) = &mut self.moves.dialog {
                w::search(ui, t, &mut d.search, &t!("Search…"), width);
            }
        }
        let query = self.moves.dialog.as_ref().map(|d| d.search.trim().to_lowercase()).unwrap_or_default();
        let games: Vec<InstalledGame> = ctx.local.games.iter().filter(|g| query.is_empty() || g.name.to_lowercase().contains(&query)).cloned().collect();
        let why: Vec<Option<String>> = games.iter().map(|g| blocked(self, ctx, g, dest.as_deref())).collect();
        egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(t.spacing.sm).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt("move-games-list").max_height(t.dense(240.0)).auto_shrink([false, true]).show(ui, |ui| {
                if games.is_empty() {
                    ui.label(w::body_m(t, t!("No installed games"), t.text_faint));
                }
                for (g, why) in games.iter().zip(&why) {
                    let size = g.size_on_disk + local::manage::workshop_size(g);
                    ui.horizontal(|ui| {
                        let Some(d) = &mut self.moves.dialog else { return };
                        let mut on = d.ticked.contains(&g.app_id) && why.is_none();
                        let changed = ui.add_enabled_ui(why.is_none(), |ui| w::check(ui, t, &mut on, w::body_m(t, &g.name, t.text))).inner.changed();
                        if changed {
                            if on {
                                d.ticked.insert(g.app_id);
                            } else {
                                d.ticked.remove(&g.app_id);
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(w::body_s(t, ui::bytes(size), t.text_dim));
                            if let Some(why) = why {
                                ui.label(w::body_s(t, why, t.away));
                            }
                        });
                    });
                }
            });
        });

        ui.add_space(t.spacing.xs);
        ui.label(w::body_s(t, t!("Move to"), t.text_dim));
        for lib in &libraries {
            let free = fs4_free(lib);
            let full = lib.display().to_string();
            let label = format!("{}   ·   {}", shorter(&full), t!("{size} free", size = free.map(ui::bytes).unwrap_or_else(|| "?".into())));
            if let Some(d) = &mut self.moves.dialog {
                let mut cur = d.dest.clone();
                if w::radio_value(ui, t, &mut cur, Some(lib.clone()), w::body_m(t, label, t.text)).on_hover_text(&full).clicked() {
                    d.dest = cur;
                }
            }
        }
        if w::button(ui, t, Some(icons::FOLDER), t!("Choose another folder…"), w::Kind::Ghost, w::Size::Sm).clicked()
            && let Some(p) = rfd::FileDialog::new().pick_folder()
        {
            let p = local::library_root_of(&p);
            if let Some(d) = &mut self.moves.dialog {
                if !libraries.iter().any(|l| local::same_path(l, &p)) {
                    d.extra.push(p.clone());
                }
                d.dest = Some(p);
            }
        }

        // Only games that can go: a ticked game that became busy doesn't count.
        let ticked: Vec<&InstalledGame> = {
            let on = self.moves.dialog.as_ref().map(|d| d.ticked.clone()).unwrap_or_default();
            ctx.local.games.iter().filter(|g| on.contains(&g.app_id) && blocked(self, ctx, g, dest.as_deref()).is_none()).collect()
        };
        let need: u64 = ticked.iter().map(|g| g.size_on_disk + local::manage::workshop_size(g)).sum();
        let free = dest.as_deref().and_then(fs4_free);
        let short_of_space = free.is_some_and(|f| f < need);
        ui.add_space(t.spacing.xs);
        ui.horizontal_wrapped(|ui| {
            ui.label(w::body_m(t, t!("Space required: {size}", size = ui::bytes(need)), t.text));
            if let Some(f) = free {
                ui.label(w::body_m(t, "·", t.text_faint));
                ui.label(w::body_m(t, t!("{size} free", size = ui::bytes(f)), if short_of_space { t.danger } else { t.text_dim }));
            }
        });
        if short_of_space {
            ui.label(w::body_m(t, format!("{}  {}", icons::WARNING, t!("Not enough free space on this drive.")), t.danger));
        }
        ui.add(egui::Label::new(w::body_s(t, t!("Files are copied and checked first. The originals are only deleted once the copy is confirmed."), t.text_faint)).wrap());
        ui.add_space(t.spacing.sm);
        ui.horizontal(|ui| {
            let ready = !ticked.is_empty() && dest.is_some() && !short_of_space;
            if ui.add_enabled_ui(ready, |ui| w::button(ui, t, Some(icons::DRIVES), t!("Move"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                if let Some(d) = &mut self.moves.dialog {
                    d.ticked = ticked.iter().map(|g| g.app_id).collect();
                }
                *start = true;
            }
            if w::button(ui, t, None, t!("Close"), w::Kind::Secondary, w::Size::Md).clicked() {
                *close = true;
            }
        });
    }
}

/// A running move: which game, what it's doing, how far along.
fn move_progress(ui: &mut Ui, t: &Theme, run: &Run, close: &mut bool) {
    let current = run.current.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let (index, fraction, phase, bytes) = match &current {
        Some((i, p)) => (*i, p.fraction(), p.phase(), p.total_bytes.load(Ordering::Relaxed) / 2),
        None => (0, 0.0, Phase::Preparing, 0),
    };
    let name = run.games.get(index).map(|g| g.1.as_str()).unwrap_or_default();
    ui.label(w::body_l(t, name, t.text));
    let dest = run.dest.display().to_string();
    ui.add(egui::Label::new(w::body_s(t, t!("Moving to {folder}", folder = dest), t.text_dim)).truncate()).on_hover_text(&dest);
    if run.games.len() > 1 {
        ui.label(w::body_s(t, t!("Game {n} of {total}", n = index + 1, total = run.games.len()), t.text_dim));
    }
    let what = match phase {
        Phase::Preparing => t!("Checking files…"),
        Phase::Copying => t!("Copying…"),
        Phase::Verifying => t!("Verifying…"),
        Phase::Finishing => t!("Switching to the new folder…"),
        Phase::Cleaning => t!("Deleting the old files…"),
    };
    let shown = if matches!(phase, Phase::Finishing | Phase::Cleaning) { 1.0 } else { fraction };
    w::progress(ui, t, shown, ui.available_width());
    ui.horizontal(|ui| {
        ui.label(w::body_s(t, what, t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(w::body_s(t, format!("{}%  ·  {}", (shown * 100.0) as u32, ui::bytes(bytes)), t.text_dim));
        });
    });
    ui.add_space(t.spacing.sm);
    let cancelling = run.cancel.load(Ordering::Relaxed);
    let can_cancel = !cancelling && !matches!(phase, Phase::Finishing | Phase::Cleaning);
    ui.horizontal(|ui| {
        let label = if cancelling { t!("Cancelling…") } else { t!("Cancel") };
        if ui.add_enabled_ui(can_cancel, |ui| w::button(ui, t, Some(icons::X), label, w::Kind::Secondary, w::Size::Md)).inner.clicked() {
            run.cancel();
        }
        if w::button(ui, t, None, t!("Hide"), w::Kind::Ghost, w::Size::Md).on_hover_text(t!("The move keeps going in the background.")).clicked() {
            *close = true;
        }
    });
    if !can_cancel && !cancelling {
        ui.label(w::body_s(t, t!("Almost done. This step can't be cancelled."), t.text_faint));
    }
}

//! The setup wizard's screens, drawn like a classic Windows installer (which is
//! what Steam's SteamSetup.exe is), plus the dark "Updating…" window.

use crate::{Mode, work};
use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, ViewportCommand, pos2, vec2};
use i18n::t;
use std::path::PathBuf;
use std::time::Duration;

const W: f32 = 500.0;
const H: f32 = 390.0;
const BANNER_W: f32 = 164.0;
const HEADER_H: f32 = 57.0;
const BAR_TOP: f32 = H - 48.0;

const WHITE: Color32 = Color32::WHITE;
const DIALOG: Color32 = Color32::from_rgb(0xF0, 0xF0, 0xF0);
const TEXT: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
const ETCH_DARK: Color32 = Color32::from_rgb(0xA0, 0xA0, 0xA0);
const BTN: Color32 = Color32::from_rgb(0xE1, 0xE1, 0xE1);
const BTN_BORDER: Color32 = Color32::from_rgb(0xAD, 0xAD, 0xAD);
const BTN_HOVER: Color32 = Color32::from_rgb(0xE5, 0xF1, 0xFB);
const BTN_DOWN: Color32 = Color32::from_rgb(0xCC, 0xE4, 0xF7);
const FOCUS: Color32 = Color32::from_rgb(0x00, 0x78, 0xD7);
const DISABLED: Color32 = Color32::from_rgb(0xA0, 0xA0, 0xA0);
const PROGRESS: Color32 = Color32::from_rgb(0x06, 0xB0, 0x25);
const NAVY_TOP: Color32 = Color32::from_rgb(0x17, 0x1A, 0x21);
const NAVY_BOTTOM: Color32 = Color32::from_rgb(0x1B, 0x28, 0x38);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Welcome,
    Language,
    Location,
    Confirm,
    Working,
    Finish,
}

pub struct Wizard {
    mode: Mode,
    page: Page,
    dir: String,
    desktop: bool,
    run_after: bool,
    remove_data: bool,
    progress: work::Shared,
    details: bool,
    lang: i18n::Lang,
    need: u64,
    free: Option<(String, u64)>,
    styled: bool,
}

/// Segoe UI, like every Windows setup; the app's fonts if it's missing.
pub fn fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts")).unwrap_or_default();
    let (reg, bold) = (std::fs::read(dir.join("segoeui.ttf")), std::fs::read(dir.join("segoeuib.ttf")));
    match (reg, bold) {
        (Ok(r), Ok(b)) => {
            fonts.font_data.insert("segoe".into(), Arc::new(egui::FontData::from_owned(r)));
            fonts.font_data.insert("segoe-bold".into(), Arc::new(egui::FontData::from_owned(b)));
            let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
            let mut prop = vec!["segoe".to_string()];
            prop.extend(fallback.iter().cloned());
            let mut bold = vec!["segoe-bold".to_string()];
            bold.extend(fallback);
            fonts.families.insert(FontFamily::Proportional, prop);
            fonts.families.insert(FontFamily::Name("bold".into()), bold);
            ctx.set_fonts(fonts);
        }
        _ => {
            ui::theme::install_fonts(ctx);
        }
    }
}

fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

/// Native Windows message box, as NSIS installers use.
fn ask(title: &str, text: &str, yes_no: bool) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MessageBoxW};
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (t, m) = (wide(title), wide(text));
        let flags = if yes_no { MB_YESNO | MB_ICONQUESTION } else { MB_OK | MB_ICONERROR };
        let r = unsafe { MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), flags | MB_TOPMOST | MB_SETFOREGROUND) };
        r == IDYES
    }
    #[cfg(not(windows))]
    {
        let _ = (title, text);
        yes_no
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

impl Wizard {
    pub fn new(mode: Mode, dir: PathBuf, progress: work::Shared) -> Self {
        Self {
            mode,
            page: if mode == Mode::Uninstall { Page::Confirm } else { Page::Welcome },
            dir: dir.display().to_string(),
            desktop: true,
            run_after: true,
            remove_data: false,
            progress,
            details: false,
            lang: i18n::language(),
            need: work::payload_size(),
            free: None,
            styled: false,
        }
    }

    fn product(&self) -> &'static str {
        "SteamInRust"
    }

    pub fn start(&mut self, ctx: &egui::Context) {
        if self.mode != Mode::Uninstall && !work::has_payload() {
            ask(&t!("SteamInRust Setup"), &t!("This setup was built without the app inside (developer build)."), false);
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if let Ok(mut g) = self.progress.lock() {
            *g = work::Progress::default();
        }
        self.page = Page::Working;
        if self.mode == Mode::Install {
            work::save_language(self.lang.code());
        }
        let (p, ctx, mode, remove_data) = (self.progress.clone(), ctx.clone(), self.mode, self.remove_data);
        let dir = PathBuf::from(self.dir.trim());
        let opts = work::Options { dir: dir.clone(), desktop: self.desktop, start_menu: true, update: mode == Mode::Update };
        std::thread::spawn(move || {
            let ticker = {
                let (ctx, p) = (ctx.clone(), p.clone());
                std::thread::spawn(move || {
                    while !p.lock().map(|g| g.done || g.error.is_some()).unwrap_or(true) {
                        ctx.request_repaint();
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    ctx.request_repaint();
                })
            };
            let result = match mode {
                Mode::Uninstall => work::uninstall(&dir, remove_data, &p),
                Mode::Update => work::install(&opts, &p, Duration::from_secs(60)),
                Mode::Install => work::install(&opts, &p, Duration::from_secs(3)),
            };
            if let Err(e) = result {
                if let Ok(mut g) = p.lock() {
                    g.error = Some(e);
                }
            }
            let _ = ticker.join();
        });
    }

    fn cancel(&self, ctx: &egui::Context) {
        let title = if self.mode == Mode::Uninstall { t!("SteamInRust Uninstall") } else { t!("SteamInRust Setup") };
        let q = if self.mode == Mode::Uninstall { t!("Are you sure you want to quit SteamInRust Uninstall?") } else { t!("Are you sure you want to quit SteamInRust Setup?") };
        if ask(&title, &q, true) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

impl eframe::App for Wizard {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.styled {
            let mut v = egui::Visuals::light();
            v.panel_fill = DIALOG;
            v.extreme_bg_color = WHITE;
            v.selection.bg_fill = FOCUS;
            v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x7A, 0x7A, 0x7A));
            v.widgets.inactive.corner_radius = CornerRadius::ZERO;
            v.widgets.hovered.corner_radius = CornerRadius::ZERO;
            v.widgets.active.corner_radius = CornerRadius::ZERO;
            v.widgets.inactive.weak_bg_fill = BTN;
            v.widgets.hovered.weak_bg_fill = BTN_HOVER;
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, FOCUS);
            v.override_text_color = Some(TEXT);
            ctx.set_visuals(v);
            ctx.global_style_mut(|s| s.interaction.selectable_labels = false);
            self.styled = true;
        }
        if self.mode == Mode::Update {
            self.updater(ui, &ctx);
            return;
        }
        // Worker finished?
        if self.page == Page::Working {
            let g = self.progress.lock().unwrap().clone();
            if let Some(e) = g.error {
                // Also kept in %TEMP%, for bug reports.
                let _ = std::fs::write(std::env::temp_dir().join("SteamInRust-Setup.log"), format!("{e}

{}", g.log.join("
")));
                let title = if self.mode == Mode::Uninstall { t!("SteamInRust Uninstall") } else { t!("SteamInRust Setup") };
                if ask(&title, &format!("{e}\n\n{}", t!("Click Yes to try again, or No to quit.")), true) {
                    self.start(&ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        let o = ui.max_rect().min;
        let at = |x: f32, y: f32| o + vec2(x, y);
        let p = ui.painter().clone();
        p.rect_filled(Rect::from_min_size(o, vec2(W, H)), CornerRadius::ZERO, DIALOG);

        match self.page {
            Page::Welcome | Page::Finish => self.side_page(ui, o),
            _ => self.inner_page(ui, &ctx, o),
        }

        // Bottom bar: etched line with the setup's name, then the buttons.
        let label = format!("{} {}", t!("SteamInRust Setup"), env!("CARGO_PKG_VERSION"));
        let g = p.layout_no_wrap(label, font(11.5), DISABLED);
        p.hline(o.x..=o.x + W, o.y + BAR_TOP, Stroke::new(1.0, ETCH_DARK));
        p.hline(o.x..=o.x + W, o.y + BAR_TOP + 1.0, Stroke::new(1.0, WHITE));
        // Just under the line, left of the buttons (never over the banner).
        let text_rect = Rect::from_min_size(at(8.0, BAR_TOP + 2.0), g.size() + vec2(6.0, 0.0));
        p.rect_filled(text_rect, CornerRadius::ZERO, DIALOG);
        p.galley(text_rect.min + vec2(3.0, 0.0), g, DISABLED);

        let cancel_r = Rect::from_min_size(at(W - 11.0 - 75.0, H - 34.0), vec2(75.0, 23.0));
        let next_r = cancel_r.translate(vec2(-85.0, 0.0));
        let back_r = next_r.translate(vec2(-75.0, 0.0));
        let working = self.page == Page::Working;
        let done = self.progress.lock().map(|g| g.done).unwrap_or(false);
        let uninstall = self.mode == Mode::Uninstall;
        let (next_label, next_on) = match self.page {
            Page::Location => (t!("Install"), !self.dir.trim().is_empty()),
            Page::Confirm => (t!("Uninstall"), true),
            Page::Working => (t!("Next >"), done),
            Page::Finish => (t!("Finish"), true),
            _ => (t!("Next >"), true),
        };
        let back_on = matches!(self.page, Page::Language | Page::Location);
        if self.page != Page::Confirm && button(ui, back_r, &t!("< Back"), back_on, false) {
            self.page = match self.page {
                Page::Location => Page::Language,
                _ => Page::Welcome,
            };
        }
        if button(ui, next_r, &next_label, next_on, true) {
            match self.page {
                Page::Welcome => self.page = Page::Language,
                Page::Language => self.page = Page::Location,
                Page::Location | Page::Confirm => self.start(&ctx),
                Page::Working => self.page = Page::Finish,
                Page::Finish => {
                    if self.run_after && !uninstall {
                        work::launch(&PathBuf::from(self.dir.trim()));
                    }
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        let cancel_on = !working && self.page != Page::Finish;
        if button(ui, cancel_r, &t!("Cancel"), cancel_on, false) {
            self.cancel(&ctx);
        }
        // The window's close button asks the same question (not mid-install).
        if ctx.input(|i| i.viewport().close_requested()) && self.page != Page::Finish {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if !working {
                self.cancel(&ctx);
            }
        }
        // Enter = the default button, Esc = Cancel, like a dialog.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && cancel_on {
            self.cancel(&ctx);
        }
    }
}

impl Wizard {
    /// Welcome and Finish: navy banner on the left, white text area.
    fn side_page(&mut self, ui: &mut Ui, o: Pos2) {
        let p = ui.painter().clone();
        let banner = Rect::from_min_size(o, vec2(BANNER_W, BAR_TOP));
        // Vertical gradient.
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(banner.left_top(), NAVY_TOP);
        mesh.colored_vertex(banner.right_top(), NAVY_TOP);
        mesh.colored_vertex(banner.right_bottom(), NAVY_BOTTOM);
        mesh.colored_vertex(banner.left_bottom(), NAVY_BOTTOM);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(mesh);
        logo(&p, pos2(banner.center().x, banner.top() + 120.0), 30.0);
        p.text(pos2(banner.center().x, banner.top() + 175.0), Align2::CENTER_CENTER, "SteamInRust", bold(15.0), WHITE);
        p.text(pos2(banner.center().x, banner.top() + 195.0), Align2::CENTER_CENTER, t!("Native Steam client"), font(11.0), Color32::from_rgb(0x8F, 0x98, 0xA0));
        let area = Rect::from_min_max(pos2(banner.right(), o.y), pos2(o.x + W, o.y + BAR_TOP));
        p.rect_filled(area, CornerRadius::ZERO, WHITE);
        let x = area.left() + 16.0;
        let width = area.width() - 32.0;
        let uninstall = self.mode == Mode::Uninstall;
        let (title, body) = match (self.page, uninstall) {
            (Page::Welcome, _) => (
                t!("Welcome to {app} Setup", app = self.product()),
                [
                    t!("Setup will guide you through the installation of {app}.", app = self.product()),
                    t!("It is recommended that you close all other applications before starting Setup. This will make it possible to update relevant system files without having to reboot your computer.").to_string(),
                    t!("Click Next to continue.").to_string(),
                ]
                .join("\n\n"),
            ),
            (_, false) => (t!("Completing {app} Setup", app = self.product()), [t!("{app} has been installed on your computer.", app = self.product()), t!("Click Finish to close Setup.").to_string()].join("\n\n")),
            (_, true) => (t!("Completing {app} Uninstall", app = self.product()), [t!("{app} has been uninstalled from your computer.", app = self.product()), t!("Click Finish to close this wizard.").to_string()].join("\n\n")),
        };
        let tg = p.layout(title, bold(16.0), TEXT, width);
        let th = tg.size().y;
        p.galley(pos2(x, o.y + 16.0), tg, TEXT);
        let bg = p.layout(body, font(12.5), TEXT, width);
        let by = o.y + 16.0 + th + 14.0;
        let bh = bg.size().y;
        p.galley(pos2(x, by), bg, TEXT);
        if self.page == Page::Finish && !uninstall {
            checkbox(ui, pos2(x, by + bh + 18.0), &t!("Run {app}", app = self.product()), &mut self.run_after);
        }
    }

    /// Inner pages: white header with title, subtitle and logo; gray body.
    fn inner_page(&mut self, ui: &mut Ui, ctx: &egui::Context, o: Pos2) {
        let p = ui.painter().clone();
        let header = Rect::from_min_size(o, vec2(W, HEADER_H));
        p.rect_filled(header, CornerRadius::ZERO, WHITE);
        p.hline(o.x..=o.x + W, o.y + HEADER_H, Stroke::new(1.0, ETCH_DARK));
        p.hline(o.x..=o.x + W, o.y + HEADER_H + 1.0, Stroke::new(1.0, WHITE));
        logo(&p, pos2(o.x + W - 34.0, o.y + HEADER_H / 2.0), 17.0);
        let uninstall = self.mode == Mode::Uninstall;
        let (title, sub) = match self.page {
            Page::Language => (t!("Language").to_string(), t!("Choose the language of {app}.", app = self.product())),
            Page::Location => (t!("Choose Install Location").to_string(), t!("Choose the folder in which to install {app}.", app = self.product())),
            Page::Confirm => (t!("Uninstall {app}", app = self.product()), t!("Remove {app} from your computer.", app = self.product())),
            _ if uninstall => (t!("Uninstalling").to_string(), t!("Please wait while {app} is being uninstalled.", app = self.product())),
            _ => (t!("Installing").to_string(), t!("Please wait while {app} is being installed.", app = self.product())),
        };
        p.text(pos2(o.x + 16.0, o.y + 12.0), Align2::LEFT_TOP, title, bold(12.5), TEXT);
        p.text(pos2(o.x + 26.0, o.y + 31.0), Align2::LEFT_TOP, sub, font(12.0), TEXT);

        let x = o.x + 24.0;
        let width = W - 48.0;
        let top = o.y + HEADER_H + 14.0;
        match self.page {
            Page::Language => {
                let g = p.layout(t!("Please select the language for {app} and this setup.", app = self.product()), font(12.5), TEXT, width);
                let h = g.size().y;
                p.galley(pos2(x, top), g, TEXT);
                let combo = Rect::from_min_size(pos2(x, top + h + 16.0), vec2(220.0, 24.0));
                let mut lang = self.lang;
                ui.scope_builder(egui::UiBuilder::new().max_rect(combo), |ui| {
                    egui::ComboBox::from_id_salt("language")
                        .width(220.0)
                        .selected_text(lang.native_name())
                        .show_ui(ui, |ui| {
                            for l in i18n::Lang::ALL {
                                ui.selectable_value(&mut lang, l, l.native_name());
                            }
                        });
                });
                if lang != self.lang {
                    self.lang = lang;
                    i18n::set_language(lang);
                    ctx.request_repaint();
                }
            }
            Page::Location => {
                let g = p.layout(
                    t!("Setup will install {app} in the following folder. To install in a different folder, click Browse and select another folder. Click Install to start the installation.", app = self.product()),
                    font(12.5),
                    TEXT,
                    width,
                );
                let h = g.size().y;
                p.galley(pos2(x, top), g, TEXT);
                let group = Rect::from_min_size(pos2(x, top + h + 18.0), vec2(width, 58.0));
                groupbox(&p, group, &t!("Destination Folder"));
                let field = Rect::from_min_size(group.min + vec2(14.0, 22.0), vec2(width - 28.0 - 90.0, 23.0));
                ui.put(field, egui::TextEdit::singleline(&mut self.dir).font(font(12.5)).margin(vec2(4.0, 3.0)));
                let browse = Rect::from_min_size(pos2(field.right() + 10.0, field.top()), vec2(80.0, 23.0));
                if button(ui, browse, &t!("Browse..."), true, false) {
                    if let Some(d) = rfd::FileDialog::new().set_directory(self.dir.trim()).pick_folder() {
                        let d = if d.ends_with("SteamInRust") { d } else { d.join("SteamInRust") };
                        self.dir = d.display().to_string();
                    }
                }
                checkbox(ui, pos2(x, group.bottom() + 14.0), &t!("Create a desktop shortcut"), &mut self.desktop);
                // Space required / available, refreshed when the drive changes.
                let drive: String = self.dir.chars().take(2).collect();
                if self.free.as_ref().is_none_or(|(d, _)| *d != drive) {
                    let free = work::free_space(&PathBuf::from(self.dir.trim())).unwrap_or(0);
                    self.free = Some((drive, free));
                }
                let free = self.free.as_ref().map(|f| f.1).unwrap_or(0);
                let y = o.y + BAR_TOP - 44.0;
                p.text(pos2(x, y), Align2::LEFT_TOP, t!("Space required: {size}", size = mb(self.need)), font(12.5), TEXT);
                p.text(pos2(x, y + 18.0), Align2::LEFT_TOP, t!("Space available: {size}", size = gb(free)), font(12.5), TEXT);
            }
            Page::Confirm => {
                let g = p.layout(t!("{app} will be uninstalled from the following folder. Click Uninstall to start the uninstallation.", app = self.product()), font(12.5), TEXT, width);
                let h = g.size().y;
                p.galley(pos2(x, top), g, TEXT);
                let y = top + h + 18.0;
                p.text(pos2(x, y + 4.0), Align2::LEFT_TOP, t!("Uninstalling from:"), font(12.5), TEXT);
                let field = Rect::from_min_size(pos2(x + 110.0, y), vec2(width - 110.0, 23.0));
                p.rect_filled(field, CornerRadius::ZERO, DIALOG);
                p.rect_stroke(field, CornerRadius::ZERO, Stroke::new(1.0, BTN_BORDER), StrokeKind::Inside);
                p.text(field.left_center() + vec2(5.0, 0.0), Align2::LEFT_CENTER, &self.dir, font(12.5), TEXT);
                checkbox(ui, pos2(x, y + 42.0), &t!("Also delete my settings, saved sign-ins and caches"), &mut self.remove_data);
            }
            Page::Working => {
                let g = self.progress.lock().unwrap().clone();
                p.text(pos2(x, top), Align2::LEFT_TOP, &g.file, font(12.5), TEXT);
                let bar = Rect::from_min_size(pos2(x, top + 22.0), vec2(width, 18.0));
                p.rect_filled(bar, CornerRadius::ZERO, Color32::from_rgb(0xE6, 0xE6, 0xE6));
                p.rect_stroke(bar, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0xBC, 0xBC, 0xBC)), StrokeKind::Inside);
                let fill = Rect::from_min_size(bar.min + vec2(1.0, 1.0), vec2((bar.width() - 2.0) * g.fraction.clamp(0.0, 1.0), bar.height() - 2.0));
                p.rect_filled(fill, CornerRadius::ZERO, PROGRESS);
                let list_top = bar.bottom() + 14.0;
                if self.details {
                    let list = Rect::from_min_max(pos2(x, list_top), pos2(x + width, o.y + BAR_TOP - 12.0));
                    p.rect_filled(list, CornerRadius::ZERO, WHITE);
                    p.rect_stroke(list, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x82, 0x87, 0x90)), StrokeKind::Inside);
                    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(list.shrink(3.0)));
                    egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(&mut inner, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for line in &g.log {
                            ui.label(egui::RichText::new(line).font(font(12.0)).color(TEXT));
                        }
                    });
                } else if button(ui, Rect::from_min_size(pos2(x, list_top), vec2(90.0, 23.0)), &t!("Show details"), true, false) {
                    self.details = true;
                }
            }
            _ => {}
        }
    }

    /// Steam's updater look: a small dark window with the logo, what it's
    /// doing, a thin bar and the bytes.
    fn updater(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let o = ui.max_rect().min;
        let p = ui.painter().clone();
        let full = Rect::from_min_size(o, vec2(420.0, 150.0));
        p.rect_filled(full, CornerRadius::ZERO, NAVY_TOP);
        p.rect_stroke(full, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x3D, 0x44, 0x50)), StrokeKind::Inside);
        let drag = ui.interact(full, egui::Id::unique("drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        logo(&p, pos2(o.x + 34.0, o.y + 34.0), 14.0);
        p.text(pos2(o.x + 56.0, o.y + 34.0), Align2::LEFT_CENTER, "SteamInRust", bold(18.0), WHITE);
        let g = self.progress.lock().unwrap().clone();
        if let Some(e) = &g.error {
            p.text(pos2(o.x + 22.0, o.y + 72.0), Align2::LEFT_TOP, t!("Update failed"), bold(13.0), Color32::from_rgb(0xE0, 0x5A, 0x4F));
            let eg = p.layout(e.clone(), font(11.5), Color32::from_rgb(0xB8, 0xBC, 0xBF), 376.0);
            p.galley(pos2(o.x + 22.0, o.y + 92.0), eg, WHITE);
            let retry = Rect::from_min_size(pos2(o.x + 420.0 - 22.0 - 150.0, o.y + 118.0), vec2(70.0, 22.0));
            let close = retry.translate(vec2(80.0, 0.0));
            if dark_button(ui, retry, &t!("Retry")) {
                self.start(ctx);
            }
            if dark_button(ui, close, &t!("Close")) {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            return;
        }
        let status = if g.done {
            t!("Verifying installation...")
        } else if g.fraction < 0.9 {
            t!("Extracting package...")
        } else {
            t!("Installing update...")
        };
        p.text(pos2(o.x + 22.0, o.y + 72.0), Align2::LEFT_TOP, t!("Updating SteamInRust..."), font(13.5), WHITE);
        let bar = Rect::from_min_size(pos2(o.x + 22.0, o.y + 98.0), vec2(376.0, 6.0));
        p.rect_filled(bar, CornerRadius::same(3), Color32::from_rgb(0x3D, 0x44, 0x50));
        let fill = Rect::from_min_size(bar.min, vec2(bar.width() * g.fraction.clamp(0.0, 1.0), bar.height()));
        p.rect_filled(fill, CornerRadius::same(3), Color32::from_rgb(0x1A, 0x9F, 0xFF));
        let (done, total) = g.bytes;
        let detail = if total > 0 && !g.done { format!("{status} ({} {} {} KB)", done / 1024, t!("of"), total / 1024) } else { status.to_string() };
        p.text(pos2(o.x + 22.0, o.y + 114.0), Align2::LEFT_TOP, detail, font(11.5), Color32::from_rgb(0x8F, 0x98, 0xA0));
        if g.done && !std::env::args().any(|a| a == "--no-launch") {
            // Straight back into the app, like Steam after its update.
            work::launch(&PathBuf::from(self.dir.trim()));
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

/// The app mark (a diamond), drawn without the app's theme.
fn logo(p: &egui::Painter, c: Pos2, r: f32) {
    let pts = |s: f32| vec![c - vec2(0.0, s), c + vec2(s, 0.0), c + vec2(0.0, s), c - vec2(s, 0.0)];
    p.add(egui::Shape::convex_polygon(pts(r), Color32::from_rgb(0x4F, 0x8C, 0xFF), Stroke::NONE));
    p.add(egui::Shape::convex_polygon(pts(r * 0.62), Color32::from_rgb(0x8B, 0x5C, 0xF6), Stroke::NONE));
    p.add(egui::Shape::convex_polygon(pts(r * 0.28), Color32::from_rgb(0x0A, 0x0D, 0x12), Stroke::NONE));
}

/// A Windows push button; `default` gets the blue focus border.
fn button(ui: &mut Ui, r: Rect, label: &str, enabled: bool, default: bool) -> bool {
    let resp = ui.interact(r, egui::Id::unique(("btn", label, r.min.x as i32, r.min.y as i32)), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let (fill, border, text) = if !enabled {
        (Color32::from_rgb(0xCC, 0xCC, 0xCC), Color32::from_rgb(0xBF, 0xBF, 0xBF), DISABLED)
    } else if resp.is_pointer_button_down_on() {
        (BTN_DOWN, Color32::from_rgb(0x00, 0x54, 0x99), TEXT)
    } else if resp.hovered() {
        (BTN_HOVER, FOCUS, TEXT)
    } else {
        (BTN, if default { FOCUS } else { BTN_BORDER }, TEXT)
    };
    p.rect_filled(r, CornerRadius::ZERO, fill);
    p.rect_stroke(r, CornerRadius::ZERO, Stroke::new(if default && enabled { 2.0 } else { 1.0 }, border), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, label, font(12.5), text);
    let enter = default && enabled && ui.input(|i| i.key_pressed(egui::Key::Enter));
    enabled && (resp.clicked() || enter)
}

fn dark_button(ui: &mut Ui, r: Rect, label: &str) -> bool {
    let resp = ui.interact(r, egui::Id::unique(("dbtn", label)), Sense::click());
    let fill = if resp.hovered() { Color32::from_rgb(0x47, 0x4F, 0x5C) } else { Color32::from_rgb(0x3D, 0x44, 0x50) };
    ui.painter().rect_filled(r, CornerRadius::same(2), fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, font(12.0), WHITE);
    resp.clicked()
}

/// A Windows check box with its label (clicking either toggles it).
fn checkbox(ui: &mut Ui, at: Pos2, label: &str, on: &mut bool) {
    let g = ui.painter().layout_no_wrap(label.to_string(), font(12.5), TEXT);
    let r = Rect::from_min_size(at, vec2(13.0 + 6.0 + g.size().x, 16.0f32.max(g.size().y)));
    let resp = ui.interact(r, egui::Id::unique(("check", label)), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let p = ui.painter();
    let bx = Rect::from_min_size(pos2(at.x, r.center().y - 6.5), vec2(13.0, 13.0));
    p.rect_filled(bx, CornerRadius::ZERO, WHITE);
    p.rect_stroke(bx, CornerRadius::ZERO, Stroke::new(1.0, if resp.hovered() { FOCUS } else { Color32::from_rgb(0x33, 0x33, 0x33) }), StrokeKind::Inside);
    if *on {
        let s = Stroke::new(1.6, TEXT);
        p.line_segment([bx.min + vec2(3.0, 6.5), bx.min + vec2(5.5, 9.0)], s);
        p.line_segment([bx.min + vec2(5.5, 9.0), bx.min + vec2(10.0, 3.5)], s);
    }
    p.galley(pos2(bx.right() + 6.0, r.center().y - g.size().y / 2.0), g, TEXT);
}

/// An etched frame with a caption, like "Destination Folder".
fn groupbox(p: &egui::Painter, r: Rect, caption: &str) {
    let g = p.layout_no_wrap(caption.to_string(), font(12.5), TEXT);
    let top = r.top() + g.size().y / 2.0;
    let frame = Rect::from_min_max(pos2(r.left(), top), r.max);
    p.rect_stroke(frame, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0xDC, 0xDC, 0xDC)), StrokeKind::Inside);
    let label = Rect::from_min_size(pos2(r.left() + 7.0, r.top()), g.size() + vec2(6.0, 0.0));
    p.rect_filled(label, CornerRadius::ZERO, DIALOG);
    p.galley(label.min + vec2(3.0, 0.0), g, TEXT);
}

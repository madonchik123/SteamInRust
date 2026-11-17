//! Store: a game's system requirements (a tab per system, like Steam's)
//! and its languages (a table of interface, full audio and subtitles).

use super::{Store, blocks};
use egui::{Ui, vec2};
use i18n::t;
use store::Details;
use store::browse::Language;
use ui::{Theme, icons, widgets as w};

/// Languages shown before "See all".
const FIRST: usize = 5;

#[derive(Default)]
pub(super) struct Specs {
    /// The system tab picked on the requirements (`None`: this computer's).
    os: Option<&'static str>,
    all_languages: bool,
}

fn os_label(os: &str) -> String {
    match os {
        "windows" => "Windows".into(),
        "macos" => "macOS".into(),
        _ => t!("SteamOS + Linux").to_string(),
    }
}

impl Store {
    pub(super) fn requirements(&mut self, ui: &mut Ui, t: &Theme, d: &Details, width: f32) {
        if d.requirements.is_empty() {
            return;
        }
        w::section(ui, t, t!("System requirements"), None);
        // This computer's system first, else the first one Steam lists.
        let here = if cfg!(target_os = "macos") { "macos" } else if cfg!(windows) { "windows" } else { "linux" };
        let chosen = self.specs.os.filter(|o| d.requirements.iter().any(|r| r.0 == *o)).or_else(|| d.requirements.iter().find(|r| r.0 == here).map(|r| r.0)).unwrap_or(d.requirements[0].0);
        if d.requirements.len() > 1 {
            ui.horizontal(|ui| {
                for (os, _, _) in &d.requirements {
                    if w::chip(ui, t, &os_label(os), *os == chosen).clicked() {
                        self.specs.os = Some(os);
                    }
                }
            });
            ui.add_space(8.0);
        }
        let Some((_, min, rec)) = d.requirements.iter().find(|r| r.0 == chosen) else { return };
        // Two columns when there's a "Recommended", else the minimum alone.
        if rec.is_empty() {
            ui.label(w::text_semibold(t!("Minimum"), 14.0, t.text_dim));
            blocks(ui, t, min, width);
        } else {
            ui.columns(2, |cols| {
                cols[0].label(w::text_semibold(t!("Minimum"), 14.0, t.text_dim));
                blocks(&mut cols[0], t, min, width / 2.0);
                cols[1].label(w::text_semibold(t!("Recommended"), 14.0, t.text_dim));
                blocks(&mut cols[1], t, rec, width / 2.0);
            });
        }
        ui.add_space(24.0);
    }

    pub(super) fn languages_table(&mut self, ui: &mut Ui, t: &Theme, langs: &[Language]) {
        let shown = if self.specs.all_languages { langs.len() } else { langs.len().min(FIRST) };
        // Each row is one rectangle with the name on the left and three
        // fixed columns on the right, so everything lines up.
        let col = 66.0;
        let row = |ui: &mut Ui, name: &str, cells: [Option<bool>; 3], line: bool| {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::hover());
            let p = ui.painter();
            if line {
                p.line_segment([r.left_top(), r.right_top()], egui::Stroke::new(1.0, t.border));
            }
            p.text(r.left_center(), egui::Align2::LEFT_CENTER, name, w::font(12.5), t.text_dim);
            for (i, c) in cells.iter().enumerate() {
                let center = egui::pos2(r.right() - col * (3 - i) as f32 + col / 2.0, r.center().y);
                match c {
                    Some(true) => {
                        p.text(center, egui::Align2::CENTER_CENTER, icons::CHECK, w::font(14.0), t.accent);
                    }
                    Some(false) => {}
                    None => {
                        let label = [t!("Interface"), t!("Full Audio"), t!("Subtitles")][i];
                        p.text(center, egui::Align2::CENTER_CENTER, label, w::font(11.0), t.text_faint);
                    }
                }
            }
        };
        ui.spacing_mut().item_spacing.y = 0.0;
        row(ui, "", [None, None, None], false);
        for l in &langs[..shown] {
            row(ui, &i18n::tr_dyn(l.name), [Some(l.interface), Some(l.full_audio), Some(l.subtitles)], true);
        }
        if langs.len() > FIRST {
            ui.add_space(4.0);
            let label = if self.specs.all_languages { t!("Show fewer languages").to_string() } else { t!("See all {n} supported languages", n = langs.len()).to_string() };
            if ui.link(w::text(label, 12.5, t.accent)).clicked() {
                self.specs.all_languages = !self.specs.all_languages;
            }
        }
    }
}

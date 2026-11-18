//! The design gallery page (`--page=gallery`): every shared control with
//! the current theme ([`ui::gallery`]). Not in the app's menus.

use egui::Ui;
use extensions::{Ctx, Extension, Manifest, PageSpec};
use ui::{icons, widgets as w};

#[derive(Default)]
pub struct Gallery(ui::gallery::Gallery);

impl Extension for Gallery {
    fn manifest(&self) -> Manifest {
        Manifest { id: "gallery", name: "Design gallery", author: "SteamInRust", description: "Every shared control", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("gallery", "Gallery", icons::PALETTE).hidden().order(2000)]
    }

    fn page_bar(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) -> bool {
        self.0.page_bar(ui, ctx.theme);
        true
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(24.0);
            w::centered(ui, 1300.0, 28.0, |ui| self.0.show(ui, &t));
            ui.add_space(40.0);
        });
    }
}

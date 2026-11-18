//! The design gallery: every shared control in each of its states, and a
//! sample page bar, drawn with any theme. The app shows it with
//! `--page=gallery`; tests draw it with every built-in theme.

use crate::{Theme, icons, widgets as w};
use egui::{Ui, vec2};

pub struct Gallery {
    tab: usize,
    search: String,
    view: usize,
    on: bool,
    off: bool,
    checked: bool,
    unchecked: bool,
    sort: usize,
    seg: usize,
    wish: bool,
}

impl Default for Gallery {
    fn default() -> Self {
        Self { tab: 0, search: String::new(), view: 0, on: true, off: false, checked: true, unchecked: false, sort: 0, seg: 0, wish: false }
    }
}

impl Gallery {
    /// A sample page bar: tabs, a search, filter and view buttons, links.
    pub fn page_bar(&mut self, ui: &mut Ui, t: &Theme) {
        let t = t.clone();
        ui.spacing_mut().item_spacing.x = 7.0;
        for (i, label) in ["Home", "Collections", "News and Updates"].iter().enumerate() {
            if w::sub_tab(ui, &t, label, self.tab == i).clicked() {
                self.tab = i;
            }
        }
        ui.add_space(30.0);
        w::bar_search(ui, &t, &mut self.search, "Search...", 374.0);
        ui.add_space(6.0);
        w::icon_box(ui, &t, icons::FUNNEL, "Filters", false);
        w::icon_box(ui, &t, icons::CLOCK, "Recent", false);
        w::icon_box(ui, &t, icons::PLAY, "Ready to play", false);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            w::sub_tab(ui, &t, "Add a game", false);
            w::sub_tab(ui, &t, "Activate a Product", false);
            ui.add_space(20.0);
            for (i, icon) in [icons::LIST, icons::GRID, icons::SQUARE].iter().enumerate().rev() {
                if w::icon_box(ui, &t, icon, "View", self.view == i).clicked() {
                    self.view = i;
                }
            }
            ui.label(w::body_m(&t, "View", t.text));
        });
    }

}

fn row(ui: &mut Ui, t: &Theme, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.label(w::caption_text(t, title));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        add(ui);
    });
    ui.add_space(18.0);
}

impl Gallery {
    /// Every shared control, in each state.
    pub fn show(&mut self, ui: &mut Ui, t: &Theme) {
        ui.label(w::heading_l(t, "Heading Large 26", t.text));
        ui.label(w::heading_m(t, "Heading Medium 22", t.text));
        ui.label(w::heading_s(t, "Heading Small 18", t.text));
        ui.label(w::body_l(t, "Body Large 16", t.text));
        ui.label(w::body_m(t, "Body Medium 14", t.text));
        ui.label(w::body_s(t, "Body Small 12", t.text_dim));
        ui.add_space(18.0);

        row(ui, t, "Colors", |ui| {
            for (name, c) in [
                ("chrome", t.chrome),
                ("bg", t.bg),
                ("raised", t.raised),
                ("surface", t.surface),
                ("surface_hover", t.surface_hover),
                ("tertiary", t.tertiary),
                ("accent_2", t.accent_2),
                ("accent", t.accent),
                ("play", t.play),
                ("danger", t.danger),
                ("away", t.away),
                ("text", t.text),
                ("text_dim", t.text_dim),
            ] {
                ui.vertical(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(84.0, 40.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, t.corner_small(), c);
                    ui.painter().rect_stroke(r, t.corner_small(), egui::Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
                    ui.label(w::body_s(t, name, t.text_dim));
                });
            }
        });

        row(ui, t, "Buttons (large)", |ui| {
            w::button(ui, t, None, "Add to Cart", w::Kind::Primary, w::Size::Lg);
            w::button(ui, t, None, "Add to Wishlist", w::Kind::Secondary, w::Size::Lg);
            w::button(ui, t, None, "Ignore", w::Kind::Tertiary, w::Size::Lg);
            w::play_button(ui, t, "Play");
            w::install_button(ui, t, "Install");
            w::pause_button(ui, t, "Pause");
            ui.add_enabled_ui(false, |ui| w::button(ui, t, None, "Disabled", w::Kind::Primary, w::Size::Lg));
        });
        row(ui, t, "Buttons (medium, small)", |ui| {
            for k in [w::Kind::Primary, w::Kind::Secondary, w::Kind::Tertiary, w::Kind::Ghost, w::Kind::Play, w::Kind::Install, w::Kind::Pause, w::Kind::Danger] {
                w::button(ui, t, None, &format!("{k:?}"), k, w::Size::Md);
            }
            for k in [w::Kind::Primary, w::Kind::Secondary, w::Kind::Ghost] {
                w::button(ui, t, Some(icons::GEAR), &format!("{k:?}"), k, w::Size::Sm);
            }
            w::icon_button(ui, t, icons::GEAR, "Icon button", false, 0);
            w::icon_button(ui, t, icons::BELL, "With a badge", false, 3);
        });
        row(ui, t, "Main CTA and price", |ui| {
            w::main_cta(ui, t, "$34.99", None, 0, None, "Add to Cart");
            w::main_cta(ui, t, "$23.09", Some("$34.99"), 34, None, "Add to Cart");
            w::main_cta(ui, t, "$23.09", Some("$34.99"), 34, Some("Until October 19"), "Add to Cart");
        });
        row(ui, t, "Price (compact)", |ui| {
            w::price_tag(ui, t, "$34.99", None, 0);
            w::price_tag(ui, t, "$23.09", Some("$34.99"), 34);
            w::price(ui, t, "$23.09", Some("$34.99"), 34);
        });
        row(ui, t, "Wishlist", |ui| {
            if w::wishlist_button(ui, t, self.wish, None).clicked() {
                self.wish = !self.wish;
            }
            w::wishlist_button(ui, t, false, Some("Wishlist"));
            w::wishlist_button(ui, t, true, Some("Wishlisted"));
            w::heart_button(ui, t, true, "Favorite");
        });
        row(ui, t, "Tags", |ui| {
            w::tag(ui, t, "Souls-like", None, w::TagState::Plain);
            w::tag(ui, t, "Souls-like", Some("5,000"), w::TagState::Unchecked);
            w::tag(ui, t, "Souls-like", Some("5,000"), w::TagState::Checked);
            w::tag(ui, t, "Souls-like", Some("5,000"), w::TagState::Current);
            w::tag(ui, t, "Souls-like", None, w::TagState::Filtered);
            w::chip(ui, t, "Chip", false);
            w::chip(ui, t, "Chip on", true);
        });
        row(ui, t, "Toggle and check box", |ui| {
            w::toggle(ui, t, &mut self.on);
            w::toggle(ui, t, &mut self.off);
            w::checkbox(ui, t, &mut self.checked, 30.0);
            w::checkbox(ui, t, &mut self.unchecked, 30.0);
            w::check(ui, t, &mut self.checked, "Check box with a label");
            w::radio(ui, t, true, "Radio");
        });
        row(ui, t, "Sort, dropdown, segmented", |ui| {
            w::sort_by(ui, t, &mut self.sort, &[(0, "New and Trending"), (1, "Top Sellers"), (2, "Top Rated"), (3, "Upcoming")]);
            let r = w::dropdown(ui, t, "New and Trending", 200.0);
            egui::Popup::menu(&r).width(220.0).show(|ui| {
                for (icon, label) in [(icons::FIRE, "New and Trending"), (icons::STAR, "Top Rated"), (icons::CALENDAR, "Upcoming")] {
                    w::menu_item(ui, t, icon, label);
                }
            });
            w::segmented(ui, t, &mut self.seg, &[(0, "All"), (1, "Installed"), (2, "Favorites")]);
        });
        row(ui, t, "Reviews and progress", |ui| {
            for p in [90, 70, 50, 20] {
                w::review_bar(ui, t, p, 138.0);
            }
            w::progress(ui, t, 0.4, 160.0);
            let (r, _) = ui.allocate_exact_size(vec2(60.0, 30.0), egui::Sense::hover());
            w::progress_ring(ui.painter(), t, r.left_center() + vec2(10.0, 0.0), 7.5, 0.25);
        });
        row(ui, t, "Search, icon boxes, top-bar buttons", |ui| {
            w::search(ui, t, &mut self.search, "Search...", 300.0);
            w::icon_box(ui, t, icons::LIST, "List", true);
            w::icon_box(ui, t, icons::GRID, "Grid", false);
            w::bar_button(ui, t, icons::USERS, "Friends", false, 4);
            w::bar_button(ui, t, icons::BELL, "Notifications", false, 1);
        });
        row(ui, t, "Menu", |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_width(220.0);
                for (icon, label) in [(icons::HOUSE, "Home"), (icons::SLIDERS, "Browse"), (icons::HEART, "Wishlist")] {
                    w::menu_item(ui, t, icon, label);
                }
                w::menu_item_tinted(ui, t, icons::TRASH, "Uninstall…", Some(t.danger));
            });
            w::card(t).show(ui, |ui| {
                ui.set_width(320.0);
                ui.label(w::body_l(t, "A card", t.text));
                ui.label(w::body_m(t, "Flat, rounded, no border.", t.text_dim));
            });
        });
    }
}

#[cfg(test)]
mod tests {
    /// Every built-in theme draws every shared control without panicking.
    #[test]
    fn every_builtin_theme_draws_the_gallery() {
        for t in crate::themes::builtins() {
            let ctx = egui::Context::default();
            t.apply(&ctx);
            let mut g = super::Gallery::default();
            let input = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0))),
                ..Default::default()
            };
            for _ in 0..2 {
                let mut out = ctx.run_ui(input(), |ui| {
                    egui::Panel::top("bar").exact_size(t.layout.page_bar_height).show(ui, |ui| ui.horizontal_centered(|ui| g.page_bar(ui, &t)));
                    egui::CentralPanel::default().show(ui, |ui| {
                        crate::widgets::paint_background(ui, &t, ui.max_rect(), "library");
                        g.show(ui, &t);
                    });
                });
                // Nothing uploads the textures in a test.
                out.textures_delta.clear();
            }
        }
    }
}

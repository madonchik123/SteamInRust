//! Edit your profile in the app, like Steam's Edit Profile pages: name,
//! real name, custom URL, location and summary; avatar (upload, animated
//! avatars, frames); profile and mini profile backgrounds; theme; featured
//! badge; and privacy settings.
//!
//! Item, theme and badge choices are previewed first and only go to Steam
//! with Save; Cancel puts back what Steam has.

use super::Profile;
use crate::builtin::settings;
use client::Command;
use egui::{Align, Align2, CornerRadius, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{ProfileEdit, ProfileItemChoice, ProfileSave, art};
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) enum Section {
    #[default]
    General,
    Avatar,
    Background,
    MiniProfile,
    Theme,
    Badge,
    Privacy,
}

impl Section {
    pub fn from_arg(s: &str) -> Section {
        match s {
            "avatar" => Section::Avatar,
            "background" => Section::Background,
            "miniprofile" => Section::MiniProfile,
            "theme" => Section::Theme,
            "badge" | "favoritebadge" => Section::Badge,
            "privacy" | "settings" => Section::Privacy,
            _ => Section::General,
        }
    }
}

#[derive(Default)]
pub(super) struct EditState {
    pub section: Section,
    /// What's being edited (a copy of Steam's values until saved).
    draft: Option<ProfileEdit>,
    asked: bool,
    /// Saves sent and not answered yet.
    pending: usize,
    /// Sent once the saves before them went through (a background's
    /// "Full Screen" needs the background equipped first).
    after: Vec<Command>,
    /// Location lists asked for.
    asked_locations: std::collections::HashSet<(Option<String>, Option<String>)>,
    /// The filter box on the background lists.
    filter: String,
    /// "See All" opened: your avatars, your frames.
    see_all: [bool; 2],
}

fn lang() -> String {
    i18n::language().steam_name().to_string()
}

/// Steam's profile themes as color swatches (their own colors, like a
/// picture from Steam): the two ends of each theme's gradient.
fn theme_colors(id: &str) -> [egui::Color32; 2] {
    let rgb = |c: u32| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let id = id.to_lowercase();
    let (a, b) = if id.contains("summer") {
        (0xE07B39, 0x8E2C2C)
    } else if id.contains("midnight") {
        (0x3B2F6B, 0x110E26)
    } else if id.contains("steel") {
        (0x66737F, 0x2C3238)
    } else if id.contains("cosmic") {
        (0x9A4FD1, 0x2A1446)
    } else if id.contains("dark") {
        (0x2B2B2B, 0x0B0B0B)
    } else {
        (0x2A475E, 0x1B2838)
    };
    [rgb(a), rgb(b)]
}

impl EditState {
    fn saving(&self) -> bool {
        self.pending > 0
    }
}

impl Profile {
    /// Profile Settings (Figma "profile settings" `476:6952`): the
    /// settings layout with the profile's sections.
    pub(super) fn edit_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        if !self.edit.asked {
            self.edit.asked = true;
            ctx.send(Command::LoadProfileEdit { language: lang() });
        }
        if self.edit.draft.is_none() {
            if let Some(Ok(e)) = &ctx.state.profile_edit {
                self.edit.draft = Some(e.clone());
            }
        }
        let (mut back, mut pick, mut shop) = (false, None, false);
        let current = self.edit.section;
        settings::layout(
            ui,
            t,
            t!("Profile Settings"),
            |ui| back = w::button(ui, t, None, t!("Back to Profile"), w::Kind::Secondary, w::Size::Md).clicked(),
            |ui| {
                for (s, label) in [
                    (Section::General, t!("General")),
                    (Section::Avatar, t!("Avatar")),
                    (Section::Background, t!("Profile Background")),
                    (Section::MiniProfile, t!("Mini Profile")),
                    (Section::Theme, t!("Theme")),
                    (Section::Badge, t!("Featured Badge")),
                ] {
                    if settings::nav_item(ui, t, label, current == s).clicked() {
                        pick = Some(s);
                    }
                }
                settings::nav_divider(ui, t);
                if settings::nav_item(ui, t, t!("Privacy Settings"), current == Section::Privacy).clicked() {
                    pick = Some(Section::Privacy);
                }
                shop = settings::nav_item(ui, t, t!("Points Shop"), false).clicked();
            },
            |ui| {
                if let (None, Some(Err(e))) = (&self.edit.draft, &ctx.state.profile_edit) {
                    let e = i18n::tr_dyn(e).to_string();
                    settings::group(ui, t, "", |ui| w::empty(ui, t, icons::WARNING, t!("Couldn't load your profile settings"), &e));
                    return;
                }
                if self.edit.draft.is_none() {
                    settings::group(ui, t, "", |ui| {
                        for _ in 0..5 {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), t.dense(34.0)), Sense::hover());
                            w::skeleton(ui, t, r, t.corner_small());
                        }
                    });
                    return;
                }
                match self.edit.section {
                    Section::General => self.edit_general(ui, t, ctx),
                    Section::Avatar => self.edit_avatar(ui, t, ctx),
                    Section::Background => self.edit_background(ui, t, ctx),
                    Section::MiniProfile => self.edit_mini(ui, t, ctx),
                    Section::Theme => self.edit_theme(ui, t, ctx),
                    Section::Badge => self.edit_badge(ui, t, ctx),
                    Section::Privacy => self.edit_privacy(ui, t, ctx),
                }
            },
        );
        if back {
            ctx.navigate("profile", None);
        }
        if let Some(s) = pick {
            if s != self.edit.section {
                self.edit.filter.clear();
            }
            self.edit.section = s;
        }
        if shop {
            crate::links::open_http("https://store.steampowered.com/points/shop");
        }
    }

    /// Send saves; their answers come back in [`Self::edit_event`].
    fn send_saves(&mut self, ctx: &mut Ctx, now: Vec<Command>, after: Vec<Command>) {
        self.edit.pending += now.len();
        for c in now {
            ctx.send(c);
        }
        self.edit.after.extend(after);
    }

    fn edit_general(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let saving = self.edit.saving();
        let Some(d) = self.edit.draft.as_mut() else { return };
        settings::group(ui, t, t!("General"), |ui| {
            field(ui, t, &format!("{} *", t!("Profile name")), t!("The name everyone sees on your profile and in chat."), |ui| {
                let width = ui.available_width();
                w::text_input(ui, t, &mut d.persona_name, "", width, 1);
            });
            field(ui, t, t!("Real name"), t!("Providing your real name can help friends find you on the Steam Community."), |ui| {
                let width = ui.available_width();
                w::text_input(ui, t, &mut d.real_name, t!("Your real name here…"), width, 1);
            });
            field(ui, t, t!("Custom URL"), t!("Your custom profile address on the Steam Community."), |ui| {
                let width = ui.available_width();
                w::text_input(ui, t, &mut d.custom_url, "", width, 1);
                let url = d.custom_url.trim();
                if !url.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(w::body_m(t, t!("Your profile will be available at:"), t.text_dim));
                        let link = format!("https://steamcommunity.com/id/{url}/");
                        if ui.link(w::body_m(t, &link, t.accent)).clicked() {
                            crate::links::open_http(&link);
                        }
                    });
                }
            });
        });

        // Location: country, then its state, then the state's city.
        let countries = ctx.state.locations.get(&(None, None)).cloned();
        let states = (!d.country.is_empty()).then(|| ctx.state.locations.get(&(Some(d.country.clone()), None)).cloned()).flatten();
        let cities = (!d.country.is_empty() && !d.state.is_empty()).then(|| ctx.state.locations.get(&(Some(d.country.clone()), Some(d.state.clone()))).cloned()).flatten();
        let mut want = vec![(None, None)];
        if !d.country.is_empty() {
            want.push((Some(d.country.clone()), None));
        }
        if !d.country.is_empty() && !d.state.is_empty() {
            want.push((Some(d.country.clone()), Some(d.state.clone())));
        }
        for key in want {
            if self.edit.asked_locations.insert(key.clone()) {
                ctx.send(Command::LoadLocations { country: key.0, state: key.1 });
            }
        }
        settings::group(ui, t, t!("Location"), |ui| {
            let mut changed_country = false;
            field(ui, t, t!("Country"), t!("Shown on your profile, if you choose one."), |ui| {
                let width = ui.available_width();
                changed_country = pick(ui, t, &countries, &mut d.country, width);
            });
            if changed_country {
                d.state.clear();
                d.city.clear();
            }
            if states.as_ref().is_some_and(|s| !s.is_empty()) {
                let mut changed = false;
                field(ui, t, t!("State or region"), "", |ui| {
                    let width = ui.available_width();
                    changed = pick(ui, t, &states, &mut d.state, width);
                });
                if changed {
                    d.city.clear();
                }
            }
            if cities.as_ref().is_some_and(|c| !c.is_empty()) {
                field(ui, t, t!("City"), "", |ui| {
                    let width = ui.available_width();
                    pick(ui, t, &cities, &mut d.city, width);
                });
            }
        });
        settings::group(ui, t, t!("Summary"), |ui| {
            ui.add(egui::Label::new(w::body_m(t, t!("A few words about you, shown under your name."), t.text_dim)).wrap());
            let width = ui.available_width();
            w::text_input(ui, t, &mut d.summary, "", width, 6);
        });
        settings::group(ui, t, t!("Awards"), |ui| {
            check_line(ui, t, &mut d.hide_awards, t!("Hide Community Awards"), t!("Hide the awards you've been given on your profile"));
        });
        let orig = original(ctx);
        let changed = orig.as_ref().is_some_and(|orig| {
            (&orig.persona_name, &orig.real_name, &orig.custom_url, &orig.country, &orig.state, &orig.city, &orig.summary, orig.hide_awards)
                != (&d.persona_name, &d.real_name, &d.custom_url, &d.country, &d.state, &d.city, &d.summary, d.hide_awards)
        });
        let (save, cancel) = save_bar(ui, t, changed, saving, !d.persona_name.trim().is_empty());
        if save {
            let c = Command::SaveProfileInfo(Box::new(d.clone()));
            self.send_saves(ctx, vec![c], vec![]);
        }
        if cancel {
            self.edit.draft = orig;
        }
    }

    /// Avatar (Steam's avatar page): the picture at its three sizes, upload,
    /// your animated avatars and your frames, previewed until saved.
    fn edit_avatar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.persona(ctx.state.steam_id);
        let saving = self.edit.saving();
        let still = art::avatar_full(me.avatar_hash.as_deref());
        let Some(d) = self.edit.draft.as_mut() else { return };
        let animated = d.animated_avatars.iter().find(|i| i.id == d.equipped[3]).map(|i| i.image.clone());
        let frame = d.frames.iter().find(|i| i.id == d.equipped[2]).map(|i| i.image.clone());
        let shown = animated.clone().unwrap_or_else(|| still.clone());
        let mut upload = None;
        settings::group(ui, t, t!("Avatar"), |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.xl;
                for size in [184.0, 64.0, 32.0] {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = t.spacing.xs;
                        let s = t.dense(size);
                        let (r, _) = ui.allocate_exact_size(vec2(s, s), Sense::hover());
                        avatar_with_frame(ui, t, r, &shown, frame.as_deref());
                        ui.label(w::body_s(t, format!("{size:.0} px"), t.text_dim));
                    });
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = t.spacing.sm;
                    if saving {
                        ui.add(egui::Spinner::new().size(t.typography.heading_s.size).color(t.accent));
                    } else if w::button(ui, t, Some(icons::IMAGE), t!("Upload your avatar"), w::Kind::Primary, w::Size::Md).clicked() {
                        upload = rfd::FileDialog::new().add_filter(t!("Images").to_string(), &["jpg", "jpeg", "png"]).pick_file();
                    }
                    ui.add(egui::Label::new(w::body_m(t, t!("Upload a JPEG or PNG at least 184 px square; Steam crops it to a square. An upload is saved right away."), t.text_dim)).wrap());
                });
            });
        });
        if let Some(path) = upload {
            self.edit.pending += 1;
            ctx.send(Command::UploadAvatar { path });
        }
        let see_all = &mut self.edit.see_all;
        let tile = vec2(t.dense(110.0), t.dense(110.0));
        // How many tiles a row holds (inside the group's padding).
        let inner = ui.available_width() - t.padding_of(&t.components.settings_group).x * 2.0;
        let per_row = (((inner + t.spacing.md) / (tile.x + t.spacing.md)).floor() as usize).max(1);
        let mut chose = None;
        titled_group(ui, t, t!("Your Avatars"), &mut see_all[0], d.animated_avatars.len() + 1 > per_row, |ui, all| {
            ui.add(egui::Label::new(w::body_m(t, t!("Animated avatars you own from the Points Shop. With none chosen, your uploaded picture shows."), t.text_dim)).wrap());
            chose = item_grid(ui, t, &d.animated_avatars, d.equipped[3], "", tile, false, all, &|ui, r, it| {
                if !super::paint_picture(ui, r, &it.image, t.corner_small()) {
                    w::skeleton(ui, t, r, t.corner_small());
                }
            }, &|ui, r| {
                super::paint_picture(ui, r, &still, t.corner_small());
            });
        });
        if let Some(id) = chose {
            d.equipped[3] = id;
        }
        let mut chose = None;
        titled_group(ui, t, t!("Your Avatar Frames"), &mut see_all[1], d.frames.len() + 1 > per_row, |ui, all| {
            ui.add(egui::Label::new(w::body_m(t, t!("Frames go around your avatar everywhere on Steam."), t.text_dim)).wrap());
            chose = item_grid(ui, t, &d.frames, d.equipped[2], "", tile, false, all, &|ui, r, it| {
                avatar_with_frame(ui, t, r, &shown, Some(&it.image));
            }, &|ui, r| {
                avatar_with_frame(ui, t, r, &shown, None);
            });
        });
        if let Some(id) = chose {
            d.equipped[2] = id;
        }
        let orig = original(ctx);
        let o = orig.as_ref().map(|o| o.equipped).unwrap_or_default();
        let changed = o[2] != d.equipped[2] || o[3] != d.equipped[3];
        let (save, cancel) = save_bar(ui, t, changed, saving, true);
        let now: Vec<Command> = [3usize, 2].into_iter().filter(|k| o[*k] != d.equipped[*k]).map(|k| Command::EquipProfileItem { kind: k as u8, id: d.equipped[k] }).collect();
        if cancel {
            d.equipped[2] = o[2];
            d.equipped[3] = o[3];
        }
        if save {
            self.send_saves(ctx, now, vec![]);
        }
    }

    /// Profile Background: a preview of your profile in front of it, Full
    /// Screen or Original Size, and the backgrounds you own.
    fn edit_background(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let saving = self.edit.saving();
        let preview = Preview::of(ctx, self.edit.draft.as_ref());
        let filter = &mut self.edit.filter;
        let Some(d) = self.edit.draft.as_mut() else { return };
        let chosen = d.backgrounds.iter().find(|i| i.id == d.equipped[0]).cloned();
        let mut chose = None;
        settings::group(ui, t, t!("Profile Background"), |ui| {
            let width = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(width, (width * 0.42).min(t.dense(420.0))), Sense::hover());
            profile_preview(ui, t, r, chosen.as_ref(), d.background_full, None, &preview);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.sm;
                ui.add_enabled_ui(chosen.is_some(), |ui| {
                    if w::chip(ui, t, t!("Full Screen"), d.background_full).clicked() {
                        d.background_full = true;
                    }
                    if w::chip(ui, t, t!("Original Size"), !d.background_full).clicked() {
                        d.background_full = false;
                    }
                });
            });
            filter_box(ui, t, filter);
            let tile = vec2(t.dense(200.0), t.dense(112.0));
            chose = item_grid(ui, t, &d.backgrounds, d.equipped[0], filter, tile, true, true, &|ui, r, it| {
                if !w::paint_cover(ui, r, if it.large.is_empty() { &it.image } else { &it.large }, t.corner_small()) {
                    w::skeleton(ui, t, r, t.corner_small());
                }
                if !it.movie.is_empty() {
                    animated_mark(ui, t, r);
                }
            }, &|ui, r| {
                ui.painter().rect_filled(r, t.corner_small(), t.raised);
            });
        });
        if let Some(id) = chose {
            d.equipped[0] = id;
        }
        let orig = original(ctx);
        let (o, o_full) = orig.as_ref().map(|o| (o.equipped[0], o.background_full)).unwrap_or_default();
        let changed = o != d.equipped[0] || (d.equipped[0] != 0 && o_full != d.background_full);
        let (save, cancel) = save_bar(ui, t, changed, saving, true);
        if cancel {
            d.equipped[0] = o;
            d.background_full = o_full;
        }
        if save {
            let (id, full) = (d.equipped[0], d.background_full);
            let flags = (id != 0 && (o != id || o_full != full)).then_some(Command::SetBackgroundFullScreen { id, full });
            if o != id {
                // The new background first, then its size.
                self.send_saves(ctx, vec![Command::EquipProfileItem { kind: 0, id }], flags.into_iter().collect());
            } else {
                self.send_saves(ctx, flags.into_iter().collect(), vec![]);
            }
        }
    }

    /// Mini Profile: the card people see when they hover your name, with
    /// the chosen background, and the backgrounds you own.
    fn edit_mini(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let saving = self.edit.saving();
        let preview = Preview::of(ctx, self.edit.draft.as_ref());
        let filter = &mut self.edit.filter;
        let Some(d) = self.edit.draft.as_mut() else { return };
        let chosen = d.mini_backgrounds.iter().find(|i| i.id == d.equipped[1]).cloned();
        let mut chose = None;
        settings::group(ui, t, t!("Mini Profile"), |ui| {
            ui.add(egui::Label::new(w::body_m(t, t!("The card people see when they point at your name in their friends list or chat."), t.text_dim)).wrap());
            let width = ui.available_width().min(t.dense(360.0));
            let (r, _) = ui.allocate_exact_size(vec2(width, t.dense(230.0)), Sense::hover());
            mini_preview(ui, t, r, chosen.as_ref(), &preview);
            filter_box(ui, t, filter);
            let tile = vec2(t.dense(200.0), t.dense(112.0));
            chose = item_grid(ui, t, &d.mini_backgrounds, d.equipped[1], filter, tile, true, true, &|ui, r, it| {
                if !w::paint_cover(ui, r, if it.large.is_empty() { &it.image } else { &it.large }, t.corner_small()) {
                    w::skeleton(ui, t, r, t.corner_small());
                }
                if !it.movie.is_empty() {
                    animated_mark(ui, t, r);
                }
            }, &|ui, r| {
                ui.painter().rect_filled(r, t.corner_small(), t.raised);
            });
        });
        if let Some(id) = chose {
            d.equipped[1] = id;
        }
        let o = original(ctx).map(|o| o.equipped[1]).unwrap_or_default();
        let (save, cancel) = save_bar(ui, t, o != d.equipped[1], saving, true);
        if cancel {
            d.equipped[1] = o;
        }
        if save {
            let c = Command::EquipProfileItem { kind: 1, id: d.equipped[1] };
            self.send_saves(ctx, vec![c], vec![]);
        }
    }

    /// Theme: your profile in each of Steam's color themes.
    fn edit_theme(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let saving = self.edit.saving();
        let preview = Preview::of(ctx, self.edit.draft.as_ref());
        let Some(d) = self.edit.draft.as_mut() else { return };
        let background = d.backgrounds.iter().find(|i| i.id == d.equipped[0]).cloned();
        // Steam's list, with Default first.
        let mut themes = d.themes.clone();
        if !themes.iter().any(|(id, _)| id.is_empty()) {
            themes.insert(0, (String::new(), t!("Default").to_string()));
        }
        settings::group(ui, t, t!("Theme"), |ui| {
            ui.add(egui::Label::new(w::body_m(t, t!("The colors of your profile page on Steam."), t.text_dim)).wrap());
            let width = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(width, (width * 0.42).min(t.dense(420.0))), Sense::hover());
            profile_preview(ui, t, r, background.as_ref(), d.background_full, Some(theme_colors(&d.theme)), &preview);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.lg, t.spacing.md);
                for (id, title) in &themes {
                    if swatch(ui, t, title, theme_colors(id), *id == d.theme) {
                        d.theme = id.clone();
                    }
                }
            });
        });
        let o = original(ctx).map(|o| o.theme).unwrap_or_default();
        let (save, cancel) = save_bar(ui, t, o != d.theme, saving, true);
        if cancel {
            d.theme = o;
        }
        if save {
            let c = Command::SetProfileTheme(d.theme.clone());
            self.send_saves(ctx, vec![c], vec![]);
        }
    }

    /// Featured Badge: one of your community badges, shown on your profile
    /// and mini profile.
    fn edit_badge(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let saving = self.edit.saving();
        let me = ctx.state.steam_id;
        let badges = ctx.state.badges.get(&me).cloned();
        if badges.is_none() && !self.badge_asked.contains("me") {
            self.badge_asked.insert("me".into());
            ctx.send(Command::LoadBadges { steam_id: me });
        }
        let Some(d) = self.edit.draft.as_mut() else { return };
        settings::group(ui, t, t!("Featured Badge"), |ui| {
            ui.add(egui::Label::new(w::body_m(t, t!("The badge shown next to your level on your profile and mini profile. Community badges can be featured here; game badges from Steam's own page."), t.text_dim)).wrap());
            match &badges {
                None => super::loading_rows(ui, t, 2, t.dense(96.0)),
                Some(Err(e)) => {
                    ui.label(w::body_m(t, i18n::tr_dyn(e), t.text_dim));
                }
                Some(Ok(b)) => {
                    let list: Vec<(u32, &model::Badge)> = b.items.iter().filter_map(|x| community_badge_id(&x.url).map(|id| (id, x))).collect();
                    let tile = vec2(t.dense(120.0), t.dense(120.0));
                    let gap = t.spacing.md;
                    let cols = (((ui.available_width() + gap) / (tile.x + gap)).floor() as usize).max(1);
                    let mut options: Vec<(u32, Option<&model::Badge>)> = vec![(0, None)];
                    options.extend(list.iter().map(|(id, b)| (*id, Some(*b))));
                    for row in options.chunks(cols) {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for (id, badge) in row {
                                let label = badge.map(|b| b.title.as_str()).unwrap_or(t!("None"));
                                let sub = badge.map(|b| b.detail.as_str()).unwrap_or("");
                                if tile_button(ui, t, tile, label, sub, *id == d.favorite_badge, |ui, r| match badge.and_then(|b| b.icon.as_deref()) {
                                    Some(u) => {
                                        if !w::paint_contain(ui, r.shrink(t.spacing.md), u, Align2::CENTER_CENTER) {
                                            w::skeleton(ui, t, r.shrink(t.spacing.md), t.corner_small());
                                        }
                                    }
                                    None => {
                                        ui.painter().rect_filled(r, t.corner_small(), t.raised);
                                    }
                                }) {
                                    d.favorite_badge = *id;
                                }
                            }
                        });
                    }
                }
            }
        });
        let o = original(ctx).map(|o| o.favorite_badge).unwrap_or_default();
        let (save, cancel) = save_bar(ui, t, o != d.favorite_badge, saving, true);
        if cancel {
            d.favorite_badge = o;
        }
        if save {
            let c = Command::SetFavoriteBadge { badge: d.favorite_badge };
            self.send_saves(ctx, vec![c], vec![]);
        }
    }

    /// Steam's privacy page (the same rows as Settings → Privacy).
    fn edit_privacy(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(d) = self.edit.draft.as_mut() else { return };
        let changed = settings::group(ui, t, t!("Privacy Settings"), |ui| {
            let changed = settings::privacy_rows(ui, t, &mut d.privacy);
            ui.label(w::body_s(t, t!("Changes are saved right away."), t.text_faint));
            changed
        });
        if changed {
            ctx.send(Command::SaveProfilePrivacy(d.privacy));
        }
    }

    pub(super) fn edit_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        let client::Event::ProfileSaved { what, result } = event else { return };
        if *what == ProfileSave::Privacy && self.edit.pending == 0 {
            // Privacy saves right away, without the save bar.
            if let Err(e) = result {
                ctx.toast(t!("Steam didn't save it: {error}", error = i18n::tr_dyn(e)));
                self.edit.draft = None;
            } else {
                ctx.toast(t!("Privacy settings saved"));
            }
            return;
        }
        self.edit.pending = self.edit.pending.saturating_sub(1);
        match result {
            Ok(()) => {
                // What waited for this save goes now.
                let after = std::mem::take(&mut self.edit.after);
                self.edit.pending += after.len();
                for c in after {
                    ctx.send(c);
                }
                if self.edit.pending > 0 {
                    return;
                }
                ctx.toast(match what {
                    ProfileSave::Info => t!("Profile saved"),
                    ProfileSave::Privacy => t!("Privacy settings saved"),
                    ProfileSave::Avatar => t!("Avatar changed"),
                    ProfileSave::Item | ProfileSave::Theme => t!("Profile updated"),
                });
                // Steam's new values, and the profile page drawn from them.
                self.edit.draft = None;
                ctx.send(Command::LoadProfileEdit { language: lang() });
                ctx.send(Command::LoadProfile { steam_id: ctx.state.steam_id });
            }
            Err(e) => {
                self.edit.after.clear();
                ctx.toast(t!("Steam didn't save it: {error}", error = i18n::tr_dyn(e)));
            }
        }
    }
}

/// Steam's values (what Cancel puts back).
fn original(ctx: &Ctx) -> Option<ProfileEdit> {
    ctx.state.profile_edit.as_ref().and_then(|r| r.as_ref().ok()).cloned()
}

/// A community badge's id from its link (`…/badges/2`); game badges
/// (`…/gamecards/730`) have none.
fn community_badge_id(url: &str) -> Option<u32> {
    let rest = url.split("/badges/").nth(1)?;
    rest.trim_end_matches('/').split(['/', '?']).next()?.parse().ok()
}

/// One field as Steam lays them out: its name (and what it's for) above,
/// the box across the whole width under it.
fn field(ui: &mut Ui, t: &Theme, label: &str, note: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = t.spacing.xs + 2.0;
        ui.add(egui::Label::new(w::heading_s(t, label, t.text)).wrap());
        if !note.is_empty() {
            ui.add(egui::Label::new(w::body_m(t, note, t.text_dim)).wrap());
        }
        add(ui);
    });
    ui.add_space(t.spacing.sm);
}

/// A check box with its name (wrapping, never cut) and a note under it.
fn check_line(ui: &mut Ui, t: &Theme, on: &mut bool, label: &str, note: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.md;
        w::checkbox(ui, t, on, t.dense(22.0));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = t.spacing.xs;
            if ui.add(egui::Label::new(w::heading_s(t, label, t.text)).wrap().sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *on = !*on;
            }
            if !note.is_empty() {
                ui.add(egui::Label::new(w::body_m(t, note, t.text_dim)).wrap());
            }
        });
    });
}

/// Cancel and Save, on the right under a section. Returns (save, cancel).
fn save_bar(ui: &mut Ui, t: &Theme, changed: bool, saving: bool, valid: bool) -> (bool, bool) {
    let (mut save, mut cancel) = (false, false);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.md;
        if saving {
            ui.add(egui::Spinner::new().size(t.typography.heading_s.size).color(t.accent));
        } else if ui.add_enabled_ui(changed && valid, |ui| w::button(ui, t, None, t!("Save"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
            save = true;
        }
        if ui.add_enabled_ui(changed && !saving, |ui| w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md)).inner.clicked() {
            cancel = true;
        }
        if changed && !saving {
            ui.label(w::body_m(t, t!("Preview: not saved yet"), t.away));
        }
    });
    (save, cancel)
}

/// A group whose title has "See All" (or "Show Less") on the right; `add`
/// gets whether everything shows.
fn titled_group(ui: &mut Ui, t: &Theme, title: &str, all: &mut bool, more: bool, add: impl FnOnce(&mut Ui, bool)) {
    let open = *all;
    let mut flip = false;
    settings::group(ui, t, "", |ui| {
        ui.horizontal(|ui| {
            ui.label(w::heading_m(t, title, t.text_dim));
            if more {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    flip = ui.link(w::body_m(t, if open { t!("Show Less") } else { t!("See All") }, t.accent)).clicked();
                });
            }
        });
        ui.add_space(t.spacing.sm);
        add(ui, open);
    });
    if flip {
        *all = !*all;
    }
}

/// The filter box over a list of backgrounds.
fn filter_box(ui: &mut Ui, t: &Theme, filter: &mut String) {
    let width = ui.available_width().min(t.dense(360.0));
    w::text_input(ui, t, filter, t!("Filter"), width, 1);
}

/// A small "moving picture" mark on an animated item.
fn animated_mark(ui: &Ui, t: &Theme, r: Rect) {
    let c = pos2(r.right() - t.spacing.md, r.top() + t.spacing.md);
    ui.painter().circle_filled(c, t.typography.body_s.size * 0.8, t.overlay_dark(3));
    ui.painter().text(c, Align2::CENTER_CENTER, icons::PLAY, t.typography.body_s.font(), t.text);
}

/// A dropdown of (code, name), "Do not display" first; true if the choice
/// changed.
fn pick(ui: &mut Ui, t: &Theme, list: &Option<Vec<(String, String)>>, value: &mut String, width: f32) -> bool {
    let mut options = vec![(String::new(), t!("Do not display").to_string())];
    options.extend(list.iter().flatten().cloned());
    if !value.is_empty() && !options.iter().any(|(c, _)| c == value) {
        options.push((value.clone(), value.clone()));
    }
    w::select(ui, t, value, &options, width)
}

/// A tile: a picture, then a name and a second line (its game), outlined
/// when chosen. True when clicked.
fn tile_button(ui: &mut Ui, t: &Theme, size: egui::Vec2, name: &str, sub: &str, on: bool, pic: impl FnOnce(&Ui, Rect)) -> bool {
    let line = t.typography.body_m.size + t.spacing.xs;
    let small = t.typography.body_s.size + t.spacing.xs;
    let (rect, resp) = ui.allocate_exact_size(vec2(size.x, size.y + t.spacing.sm + line + small), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp.clicked();
    }
    let img = Rect::from_min_size(rect.min, size);
    pic(ui, img);
    let hover = resp.hovered();
    let stroke = if on { Stroke::new(t.dense(2.0), t.accent) } else if hover { Stroke::new(1.0, t.text_faint) } else { Stroke::new(1.0, t.surface_hover) };
    ui.painter().rect_stroke(img, t.corner_small(), stroke, egui::StrokeKind::Inside);
    if on {
        let size = t.typography.body_s.size;
        let c = pos2(img.right() - size, img.bottom() - size);
        ui.painter().circle_filled(c, size * 0.75, t.accent);
        ui.painter().text(c, Align2::CENTER_CENTER, icons::CHECK, t.typography.body_s.font(), t.on_accent);
    }
    let text = |s: &str, font: egui::FontId, color, y: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(s.to_string(), font, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(size.x);
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(img.left(), y), g, color);
    };
    text(name, t.typography.body_m.font(), if on { t.accent } else { t.text }, img.bottom() + t.spacing.sm);
    if !sub.is_empty() {
        text(sub, t.typography.body_s.font(), t.text_dim, img.bottom() + t.spacing.sm + line);
    }
    let tip = if sub.is_empty() { name.to_string() } else { format!("{name}\n{sub}") };
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Owned items as tiles with their names (and games), "None" first.
/// `filter` matches names and games; without `all` only the first row
/// shows. The chosen id when one is clicked.
#[allow(clippy::too_many_arguments)]
fn item_grid(
    ui: &mut Ui,
    t: &Theme,
    items: &[ProfileItemChoice],
    chosen: u64,
    filter: &str,
    tile: egui::Vec2,
    with_game: bool,
    all: bool,
    pic: &dyn Fn(&Ui, Rect, &ProfileItemChoice),
    none_pic: &dyn Fn(&Ui, Rect),
) -> Option<u64> {
    if items.is_empty() {
        ui.label(w::body_m(t, t!("You don't own any yet. They're in the Points Shop."), t.text_faint));
        if ui.link(w::body_m(t, t!("Open the Points Shop"), t.accent)).clicked() {
            crate::links::open_http("https://store.steampowered.com/points/shop");
        }
        return None;
    }
    let f = filter.trim().to_lowercase();
    let shown: Vec<&ProfileItemChoice> = items.iter().filter(|i| f.is_empty() || i.name.to_lowercase().contains(&f) || i.app_name.to_lowercase().contains(&f)).collect();
    let gap = t.spacing.md;
    let cols = (((ui.available_width() + gap) / (tile.x + gap)).floor() as usize).max(1);
    let mut cells: Vec<Option<&ProfileItemChoice>> = if f.is_empty() { vec![None] } else { Vec::new() };
    cells.extend(shown.iter().copied().map(Some));
    if !all {
        cells.truncate(cols);
    }
    if cells.is_empty() {
        ui.label(w::body_m(t, t!("Nothing matches the filter."), t.text_faint));
        return None;
    }
    let mut picked = None;
    for row in cells.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for cell in row {
                let clicked = match cell {
                    None => tile_button(ui, t, tile, t!("None"), "", chosen == 0, |ui, r| none_pic(ui, r)),
                    Some(it) => {
                        let game = if with_game { it.app_name.as_str() } else { "" };
                        tile_button(ui, t, tile, &it.name, game, it.id == chosen, |ui, r| pic(ui, r, it))
                    }
                };
                if clicked {
                    picked = Some(cell.map(|i| i.id).unwrap_or(0));
                }
            }
        });
    }
    picked
}

/// An avatar with a frame around it (the frame covers the whole square,
/// the picture sits inside it), or a plain border without one.
fn avatar_with_frame(ui: &Ui, t: &Theme, r: Rect, avatar: &str, frame: Option<&str>) {
    let inner = if frame.is_some() { r.shrink(r.width() * 0.09) } else { r };
    if !super::paint_picture(ui, inner, avatar, CornerRadius::ZERO) {
        w::skeleton(ui, t, inner, CornerRadius::ZERO);
    }
    match frame {
        Some(f) => {
            super::paint_picture(ui, r, f, CornerRadius::ZERO);
        }
        None => {
            ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(t.dense(2.0), t.text_dim), egui::StrokeKind::Inside);
        }
    }
}

/// Who the previews show: you, with the avatar and frame being chosen.
struct Preview {
    name: String,
    status: String,
    avatar: String,
    frame: Option<String>,
    level: Option<u32>,
    badge: Option<model::Badge>,
}

impl Preview {
    fn of(ctx: &Ctx, d: Option<&ProfileEdit>) -> Self {
        let me = ctx.state.persona(ctx.state.steam_id);
        let name = if ctx.state.persona_name.is_empty() { me.name.clone() } else { ctx.state.persona_name.clone() };
        let still = art::avatar_full(me.avatar_hash.as_deref());
        let avatar = d.and_then(|d| d.animated_avatars.iter().find(|i| i.id == d.equipped[3])).map(|i| i.image.clone()).unwrap_or(still);
        let frame = d.and_then(|d| d.frames.iter().find(|i| i.id == d.equipped[2])).map(|i| i.image.clone());
        let badges = ctx.state.badges.get(&ctx.state.steam_id).and_then(|b| b.as_ref().ok());
        let featured = d.map(|d| d.favorite_badge).unwrap_or(0);
        let badge = badges.and_then(|b| b.items.iter().find(|x| featured != 0 && community_badge_id(&x.url) == Some(featured)).cloned());
        Self { name, status: crate::shell::status_text(ctx.state, &me), avatar, frame, level: badges.map(|b| b.level), badge }
    }
}

/// Your profile in miniature in front of a background (and in a theme's
/// colors): what Steam's edit pages show above the lists.
fn profile_preview(ui: &mut Ui, t: &Theme, r: Rect, bg: Option<&ProfileItemChoice>, full: bool, colors: Option<[egui::Color32; 2]>, p: &Preview) {
    let painter = ui.painter_at(r);
    painter.rect_filled(r, t.corner(), t.chrome);
    let still = bg.map(|b| if b.large.is_empty() { b.image.as_str() } else { b.large.as_str() });
    let video = bg.map(|b| b.movie.as_str()).filter(|m| !m.is_empty());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
    child.set_clip_rect(r);
    if !super::paint_profile_background(&child, t, r, still, video, full) {
        w::paint_background(&mut child, t, r, "profile");
    }
    // The profile's panel, in the theme's colors when previewing one.
    let panel = Rect::from_center_size(pos2(r.center().x, r.center().y + r.height() * 0.06), vec2(r.width() * 0.66, r.height() * 0.82));
    match colors {
        Some([a, b]) => {
            let mut mesh = egui::Mesh::default();
            let (ta, tb) = (a.gamma_multiply(0.92), b.gamma_multiply(0.92));
            mesh.colored_vertex(panel.left_top(), ta);
            mesh.colored_vertex(panel.right_top(), ta);
            mesh.colored_vertex(panel.right_bottom(), tb);
            mesh.colored_vertex(panel.left_bottom(), tb);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
        }
        None => {
            painter.rect_filled(panel, CornerRadius::ZERO, t.overlay_dark(2));
        }
    }
    let pad = panel.width() * 0.04;
    let av = Rect::from_min_size(panel.min + vec2(pad, pad), egui::Vec2::splat(panel.height() * 0.3));
    avatar_with_frame(ui, t, av, &p.avatar, p.frame.as_deref());
    let x = av.right() + pad;
    painter.text(pos2(x, av.top()), Align2::LEFT_TOP, &p.name, t.typography.heading_m.font(), t.text);
    painter.text(pos2(x, av.top() + t.typography.heading_m.size + t.spacing.sm), Align2::LEFT_TOP, &p.status, t.typography.body_m.font(), t.accent);
    if let Some(level) = p.level {
        let c = pos2(panel.right() - pad - t.dense(16.0), av.top() + t.dense(16.0));
        painter.circle_stroke(c, t.dense(15.0), Stroke::new(t.dense(2.0), t.text_dim));
        painter.text(c, Align2::CENTER_CENTER, level.to_string(), t.typography.body_m.font(), t.text);
    }
    // Stand-ins for the profile's sections.
    let top = av.bottom() + pad;
    let main = Rect::from_min_max(pos2(panel.left() + pad, top), pos2(panel.left() + panel.width() * 0.68, panel.bottom() - pad));
    let side = Rect::from_min_max(pos2(main.right() + pad, top), pos2(panel.right() - pad, panel.bottom() - pad));
    for (i, h) in [0.42f32, 0.3].iter().enumerate() {
        let y = main.top() + main.height() * if i == 0 { 0.0 } else { 0.5 };
        painter.rect_filled(Rect::from_min_size(pos2(main.left(), y), vec2(main.width(), main.height() * h)), t.corner_small(), t.overlay_dark(1));
    }
    let row = side.height() / 6.0;
    for i in 0..5 {
        let y = side.top() + row * i as f32;
        painter.rect_filled(Rect::from_min_size(pos2(side.left(), y), vec2(side.width(), row * 0.7)), t.corner_small(), t.overlay_dark(1));
    }
}

/// The mini profile card, as Steam shows it on hover: the background, your
/// avatar, name and status, then your level and featured badge.
fn mini_preview(ui: &mut Ui, t: &Theme, r: Rect, bg: Option<&ProfileItemChoice>, p: &Preview) {
    let painter = ui.painter_at(r);
    painter.rect_filled(r, t.corner(), t.raised);
    let still = bg.map(|b| if b.large.is_empty() { b.image.as_str() } else { b.large.as_str() });
    let video = bg.map(|b| b.movie.as_str()).filter(|m| !m.is_empty());
    let top = Rect::from_min_size(r.min, vec2(r.width(), r.height() * 0.58));
    let child = ui.new_child(egui::UiBuilder::new().max_rect(top));
    let drawn = video.is_some_and(|v| super::paint_video(&child, top, v, CornerRadius::ZERO)) || still.is_some_and(|s| w::paint_cover(&child, top, s, CornerRadius::ZERO));
    if drawn {
        painter.rect_filled(top, CornerRadius::ZERO, t.overlay_dark(0));
    }
    let pad = t.spacing.md;
    let av = Rect::from_min_size(pos2(r.left() + pad, top.bottom() - t.dense(64.0) - pad), egui::Vec2::splat(t.dense(64.0)));
    avatar_with_frame(ui, t, av, &p.avatar, p.frame.as_deref());
    let x = av.right() + pad;
    painter.text(pos2(x, av.center().y - t.spacing.xs), Align2::LEFT_BOTTOM, &p.name, t.typography.heading_s.font(), t.text);
    painter.text(pos2(x, av.center().y + t.spacing.xs), Align2::LEFT_TOP, &p.status, t.typography.body_m.font(), t.accent);
    // Level and featured badge.
    let y = top.bottom() + (r.bottom() - top.bottom()) / 2.0;
    if let Some(level) = p.level {
        let c = pos2(r.left() + pad + t.dense(15.0), y);
        painter.circle_stroke(c, t.dense(15.0), Stroke::new(t.dense(2.0), t.text_dim));
        painter.text(c, Align2::CENTER_CENTER, level.to_string(), t.typography.body_m.font(), t.text);
        painter.text(pos2(c.x + t.dense(24.0), y), Align2::LEFT_CENTER, t!("Steam Level"), t.typography.body_m.font(), t.text_dim);
    }
    if let Some(b) = &p.badge {
        let size = t.dense(40.0);
        let br = Rect::from_center_size(pos2(r.right() - pad - size / 2.0, y), egui::Vec2::splat(size));
        if let Some(u) = &b.icon {
            w::paint_contain(ui, br, u, Align2::CENTER_CENTER);
        }
        let mut job = egui::text::LayoutJob::simple_singleline(b.title.clone(), t.typography.body_s.font(), t.text_dim);
        job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() * 0.35);
        let g = ui.painter().layout_job(job);
        let at = pos2(br.left() - t.spacing.sm - g.size().x, y - g.size().y / 2.0);
        ui.painter().galley(at, g, t.text_dim);
    }
}

/// A round swatch of a profile theme with its name; true when clicked.
fn swatch(ui: &mut Ui, t: &Theme, name: &str, colors: [egui::Color32; 2], on: bool) -> bool {
    let d = t.dense(56.0);
    let label_h = t.typography.body_m.size + t.spacing.sm;
    let (rect, resp) = ui.allocate_exact_size(vec2(d + t.spacing.xl, d + t.spacing.sm + label_h), Sense::click());
    let c = pos2(rect.center().x, rect.top() + d / 2.0);
    let p = ui.painter();
    p.circle_filled(c, d / 2.0, colors[1]);
    p.circle_filled(c - vec2(0.0, d * 0.12), d * 0.34, colors[0]);
    let ring = if on { Stroke::new(t.dense(3.0), t.accent) } else if resp.hovered() { Stroke::new(t.dense(2.0), t.text_dim) } else { Stroke::new(1.0, t.surface_hover) };
    p.circle_stroke(c, d / 2.0 + t.dense(3.0), ring);
    p.text(pos2(c.x, rect.bottom()), Align2::CENTER_BOTTOM, name, t.typography.body_m.font(), if on { t.accent } else { t.text });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

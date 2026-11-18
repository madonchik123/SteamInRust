//! Edit your profile in the app, like Steam's Edit Profile pages: name,
//! real name, custom URL, location and summary; avatar (upload, animated
//! avatars, frames); profile and mini profile backgrounds; theme; and
//! privacy settings.

use super::Profile;
use crate::builtin::settings;
use client::Command;
use egui::{Align, Align2, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
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
    Privacy,
}

impl Section {
    pub fn from_arg(s: &str) -> Section {
        match s {
            "avatar" => Section::Avatar,
            "background" => Section::Background,
            "miniprofile" => Section::MiniProfile,
            "theme" => Section::Theme,
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
    saving: bool,
    /// Location lists asked for.
    asked_locations: std::collections::HashSet<(Option<String>, Option<String>)>,
}

fn lang() -> String {
    i18n::language().steam_name().to_string()
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
                    (Section::General, t!("About")),
                    (Section::Avatar, t!("Avatar")),
                    (Section::Background, t!("Profile Background")),
                    (Section::MiniProfile, t!("Mini Profile")),
                    (Section::Theme, t!("Theme")),
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
                    Section::Background => self.edit_items(ui, t, ctx, 0),
                    Section::MiniProfile => self.edit_items(ui, t, ctx, 1),
                    Section::Theme => self.edit_theme(ui, t, ctx),
                    Section::Privacy => self.edit_privacy(ui, t, ctx),
                }
            },
        );
        if back {
            ctx.navigate("profile", None);
        }
        if let Some(s) = pick {
            self.edit.section = s;
        }
        if shop {
            crate::links::open_http("https://store.steampowered.com/points/shop");
        }
    }

    fn edit_general(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let saving = self.edit.saving;
        let Some(d) = self.edit.draft.as_mut() else { return };
        let field_w = |ui: &Ui| (ui.available_width() * 0.32).clamp(180.0, 297.0);
        settings::group(ui, t, t!("General"), |ui| {
            settings::row(ui, t, &format!("{} *", t!("Profile name")), t!("The name everyone sees on your profile and in chat."), |ui| {
                let width = field_w(ui);
                w::text_input(ui, t, &mut d.persona_name, "", width, 1);
            }, |_| {});
            settings::row(ui, t, t!("Real name"), t!("Providing your real name can help friends find you on the Steam Community."), |ui| {
                let width = field_w(ui);
                w::text_input(ui, t, &mut d.real_name, t!("Your real name here…"), width, 1);
            }, |_| {});
            let url = d.custom_url.trim().to_string();
            settings::row(ui, t, t!("Username"), t!("Your custom profile address on the Steam Community."), |ui| {
                let width = field_w(ui);
                w::text_input(ui, t, &mut d.custom_url, "", width, 1);
            }, |ui| {
                if !url.is_empty() {
                    ui.label(w::body_l(t, t!("Available at"), t.text_dim));
                    let link = format!("https://steamcommunity.com/id/{url}/");
                    if ui.link(w::body_l(t, &link, t.accent)).clicked() {
                        crate::links::open_http(&link);
                    }
                }
            });
            ui.horizontal_top(|ui| {
                let lw = (ui.available_width() * 0.28).clamp(150.0, 265.0);
                ui.allocate_ui_with_layout(vec2(lw, t.dense(44.0)), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_width(lw);
                    ui.spacing_mut().item_spacing.x = t.spacing.xs + 1.0;
                    ui.label(w::heading_s(t, t!("Bio"), t.text));
                    w::info_tip(ui, t, t!("A few words about you, shown under your name."));
                });
                let width = ui.available_width();
                w::text_input(ui, t, &mut d.summary, "", width, 3);
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
            settings::row(ui, t, t!("Country"), t!("Shown on your profile, if you choose one."), |ui| {
                let width = field_w(ui);
                changed_country = pick(ui, t, &countries, &mut d.country, width);
            }, |_| {});
            if changed_country {
                d.state.clear();
                d.city.clear();
            }
            if states.as_ref().is_some_and(|s| !s.is_empty()) {
                let mut changed = false;
                settings::row(ui, t, t!("State or region"), "", |ui| {
                    let width = field_w(ui);
                    changed = pick(ui, t, &states, &mut d.state, width);
                }, |_| {});
                if changed {
                    d.city.clear();
                }
            }
            if cities.as_ref().is_some_and(|c| !c.is_empty()) {
                settings::row(ui, t, t!("City"), "", |ui| {
                    let width = field_w(ui);
                    pick(ui, t, &cities, &mut d.city, width);
                }, |_| {});
            }
        });
        settings::group(ui, t, t!("Awards"), |ui| {
            settings::row(ui, t, t!("Hide Community Awards"), t!("Hide the awards you've been given on your profile"), |ui| {
                w::toggle(ui, t, &mut d.hide_awards);
            }, |_| {});
        });
        let changed = ctx.state.profile_edit.as_ref().and_then(|r| r.as_ref().ok()).is_some_and(|orig| {
            (&orig.persona_name, &orig.real_name, &orig.custom_url, &orig.country, &orig.state, &orig.city, &orig.summary, orig.hide_awards)
                != (&d.persona_name, &d.real_name, &d.custom_url, &d.country, &d.state, &d.city, &d.summary, d.hide_awards)
        });
        let (mut save, mut cancel) = (false, false);
        let can_save = changed && !d.persona_name.trim().is_empty();
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = t.spacing.md;
            if saving {
                ui.add(egui::Spinner::new().size(t.typography.heading_s.size).color(t.accent));
            } else if ui.add_enabled_ui(can_save, |ui| w::button(ui, t, None, t!("Save"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                save = true;
            }
            if ui.add_enabled_ui(changed && !saving, |ui| w::button(ui, t, None, t!("Cancel"), w::Kind::Secondary, w::Size::Md)).inner.clicked() {
                cancel = true;
            }
        });
        if save {
            ctx.send(Command::SaveProfileInfo(Box::new(d.clone())));
            self.edit.saving = true;
        }
        if cancel {
            self.edit.draft = ctx.state.profile_edit.as_ref().and_then(|r| r.as_ref().ok()).cloned();
        }
    }

    fn edit_avatar(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.persona(ctx.state.steam_id);
        let saving = self.edit.saving;
        let mut upload = None;
        settings::group(ui, t, t!("Avatar"), |ui| {
            ui.label(w::body_m(t, t!("Choose a picture for your profile. It's shown at 184×184."), t.text_dim));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.md;
                let (r, _) = ui.allocate_exact_size(vec2(t.dense(96.0), t.dense(96.0)), Sense::hover());
                w::image_or_skeleton(ui, t, r, &art::avatar_full(me.avatar_hash.as_deref()), t.corner_small());
                ui.vertical(|ui| {
                    if saving {
                        ui.add(egui::Spinner::new().size(t.typography.heading_s.size).color(t.accent));
                    } else if w::button(ui, t, Some(icons::IMAGE), t!("Upload your avatar"), w::Kind::Primary, w::Size::Md).clicked() {
                        upload = rfd::FileDialog::new().add_filter(t!("Images").to_string(), &["jpg", "jpeg", "png"]).pick_file();
                    }
                    ui.label(w::body_s(t, t!("JPEG or PNG. Steam crops it to a square."), t.text_faint));
                });
            });
        });
        if let Some(path) = upload {
            self.edit.saving = true;
            ctx.send(Command::UploadAvatar { path });
        }
        let Some(d) = self.edit.draft.clone() else { return };
        settings::group(ui, t, t!("Animated avatar"), |ui| {
            ui.label(w::body_m(t, t!("Animated avatars you own (from the Points Shop)."), t.text_dim));
            item_grid(ui, t, ctx, &d.animated_avatars, d.equipped[3], 3, t.dense(72.0), t.dense(72.0));
        });
        settings::group(ui, t, t!("Avatar frame"), |ui| {
            ui.label(w::body_m(t, t!("Frames you own go around your avatar everywhere on Steam."), t.text_dim));
            item_grid(ui, t, ctx, &d.frames, d.equipped[2], 2, t.dense(72.0), t.dense(72.0));
        });
    }

    fn edit_items(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, kind: u8) {
        let Some(d) = self.edit.draft.clone() else { return };
        let (title, body, list) = if kind == 0 {
            (t!("Profile background"), t!("Backgrounds you own, shown behind your profile."), &d.backgrounds)
        } else {
            (t!("Mini profile"), t!("The background people see when they hover over your name."), &d.mini_backgrounds)
        };
        settings::group(ui, t, title, |ui| {
            ui.label(w::body_m(t, body, t.text_dim));
            item_grid(ui, t, ctx, list, d.equipped[kind as usize], kind, t.dense(192.0), t.dense(108.0));
        });
    }

    fn edit_theme(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(d) = self.edit.draft.as_mut() else { return };
        let mut pick = None;
        settings::group(ui, t, t!("Theme"), |ui| {
            ui.label(w::body_m(t, t!("The colors of your profile page on Steam."), t.text_dim));
            if d.themes.is_empty() {
                ui.label(w::body_m(t, t!("No themes available."), t.text_faint));
                return;
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(t.spacing.sm, t.spacing.sm);
                for (id, title) in &d.themes {
                    if w::chip(ui, t, title, *id == d.theme).clicked() && *id != d.theme {
                        pick = Some(id.clone());
                    }
                }
            });
        });
        if let Some(id) = pick {
            d.theme = id.clone();
            ctx.send(Command::SetProfileTheme(id));
        }
    }

    /// The same privacy rows as Settings → Privacy (Figma `477:7327`).
    fn edit_privacy(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(d) = self.edit.draft.as_mut() else { return };
        let changed = settings::group(ui, t, t!("Profile Privacy"), |ui| {
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
        self.edit.saving = false;
        match result {
            Ok(()) => {
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
                ctx.toast(t!("Steam didn't save it: {error}", error = i18n::tr_dyn(e)));
                if *what == ProfileSave::Privacy {
                    self.edit.draft = None;
                }
            }
        }
    }
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

/// Owned items to pick from, with "None" first; clicking equips at once.
#[allow(clippy::too_many_arguments)]
fn item_grid(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, items: &[ProfileItemChoice], equipped: u64, kind: u8, w_: f32, h: f32) {
    if items.is_empty() {
        ui.label(w::body_m(t, t!("You don't own any yet. They're in the Points Shop."), t.text_faint));
        if ui.link(w::body_m(t, t!("Open the Points Shop"), t.accent)).clicked() {
            crate::links::open_http("https://store.steampowered.com/points/shop");
        }
        return;
    }
    let mut choose = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.spacing.sm + 2.0, t.spacing.sm + 2.0);
        // "None".
        let (r, resp) = ui.allocate_exact_size(vec2(w_, h), Sense::click());
        ui.painter().rect_filled(r, t.corner_small(), t.raised);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, t!("None"), t.typography.body_m.font(), t.text_dim);
        outline(ui, t, r, equipped == 0, resp.hovered());
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && equipped != 0 {
            choose = Some(0);
        }
        for it in items {
            let (r, resp) = ui.allocate_exact_size(vec2(w_, h), Sense::click());
            ui.painter().rect_filled(r, t.corner_small(), t.raised);
            if !w::paint_contain(ui, r.shrink(4.0), &it.image, Align2::CENTER_CENTER) {
                w::skeleton(ui, t, r.shrink(4.0), t.corner_small());
            }
            outline(ui, t, r, it.id == equipped, resp.hovered());
            if resp.on_hover_text(&it.name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && it.id != equipped {
                choose = Some(it.id);
            }
        }
    });
    if let Some(id) = choose {
        ctx.send(Command::EquipProfileItem { kind, id });
    }
}

fn outline(ui: &Ui, t: &Theme, r: Rect, on: bool, hover: bool) {
    let stroke = if on { Stroke::new(2.0, t.accent) } else if hover { Stroke::new(1.0, t.text_faint) } else { Stroke::new(1.0, t.surface_hover) };
    ui.painter().rect_stroke(r, t.corner_small(), stroke, egui::StrokeKind::Inside);
    if on {
        let size = t.typography.body_s.size;
        let c = pos2(r.right() - size, r.top() + size);
        ui.painter().circle_filled(c, size * 0.75, t.accent);
        ui.painter().text(c, Align2::CENTER_CENTER, icons::CHECK, t.typography.body_s.font(), t.on_accent);
    }
}

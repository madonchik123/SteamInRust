//! Edit your profile in the app, like Steam's Edit Profile pages: name,
//! real name, custom URL, location and summary; avatar (upload, animated
//! avatars, frames); profile and mini profile backgrounds; theme; and
//! privacy settings.

use super::{Profile, panel};
use client::Command;
use egui::{Align, Align2, CornerRadius, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::Ctx;
use i18n::t;
use model::{ProfileEdit, ProfileItemChoice, ProfileSave, art};
use ui::widgets::EditMenu;
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
    pub(super) fn edit_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let me = ctx.state.steam_id;
        if !self.edit.asked {
            self.edit.asked = true;
            ctx.send(Command::LoadProfileEdit { language: lang() });
        }
        if self.edit.draft.is_none() {
            if let Some(Ok(e)) = &ctx.state.profile_edit {
                self.edit.draft = Some(e.clone());
            }
        }
        ui.horizontal(|ui| {
            if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Back to profile"), w::Kind::Ghost, w::Size::Sm).clicked() {
                ctx.navigate("profile", None);
            }
            ui.add_space(6.0);
            ui.label(w::text_bold(t!("Edit profile"), 22.0, t.text));
        });
        ui.add_space(12.0);
        if let (None, Some(Err(e))) = (&self.edit.draft, &ctx.state.profile_edit) {
            panel(ui, t, |ui| w::empty(ui, t, icons::WARNING, t!("Couldn't load your profile settings"), &i18n::tr_dyn(e)));
            return;
        }
        let side = 220.0;
        ui.horizontal_top(|ui| {
            // Sections, like Steam's left column.
            ui.allocate_ui_with_layout(vec2(side, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(side);
                panel(ui, t, |ui| {
                    let me_p = ctx.state.persona(me);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                        w::image_or_skeleton(ui, t, r, &art::avatar_full(me_p.avatar_hash.as_deref()), CornerRadius::same(4));
                        ui.label(w::text_semibold(&ctx.state.persona_name, 14.0, t.text));
                    });
                    ui.add_space(10.0);
                    for (s, icon, label) in [
                        (Section::General, icons::USER_CIRCLE, t!("General")),
                        (Section::Avatar, icons::IMAGE, t!("Avatar")),
                        (Section::Background, icons::IMAGE, t!("Profile background")),
                        (Section::MiniProfile, icons::IMAGE, t!("Mini profile")),
                        (Section::Theme, icons::PALETTE, t!("Theme")),
                        (Section::Privacy, icons::EYE_SLASH, t!("Privacy settings")),
                    ] {
                        let on = self.edit.section == s;
                        if w::menu_item_tinted(ui, t, icon, label, on.then_some(t.accent)).clicked() {
                            self.edit.section = s;
                        }
                    }
                });
            });
            ui.add_space(16.0);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(ui.available_width());
                panel(ui, t, |ui| {
                    let Some(_) = &self.edit.draft else {
                        for _ in 0..5 {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
                            w::skeleton(ui, t, r.shrink2(vec2(0.0, 5.0)), t.corner_small());
                        }
                        return;
                    };
                    match self.edit.section {
                        Section::General => self.edit_general(ui, t, ctx),
                        Section::Avatar => self.edit_avatar(ui, t, ctx),
                        Section::Background => self.edit_items(ui, t, ctx, 0),
                        Section::MiniProfile => self.edit_items(ui, t, ctx, 1),
                        Section::Theme => self.edit_theme(ui, t, ctx),
                        Section::Privacy => self.edit_privacy(ui, t, ctx),
                    }
                });
            });
        });
    }

    fn edit_general(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let saving = self.edit.saving;
        let Some(d) = self.edit.draft.as_mut() else { return };
        section_title(ui, t, &t!("About"), &t!("Set your profile name and details. Providing additional information like your real name can help friends find you on the Steam Community."));
        let field = |ui: &mut Ui, label: &str, value: &mut String, hint: &str| {
            ui.label(w::text_semibold(label, 13.0, t.text_dim));
            ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(ui.available_width().min(520.0)).margin(vec2(10.0, 8.0))).with_edit_menu();
            ui.add_space(10.0);
        };
        field(ui, &t!("Profile name"), &mut d.persona_name, "");
        field(ui, &t!("Real name"), &mut d.real_name, &t!("Optional"));
        ui.label(w::text_semibold(t!("Custom URL"), 13.0, t.text_dim));
        ui.horizontal(|ui| {
            ui.label(w::text("steamcommunity.com/id/", 13.0, t.text_faint));
            ui.add(egui::TextEdit::singleline(&mut d.custom_url).desired_width(240.0).margin(vec2(10.0, 8.0))).with_edit_menu();
        });
        ui.add_space(14.0);

        // Location: country, then its state, then the state's city.
        section_title(ui, t, &t!("Location"), "");
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
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
            if pick(ui, t, "loc-country", &t!("Country"), &countries, &mut d.country) {
                d.state.clear();
                d.city.clear();
            }
            if states.as_ref().is_some_and(|s| !s.is_empty()) && pick(ui, t, "loc-state", &t!("State or region"), &states, &mut d.state) {
                d.city.clear();
            }
            if cities.as_ref().is_some_and(|c| !c.is_empty()) {
                pick(ui, t, "loc-city", &t!("City"), &cities, &mut d.city);
            }
        });
        ui.add_space(14.0);
        section_title(ui, t, &t!("Summary"), "");
        ui.add(egui::TextEdit::multiline(&mut d.summary).desired_rows(5).desired_width(ui.available_width()).margin(vec2(10.0, 8.0))).with_edit_menu();
        ui.add_space(8.0);
        w::check(ui, t, &mut d.hide_awards, w::text(t!("Hide the awards you've been given on your profile"), 13.5, t.text));
        ui.add_space(14.0);
        let changed = ctx.state.profile_edit.as_ref().and_then(|r| r.as_ref().ok()).is_some_and(|orig| {
            (&orig.persona_name, &orig.real_name, &orig.custom_url, &orig.country, &orig.state, &orig.city, &orig.summary, orig.hide_awards)
                != (&d.persona_name, &d.real_name, &d.custom_url, &d.country, &d.state, &d.city, &d.summary, d.hide_awards)
        });
        let (mut save, mut cancel) = (false, false);
        let can_save = changed && !d.persona_name.trim().is_empty();
        ui.horizontal(|ui| {
            if saving {
                ui.add(egui::Spinner::new().size(18.0));
            } else if ui.add_enabled_ui(can_save, |ui| w::button(ui, t, Some(icons::CHECK), t!("Save"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                save = true;
            }
            if changed && !saving && w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
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
        section_title(ui, t, &t!("Avatar"), &t!("Choose a picture for your profile. It's shown at 184×184."));
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
            w::image_or_skeleton(ui, t, r, &art::avatar_full(me.avatar_hash.as_deref()), CornerRadius::same(6));
            ui.add_space(12.0);
            ui.vertical(|ui| {
                if self.edit.saving {
                    ui.add(egui::Spinner::new().size(18.0));
                } else if w::button(ui, t, Some(icons::IMAGE), t!("Upload your avatar"), w::Kind::Primary, w::Size::Md).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter(t!("Images").to_string(), &["jpg", "jpeg", "png"]).pick_file() {
                        self.edit.saving = true;
                        ctx.send(Command::UploadAvatar { path });
                    }
                }
                ui.label(w::text(t!("JPEG or PNG. Steam crops it to a square."), 12.0, t.text_faint));
            });
        });
        let Some(d) = self.edit.draft.clone() else { return };
        ui.add_space(16.0);
        section_title(ui, t, &t!("Animated avatar"), &t!("Animated avatars you own (from the Points Shop)."));
        item_grid(ui, t, ctx, &d.animated_avatars, d.equipped[3], 3, 72.0, 72.0);
        ui.add_space(16.0);
        section_title(ui, t, &t!("Avatar frame"), &t!("Frames you own go around your avatar everywhere on Steam."));
        item_grid(ui, t, ctx, &d.frames, d.equipped[2], 2, 72.0, 72.0);
    }

    fn edit_items(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, kind: u8) {
        let Some(d) = self.edit.draft.clone() else { return };
        let (title, body, list, size) = if kind == 0 {
            (t!("Profile background"), t!("Backgrounds you own, shown behind your profile."), &d.backgrounds, (192.0, 108.0))
        } else {
            (t!("Mini profile"), t!("The background people see when they hover over your name."), &d.mini_backgrounds, (192.0, 108.0))
        };
        section_title(ui, t, &title, &body);
        item_grid(ui, t, ctx, list, d.equipped[kind as usize], kind, size.0, size.1);
    }

    fn edit_theme(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(d) = self.edit.draft.as_mut() else { return };
        section_title(ui, t, &t!("Theme"), &t!("The colors of your profile page on Steam."));
        if d.themes.is_empty() {
            ui.label(w::text(t!("No themes available."), 13.0, t.text_faint));
            return;
        }
        let mut pick = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for (id, title) in &d.themes {
                if w::chip(ui, t, title, *id == d.theme).clicked() && *id != d.theme {
                    pick = Some(id.clone());
                }
            }
        });
        if let Some(id) = pick {
            d.theme = id.clone();
            ctx.send(Command::SetProfileTheme(id));
        }
    }

    fn edit_privacy(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(d) = self.edit.draft.as_mut() else { return };
        section_title(ui, t, &t!("Privacy settings"), &t!("Choose who can see your profile and its parts. Changes are saved right away."));
        let before = d.privacy;
        let p = &mut d.privacy;
        let opts = [(3u8, t!("Public")), (2u8, t!("Friends only")), (1u8, t!("Private"))];
        let row = |ui: &mut Ui, label: &str, help: &str, value: &mut u8, max: u8| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 340.0).max(200.0));
                    ui.label(w::text_semibold(label, 14.0, t.text));
                    if !help.is_empty() {
                        ui.add(egui::Label::new(w::text(help, 12.0, t.text_faint)).wrap());
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Nothing can be more public than the profile itself.
                    let list: Vec<(u8, &str)> = opts.iter().filter(|(v, _)| *v <= max).map(|(v, l)| (*v, &**l)).collect();
                    *value = (*value).min(max);
                    w::segmented(ui, t, value, &list);
                });
            });
            ui.add_space(10.0);
        };
        row(ui, &t!("My profile"), &t!("Your level, badges, friends, groups and summary."), &mut p.profile, 3);
        let top = p.profile;
        row(ui, &t!("Game details"), &t!("The games you own, your playtime and achievements."), &mut p.games, top);
        let mut hide_time = p.playtime == 1;
        if p.games > 1 && w::check(ui, t, &mut hide_time, w::text(t!("Always keep my total playtime private"), 13.5, t.text)).changed() {
            p.playtime = if hide_time { 1 } else { p.games };
        }
        if !hide_time {
            p.playtime = p.games;
        }
        ui.add_space(6.0);
        row(ui, &t!("Friends list"), &t!("Who can see your friends."), &mut p.friends, top);
        row(ui, &t!("Inventory"), &t!("Your items, trading cards and gifts."), &mut p.inventory, top);
        let mut hide_gifts = p.gifts == 1;
        if p.inventory > 1 && w::check(ui, t, &mut hide_gifts, w::text(t!("Always keep Steam Gifts private"), 13.5, t.text)).changed() {
            p.gifts = if hide_gifts { 1 } else { p.inventory };
        }
        if !hide_gifts {
            p.gifts = p.inventory;
        }
        ui.add_space(6.0);
        // Comments: 1 everyone, 0 friends, 2 nobody.
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 340.0).max(200.0));
                ui.label(w::text_semibold(t!("Can post comments on my profile"), 14.0, t.text));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                w::segmented(ui, t, &mut p.comments, &[(1, &t!("Public")), (0, &t!("Friends only")), (2, &t!("Private"))]);
            });
        });
        if d.privacy != before {
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
                self.requested.remove(&ctx.state.steam_id);
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

fn section_title(ui: &mut Ui, t: &Theme, title: &str, body: &str) {
    ui.label(w::text_bold(title, 17.0, t.text));
    if !body.is_empty() {
        ui.add(egui::Label::new(w::text(body, 12.5, t.text_dim)).wrap());
    }
    ui.add_space(10.0);
}

/// A dropdown of (code, name); true if the choice changed.
fn pick(ui: &mut Ui, t: &Theme, id: &str, label: &str, list: &Option<Vec<(String, String)>>, value: &mut String) -> bool {
    let shown = list.as_ref().and_then(|l| l.iter().find(|(c, _)| c == value)).map(|(_, n)| n.clone()).unwrap_or_else(|| if value.is_empty() { t!("Not set").to_string() } else { value.clone() });
    let mut changed = false;
    ui.vertical(|ui| {
        ui.label(w::text_semibold(label, 12.5, t.text_dim));
        egui::ComboBox::from_id_salt(id).selected_text(shown).width(220.0).height(360.0).show_ui(ui, |ui| {
            if ui.selectable_label(value.is_empty(), t!("Not set")).clicked() && !value.is_empty() {
                value.clear();
                changed = true;
            }
            for (code, name) in list.iter().flatten() {
                if ui.selectable_label(code == value, name).clicked() && code != value {
                    *value = code.clone();
                    changed = true;
                }
            }
        });
    });
    changed
}

/// Owned items to pick from, with "None" first; clicking equips at once.
#[allow(clippy::too_many_arguments)]
fn item_grid(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, items: &[ProfileItemChoice], equipped: u64, kind: u8, w_: f32, h: f32) {
    if items.is_empty() {
        ui.label(w::text(t!("You don't own any yet. They're in the Points Shop."), 13.0, t.text_faint));
        if ui.link(w::text(t!("Open the Points Shop"), 12.5, t.accent)).clicked() {
            let _ = open::that_detached("https://store.steampowered.com/points/shop");
        }
        return;
    }
    let mut choose = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
        // "None".
        let (r, resp) = ui.allocate_exact_size(vec2(w_, h), Sense::click());
        ui.painter().rect_filled(r, t.corner_small(), t.raised);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, t!("None"), w::font_medium(13.0), t.text_dim);
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
    let stroke = if on { Stroke::new(2.0, t.accent) } else if hover { Stroke::new(1.0, t.text_faint) } else { Stroke::new(1.0, t.border) };
    ui.painter().rect_stroke(r, t.corner_small(), stroke, egui::StrokeKind::Inside);
    if on {
        let c = pos2(r.right() - 12.0, r.top() + 12.0);
        ui.painter().circle_filled(c, 9.0, t.accent);
        ui.painter().text(c, Align2::CENTER_CENTER, icons::CHECK, w::font(11.0), egui::Color32::WHITE);
    }
}

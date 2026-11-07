//! A game's achievements: yours, and optionally a friend's side by side,
//! like Steam's "Compare achievements".

use super::{HubView, Io, View};
use client::Command;
use client::hub::Request;
use egui::{Align, Align2, CornerRadius, Layout, Rect, Sense, Ui, vec2};
use i18n::t;
use model::{Achievement, AppId, SteamId, art};
use std::collections::HashMap;
use ui::{Theme, icons, widgets as w};

pub(super) fn show(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId, friend: Option<SteamId>) {
    let lang = io.lang();
    let have = io.state.achievement_list.contains_key(&app);
    if !have && hv.asked.insert(format!("ach{app}")) {
        io.send(Command::LoadAchievements { app_id: app, language: lang.clone() });
    }
    if let Some(f) = friend {
        let have = io.state.hub.achievements.contains_key(&(f, app));
        hv.ask(io, format!("ach{app}/{f}"), have, || Request::Achievements { steam_id: f, app_id: app, language: lang.clone() });
    }
    let name = io.state.app_name(app).unwrap_or_default().to_string();
    ui.add_space(14.0);
    // Header: the game and the page.
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(138.0, 64.0), Sense::click());
        w::image_or_title(ui, t, r, &art::header(app), &name, CornerRadius::same(4));
        if resp.on_hover_text(t!("Community hub")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            io.go(View::Hub { app, tab: super::HubTab::All });
        }
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label(w::text_bold(&name, if io.overlay { 18.0 } else { 24.0 }, t.text));
            ui.label(w::text(t!("Achievements"), 13.5, t.text_dim));
        });
    });
    ui.add_space(12.0);
    let mine = match io.state.achievement_list.get(&app) {
        None => return w::loading(ui, t, &t!("Loading achievements…")),
        Some(Err(e)) => {
            if super::error(ui, t, &t!("Couldn't load achievements: {error}", error = e)) {
                io.send(Command::LoadAchievements { app_id: app, language: lang });
            }
            return;
        }
        Some(Ok(l)) if l.is_empty() => return w::empty(ui, t, icons::TROPHY, t!("No achievements"), t!("This game has no achievements.")),
        Some(Ok(l)) => l.clone(),
    };
    let theirs: Option<Result<HashMap<String, Achievement>, String>> = friend.and_then(|f| io.state.hub.achievements.get(&(f, app))).map(|r| r.as_ref().map(|l| l.iter().map(|a| (key(a), a.clone())).collect()).map_err(|e| e.clone()));

    // Summary bars.
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let done = mine.iter().filter(|a| a.unlocked).count();
        summary(ui, t, &art::avatar(io.state.me().and_then(|m| m.avatar_hash.as_deref())), &t!("You"), done, mine.len());
        if let Some(f) = friend {
            ui.add_space(10.0);
            let p = io.state.persona(f);
            match &theirs {
                None => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(t.accent));
                        ui.label(w::text(t!("Loading {name}'s achievements…", name = p.name), 13.0, t.text_dim));
                    });
                }
                Some(Err(e)) if e == "private" => {
                    ui.label(w::text(t!("{name}'s game details are private.", name = p.name), 13.0, t.text_dim));
                }
                Some(Err(e)) if e == "none" => {
                    ui.label(w::text(t!("{name} doesn't have this game, or keeps game details private.", name = p.name), 13.0, t.text_dim));
                }
                Some(Err(e)) => {
                    ui.label(w::text(t!("Couldn't load {name}'s achievements: {error}", name = p.name, error = e), 13.0, t.danger));
                }
                Some(Ok(map)) => {
                    let done = map.values().filter(|a| a.unlocked).count();
                    summary(ui, t, &art::avatar(p.avatar_hash.as_deref()), &p.name, done, mine.len());
                }
            }
        }
    });
    ui.add_space(10.0);

    // Compare picker, filter and order.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        ui.label(w::text(t!("Compare with"), 13.0, t.text_dim));
        let current = friend.map(|f| io.state.persona(f).name).unwrap_or_else(|| t!("Nobody").to_string());
        let mut pick: Option<Option<SteamId>> = None;
        egui::ComboBox::from_id_salt(("ach-compare", app)).selected_text(current).width(200.0).show_ui(ui, |ui| {
            if ui.selectable_label(friend.is_none(), t!("Nobody")).clicked() {
                pick = Some(None);
            }
            for p in io.state.friend_list() {
                if ui.selectable_label(friend == Some(p.steam_id), &p.name).clicked() {
                    pick = Some(Some(p.steam_id));
                }
            }
        });
        if let Some(f) = pick {
            if io.overlay {
                // Stay in this window: swap the view in place.
                if let Some(last) = hv.stack.last_mut() {
                    *last = View::Achievements { app, friend: f };
                }
            } else {
                io.go(View::Achievements { app, friend: f });
            }
        }
        ui.add_space(12.0);
        w::segmented(ui, t, &mut hv.ach_filter, &[(0, &t!("All")), (1, &t!("Unlocked")), (2, &t!("Locked"))]);
        ui.add_space(6.0);
        w::segmented(ui, t, &mut hv.ach_sort, &[(0, &t!("Default order")), (1, &t!("Recently unlocked")), (2, &t!("Rarest first"))]);
    });
    ui.add_space(10.0);

    let mut list: Vec<&Achievement> = mine
        .iter()
        .filter(|a| match hv.ach_filter {
            1 => a.unlocked,
            2 => !a.unlocked,
            _ => true,
        })
        .collect();
    match hv.ach_sort {
        1 => list.sort_by(|a, b| b.unlocked.cmp(&a.unlocked).then(b.unlock_time.cmp(&a.unlock_time))),
        2 => list.sort_by(|a, b| a.global_percent.total_cmp(&b.global_percent)),
        _ => {}
    }
    if list.is_empty() {
        ui.label(w::text(t!("Nothing to show with this filter."), 13.0, t.text_faint));
    }
    let friend_name = friend.map(|f| io.state.persona(f).name);
    for a in list {
        let other = match &theirs {
            Some(Ok(map)) => map.get(&key(a)).cloned(),
            _ => None,
        };
        row(ui, t, a, other.as_ref(), friend_name.as_deref());
        ui.add_space(4.0);
    }
}

/// Achievements are matched by their picture (names are translated alike).
fn key(a: &Achievement) -> String {
    format!("{}|{}", a.icon, a.name)
}

fn summary(ui: &mut Ui, t: &Theme, avatar: &str, who: &str, done: usize, all: usize) {
    ui.horizontal(|ui| {
        w::avatar(ui, t, avatar, 34.0, None);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            let pct = if all == 0 { 0 } else { done * 100 / all };
            ui.horizontal(|ui| {
                ui.label(w::text_semibold(who, 13.5, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(w::text(t!("{done} of {all} ({pct}%)", done = done, all = all, pct = pct), 12.5, t.text_dim));
                });
            });
            w::progress(ui, t, if all == 0 { 0.0 } else { done as f32 / all as f32 }, ui.available_width());
        });
    });
}

fn row(ui: &mut Ui, t: &Theme, a: &Achievement, other: Option<&Achievement>, friend: Option<&str>) {
    let width = ui.available_width();
    let compare = friend.is_some();
    let (r, resp) = ui.allocate_exact_size(vec2(width, 72.0), Sense::hover());
    ui.painter().rect_filled(r, t.corner_small(), if resp.hovered() { t.surface_hover } else { t.surface });
    let icon_rect = Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(56.0, 56.0));
    let icon = if a.unlocked || a.icon_gray.is_empty() { &a.icon } else { &a.icon_gray };
    w::image_or_skeleton(ui, t, icon_rect, icon, CornerRadius::same(4));
    let secret = a.hidden && !a.unlocked && !other.is_some_and(|o| o.unlocked);
    let name = if secret { t!("Hidden achievement").to_string() } else { a.name.clone() };
    let desc = if secret { t!("Details are revealed once unlocked.").to_string() } else { a.description.clone() };
    let right_w = if compare { 330.0 } else { 190.0 };
    let text_w = (width - 80.0 - right_w).max(60.0);
    let x = icon_rect.right() + 12.0;
    let clip = ui.painter().with_clip_rect(Rect::from_min_size(egui::pos2(x, r.top()), vec2(text_w, r.height())));
    clip.text(egui::pos2(x, r.top() + 20.0), Align2::LEFT_CENTER, &name, w::font_semibold(14.0), if a.unlocked { t.text } else { t.text_dim });
    let g = ui.painter().layout(desc, w::font(12.0), t.text_faint, text_w);
    clip.galley(egui::pos2(x, r.top() + 31.0), g, t.text_faint);
    // Global rarity bar.
    let bar = Rect::from_min_size(egui::pos2(x, r.bottom() - 12.0), vec2(text_w.min(220.0), 4.0));
    ui.painter().rect_filled(bar, CornerRadius::same(2), t.raised);
    ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * (a.global_percent / 100.0).clamp(0.0, 1.0), 4.0)), CornerRadius::same(2), t.accent.gamma_multiply(0.7));
    ui.painter().text(egui::pos2(bar.right() + 8.0, bar.center().y), Align2::LEFT_CENTER, t!("{pct}% of players", pct = format!("{:.1}", a.global_percent)), w::font(11.0), t.text_faint);
    // Your column (and the friend's).
    let status = |unlocked: bool, time: u64| -> (String, egui::Color32) {
        if unlocked {
            (if time > 0 { t!("Unlocked {date}", date = ui::date(time)) } else { t!("Unlocked").to_string() }, t.in_game)
        } else {
            (t!("Locked").to_string(), t.text_faint)
        }
    };
    let (mine, c) = status(a.unlocked, a.unlock_time);
    let col = if compare { r.right() - 170.0 } else { r.right() - 12.0 };
    let align = if compare { Align2::RIGHT_CENTER } else { Align2::RIGHT_CENTER };
    ui.painter().text(egui::pos2(col, r.center().y - 8.0), align, if a.unlocked { icons::CHECK } else { icons::LOCK_SIMPLE }, w::font(15.0), c);
    ui.painter().text(egui::pos2(col, r.center().y + 10.0), align, mine, w::font(11.5), c);
    if compare {
        let (theirs, c) = match other {
            Some(o) => status(o.unlocked, o.unlock_time),
            None => ("—".to_string(), t.text_faint),
        };
        let x = r.right() - 12.0;
        ui.painter().text(egui::pos2(x, r.center().y - 8.0), Align2::RIGHT_CENTER, if other.is_some_and(|o| o.unlocked) { icons::CHECK } else { icons::LOCK_SIMPLE }, w::font(15.0), c);
        ui.painter().text(egui::pos2(x, r.center().y + 10.0), Align2::RIGHT_CENTER, theirs, w::font(11.5), c);
        ui.painter().vline(r.right() - 160.0, (r.top() + 10.0)..=(r.bottom() - 10.0), egui::Stroke::new(1.0, t.border));
    }
}

//! Dynamic collections: made from the library's filters or edited here, and
//! saved in Steam's own format, so the official client shows the same thing.

use super::shortcuts;
use super::{AdvFilters, Entry, Filter, Library, Tab, entries};
use client::Command;
use client::collections::{self as steam, DynamicFilter, FilterGroup, GameFacts};
use egui::{Align, Layout, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::AppId;
use std::collections::{BTreeSet, HashSet};
use ui::widgets::EditMenu;
use ui::{Theme, icons, widgets as w};

/// Steam's play-state options this editor offers.
const PLAY: &[(u64, &str)] = &[(2, "Ready to play"), (1, "Installed"), (3, "Played"), (4, "Never played"), (5, "Runs on this computer")];

/// Steam's feature options this editor offers.
const FEATURES: &[(u64, &str)] = &[
    (7, "Single-player"),
    (8, "Multi-player"),
    (9, "Co-op"),
    (27, "Shared/split screen"),
    (1, "Full controller support"),
    (2, "Partial controller support"),
    (3, "VR supported"),
    (26, "Doesn't need VR"),
    (6, "Steam Achievements"),
    (10, "Steam Cloud"),
    (4, "Steam Trading Cards"),
    (5, "Steam Workshop"),
    (11, "Remote Play Together"),
    (12, "Steam Deck Verified"),
    (13, "Steam Deck Playable"),
];

/// The genres in Steam's filter (they're saved as store tags).
const GENRES: &[(u64, &str)] = &[
    (19, "Action"),
    (21, "Adventure"),
    (597, "Casual"),
    (492, "Indie"),
    (128, "Massively Multiplayer"),
    (699, "Racing"),
    (122, "RPG"),
    (599, "Simulation"),
    (701, "Sports"),
    (9, "Strategy"),
];

/// Game languages, by Steam's language id.
const LANGUAGES: &[(u64, &str)] = &[
    (0, "English"),
    (1, "German"),
    (2, "French"),
    (3, "Italian"),
    (4, "Korean"),
    (5, "Spanish - Spain"),
    (27, "Spanish - Latin America"),
    (6, "Simplified Chinese"),
    (7, "Traditional Chinese"),
    (8, "Russian"),
    (9, "Thai"),
    (10, "Japanese"),
    (11, "Portuguese - Portugal"),
    (22, "Portuguese - Brazil"),
    (12, "Polish"),
    (13, "Danish"),
    (14, "Dutch"),
    (15, "Finnish"),
    (16, "Norwegian"),
    (17, "Swedish"),
    (18, "Hungarian"),
    (19, "Czech"),
    (20, "Romanian"),
    (21, "Turkish"),
    (23, "Bulgarian"),
    (24, "Greek"),
    (25, "Arabic"),
    (26, "Ukrainian"),
    (28, "Vietnamese"),
    (30, "Indonesian"),
    (31, "Malay"),
];

/// The library's genre ids and the store tag Steam's filter uses for each.
const GENRE_TAGS: &[(u32, u64)] = &[(1, 19), (25, 21), (4, 597), (23, 492), (29, 128), (9, 699), (3, 122), (28, 599), (18, 701), (2, 9)];

/// The library's feature categories and Steam's closest feature option.
const FEATURE_OPTIONS: &[(u32, u64)] = &[(2, 7), (1, 8), (9, 9), (38, 9), (24, 27), (28, 1), (22, 6), (23, 10), (29, 4), (30, 5), (44, 11)];

/// Options Steam won't let you pick together.
const EXCLUSIVE: &[(u64, u64)] = &[(3, 4), (3, 26)];

/// A dynamic collection from Steam.
#[derive(Clone)]
pub(super) struct DynCollection {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) filter: DynamicFilter,
    /// The filters exactly as Steam stores them.
    pub(super) raw: String,
    /// Games put in or taken out by hand, on top of the filters.
    pub(super) added: Vec<AppId>,
    pub(super) removed: Vec<AppId>,
}

impl DynCollection {
    pub(super) fn from_steam(c: &model::SteamCollection) -> Option<Self> {
        let raw = c.filter.clone()?;
        let filter = DynamicFilter::parse(&raw)?;
        Some(Self { id: c.id.clone(), name: c.name.clone(), filter, raw, added: c.added.clone(), removed: c.removed.clone() })
    }
}

/// The open editor.
#[derive(Default)]
pub(super) struct DynEditor {
    /// The collection being edited; none for a new one.
    id: Option<String>,
    name: String,
    filter: DynamicFilter,
    /// The filters it started from, exactly as Steam stores them.
    base: Option<String>,
    /// Filters this app can't write back exactly: only the name can change.
    locked: bool,
    /// Library filters Steam has no equivalent for.
    left_out: Vec<String>,
    /// Made from the library's filters: the games they show right now, for
    /// saving a plain list instead.
    games: Option<Vec<AppId>>,
}

/// The library's filters in Steam's terms, and the ones Steam can't express.
pub(super) fn filter_from_library(adv: &AdvFilters, search: &str, installed_only: bool) -> (DynamicFilter, Vec<String>) {
    let mut f = DynamicFilter { search: search.trim().to_string(), ..Default::default() };
    let mut left: Vec<String> = Vec::new();

    let mut play: Vec<u64> = Vec::new();
    if installed_only || adv.play.contains(&0) {
        play.push(2);
    }
    if adv.play.contains(&2) {
        play.push(3);
    }
    if adv.play.contains(&3) {
        play.push(4);
    }
    if adv.play.contains(&1) {
        left.push(t!("Not installed").to_string());
    }
    let here_only = adv.systems.len() == 1 && adv.systems.contains(model::THIS_OS);
    if !adv.systems.is_empty() && !here_only {
        left.push(t!("Runs on").to_string());
    }
    if here_only && play.len() > 1 {
        // "Any of these states, and runs here" can't share one Steam group.
        left.push(t!("Runs on").to_string());
    } else if here_only {
        play.push(5);
    }
    let any = play.len() > 1 && !play.contains(&5);
    f.groups[steam::PLAY_STATE] = FilterGroup { options: play, any };

    if !adv.kinds.is_empty() {
        left.push(t!("Type").to_string());
    }

    let mut tags = Vec::new();
    for g in &adv.genres {
        match GENRE_TAGS.iter().find(|(id, _)| id == g) {
            Some((_, tag)) => tags.push(*tag),
            None => left.push(super::GENRES.iter().find(|(id, _)| id == g).map(|(_, n)| i18n::tr_dyn(n)).unwrap_or_else(|| g.to_string())),
        }
    }
    f.groups[steam::TAGS] = FilterGroup { options: tags, any: true };

    let mut features: Vec<u64> = Vec::new();
    for c in &adv.features {
        match FEATURE_OPTIONS.iter().find(|(id, _)| id == c) {
            Some((_, o)) if !features.contains(o) => features.push(*o),
            Some(_) => {}
            None => left.push(super::FEATURES.iter().find(|(id, _)| id == c).map(|(_, n)| i18n::tr_dyn(n)).unwrap_or_else(|| c.to_string())),
        }
    }
    if adv.deck.contains(&2) {
        features.push(13); // Playable or better.
    } else if adv.deck.contains(&3) {
        features.push(12);
    }
    if adv.deck.contains(&0) || adv.deck.contains(&1) {
        left.push(t!("Steam Deck").to_string());
    }
    f.groups[steam::FEATURES].options = features;
    (f, left)
}

/// One filter group: its title, any/all, and a check box per option.
fn section(ui: &mut Ui, t: &Theme, title: &str, group: &mut FilterGroup, items: &[(u64, String)]) {
    ui.horizontal(|ui| {
        ui.label(w::body_l(t, title, t.text));
        if items.len() > 1 {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                w::segmented(ui, t, &mut group.any, &[(true, t!("Any")), (false, t!("All"))]);
            });
        }
    });
    ui.add_space(t.spacing.xs);
    ui.spacing_mut().item_spacing.y = t.spacing.xs;
    for (option, label) in items {
        let mut on = group.options.contains(option);
        if w::check(ui, t, &mut on, w::body_s(t, label, t.text)).changed() {
            if on {
                group.options.retain(|o| !EXCLUSIVE.iter().any(|(a, b)| (a == option && b == o) || (b == option && a == o)));
                group.options.push(*option);
            } else {
                group.options.retain(|o| o != option);
            }
        }
    }
    let extra = group.options.iter().filter(|o| !items.iter().any(|(v, _)| v == *o)).count();
    if extra > 0 {
        ui.label(w::body_s(t, t!("+{n} more set in Steam", n = extra), t.text_dim));
    }
    ui.add_space(t.spacing.lg);
}

fn labels(list: &[(u64, &str)]) -> Vec<(u64, String)> {
    list.iter().map(|(o, l)| (*o, i18n::tr_dyn(l))).collect()
}

impl Library {
    /// What the filters look at for one game.
    pub(super) fn facts<'b>(&'b self, e: &Entry<'b>) -> GameFacts<'b> {
        let o = e.owned;
        GameFacts {
            app_id: e.app_id,
            name: e.name,
            installed: e.installed.is_some(),
            played: e.installed.and_then(|g| g.last_played).is_some_and(|t| t > 0) || self.played_ids.contains(&e.app_id),
            runs_here: o.is_none_or(client::compat::playable_here),
            categories: o.map(|o| o.categories.as_slice()).unwrap_or_default(),
            tags: o.map(|o| o.tags.as_slice()).unwrap_or_default(),
            deck: o.map(|o| o.deck).unwrap_or(0),
            machine: o.map(|o| o.machine).unwrap_or(0),
            frame: o.map(|o| o.frame).unwrap_or(0),
            languages: o.map(|o| o.languages).unwrap_or(0),
            master_sub: o.map(|o| o.master_sub).unwrap_or(0),
            friends: Some(&self.friend_games),
            owner: 0,
        }
    }

    /// Bring the play history and the games of friends named in filters up
    /// to date, asking Steam for any friend not loaded yet. An empty list is
    /// what a private profile gives, so it counts as "can't tell".
    pub(super) fn refresh_dynamic_data(&mut self, ctx: &mut Ctx) {
        let state = ctx.state;
        self.played_ids = match state.player_games.get(&state.steam_id) {
            Some(Ok(list)) => list.iter().filter(|g| g.last_played.is_some_and(|t| t > 0) || g.playtime_minutes > 0).map(|g| g.app_id).collect(),
            _ => HashSet::new(),
        };
        let wanted: BTreeSet<u32> = self
            .dynamic
            .iter()
            .map(|d| &d.filter)
            .chain(self.dyn_editor.iter().map(|e| &e.filter))
            .flat_map(|f| f.groups[steam::FRIENDS].options.iter().map(|o| *o as u32))
            .collect();
        self.friend_games.clear();
        for account in wanted {
            let id = model::chat_admin::INDIVIDUAL + account as u64;
            match state.player_games.get(&id) {
                Some(Ok(list)) if !list.is_empty() => drop(self.friend_games.insert(account, list.iter().map(|g| g.app_id).collect())),
                Some(_) => {}
                None => {
                    if self.asked_friends.insert(id) {
                        ctx.send(Command::LoadPlayerGames { steam_id: id, language: i18n::language().steam_name().into() });
                    }
                }
            }
        }
    }

    /// Open the editor on a new collection made from the library's filters.
    pub(super) fn new_dynamic_from_filters(&mut self, games: Vec<AppId>) {
        let (filter, left_out) = filter_from_library(&self.adv, &self.search, self.filter == Filter::Installed);
        self.dyn_editor = Some(DynEditor { filter, left_out, games: Some(games), ..Default::default() });
    }

    /// Open the editor on one of Steam's dynamic collections.
    pub(super) fn edit_dynamic(&mut self, id: &str) {
        if let Some(d) = self.dynamic.iter().find(|d| d.id == id) {
            self.dyn_editor = Some(DynEditor {
                id: Some(d.id.clone()),
                name: d.name.clone(),
                filter: d.filter.clone(),
                base: Some(d.raw.clone()),
                locked: !DynamicFilter::is_exact(&d.raw),
                ..Default::default()
            });
        }
    }

    /// Open what a "collection:" link asked for, once it's known.
    pub(super) fn open_linked_dynamic(&mut self) {
        let Some(id) = self.open_dynamic.clone() else { return };
        if id == "new" {
            self.open_dynamic = None;
        } else if self.dynamic.iter().any(|d| d.id == id) {
            self.edit_dynamic(&id);
            self.open_dynamic = None;
        } else if self.steam_loaded {
            self.open_dynamic = None;
        }
    }

    pub(super) fn delete_dynamic(&mut self, id: &str, ctx: &mut Ctx) {
        self.dynamic.retain(|d| d.id != id);
        ctx.send(Command::SaveSteamCollections(vec![model::CollectionEdit { id: id.to_string(), deleted: true, ..Default::default() }]));
    }

    fn save_dynamic(&mut self, mut ed: DynEditor, ctx: &mut Ctx) {
        let name = ed.name.trim().to_string();
        // Filters left alone keep Steam's exact text.
        let raw = match &ed.base {
            Some(b) if ed.locked || DynamicFilter::parse(b).as_ref() == Some(&ed.filter) => b.clone(),
            _ => {
                // Steam removes this one when it loads a filter.
                ed.filter.groups[steam::FEATURES].options.retain(|o| *o != 15);
                ed.filter.to_json()
            }
        };
        let is_new = ed.id.is_none();
        let id = ed.id.take().unwrap_or_else(steam::new_id);
        ctx.send(Command::SaveSteamCollections(vec![model::CollectionEdit {
            id: id.clone(),
            name: name.clone(),
            filter: Some(raw.clone()),
            based_on: ed.base.clone(),
            ..Default::default()
        }]));
        let filter = DynamicFilter::parse(&raw).unwrap_or(ed.filter);
        match self.dynamic.iter_mut().find(|d| d.id == id) {
            Some(d) => {
                d.name = name;
                d.filter = filter;
                d.raw = raw;
            }
            None => self.dynamic.push(DynCollection { id, name, filter, raw, added: Vec::new(), removed: Vec::new() }),
        }
        if is_new {
            self.tab = Tab::Collections;
            self.selected = None;
        }
        self.refresh_dynamic_data(ctx);
    }

    /// The editor: a name, Steam's filter groups and how many games match.
    pub(super) fn dynamic_dialog(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx) {
        let Some(mut ed) = self.dyn_editor.take() else { return };
        let (state, local) = (ctx.state, ctx.local);
        let before = ed.filter.groups[steam::FRIENDS].options.clone();
        let (added, removed) = ed.id.as_ref().and_then(|id| self.dynamic.iter().find(|d| d.id == *id)).map(|d| (d.added.clone(), d.removed.clone())).unwrap_or_default();
        let mut all = entries(state, local);
        all.extend(self.custom.iter().map(|g| Entry { app_id: g.id, name: shortcuts::intern(&g.name), installed: None, owned: None }));
        let count = all.iter().filter(|e| ed.filter.contains(&self.facts(e), &added, &removed)).count();

        let mut friends: Vec<(u64, String)> = state
            .friends
            .iter()
            .filter(|(_, r)| **r == model::Relationship::Friend)
            .map(|(id, _)| (model::account_id(*id) as u64, state.personas.get(id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string())))
            .collect();
        friends.sort_by_key(|(_, n)| n.to_lowercase());
        let waiting = ed.filter.groups[steam::FRIENDS].options.iter().any(|o| !state.player_games.contains_key(&(model::chat_admin::INDIVIDUAL + *o)));
        let unseen = ed.filter.groups[steam::FRIENDS].options.iter().any(|o| match state.player_games.get(&(model::chat_admin::INDIVIDUAL + *o)) {
            Some(Ok(l)) => l.is_empty(),
            Some(Err(_)) => true,
            None => false,
        });

        let screen = ui.ctx().content_rect();
        let width = (screen.width() - 80.0).clamp(360.0, 860.0);
        let list_h = (screen.height() - 330.0).max(160.0);
        let mut done = None;
        egui::Modal::new(egui::Id::unique("dynamic-collection")).show(ui.ctx(), |ui| {
            ui.set_width(width);
            ui.label(w::heading_s(t, if ed.id.is_some() { t!("Edit dynamic collection") } else { t!("New dynamic collection") }, t.text));
            ui.add_space(t.spacing.xs);
            ui.label(w::body_s(t, t!("Steam keeps its games up to date from these filters, here and in the official client."), t.text_dim));
            ui.add_space(t.spacing.md);
            let r = ui.add(egui::TextEdit::singleline(&mut ed.name).hint_text(t!("Name, e.g. Co-op with friends")).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
            if ed.name.is_empty() && !r.has_focus() && ed.id.is_none() && ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.add_space(t.spacing.sm);
            if ed.locked {
                ui.label(w::body_s(t, t!("These filters use something this app can't save back exactly, so they're kept as they are. You can rename the collection here and change its filters in Steam."), t.text_dim));
            } else {
                ui.add(egui::TextEdit::singleline(&mut ed.filter.search).hint_text(t!("Name contains…")).desired_width(f32::INFINITY).margin(vec2(10.0, 8.0))).with_edit_menu();
                ui.add_space(t.spacing.md);
                egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
                    let cols = if width >= 720.0 { 3 } else { 2 };
                    ui.columns(cols, |c| {
                        section(&mut c[0], t, t!("Play state"), &mut ed.filter.groups[steam::PLAY_STATE], &labels(PLAY));
                        section(&mut c[0], t, t!("Subscriptions"), &mut ed.filter.groups[steam::SUBSCRIPTIONS], &[(steam::EA_PLAY, "EA Play".to_string())]);
                        section(&mut c[0], t, t!("Friends who own it"), &mut ed.filter.groups[steam::FRIENDS], &friends);
                        if waiting {
                            c[0].label(w::body_s(t, t!("Loading your friends' games…"), t.text_dim));
                        } else if unseen {
                            c[0].label(w::body_s(t, t!("Some of these friends keep their games private, so they aren't checked."), t.text_dim));
                        }
                        section(&mut c[1], t, t!("Features"), &mut ed.filter.groups[steam::FEATURES], &labels(FEATURES));
                        let last = cols - 1;
                        section(&mut c[last], t, t!("Genre"), &mut ed.filter.groups[steam::TAGS], &labels(GENRES));
                        section(&mut c[last], t, t!("Languages"), &mut ed.filter.groups[steam::LANGUAGES], &labels(LANGUAGES));
                    });
                });
            }
            if !ed.left_out.is_empty() {
                ui.add_space(t.spacing.sm);
                ui.label(w::body_s(t, t!("Steam has no filter for these, so they were left out: {list}", list = ed.left_out.join(", ")), t.away));
            }
            ui.add_space(t.spacing.md);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.md;
                let usable = !ed.name.trim().is_empty() && (ed.locked || !ed.filter.is_empty()) && self.steam_loaded;
                if ui.add_enabled_ui(usable, |ui| w::button(ui, t, Some(icons::CHECK), t!("Save"), w::Kind::Primary, w::Size::Md)).inner.clicked() {
                    done = Some(1);
                }
                if ed.games.is_some() && ui.add_enabled_ui(!ed.name.trim().is_empty(), |ui| w::button(ui, t, None, t!("Save as a fixed list"), w::Kind::Secondary, w::Size::Md)).inner.on_hover_text(t!("Keep just the games these filters show now")).clicked() {
                    done = Some(2);
                }
                if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                    done = Some(0);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let note = if !self.steam_loaded {
                        t!("Connect to Steam to save this collection").to_string()
                    } else if !ed.locked && ed.filter.is_empty() {
                        t!("Pick at least one filter").to_string()
                    } else {
                        t!("{n} games match", n = count)
                    };
                    ui.label(w::body_m(t, note, t.text_dim));
                });
            });
        });
        if done.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = Some(0);
        }
        match done {
            Some(1) => self.save_dynamic(ed, ctx),
            Some(2) => {
                let games = ed.games.take().unwrap_or_default();
                self.collections.push(super::Collection { name: ed.name.trim().to_string(), games: games.into_iter().collect(), steam_id: None });
                self.tab = Tab::Collections;
                self.selected = None;
            }
            Some(_) => {}
            None => {
                let changed = ed.filter.groups[steam::FRIENDS].options != before;
                self.dyn_editor = Some(ed);
                if changed {
                    self.refresh_dynamic_data(ctx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_filters_become_steams() {
        let adv = AdvFilters { play: [0].into(), genres: [1, 23].into(), features: [2, 28].into(), deck: [3].into(), ..Default::default() };
        let (f, left) = filter_from_library(&adv, " portal ", false);
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(f.search, "portal");
        assert_eq!(f.groups[steam::PLAY_STATE].options, vec![2]);
        assert_eq!(f.groups[steam::TAGS], FilterGroup { options: vec![19, 492], any: true });
        assert_eq!(f.groups[steam::FEATURES], FilterGroup { options: vec![7, 1, 12], any: false });
        assert!(DynamicFilter::is_exact(&f.to_json()));
    }

    #[test]
    fn says_what_steam_cant_filter_by() {
        let adv = AdvFilters { play: [1].into(), kinds: ["game".to_string()].into(), genres: [37].into(), features: [41].into(), deck: [1].into(), ..Default::default() };
        let (f, left) = filter_from_library(&adv, "", false);
        assert!(f.is_empty());
        assert_eq!(left.len(), 5, "{left:?}");
    }

    #[test]
    fn several_play_states_mean_any_of_them() {
        let adv = AdvFilters { play: [2, 3].into(), ..Default::default() };
        let (f, _) = filter_from_library(&adv, "", false);
        assert_eq!(f.groups[steam::PLAY_STATE], FilterGroup { options: vec![3, 4], any: true });
    }

    #[test]
    fn this_system_joins_a_single_play_state() {
        let adv = AdvFilters { play: [0].into(), systems: [model::THIS_OS].into(), ..Default::default() };
        let (f, left) = filter_from_library(&adv, "", false);
        assert!(left.is_empty());
        assert_eq!(f.groups[steam::PLAY_STATE], FilterGroup { options: vec![2, 5], any: false });
    }

    #[test]
    fn the_installed_view_counts_as_ready_to_play() {
        let (f, _) = filter_from_library(&AdvFilters::default(), "", true);
        assert_eq!(f.groups[steam::PLAY_STATE].options, vec![2]);
    }
}

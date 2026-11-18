//! Profiles: yours and your friends' (Figma "profile" `435:7035`). The
//! header (avatar, name, summary, level) and the side column (status,
//! badges, links, friends) are the profile's frame ([`frame`]); the
//! profile itself, its games (`455:9708`), friends, reviews, groups,
//! badges and screenshots (`457:12553`, drawn by the community hub) fill
//! the main column.

use client::Command;
use egui::{Align, Align2, CornerRadius, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use extensions::{Action, Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use client::hub::FileKind;
use model::{Persona, ProfileInfo, Relationship, SteamId, Visibility, art};
use std::collections::HashSet;
use std::sync::Mutex;
use ui::{Theme, icons, widgets as w};

#[path = "profile_lists.rs"]
mod lists;
#[path = "profile_edit.rs"]
mod edit;
#[path = "profile_badges.rs"]
mod badges;
#[path = "profile_files.rs"]
pub(crate) mod files;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum GameSort {
    #[default]
    Recent,
    Playtime,
    Name,
}

/// Games per page on a profile's games list.
const GAMES_PER_PAGE: usize = 10;

#[derive(Default)]
pub struct Profile {
    /// `None` = your own profile.
    viewing: Option<SteamId>,
    games_requested: HashSet<SteamId>,
    /// A list shown instead of the profile (games, friends, reviews, groups).
    sub: Sub,
    games_search: String,
    games_sort: GameSort,
    games_page: usize,
    showcase_requested: HashSet<SteamId>,
    comments_requested: HashSet<SteamId>,
    comment_draft: String,
    /// The badge whose own page is open (its link), on the badges page.
    badge_open: Option<String>,
    badge_asked: HashSet<String>,
    /// Crafting a badge: asking first, waiting for Steam, and which page.
    craft_confirm: Option<model::AppId>,
    crafting: bool,
    craft_url: Option<String>,
    /// Friends, reviews and groups asked for, per profile.
    lists_requested: HashSet<(SteamId, &'static str)>,
    /// The search box on a profile's friends list.
    list_search: String,
    /// Which page of friends or groups shows.
    list_page: usize,
    /// Which page of screenshots (or other files) shows, from 1.
    files_page: u32,
    /// Pages of files asked for.
    files_asked: HashSet<String>,
    /// Which page of comments shows (first comment's index), per profile.
    comment_start: std::collections::HashMap<SteamId, u32>,
    /// Editing your profile (the "Edit profile" pages).
    edit: edit::EditState,
}

impl Extension for Profile {
    fn manifest(&self) -> Manifest {
        Manifest { id: "profile", name: "Profile", author: "SteamInRust", description: "Profiles", core: true, ..Manifest::DEFAULT }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![PageSpec::new("profile", "Profile", icons::USER_CIRCLE).main().order(40)]
    }

    fn on_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        self.edit_event(event, ctx);
        self.badge_event(event, ctx);
        if let client::Event::CommentPosted { result, .. } = event {
            match result {
                Ok(()) => {
                    self.comment_draft.clear();
                    ctx.toast(t!("Comment posted"));
                }
                Err(e) => ctx.toast(t!("Couldn't post the comment: {error}", error = i18n::tr_dyn(e))),
            }
        }
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        // "edit" or "edit/<section>": your profile's settings.
        if let Some(rest) = arg.and_then(|a| a.strip_prefix("edit")) {
            self.viewing = None;
            self.sub = Sub::Edit;
            self.edit.section = edit::Section::from_arg(rest.trim_start_matches('/'));
            return;
        }
        let mut parts = arg.unwrap_or_default().split('/');
        self.viewing = parts.next().and_then(|a| a.parse().ok());
        self.sub = match parts.next() {
            Some("games") => Sub::Games,
            Some("friends") => Sub::Friends,
            Some("reviews") => Sub::Reviews,
            Some("groups") => Sub::Groups,
            Some("badges") => Sub::Badges,
            Some("screenshots") => Sub::Files(FileKind::Screenshots),
            Some("artwork") => Sub::Files(FileKind::Art),
            Some("videos") => Sub::Files(FileKind::Videos),
            Some("workshop") | Some("collections") => Sub::Files(FileKind::Items),
            Some("guides") => Sub::Files(FileKind::Guides),
            _ => Sub::None,
        };
        // Just "badges": your own.
        if arg == Some("badges") {
            self.sub = Sub::Badges;
        }
        self.badge_open = None;
        // "<id>/badges/<app>": that game's badge.
        if let (Some(id), Some(app)) = (self.viewing, parts.next().and_then(|a| a.parse::<u32>().ok())) {
            self.badge_open = Some(format!("https://steamcommunity.com/profiles/{id}/gamecards/{app}/"));
        }
        self.games_search.clear();
        self.games_page = 0;
        self.list_search.clear();
        self.list_page = 0;
        self.files_page = 1;
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        if self.sub == Sub::Edit {
            return self.edit_page(ui, &t, ctx);
        }
        let id = self.viewing.filter(|id| *id != ctx.state.steam_id).unwrap_or(ctx.state.steam_id);
        let data = FrameData::gather(ctx.state, ctx.local, id);
        for c in &data.wants {
            ctx.send(c.clone());
        }
        if id != 0 && data.open && self.games_requested.insert(id) {
            ctx.send(Command::LoadPlayerGames { steam_id: id, language: i18n::language().steam_name().into() });
        }
        let here = match self.sub {
            Sub::Games => Here::Games,
            Sub::Friends => Here::Friends,
            Sub::Reviews => Here::Reviews,
            Sub::Groups => Here::Groups,
            Sub::Badges => Here::Badges,
            Sub::Files(k) => here_of(k),
            Sub::None | Sub::Edit => Here::Profile,
        };
        // The links swap only this column; comments stay under it.
        let go = frame(ui, &t, &data, here, false, |ui| {
            match self.sub {
                Sub::None | Sub::Edit => return self.overview(ui, &t, ctx, &data),
                Sub::Games => self.games_page(ui, &t, ctx, &data),
                Sub::Friends => self.friends_page(ui, &t, ctx, id, data.mine),
                Sub::Reviews => self.reviews_page(ui, &t, ctx, id, data.mine),
                Sub::Groups => self.groups_page(ui, &t, ctx, id, data.mine),
                Sub::Badges => self.badges_page(ui, &t, ctx, id, &data.name),
                Sub::Files(k) => self.files_page(ui, &t, ctx, id, k),
            }
            self.comments(ui, &t, ctx, id, data.mine);
        });
        for g in go {
            match g {
                Go::Nav(page, arg) => ctx.navigate(page, arg),
                Go::Act(a) => ctx.act(a),
                Go::Game(app) => ctx.act(crate::shell::game_page(ctx.state, app)),
                Go::Send(c) => ctx.send(c),
            }
        }
    }
}

/// The list a profile page shows instead of the profile itself.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum Sub {
    #[default]
    None,
    Games,
    Friends,
    Reviews,
    Groups,
    /// Your profile's settings.
    Edit,
    /// Every badge, and a badge's own page.
    Badges,
    /// Screenshots, artwork, videos, Workshop items or guides.
    Files(FileKind),
}

/// Which link a kind of files is.
pub(crate) fn here_of(kind: FileKind) -> Here {
    match kind {
        FileKind::Screenshots => Here::Screenshots,
        FileKind::Art => Here::Artwork,
        FileKind::Videos => Here::Videos,
        FileKind::Guides => Here::Guides,
        FileKind::Items | FileKind::Collections => Here::Workshop,
    }
}

// ---------- the profile's frame (also used by the screenshots pages) ----------

/// Which of a profile's pages is showing (its link in the side column is
/// marked).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Here {
    Profile,
    Games,
    Friends,
    Reviews,
    Groups,
    Badges,
    Screenshots,
    Artwork,
    Videos,
    Workshop,
    Guides,
}

/// What the frame's buttons and links asked for; the page carries it out.
pub(crate) enum Go {
    Nav(&'static str, Option<String>),
    Act(Action),
    /// A game: your library if you own it, else the store.
    Game(model::AppId),
    /// Something for Steam (add friend, block…).
    Send(Command),
}

/// Everything the frame shows, read from the state up front (so the page
/// can use its context while the frame draws).
pub(crate) struct FrameData {
    pub id: SteamId,
    pub mine: bool,
    pub name: String,
    persona: Persona,
    info: Option<ProfileInfo>,
    badges: Option<model::Badges>,
    /// Whether Steam shows us this profile's games, inventory and friends.
    pub open: bool,
    visibility: Visibility,
    status: String,
    games: String,
    items: String,
    wishlist: String,
    friends: Vec<Persona>,
    friends_total: Option<usize>,
    /// Whether they're your friend, asked to be, or blocked.
    relationship: Relationship,
    /// Requests for what isn't loaded yet (send them).
    pub wants: Vec<Command>,
}

/// Profiles and badges asked for, so each is asked once.
static ASKED: Mutex<Option<HashSet<(SteamId, u8)>>> = Mutex::new(None);

fn ask_once(id: SteamId, what: u8) -> bool {
    ASKED.lock().unwrap().get_or_insert_with(Default::default).insert((id, what))
}

impl FrameData {
    pub(crate) fn gather(state: &client::State, local: &local::LocalState, id: SteamId) -> Self {
        let mine = id == state.steam_id;
        let mut wants = Vec::new();
        if id != 0 && ask_once(id, 0) {
            wants.push(Command::LoadProfile { steam_id: id });
        }
        // Someone who isn't your friend: Steam only sends their name and
        // picture when asked.
        if id != 0 && !mine && !state.personas.get(&id).is_some_and(|p| !p.name.is_empty()) && ask_once(id, 2) {
            wants.push(Command::LoadPersonas { ids: vec![id] });
        }
        let persona = state.persona(id);
        let info = state.profiles.get(&id).cloned();
        let friend = state.friends.get(&id) == Some(&Relationship::Friend);
        let visibility = info.as_ref().map(|i| i.visibility).unwrap_or_default();
        let open = mine
            || match visibility {
                Visibility::Public | Visibility::Unknown => true,
                Visibility::FriendsOnly => friend,
                Visibility::Private => false,
            };
        if id != 0 && open && ask_once(id, 1) {
            wants.push(Command::LoadBadges { steam_id: id });
        }
        let name = if mine && !state.persona_name.is_empty() { state.persona_name.clone() } else { persona.name.clone() };
        let owned = state.apps.as_ref().map(|a| a.values().filter(|a| !a.is_dlc()).count());
        let games = match state.player_games.get(&id) {
            Some(Ok(g)) if mine && g.is_empty() => owned.map(|n| n.to_string()).unwrap_or_default(),
            Some(Ok(g)) => g.len().to_string(),
            Some(Err(_)) => String::new(),
            None if mine => owned.map(|n| n.to_string()).unwrap_or_default(),
            None => String::new(),
        };
        let items: u64 = match state.inventory_apps.get(&id) {
            Some(Ok(apps)) => apps.iter().flat_map(|a| a.contexts.iter()).map(|c| c.2 as u64).sum(),
            _ => state.inventories.iter().filter(|(k, _)| k.0 == id).map(|(_, v)| v.total as u64).sum(),
        };
        let (friends, friends_total) = if mine {
            let all = state.friend_list();
            let n = all.len();
            (all.into_iter().take(6).collect(), Some(n))
        } else {
            match state.profile_friends.get(&id) {
                Some(Ok(ids)) => (ids.iter().take(6).map(|f| state.persona(*f)).collect(), Some(ids.len())),
                _ => (Vec::new(), None),
            }
        };
        let _ = local;
        Self {
            id,
            mine,
            name,
            status: crate::shell::status_text(state, &persona),
            persona,
            info,
            badges: state.badges.get(&id).and_then(|b| b.as_ref().ok()).cloned(),
            open,
            visibility,
            games,
            items: if items > 0 { items.to_string() } else { String::new() },
            wishlist: if mine { state.wishlist.as_ref().map(|w| w.len().to_string()).unwrap_or_default() } else { String::new() },
            friends,
            friends_total,
            relationship: state.friends.get(&id).copied().unwrap_or(Relationship::None),
            wants,
        }
    }
}

/// The profile's frame: background, the panel, the header across the top,
/// `main` in the wide column and the side column beside it. `embedded`
/// draws it inside a page that already scrolls. Returns what the frame's
/// links and buttons asked for.
pub(crate) fn frame(ui: &mut Ui, t: &Theme, d: &FrameData, here: Here, embedded: bool, main: impl FnOnce(&mut Ui)) -> Vec<Go> {
    let mut go = Vec::new();
    let full = if embedded { ui.clip_rect() } else { ui.max_rect() };
    // The profile's own background from Steam (a picture they chose), or the theme's.
    let painted = d.info.as_ref().is_some_and(|i| paint_profile_background(ui, t, full, i.background.as_deref(), i.background_video.as_deref(), i.background_full));
    if painted {
        ui.painter().rect_filled(full, CornerRadius::ZERO, t.overlay_dark(2));
    } else {
        w::paint_background(ui, t, full, "profile");
    }
    let body = |ui: &mut Ui| {
        ui.add_space(t.spacing.xl * 2.0);
        w::centered(ui, 1241.0, t.spacing.xl, |ui| {
            w::component_frame(t, &t.components.profile_panel).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let gap = t.spacing.xl + t.spacing.xs;
                let side = (ui.available_width() * 0.25).clamp(220.0, 293.0);
                let main_w = (ui.available_width() - side - gap).max(200.0);
                header(ui, t, d, here, side, &mut go);
                ui.add_space(gap);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(vec2(main_w, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(main_w);
                        ui.spacing_mut().item_spacing.y = t.spacing.lg + t.spacing.xs + 3.0;
                        main(ui);
                    });
                    ui.allocate_ui_with_layout(vec2(side, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(side);
                        ui.spacing_mut().item_spacing.y = t.spacing.lg + t.spacing.xs;
                        side_column(ui, t, d, here, &mut go);
                    });
                });
            });
        });
        ui.add_space(t.spacing.xl * 2.0);
    };
    if embedded {
        body(ui);
    } else {
        egui::ScrollArea::vertical().id_salt(("profile-frame", d.id)).auto_shrink([false, false]).show(ui, body);
    }
    go
}

/// Avatar, name, status and summary; buttons and level on the right.
fn header(ui: &mut Ui, t: &Theme, d: &FrameData, here: Here, side: f32, go: &mut Vec<Go>) {
    ui.horizontal_top(|ui| {
        let left_w = (ui.available_width() - side - t.spacing.xl).max(200.0);
        ui.allocate_ui_with_layout(vec2(left_w, 0.0), Layout::left_to_right(Align::Min), |ui| {
            ui.set_width(left_w);
            ui.spacing_mut().item_spacing.x = t.spacing.xl + t.spacing.sm + 1.0;
            // The avatar in a frame: Steam's avatar frame if one is equipped.
            let size = (left_w * 0.3).clamp(110.0, 221.0);
            let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
            let frame_url = d.info.as_ref().and_then(|i| i.avatar_frame.as_deref());
            let img = if frame_url.is_some() { r.shrink(size * 0.08) } else { r };
            let still = art::avatar_full(d.persona.avatar_hash.as_deref());
            let avatar = d.info.as_ref().and_then(|i| i.animated_avatar.clone()).unwrap_or(still);
            if !paint_picture(ui, img, &avatar, t.corner_small()) {
                w::skeleton(ui, t, img, t.corner_small());
            }
            match frame_url {
                Some(f) => {
                    paint_picture(ui, r, f, CornerRadius::ZERO);
                }
                None => {
                    ui.painter().rect_stroke(r, t.corner_small(), Stroke::new(t.dense(4.0), t.text_dim), egui::StrokeKind::Inside);
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = t.spacing.xs + 2.0;
                ui.horizontal(|ui| {
                    ui.label(w::heading_l(t, &d.name, t.text));
                    // The name's menu: copy the SteamID, open on Steam.
                    let r = ui.add(egui::Label::new(w::body_l(t, icons::CARET_DOWN, t.text)).sense(Sense::click()));
                    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Copy SteamID"));
                    let id = d.id;
                    if let Some(inner) = egui::Popup::menu(&r).show(|ui| {
                        let copied = super::steam_id_menu(ui, t, id);
                        if w::menu_item(ui, t, icons::EXTERNAL, t!("View on Steam")).clicked() {
                            crate::links::open_http(&format!("https://steamcommunity.com/profiles/{id}"));
                        }
                        copied
                    }) {
                        if let Some(what) = inner.inner {
                            go.push(Go::Act(Action::Toast(t!("Copied: {what}", what = what))));
                        }
                    }
                });
                let color = crate::shell::status_color(t, &d.persona);
                let r = ui.add(egui::Label::new(w::body_m(t, &d.status, color)).sense(Sense::click()));
                if let Some(game) = d.persona.game_app_id.filter(|_| d.persona.in_game()) {
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        go.push(Go::Game(game));
                    }
                }
                if let Some(i) = &d.info {
                    let sub: Vec<&str> = [i.real_name.as_str(), i.location.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                    if !sub.is_empty() {
                        ui.label(w::body_m(t, sub.join("  ·  "), t.text_dim));
                    }
                }
                ui.add_space(t.spacing.md);
                match d.info.as_ref().map(|i| i.summary.trim().to_string()) {
                    Some(s) if !s.is_empty() => {
                        w::markup_label(ui, &s, t.typography.body_l.size, t.text);
                    }
                    Some(_) => {
                        ui.label(w::body_m(t, t!("No profile summary yet."), t.text_faint));
                    }
                    None => {
                        for wdt in [380.0f32, 300.0] {
                            let (r, _) = ui.allocate_exact_size(vec2(wdt.min(ui.available_width()), 12.0), Sense::hover());
                            w::skeleton(ui, t, r, t.corner_small());
                        }
                    }
                }
                if let Some(since) = d.info.as_ref().and_then(|i| i.member_since) {
                    ui.label(w::body_s(t, t!("Member since {date}", date = ui::date(since)), t.text_faint)).on_hover_text(ui::ago(since));
                }
            });
        });
        // Buttons, then level and years of service.
        ui.allocate_ui_with_layout(vec2(side, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(side);
            ui.spacing_mut().item_spacing.y = t.spacing.lg + t.spacing.xs;
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = t.spacing.sm + 2.0;
                    if d.mine {
                        if w::button(ui, t, None, t!("Edit Profile"), w::Kind::Secondary, w::Size::Md).clicked() {
                            go.push(Go::Nav("profile", Some("edit".into())));
                        }
                        if here != Here::Profile && w::button(ui, t, None, t!("Back to Profile"), w::Kind::Secondary, w::Size::Md).clicked() {
                            go.push(Go::Nav("profile", Some(d.id.to_string())));
                        }
                    } else {
                        their_actions(ui, t, d, go);
                    }
                });
            });
            // Someone else's: their buttons fill the row, so "Back" goes under them.
            if !d.mine && here != Here::Profile {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if w::button(ui, t, Some(icons::ARROW_LEFT), t!("Back to Profile"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        go.push(Go::Nav("profile", Some(d.id.to_string())));
                    }
                });
            }
            confirm(ui, t, d, go);
            if let Some(b) = &d.badges {
                w::component_frame(t, &t.components.profile_inset).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = t.spacing.md;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.xl - 1.0;
                        level_badge(ui, t, b.level, t.dense(49.0));
                        ui.vertical(|ui| {
                            ui.label(w::body_m(t, t!("Level {n}", n = b.level), t.text));
                            if !b.xp.is_empty() {
                                ui.label(w::body_m(t, &b.xp, t.text_dim));
                            }
                        });
                    });
                    if !b.next.is_empty() {
                        let width = ui.available_width();
                        w::progress(ui, t, b.progress, width);
                        ui.label(w::body_s(t, next_level_text(&b.next), t.text_faint));
                    }
                    // Years of Service, like the design's second row.
                    if let Some(years) = b.items.iter().find(|x| x.title.to_lowercase().contains("years of service")) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = t.spacing.lg + 1.0;
                            badge_icon(ui, t, years, t.dense(55.0));
                            ui.vertical(|ui| {
                                ui.label(w::body_m(t, &years.title, t.text));
                                if !years.detail.is_empty() {
                                    ui.label(w::body_m(t, &years.detail, t.text_dim));
                                }
                            });
                        });
                    }
                });
            }
        });
    });
}

/// Status, badges, links with counts; then friends.
fn side_column(ui: &mut Ui, t: &Theme, d: &FrameData, here: Here, go: &mut Vec<Go>) {
    let row = &t.components.profile_row;
    let id = d.id;
    w::component_frame(t, &t.components.profile_inset).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = t.spacing.xs + 2.0;
        let color = crate::shell::status_color(t, &d.persona);
        let status = i18n::tr_dyn(d.persona.state.label());
        w::list_row(ui, t, row, None, t!("Status"), &status, Some(color), false);
        if !d.open {
            ui.label(w::body_m(t, if d.visibility == Visibility::FriendsOnly { t!("This profile is visible to friends only") } else { t!("This profile is private") }, t.text_dim));
            return;
        }
        // Badges: the count, then the first few.
        if let Some(b) = d.badges.as_ref().filter(|b| !b.items.is_empty()) {
            let look = t.look(row, ui::theme::Interact { selected: here == Here::Badges, ..Default::default() });
            let frame = w::component_frame(t, row).fill(match look.fill {
                ui::theme::ResolvedFill::Color(c) => c,
                _ => w::solid_fill(t, row),
            });
            let resp = frame
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(t!("Badges")).font(t.font_of(row)).color(look.text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(egui::RichText::new(b.items.len().to_string()).font(t.font_of(row)).color(look.icon));
                        });
                    });
                    ui.add_space(t.spacing.xs);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = t.spacing.xs + 1.0;
                        let size = ((ui.available_width() - 4.0 * (t.spacing.xs + 1.0)) / 5.0).min(t.dense(45.0));
                        for badge in b.items.iter().take(5) {
                            badge_icon(ui, t, badge, size);
                        }
                    });
                })
                .response;
            let resp = ui.interact(resp.rect, ui.scope_id().with("badges-link"), Sense::click());
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                go.push(Go::Nav("profile", Some(format!("{id}/badges"))));
            }
        }
        // Every link stays on the profile: only the main column changes.
        let files = |what: &str| Go::Nav("profile", Some(format!("{id}/{what}")));
        let sub = |name: &str| Go::Nav("profile", Some(format!("{id}/{name}")));
        let mut links: Vec<(String, String, Here, Go)> = vec![(t!("Games").to_string(), d.games.clone(), Here::Games, sub("games"))];
        if d.mine {
            links.push((t!("Wishlist").to_string(), d.wishlist.clone(), Here::Profile, Go::Nav("store", Some("wishlist".into()))));
        }
        links.extend([
            (t!("Inventory").to_string(), d.items.clone(), Here::Profile, Go::Nav("inventory", Some(id.to_string()))),
            (t!("Screenshots").to_string(), String::new(), Here::Screenshots, files("screenshots")),
            (t!("Videos").to_string(), String::new(), Here::Videos, files("videos")),
            (t!("Workshop Items").to_string(), String::new(), Here::Workshop, files("workshop")),
            (t!("Reviews").to_string(), String::new(), Here::Reviews, sub("reviews")),
            (t!("Guides").to_string(), String::new(), Here::Guides, files("guides")),
            (t!("Artworks").to_string(), String::new(), Here::Artwork, files("artwork")),
            (t!("Groups").to_string(), String::new(), Here::Groups, sub("groups")),
        ]);
        if !d.mine {
            links.push((t!("Friends").to_string(), d.friends_total.map(|n| n.to_string()).unwrap_or_default(), Here::Friends, sub("friends")));
        }
        for (label, count, page, target) in links {
            let on = page == here && page != Here::Profile;
            if w::list_row(ui, t, row, None, &label, &count, None, on).clicked() && !on {
                go.push(target);
            }
        }
    });
    if !d.open || d.friends.is_empty() {
        return;
    }
    w::component_frame(t, &t.components.profile_inset).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = t.spacing.xs + 2.0;
        ui.horizontal(|ui| {
            let n = d.friends_total.unwrap_or(d.friends.len());
            let r = ui.add(egui::Label::new(w::body_l(t, t!("Friends ({n})", n = n), t.text_dim)).sense(Sense::click()));
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                go.push(if d.mine { Go::Nav("friends", None) } else { Go::Nav("profile", Some(format!("{id}/friends"))) });
            }
            if d.mine {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::USER_PLUS, t!("Add a friend"), false, 0).clicked() {
                        go.push(Go::Nav("friends", None));
                    }
                });
            }
        });
        for p in &d.friends {
            if friend_row(ui, t, p).clicked() {
                go.push(Go::Nav("profile", Some(p.steam_id.to_string())));
            }
        }
    });
}

/// A friend in the side column: picture with a status-colored edge, name
/// and what they're doing.
fn friend_row(ui: &mut Ui, t: &Theme, p: &Persona) -> egui::Response {
    let c = &t.components.profile_row;
    let pad = t.padding_of(c);
    let h = t.dense(46.0) + pad.y * 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, w::interact(ui, &resp, false));
        w::paint_box(ui, t, rect, c, &look, 0.0);
        let av = Rect::from_min_size(rect.min + pad, vec2(h - pad.y * 2.0, h - pad.y * 2.0));
        if !w::paint_cover(ui, av, &art::avatar_full(p.avatar_hash.as_deref()), CornerRadius::ZERO) {
            w::skeleton(ui, t, av, CornerRadius::ZERO);
        }
        let color = crate::shell::status_color(t, p);
        ui.painter().rect_stroke(av, CornerRadius::ZERO, Stroke::new(t.dense(2.0), color), egui::StrokeKind::Inside);
        let x = av.right() + pad.x;
        let fid = t.font_of(c);
        let mut job = egui::text::LayoutJob::simple_singleline(p.name.clone(), fid, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - x - pad.x).max(10.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, av.top() + t.spacing.xs), g, color);
        let status = p.game_name.clone().filter(|_| p.in_game()).unwrap_or_else(|| i18n::tr_dyn(p.state.label()).to_string());
        let mut job = egui::text::LayoutJob::simple_singleline(status, t.typography.body_s.font(), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - x - pad.x).max(10.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, av.bottom() - g.size().y - t.spacing.xs), g, color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A titled box in the main column ("Recent Activity", "Comments").
pub(crate) fn section<R>(ui: &mut Ui, t: &Theme, title: &str, note: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    w::titled_box(ui, t, &t.components.profile_box, title, note, |ui| {
        ui.spacing_mut().item_spacing.y = t.spacing.sm + 3.0;
        add(ui)
    })
}

/// The darker block inside a section.
pub(crate) fn inset<R>(ui: &mut Ui, t: &Theme, add: impl FnOnce(&mut Ui) -> R) -> R {
    w::component_frame(t, &t.components.profile_inset)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Loading placeholders: `n` bars `h` tall.
pub(crate) fn loading_rows(ui: &mut Ui, t: &Theme, n: usize, h: f32) {
    for _ in 0..n {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        w::skeleton(ui, t, r, t.corner_small());
    }
}

/// Prev, page numbers, Next (like the design's games list). Returns the
/// page picked.
pub(crate) fn pager(ui: &mut Ui, t: &Theme, page: usize, pages: usize) -> Option<usize> {
    if pages <= 1 {
        return None;
    }
    let mut pick = None;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = t.spacing.xs;
        ui.add_enabled_ui(page + 1 < pages, |ui| {
            if w::button(ui, t, None, t!("Next"), w::Kind::Ghost, w::Size::Sm).clicked() {
                pick = Some(page + 1);
            }
        });
        // The pages near this one, the first and the last.
        let mut shown: Vec<usize> = (0..pages).filter(|p| *p == 0 || *p + 1 == pages || p.abs_diff(page) <= 2).collect();
        shown.reverse();
        let mut last = None;
        for p in shown {
            if last.is_some_and(|l: usize| l > p + 1) {
                ui.label(w::body_m(t, "…", t.text_dim));
            }
            if w::button(ui, t, None, &(p + 1).to_string(), if p == page { w::Kind::Secondary } else { w::Kind::Ghost }, w::Size::Sm).clicked() && p != page {
                pick = Some(p);
            }
            last = Some(p);
        }
        ui.add_enabled_ui(page > 0, |ui| {
            if w::button(ui, t, None, t!("Prev"), w::Kind::Ghost, w::Size::Sm).clicked() {
                pick = Some(page.saturating_sub(1));
            }
        });
    });
    pick
}

// ---------- the profile itself ----------

impl Profile {
    /// The main column of the profile: in game, games, recent activity,
    /// item showcase, comments.
    fn overview(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, d: &FrameData) {
        let id = d.id;
        if d.persona.in_game() {
            let game = d.persona.game_name.clone().or_else(|| d.persona.game_app_id.and_then(|g| ctx.state.app_name(g).map(String::from))).unwrap_or_default();
            section(ui, t, t!("Currently in game"), "", |ui| {
                inset(ui, t, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(g) = d.persona.game_app_id {
                            let (r, resp) = ui.allocate_exact_size(vec2(t.dense(184.0), t.dense(69.0)), Sense::click());
                            w::image_or_skeleton(ui, t, r, &art::header(g), t.corner_small());
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                ctx.act(crate::shell::game_page(ctx.state, g));
                            }
                        }
                        ui.label(w::heading_s(t, game, t.in_game));
                    });
                });
            });
        }
        if !d.open {
            section(ui, t, "", "", |ui| {
                ui.horizontal(|ui| {
                    ui.label(w::heading_m(t, icons::LOCK_SIMPLE, t.text_dim));
                    ui.vertical(|ui| {
                        let title = if d.visibility == Visibility::FriendsOnly { t!("This profile is visible to friends only") } else { t!("This profile is private") };
                        ui.label(w::heading_s(t, title, t.text));
                        ui.label(w::body_m(t, t!("Games, inventory and friends are hidden by its owner."), t.text_dim));
                    });
                });
            });
            self.comments(ui, t, ctx, id, d.mine);
            return;
        }
        self.collection(ui, t, ctx, d);
        self.recent_games(ui, t, ctx, id);
        self.showcase(ui, t, ctx, id);
        self.comments(ui, t, ctx, id, d.mine);
    }

    /// The games someone has (or this PC's history for you).
    fn player_games(ctx: &Ctx, id: SteamId) -> Option<Result<Vec<model::PlayerGame>, String>> {
        let mut r = ctx.state.player_games.get(&id).cloned()?;
        if let Ok(games) = &mut r {
            if games.is_empty() && id == ctx.state.steam_id {
                *games = ctx
                    .local
                    .games
                    .iter()
                    .map(|g| model::PlayerGame { app_id: g.app_id, name: g.name.clone(), playtime_minutes: g.playtime_minutes.unwrap_or(0), playtime_2weeks: 0, last_played: g.last_played, icon: None })
                    .collect();
            }
        }
        Some(r)
    }

    /// Counts and the most played games (Figma "Game Collector").
    fn collection(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, d: &FrameData) {
        let Some(Ok(mut games)) = Self::player_games(ctx, d.id) else { return };
        if games.is_empty() {
            return;
        }
        games.sort_by_key(|g| std::cmp::Reverse(g.playtime_minutes));
        let hours: u64 = games.iter().map(|g| g.playtime_minutes).sum::<u64>() / 60;
        let mut stats = vec![(games.len().to_string(), t!("Games owned").to_string())];
        if d.mine {
            if let Some(dlc) = ctx.state.apps.as_ref().map(|a| a.values().filter(|a| a.is_dlc()).count()).filter(|n| *n > 0) {
                stats.push((dlc.to_string(), t!("DLCs owned").to_string()));
            }
        }
        stats.push((hours.to_string(), t!("Hours played").to_string()));
        if !d.wishlist.is_empty() {
            stats.push((d.wishlist.clone(), t!("Wishlisted").to_string()));
        }
        let mut open = None;
        section(ui, t, t!("Game collection"), "", |ui| {
            inset(ui, t, |ui| {
                ui.spacing_mut().item_spacing.y = t.spacing.md + 1.0;
                ui.horizontal_top(|ui| {
                    let each = ui.available_width() / stats.len().max(4) as f32;
                    ui.add_space(((ui.available_width() - each * stats.len() as f32) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (value, label) in &stats {
                        ui.allocate_ui_with_layout(vec2(each, 0.0), Layout::top_down(Align::Center), |ui| {
                            ui.set_width(each);
                            ui.label(w::heading_m(t, value, t.text));
                            ui.label(w::body_l(t, label, t.text_dim));
                        });
                    }
                });
                let gap = t.spacing.md;
                let tile_w = (ui.available_width() - gap * 4.0) / 5.0;
                for row in games.chunks(5).take(2) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for g in row {
                            let (r, resp) = ui.allocate_exact_size(vec2(tile_w, tile_w * 215.0 / 460.0), Sense::click());
                            w::image_or_title(ui, t, r, &art::header(g.app_id), &g.name, CornerRadius::ZERO);
                            if resp.hovered() {
                                ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(t.dense(2.0), t.accent), egui::StrokeKind::Inside);
                            }
                            let tip = format!("{}\n{}", g.name, t!("{time} on record", time = ui::playtime(g.playtime_minutes)));
                            if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                open = Some(g.app_id);
                            }
                        }
                    });
                }
                if games.len() > 10 && ui.link(w::body_m(t, t!("See all {n} games", n = games.len()), t.accent)).clicked() {
                    ctx.navigate("profile", Some(format!("{}/games", d.id)));
                }
            });
        });
        if let Some(app) = open {
            ctx.act(crate::shell::game_page(ctx.state, app));
        }
    }

    /// Recently played games (Figma "Recent Activity").
    fn recent_games(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId) {
        let Some(result) = Self::player_games(ctx, id) else {
            section(ui, t, t!("Recent Activity"), "", |ui| loading_rows(ui, t, 3, t.dense(94.0)));
            return;
        };
        let Ok(mut games) = result else { return };
        let recent_minutes: u64 = games.iter().map(|g| g.playtime_2weeks).sum();
        games.sort_by_key(|g| std::cmp::Reverse((g.playtime_2weeks, g.last_played)));
        let note = t!("{hours} hours past 2 weeks", hours = format!("{:.1}", recent_minutes as f64 / 60.0));
        let shown: Vec<_> = games.iter().filter(|g| g.last_played.is_some() || g.playtime_2weeks > 0).take(3).cloned().collect();
        let mut open = None;
        section(ui, t, t!("Recent Activity"), &note, |ui| {
            if shown.is_empty() {
                ui.label(w::body_m(t, t!("No recent games."), t.text_faint));
            }
            for g in &shown {
                if game_row(ui, t, ctx, g) {
                    open = Some(g.app_id);
                }
            }
            if ui.link(w::body_m(t, t!("See all {n} games", n = games.len()), t.accent)).clicked() {
                ctx.navigate("profile", Some(format!("{id}/games")));
            }
        });
        if let Some(app) = open {
            ctx.act(crate::shell::game_page(ctx.state, app));
        }
    }

    /// A few items from the Steam inventory, like Steam's item showcase.
    fn showcase(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId) {
        if self.showcase_requested.insert(id) {
            ctx.send(Command::LoadInventory { steam_id: id, app_id: 753, context_id: 6, language: i18n::language().steam_name().into() });
        }
        let Some(inv) = ctx.state.inventories.get(&(id, 753, 6)) else { return };
        if inv.items.is_empty() || inv.error.is_some() {
            return;
        }
        // Prefer backgrounds and emoticons, then anything colorful.
        let mut items: Vec<model::InventoryItem> = inv.items.clone();
        items.sort_by_key(|i| std::cmp::Reverse((i.kind.to_lowercase().contains("background") as u8, i.marketable as u8)));
        let mut go = false;
        section(ui, t, t!("Item showcase"), "", |ui| {
            inset(ui, t, |ui| {
                let size = t.dense(84.0);
                let gap = t.spacing.sm;
                let cols = (((ui.available_width() + gap) / (size + gap)).floor() as usize).max(1);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for item in items.iter().take(cols) {
                        let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                        ui.painter().rect_filled(r, t.corner_small(), t.raised);
                        if !w::paint_contain(ui, r.shrink(6.0), &art::econ(&item.icon, 192), Align2::CENTER_CENTER) {
                            w::skeleton(ui, t, r.shrink(6.0), t.corner_small());
                        }
                        if let Some(c) = crate::builtin::inventory::rarity_color(item) {
                            let bar = Rect::from_min_max(pos2(r.left() + 8.0, r.bottom() - 5.0), pos2(r.right() - 8.0, r.bottom() - 2.5));
                            ui.painter().rect_filled(bar, CornerRadius::same(2), c);
                        }
                        if resp.on_hover_text(format!("{}\n{}", item.name, item.kind)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            go = true;
                        }
                    }
                });
            });
            if ui.link(w::body_m(t, t!("Open inventory"), t.accent)).clicked() {
                ctx.navigate("inventory", Some(id.to_string()));
            }
        });
        if go {
            ctx.navigate("inventory", Some(format!("{id}/753")));
        }
    }

    /// Profile comments, with posting. Respects profiles that hide or turn
    /// off comments (Steam answers with the reason, which is shown).
    fn comments(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, mine: bool) {
        if self.comments_requested.insert(id) {
            ctx.send(Command::LoadComments { steam_id: id, start: 0 });
        }
        let data = ctx.state.comments.get(&id).cloned();
        let note = match &data {
            Some(Ok(c)) if c.total == 0 => t!("No comments yet").to_string(),
            Some(Ok(c)) => t!("{n} comments", n = c.total).to_string(),
            _ => String::new(),
        };
        section(ui, t, t!("Comments"), &note, |ui| {
            let data = match data {
                None => return loading_rows(ui, t, 2, t.dense(44.0)),
                Some(Err(e)) => {
                    let why = if e.to_lowercase().contains("private") {
                        t!("This profile's comments are private.").to_string()
                    } else {
                        t!("Comments aren't available on this profile: {reason}", reason = i18n::tr_dyn(&e))
                    };
                    ui.horizontal(|ui| {
                        ui.label(w::body_m(t, icons::LOCK_SIMPLE, t.text_faint));
                        ui.label(w::body_m(t, why, t.text_dim));
                    });
                    return;
                }
                Some(Ok(c)) => c,
            };
            // Composer: your picture and the field, like the design.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.spacing.md - 1.0;
                let me = ctx.state.persona(ctx.state.steam_id);
                let size = t.height_of(&t.components.input, 34.0).max(t.dense(41.0));
                let (av, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
                if !w::paint_cover(ui, av, &art::avatar(me.avatar_hash.as_deref()), t.corner_small()) {
                    w::skeleton(ui, t, av, t.corner_small());
                }
                ui.painter().rect_stroke(av, t.corner_small(), Stroke::new(t.dense(2.0), t.text_dim), egui::StrokeKind::Inside);
                let hint = if mine { t!("Write a comment on your profile…") } else { t!("Add a comment…") };
                let rows = if self.comment_draft.is_empty() { 1 } else { 3 };
                let width = ui.available_width();
                w::text_input(ui, t, &mut self.comment_draft, hint, width, rows);
            });
            if !self.comment_draft.is_empty() {
                ui.horizontal(|ui| {
                    let text = self.comment_draft.trim().to_string();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_enabled_ui(!text.is_empty() && text.chars().count() <= 1000, |ui| {
                            if w::button(ui, t, Some(icons::SEND), t!("Post comment"), w::Kind::Primary, w::Size::Sm).clicked() {
                                ctx.send(Command::PostComment { steam_id: id, text });
                            }
                        });
                        ui.label(w::body_s(t, format!("{}/1000", self.comment_draft.chars().count()), t.text_faint));
                    });
                });
            }
            for c in &data.items {
                inset(ui, t, |ui| {
                    ui.horizontal_top(|ui| {
                        let (av, resp) = ui.allocate_exact_size(vec2(t.dense(36.0), t.dense(36.0)), Sense::click());
                        match &c.avatar {
                            Some(u) => {
                                if !w::paint_cover(ui, av, u, t.corner_small()) {
                                    w::skeleton(ui, t, av, t.corner_small());
                                }
                            }
                            None => w::placeholder(ui, t, av, "", t.corner_small()),
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            ctx.navigate("profile", Some(c.author.to_string()));
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal(|ui| {
                                if ui.add(egui::Label::new(w::body_m(t, &c.name, t.accent)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    ctx.navigate("profile", Some(c.author.to_string()));
                                }
                                if c.time > 0 {
                                    ui.label(w::body_s(t, ui::ago(c.time), t.text_faint));
                                }
                                // Steam lets you delete any comment on your profile, and your own anywhere.
                                if mine || c.author == ctx.state.steam_id {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if w::icon_button(ui, t, icons::TRASH, t!("Delete comment"), false, 0).clicked() {
                                            ctx.send(Command::DeleteComment { steam_id: id, comment_id: c.id });
                                        }
                                    });
                                }
                            });
                            w::markup_label(ui, c.html.trim(), t.typography.body_m.size, t.text);
                        });
                    });
                });
            }
            // Older and newer pages, 20 at a time (like Steam's comment pages).
            let start = self.comment_start.get(&id).copied().unwrap_or(0);
            if data.total as usize > data.items.len() {
                ui.horizontal(|ui| {
                    let mut go = None;
                    ui.add_enabled_ui(start > 0, |ui| {
                        if w::button(ui, t, Some(icons::CARET_LEFT), t!("Newer"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            go = Some(start.saturating_sub(20));
                        }
                    });
                    let page = start / 20 + 1;
                    let pages = data.total.div_ceil(20).max(1);
                    ui.label(w::body_s(t, t!("Page {page} of {pages}", page = page, pages = pages), t.text_dim));
                    ui.add_enabled_ui(start + 20 < data.total, |ui| {
                        if w::button(ui, t, Some(icons::CARET_RIGHT), t!("Older"), w::Kind::Ghost, w::Size::Sm).clicked() {
                            go = Some(start + 20);
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::icon_button(ui, t, icons::REFRESH, t!("Refresh"), false, 0).clicked() {
                            go = Some(0);
                        }
                    });
                    if let Some(s) = go {
                        self.comment_start.insert(id, s);
                        ctx.send(Command::LoadComments { steam_id: id, start: s });
                    }
                });
            }
        });
    }

    /// Someone's games (Figma "games" `455:9708`): sorting and search
    /// above, ten to a page.
    fn games_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, d: &FrameData) {
        let games = match Self::player_games(ctx, d.id) {
            None => return section(ui, t, "", "", |ui| loading_rows(ui, t, 6, t.dense(94.0))),
            Some(Err(e)) => {
                let body = if e == "private" { t!("Its owner has chosen not to show their games.").to_string() } else { e };
                return section(ui, t, "", "", |ui| w::empty(ui, t, icons::LOCK_SIMPLE, t!("These games are hidden"), &body));
            }
            Some(Ok(g)) => g,
        };
        let hours: u64 = games.iter().map(|g| g.playtime_minutes).sum::<u64>() / 60;
        let shared = if d.mine { 0 } else { games.iter().filter(|g| ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&g.app_id))).count() };
        let mut note = t!("{n} games · {hours} hours on record", n = games.len(), hours = hours);
        if !d.mine && shared > 0 {
            note = format!("{note} · {}", t!("{n} in common with you", n = shared));
        }
        let q = self.games_search.trim().to_lowercase();
        let mut list: Vec<_> = games.iter().filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q)).cloned().collect();
        match self.games_sort {
            GameSort::Recent => list.sort_by_key(|g| std::cmp::Reverse(g.last_played.unwrap_or(0))),
            GameSort::Playtime => list.sort_by_key(|g| std::cmp::Reverse(g.playtime_minutes)),
            GameSort::Name => list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        }
        let pages = list.len().div_ceil(GAMES_PER_PAGE).max(1);
        self.games_page = self.games_page.min(pages - 1);
        let mut open = None;
        section(ui, t, "", &note, |ui| {
            // The tabs strip: sorting on the left, search on the right.
            w::component_frame(t, &t.components.segmented).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let before = self.games_sort;
                    for (s, label) in [(GameSort::Recent, t!("Recently Played")), (GameSort::Playtime, t!("Playtime")), (GameSort::Name, t!("Name"))] {
                        if w::sort_chip(ui, t, label, self.games_sort == s).clicked() {
                            self.games_sort = s;
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let width = ui.available_width().clamp(120.0, 240.0);
                        if w::search(ui, t, &mut self.games_search, t!("Search games"), width).changed() {
                            self.games_page = 0;
                        }
                    });
                    if before != self.games_sort {
                        self.games_page = 0;
                    }
                });
            });
            if list.is_empty() {
                ui.label(w::body_m(t, t!("No games match your search."), t.text_faint));
            }
            for g in list.iter().skip(self.games_page * GAMES_PER_PAGE).take(GAMES_PER_PAGE) {
                if game_row(ui, t, ctx, g) {
                    open = Some(g.app_id);
                }
            }
            if let Some(p) = pager(ui, t, self.games_page, pages) {
                self.games_page = p;
            }
        });
        if let Some(app) = open {
            ctx.act(crate::shell::game_page(ctx.state, app));
        }
    }
}

/// One game with its picture, last played date and hours (Figma's recent
/// activity rows); true when clicked. Opens it in your library if you own
/// it, else the store.
fn game_row(ui: &mut Ui, t: &Theme, ctx: &Ctx, g: &model::PlayerGame) -> bool {
    let owned = ctx.state.apps.as_ref().is_some_and(|a| a.contains_key(&g.app_id));
    let c = &t.components.profile_inset;
    let pad = t.padding_of(c);
    let img_h = t.dense(94.0);
    let h = img_h + pad.y * 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    if ui.is_rect_visible(rect) {
        let look = t.look(c, w::interact(ui, &resp, false));
        w::paint_box(ui, t, rect, c, &look, 0.0);
        if resp.hovered() {
            ui.painter().rect_filled(rect, t.radius_of(c), t.overlay_light(0));
        }
        let img = Rect::from_min_size(rect.min + pad, vec2(img_h * 460.0 / 215.0, img_h));
        w::image_or_title(ui, t, img, &art::header(g.app_id), &g.name, CornerRadius::ZERO);
        let x = img.right() + t.spacing.md;
        let right = rect.right() - pad.x;
        let p = ui.painter();
        // The dates and hours on the right of the name.
        let mut meta = vec![(icons::CLOCK, t!("{time} on record", time = ui::playtime(g.playtime_minutes)).to_string())];
        if let Some(when) = g.last_played {
            meta.insert(0, (icons::CALENDAR, ui::date(when)));
        }
        let mut mx = right;
        let small = t.typography.body_s.font();
        for (icon, text) in meta.iter().rev() {
            let gt = p.layout_no_wrap(text.clone(), small.clone(), t.text);
            mx -= gt.size().x;
            p.galley(pos2(mx, img.top() + t.spacing.sm), gt, t.text);
            mx -= t.spacing.xs + small.size;
            p.text(pos2(mx + small.size / 2.0, img.top() + t.spacing.sm + small.size * 0.6), Align2::CENTER_CENTER, *icon, small.clone(), t.text_dim);
            mx -= t.spacing.md;
        }
        let mut job = egui::text::LayoutJob::simple_singleline(g.name.clone(), t.typography.body_l.font(), t.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((mx - x).max(20.0));
        let gt = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, img.top() + t.spacing.xs), gt, t.text);
        // A strip under the name: hours lately and whether you own it.
        let strip = Rect::from_min_max(pos2(x, img.bottom() - t.dense(39.0)), pos2(right, img.bottom()));
        ui.painter().rect_filled(strip, t.corner_small(), t.overlay_dark(1));
        let mut line = String::new();
        if g.playtime_2weeks > 0 {
            line = t!("{time} past 2 weeks", time = ui::playtime(g.playtime_2weeks)).to_string();
        }
        ui.painter().text(pos2(strip.left() + t.spacing.md, strip.center().y), Align2::LEFT_CENTER, line, small.clone(), t.text_dim);
        if owned {
            ui.painter().text(pos2(strip.right() - t.spacing.md, strip.center().y), Align2::RIGHT_CENTER, format!("{}  {}", icons::CHECK, t!("In your library")), small, t.online);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Steam-style level circle; its color steps every 10 levels (Steam's own
/// level colors, like a picture from Steam).
pub(super) fn level_badge(ui: &mut Ui, t: &Theme, level: u32, size: f32) {
    let colors = [0x9B9B9B, 0xC02942, 0xD95B43, 0xFECC23, 0x467A3C, 0x4E8DDB, 0x7652C9, 0xC252C9, 0x542437, 0x997C52];
    let c = colors[(level as usize / 10) % colors.len()];
    let color = egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().circle_stroke(r.center(), size / 2.0 - 1.0, Stroke::new(t.dense(2.0), color));
    ui.painter().text(r.center(), Align2::CENTER_CENTER, level.to_string(), t.typography.heading_s.font(), t.text);
}

/// "85 XP to reach Level 19" in your language (other XP lines stay as Steam
/// writes them).
fn next_level_text(s: &str) -> String {
    let num = |x: &str| -> Option<String> { let x = x.trim(); (!x.is_empty() && x.chars().all(|c| c.is_ascii_digit() || c == ',')).then(|| x.to_string()) };
    match s.trim().split_once(" XP to reach Level ") {
        Some((xp, level)) if num(xp).is_some() && num(level).is_some() => t!("{xp} XP to reach level {level}", xp = xp.trim(), level = level.trim()),
        _ => s.to_string(),
    }
}

/// "Unlocked …" in your language and time zone.
fn unlocked_text(b: &model::Badge) -> String {
    if b.unlocked_at > 0 { t!("Unlocked {date}", date = ui::date_time(b.unlocked_at)) } else { b.unlocked.clone() }
}

/// A badge picture with its name and details on hover.
pub(super) fn badge_icon(ui: &mut Ui, t: &Theme, badge: &model::Badge, size: f32) {
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    match &badge.icon {
        Some(u) => {
            if !w::paint_contain(ui, r, u, Align2::CENTER_CENTER) {
                w::skeleton(ui, t, r, t.corner_small());
            }
        }
        None => w::placeholder(ui, t, r, "", t.corner_small()),
    }
    let tip = [badge.title.clone(), badge.detail.clone(), unlocked_text(badge)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !badge.url.is_empty() {
        crate::links::open_http(&badge.url);
    }
}

/// What [`their_actions`] asks to confirm first.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Remove,
    Block,
}

fn confirm_id(id: SteamId) -> egui::Id {
    egui::Id::unique(("profile-confirm", id))
}

/// The buttons on someone else's profile, like Steam's: what fits your
/// relationship (add, accept, cancel, unblock), Message for friends, and
/// "More" with the rest. Drawn right to left.
fn their_actions(ui: &mut Ui, t: &Theme, d: &FrameData, go: &mut Vec<Go>) {
    let id = d.id;
    let rel = d.relationship;
    let link = format!("https://steamcommunity.com/profiles/{id}");
    // "More": the rest of what you can do.
    let more = w::icon_box(ui, t, icons::DOTS, t!("More"), false);
    egui::Popup::menu(&more).width(t.dense(240.0)).show(|ui| {
        if rel == Relationship::RequestRecipient && w::menu_item(ui, t, icons::X, t!("Ignore")).clicked() {
            go.push(Go::Send(Command::RemoveFriend { steam_id: id }));
        }
        if w::menu_item(ui, t, icons::LINK, t!("Copy profile link")).clicked() {
            ui.ctx().copy_text(link.clone());
            go.push(Go::Act(Action::Toast(t!("Copied: {what}", what = link.as_str()))));
        }
        if w::menu_item(ui, t, icons::EXTERNAL, t!("View on Steam")).clicked() {
            crate::links::open_http(&link);
        }
        if w::menu_item(ui, t, icons::BACKPACK, t!("Inventory")).clicked() {
            go.push(Go::Nav("inventory", Some(id.to_string())));
        }
        ui.separator();
        if rel == Relationship::Friend && w::menu_item_tinted(ui, t, icons::USER, t!("Remove friend"), Some(t.danger)).clicked() {
            ui.ctx().data_mut(|m| m.insert_temp(confirm_id(id), Confirm::Remove));
        }
        if rel == Relationship::Blocked {
            if w::menu_item(ui, t, icons::LOCK_SIMPLE, t!("Unblock")).clicked() {
                go.push(Go::Send(Command::BlockFriend { steam_id: id, block: false }));
            }
        } else if w::menu_item_tinted(ui, t, icons::LOCK_SIMPLE, t!("Block"), Some(t.danger)).clicked() {
            ui.ctx().data_mut(|m| m.insert_temp(confirm_id(id), Confirm::Block));
        }
        // Steam takes reports on its own page (under its "More" button there).
        if w::menu_item(ui, t, icons::WARNING, t!("Report on Steam…")).clicked() {
            crate::links::open_http(&link);
        }
    });
    let button = |ui: &mut Ui, icon: &str, label: &str, kind: w::Kind| w::button(ui, t, Some(icon), label, kind, w::Size::Md).clicked();
    match rel {
        Relationship::Friend => {
            if button(ui, icons::CHAT, t!("Message"), w::Kind::Primary) {
                go.push(Go::Act(Action::OpenChat(id)));
            }
        }
        Relationship::RequestRecipient => {
            if button(ui, icons::CHECK, t!("Accept"), w::Kind::Primary) {
                go.push(Go::Send(Command::AddFriend { steam_id: id }));
            }
        }
        Relationship::RequestInitiator => {
            if button(ui, icons::X, t!("Cancel invite"), w::Kind::Secondary) {
                go.push(Go::Send(Command::RemoveFriend { steam_id: id }));
            }
        }
        Relationship::Blocked => {
            ui.label(w::body_m(t, t!("Blocked"), t.text_dim));
        }
        _ => {
            if button(ui, icons::USER_PLUS, t!("Add Friend"), w::Kind::Primary) {
                go.push(Go::Send(Command::AddFriend { steam_id: id }));
                go.push(Go::Act(Action::Toast(t!("Friend invite sent").to_string())));
            }
        }
    }
}

/// "Are you sure?" for removing or blocking, asked from [`their_actions`].
fn confirm(ui: &mut Ui, t: &Theme, d: &FrameData, go: &mut Vec<Go>) {
    let key = confirm_id(d.id);
    let Some(what) = ui.ctx().data(|m| m.get_temp::<Confirm>(key)) else { return };
    let mut close = false;
    egui::Modal::new(key.with("modal")).show(ui.ctx(), |ui| {
        ui.set_width(t.dense(400.0));
        let (title, body, label, cmd) = match what {
            Confirm::Remove => (t!("Remove {name} from your friends?", name = d.name.as_str()), String::new(), t!("Remove friend"), Command::RemoveFriend { steam_id: d.id }),
            Confirm::Block => (
                t!("Block all communication with {name}?", name = d.name.as_str()),
                t!("They'll be removed from your friends and can't message you or invite you.").to_string(),
                t!("Block"),
                Command::BlockFriend { steam_id: d.id, block: true },
            ),
        };
        ui.add(egui::Label::new(w::heading_s(t, title, t.text)).wrap());
        if !body.is_empty() {
            ui.add_space(t.spacing.xs);
            ui.add(egui::Label::new(w::body_m(t, body, t.text_dim)).wrap());
        }
        ui.add_space(t.spacing.lg);
        ui.horizontal(|ui| {
            if w::button(ui, t, None, label, w::Kind::Danger, w::Size::Md).clicked() {
                go.push(Go::Send(cmd));
                close = true;
            }
            if w::button(ui, t, None, t!("Cancel"), w::Kind::Ghost, w::Size::Md).clicked() {
                close = true;
            }
        });
    });
    if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.ctx().data_mut(|m| m.remove::<Confirm>(key));
    }
}

// ---------- moving pictures: animated avatars, frames and backgrounds ----------

/// The app's async runtime, for the background videos.
static RUNTIME: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();

/// Called once when the extensions are made.
pub fn set_runtime(rt: tokio::runtime::Handle) {
    let _ = RUNTIME.set(rt);
}

thread_local! {
    /// Background videos playing, by address, with the frame each was last
    /// drawn in. A player that isn't drawn pauses itself.
    static VIDEOS: std::cell::RefCell<std::collections::HashMap<String, (video::Player, u64)>> = Default::default();
}

/// Play an MP4 looped and muted, covering `rect`. False until its first
/// frame is ready, and when it can't play (draw the still picture then).
pub(crate) fn paint_video(ui: &Ui, rect: Rect, url: &str, radius: CornerRadius) -> bool {
    let Some(rt) = RUNTIME.get() else { return false };
    if crate::links::validate_http_url(url).is_none() {
        return false;
    }
    let pass = ui.ctx().cumulative_pass_nr();
    VIDEOS.with(|v| {
        let mut v = v.borrow_mut();
        // Videos not shown for a while are dropped (only a few at a time).
        v.retain(|k, (_, last)| k == url || pass.saturating_sub(*last) < 300);
        let entry = v.entry(url.to_string()).or_insert_with(|| {
            let opts = video::Options { autoplay: true, muted: true, volume: 0.0, looping: true, width: 1920 };
            (video::Player::new(rt, ui.ctx(), video::Source::Mp4(url.to_string()), opts), pass)
        });
        entry.1 = pass;
        let p = &mut entry.0;
        if p.status() == video::Status::Failed {
            return false;
        }
        p.poll(ui.ctx());
        let Some((tex, size)) = p.texture() else { return false };
        let (want, have) = (rect.width() / rect.height().max(1.0), size.x / size.y.max(1.0));
        let uv = if have > want {
            let w = want / have;
            Rect::from_min_max(egui::pos2((1.0 - w) / 2.0, 0.0), egui::pos2((1.0 + w) / 2.0, 1.0))
        } else {
            let h = have / want;
            Rect::from_min_max(egui::pos2(0.0, (1.0 - h) / 2.0), egui::pos2(1.0, (1.0 + h) / 2.0))
        };
        ui.painter_at(rect).add(egui::epaint::RectShape::filled(rect, radius, egui::Color32::WHITE).with_texture(tex, uv));
        true
    })
}

/// A picture covering `rect`; an animated GIF (animated avatars) plays.
pub(crate) fn paint_picture(ui: &Ui, rect: Rect, url: &str, radius: CornerRadius) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_lowercase();
    if path.ends_with(".gif") {
        if w::image_state(ui, url) == w::ImageState::Missing {
            return false;
        }
        egui::Image::new(url.to_string()).corner_radius(radius).paint_at(ui, rect);
        return true;
    }
    w::paint_cover(ui, rect, url, radius)
}

/// A profile background in `rect`: its video when it has one (the still
/// until the video shows, or if it can't play), full screen or at its own
/// size (across the top, not stretched to the bottom). False when there's
/// nothing to show.
pub(crate) fn paint_profile_background(ui: &Ui, t: &Theme, rect: Rect, still: Option<&str>, video: Option<&str>, full: bool) -> bool {
    let area = if full { rect } else { Rect::from_min_size(rect.min, vec2(rect.width(), (rect.width() * 9.0 / 16.0).min(rect.height()))) };
    if !full {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, t.chrome);
    }
    if video.is_some_and(|v| paint_video(ui, area, v, CornerRadius::ZERO)) {
        return true;
    }
    still.is_some_and(|s| w::paint_cover(ui, area, s, CornerRadius::ZERO))
}

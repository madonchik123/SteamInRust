//! Profiles: yours and your friends'. Shows the equipped profile background,
//! avatar frame, level, bio, and (for you) stats and game history.

use client::Command;
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Rect, Sense, Stroke, Ui, pos2, vec2,
};
use extensions::{Action, Ctx, Extension, Manifest, PageSpec};
use i18n::t;
use model::{Relationship, SteamId, Visibility, art};
use std::collections::HashSet;
use ui::{Theme, icons, widgets as w};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum GameSort {
    #[default]
    Recent,
    Playtime,
    Name,
}

#[derive(Default)]
pub struct Profile {
    /// `None` = your own profile.
    viewing: Option<SteamId>,
    requested: HashSet<SteamId>,
    games_requested: HashSet<SteamId>,
    /// Showing the full games list instead of the profile.
    games_view: bool,
    games_search: String,
    games_sort: GameSort,
    showcase_requested: HashSet<SteamId>,
    social_requested: HashSet<SteamId>,
    comment_draft: String,
    all_badges: Option<SteamId>,
    /// Which page of comments shows (first comment's index), per profile.
    comment_start: std::collections::HashMap<SteamId, u32>,
}

impl Extension for Profile {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "profile",
            name: "Profile",
            author: "SteamInRust",
            description: "Profiles",
            core: true,
            ..Manifest::DEFAULT
        }
    }

    fn pages(&self) -> Vec<PageSpec> {
        vec![
            PageSpec::new("profile", "Profile", icons::USER_CIRCLE)
                .main()
                .order(40),
        ]
    }

    fn on_event(&mut self, event: &client::Event, ctx: &mut Ctx) {
        if let client::Event::CommentPosted { result, .. } = event {
            match result {
                Ok(()) => {
                    self.comment_draft.clear();
                    ctx.toast(t!("Comment posted"));
                }
                Err(e) => ctx.toast(t!(
                    "Couldn't post the comment: {error}",
                    error = i18n::tr_dyn(e)
                )),
            }
        }
    }

    fn on_navigate(&mut self, _page: &str, arg: Option<&str>) {
        let mut parts = arg.unwrap_or_default().split('/');
        self.viewing = parts.next().and_then(|a| a.parse().ok());
        self.games_view = parts.next() == Some("games");
        self.games_search.clear();
    }

    fn page(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) {
        let t = ctx.theme.clone();
        let id = self
            .viewing
            .filter(|id| *id != ctx.state.steam_id)
            .unwrap_or(ctx.state.steam_id);
        let mine = id == ctx.state.steam_id;
        if id != 0 && self.requested.insert(id) {
            ctx.send(Command::LoadProfile { steam_id: id });
        }
        let persona = ctx.state.persona(id);
        let info = ctx.state.profiles.get(&id).cloned();
        let friend = ctx.state.friends.get(&id) == Some(&Relationship::Friend);
        let visibility = info.as_ref().map(|i| i.visibility).unwrap_or_default();
        // Whether Steam will show us this profile's games, inventory and friends.
        let open = mine
            || match visibility {
                Visibility::Public | Visibility::Unknown => true,
                Visibility::FriendsOnly => friend,
                Visibility::Private => false,
            };
        if id != 0 && open && self.games_requested.insert(id) {
            ctx.send(Command::LoadPlayerGames {
                steam_id: id,
                language: i18n::language().steam_name().into(),
            });
        }
        let display_name = if mine && !ctx.state.persona_name.is_empty() {
            ctx.state.persona_name.clone()
        } else {
            persona.name.clone()
        };

        // Full-page background, like a Steam profile.
        let full = ui.max_rect();
        let bg_url = info.as_ref().and_then(|i| i.background.clone());
        let painted = bg_url
            .as_deref()
            .is_some_and(|u| w::paint_cover(ui, full, u, CornerRadius::ZERO));
        if painted {
            ui.painter()
                .rect_filled(full, CornerRadius::ZERO, Color32::from_black_alpha(120));
        } else {
            w::gradient(
                ui.painter(),
                Rect::from_min_size(full.min, vec2(full.width(), 340.0)),
                t.accent.gamma_multiply(0.30),
                t.accent_2.gamma_multiply(0.18),
                true,
            );
            w::fade_down(
                ui,
                Rect::from_min_size(full.min, vec2(full.width(), 340.0)),
                t.bg,
                0.0,
            );
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(40.0);
            w::centered(ui, 1180.0, 32.0, |ui| {
                if self.games_view {
                    self.games_page(ui, &t, ctx, id, &display_name, mine);
                    return;
                }
                // Identity header on a translucent panel.
                egui::Frame::new()
                    .fill(t.chrome.gamma_multiply(0.82))
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(t.corner())
                    .inner_margin(Margin::same(24))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_top(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(164.0, 164.0), Sense::hover());
                            let img = r.shrink(14.0);
                            if !w::paint_cover(ui, img, &art::avatar_full(persona.avatar_hash.as_deref()), CornerRadius::same(6)) {
                                w::skeleton(ui, &t, img, CornerRadius::same(6));
                            }
                            if let Some(frame) = info.as_ref().and_then(|i| i.avatar_frame.as_deref()) {
                                w::paint_cover(ui, r, frame, CornerRadius::ZERO);
                            }
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                ui.add_space(10.0);
                                let name = if mine && !ctx.state.persona_name.is_empty() { ctx.state.persona_name.clone() } else { persona.name.clone() };
                                ui.horizontal(|ui| {
                                    ui.label(w::text_bold(name, 30.0, Color32::WHITE));
                                    if mine {
                                        if let Some(level) = ctx.state.level {
                                            level_badge(ui, &t, level);
                                        }
                                    }
                                });
                                if let Some(i) = &info {
                                    let mut sub = Vec::new();
                                    if !i.real_name.is_empty() {
                                        sub.push(i.real_name.clone());
                                    }
                                    if !i.location.is_empty() {
                                        sub.push(i.location.clone());
                                    }
                                    if !sub.is_empty() {
                                        ui.label(w::text(sub.join("  ·  "), 14.0, t.text_dim));
                                    }
                                } else {
                                    let (r, _) = ui.allocate_exact_size(vec2(220.0, 14.0), Sense::hover());
                                    w::skeleton(ui, &t, r, CornerRadius::same(4));
                                }
                                let r = ui.add(egui::Label::new(w::text_medium(crate::shell::status_text(ctx.state, &persona), 14.0, crate::shell::status_color(&t, &persona))).sense(Sense::click()));
                                if let Some(game) = persona.game_app_id.filter(|_| persona.in_game()) {
                                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                        ctx.act(crate::shell::game_page(ctx.state, game));
                                    }
                                }
                                ui.add_space(10.0);
                                match info.as_ref().map(|i| i.summary.trim().to_string()) {
                                    Some(s) if !s.is_empty() => {
                                        w::markup_label(ui, &s, 14.0, t.text);
                                    }
                                    Some(_) => {
                                        ui.label(w::text(t!("No profile summary yet."), 13.5, t.text_faint));
                                    }
                                    None => {
                                        for wdt in [380.0, 300.0] {
                                            let (r, _) = ui.allocate_exact_size(vec2(wdt, 12.0), Sense::hover());
                                            w::skeleton(ui, &t, r, CornerRadius::same(4));
                                        }
                                    }
                                }
                                if let Some(since) = info.as_ref().and_then(|i| i.member_since) {
                                    ui.add_space(6.0);
                                    ui.label(w::text(t!("Member since {date}", date = ui::ago(since)), 12.5, t.text_faint));
                                }
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    if !mine && w::button(ui, &t, Some(icons::CHAT), t!("Send a message"), w::Kind::Primary, w::Size::Sm).clicked() {
                                        ctx.act(Action::OpenChat(id));
                                    }
                                    let label = if mine { t!("Edit profile on Steam") } else { t!("View on Steam") };
                                    if w::button(ui, &t, Some(icons::EXTERNAL), label, w::Kind::Secondary, w::Size::Sm).clicked() {
                                        let url = if mine { format!("https://steamcommunity.com/profiles/{id}/edit/info") } else { format!("https://steamcommunity.com/profiles/{id}") };
                                        let _ = open::that_detached(url);
                                    }
                                    if w::button(ui, &t, Some(icons::COPY), t!("Copy SteamID"), w::Kind::Ghost, w::Size::Sm).clicked() {
                                        ui.ctx().copy_text(id.to_string());
                                        ctx.toast(t!("SteamID copied"));
                                    }
                                });
                            });
                        });
                    });
                ui.add_space(16.0);

                // Two columns, like Steam: showcases on the left, links and
                // friends on the right.
                let side = 320.0;
                let gap = 18.0;
                let main_w = ui.available_width() - side - gap;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(vec2(main_w, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(main_w);
                        if persona.in_game() {
                            let game = persona.game_name.clone().or_else(|| persona.game_app_id.and_then(|g| ctx.state.app_name(g).map(String::from))).unwrap_or_default();
                            panel(ui, &t, |ui| {
                                ui.horizontal(|ui| {
                                    if let Some(g) = persona.game_app_id {
                                        let (r, _) = ui.allocate_exact_size(vec2(184.0, 69.0), Sense::hover());
                                        w::image_or_skeleton(ui, &t, r, &art::header(g), t.corner_small());
                                    }
                                    ui.vertical(|ui| {
                                        ui.label(w::text_bold(t!("CURRENTLY IN GAME"), 11.0, t.in_game));
                                        ui.label(w::text_semibold(game, 18.0, t.text));
                                    });
                                });
                            });
                            ui.add_space(14.0);
                        }
                        if open {
                            self.recent_games(ui, &t, ctx, id);
                            ui.add_space(14.0);
                            self.showcase(ui, &t, ctx, id);
                            ui.add_space(14.0);
                            self.most_played(ui, &t, ctx, id);
                            ui.add_space(14.0);
                            self.comments(ui, &t, ctx, id, mine);
                        } else {
                            panel(ui, &t, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(w::text(icons::LOCK_SIMPLE, 26.0, t.text_dim));
                                    ui.add_space(6.0);
                                    ui.vertical(|ui| {
                                        let title = if visibility == Visibility::FriendsOnly { t!("This profile is visible to friends only") } else { t!("This profile is private") };
                                        ui.label(w::text_bold(title, 16.0, t.text));
                                        ui.label(w::text(t!("Games, inventory and friends are hidden by its owner."), 13.0, t.text_dim));
                                    });
                                });
                            });
                        }
                    });
                    ui.allocate_ui_with_layout(vec2(side, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(side);
                        self.badges(ui, &t, ctx, id, open);
                        self.sidebar(ui, &t, ctx, id, mine, open);
                    });
                });
            });
            ui.add_space(40.0);
        });
    }
}

/// Where a profile link goes.
enum Link {
    Page(&'static str, Option<String>),
    Games,
    Web(String),
}

impl Profile {
    /// Right column: level, links with counts, and friends.
    fn sidebar(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        ctx: &mut Ctx,
        id: SteamId,
        mine: bool,
        open: bool,
    ) {
        if !open {
            return;
        }
        let owned = ctx
            .state
            .apps
            .as_ref()
            .map(|a| a.values().filter(|a| !a.is_dlc()).count());
        let games = match ctx.state.player_games.get(&id) {
            Some(Ok(g)) if mine && g.is_empty() => owned.map(|n| n.to_string()).unwrap_or_default(),
            Some(Ok(g)) => g.len().to_string(),
            Some(Err(_)) => "—".into(),
            None if mine => owned.map(|n| n.to_string()).unwrap_or_default(),
            None => String::new(),
        };
        let items: u64 = match ctx.state.inventory_apps.get(&id) {
            Some(Ok(apps)) => apps
                .iter()
                .flat_map(|a| a.contexts.iter())
                .map(|c| c.2 as u64)
                .sum(),
            _ => ctx
                .state
                .inventories
                .iter()
                .filter(|(k, _)| k.0 == id)
                .map(|(_, v)| v.total as u64)
                .sum(),
        };
        let web =
            |path: &str| Link::Web(format!("https://steamcommunity.com/profiles/{id}/{path}"));
        let friends_link = if mine {
            Link::Page("community", None)
        } else {
            web("friends/")
        };
        let friends_count = if mine {
            ctx.state.friend_list().len().to_string()
        } else {
            String::new()
        };
        let links: Vec<(&str, &str, String, Link)> = vec![
            (icons::BOOKS, t!("Games"), games, Link::Games),
            (
                icons::BACKPACK,
                t!("Inventory"),
                if items > 0 {
                    items.to_string()
                } else {
                    String::new()
                },
                Link::Page("inventory", Some(id.to_string())),
            ),
            (icons::USERS, t!("Friends"), friends_count, friends_link),
            (icons::MEDAL, t!("Badges"), String::new(), web("badges/")),
            (
                icons::CAMERA,
                t!("Screenshots"),
                String::new(),
                Link::Page("user_files", Some(format!("{id}/screenshots"))),
            ),
            (
                icons::CUBE,
                t!("Workshop items"),
                String::new(),
                Link::Page("user_files", Some(format!("{id}/workshop"))),
            ),
            (
                icons::BOOKS,
                t!("Guides"),
                String::new(),
                Link::Page("user_files", Some(format!("{id}/guides"))),
            ),
            (
                icons::THUMBS_UP,
                t!("Reviews"),
                String::new(),
                web("recommended/"),
            ),
            (
                icons::USERS_THREE,
                t!("Groups"),
                String::new(),
                web("groups/"),
            ),
        ];
        panel(ui, t, |ui| {
            for (icon, label, count, link) in links {
                let external = matches!(link, Link::Web(_));
                let (r, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                let hover = ui
                    .ctx()
                    .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
                let p = ui.painter();
                p.rect_filled(
                    r,
                    t.corner_small(),
                    w::mix(Color32::TRANSPARENT, t.surface_hover, hover),
                );
                p.text(
                    pos2(r.left() + 14.0, r.center().y),
                    Align2::CENTER_CENTER,
                    icon,
                    w::font(16.0),
                    t.accent,
                );
                p.text(
                    pos2(r.left() + 34.0, r.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    w::font_medium(14.0),
                    w::mix(t.text_dim, t.text, hover),
                );
                if !count.is_empty() {
                    p.text(
                        pos2(r.right() - 10.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        &count,
                        w::font_bold(14.0),
                        t.text,
                    );
                } else if external {
                    p.text(
                        pos2(r.right() - 10.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        icons::EXTERNAL,
                        w::font(12.0),
                        t.text_faint,
                    );
                }
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                let resp = if external {
                    resp.on_hover_text(t!("Opens in your browser"))
                } else {
                    resp
                };
                if resp.clicked() {
                    match link {
                        Link::Page(page, arg) => ctx.navigate(page, arg),
                        Link::Games => ctx.navigate("profile", Some(format!("{id}/games"))),
                        Link::Web(url) => {
                            let _ = open::that_detached(url);
                        }
                    }
                }
            }
        });
        if mine {
            let friends = ctx.state.friend_list();
            if !friends.is_empty() {
                ui.add_space(12.0);
                panel(ui, t, |ui| {
                    let online = friends
                        .iter()
                        .filter(|p| p.state != model::PersonaState::Offline)
                        .count();
                    ui.horizontal(|ui| {
                        ui.label(w::text_bold(t!("Friends"), 16.0, t.text));
                        ui.label(w::text(t!("{n} online", n = online), 12.5, t.online));
                    });
                    ui.add_space(6.0);
                    for p in friends.iter().take(8) {
                        let (r, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                        if resp.hovered() {
                            ui.painter()
                                .rect_filled(r, t.corner_small(), t.surface_hover);
                        }
                        let av = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(32.0, 32.0));
                        if !w::paint_cover(
                            ui,
                            av,
                            &art::avatar(p.avatar_hash.as_deref()),
                            CornerRadius::same(4),
                        ) {
                            w::skeleton(ui, t, av, CornerRadius::same(4));
                        }
                        let color = crate::shell::status_color(t, p);
                        ui.painter().text(
                            pos2(av.right() + 10.0, r.top() + 13.0),
                            Align2::LEFT_CENTER,
                            &p.name,
                            w::font_semibold(13.0),
                            t.text,
                        );
                        ui.painter().text(
                            pos2(av.right() + 10.0, r.top() + 29.0),
                            Align2::LEFT_CENTER,
                            crate::shell::status_text(ctx.state, p),
                            w::font(11.5),
                            color,
                        );
                        if resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            ctx.navigate("profile", Some(p.steam_id.to_string()));
                        }
                    }
                    if friends.len() > 8
                        && ui
                            .link(w::text(
                                t!("See all {n} friends", n = friends.len()),
                                12.5,
                                t.accent,
                            ))
                            .clicked()
                    {
                        ctx.navigate("community", None);
                    }
                });
            }
        }
    }

    fn request_social(&mut self, ctx: &mut Ctx, id: SteamId) {
        if self.social_requested.insert(id) {
            ctx.send(Command::LoadBadges { steam_id: id });
            ctx.send(Command::LoadComments {
                steam_id: id,
                start: 0,
            });
        }
    }

    /// Level, XP and badges, like the top of Steam's profile sidebar.
    fn badges(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, open: bool) {
        if !open {
            return;
        }
        self.request_social(ctx, id);
        let Some(Ok(b)) = ctx.state.badges.get(&id).cloned() else {
            return;
        };
        panel(ui, t, |ui| {
            ui.horizontal(|ui| {
                level_badge(ui, t, b.level);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(w::text_bold(t!("Level {n}", n = b.level), 16.0, t.text));
                    if !b.xp.is_empty() {
                        ui.label(w::text(&b.xp, 12.0, t.text_dim));
                    }
                });
            });
            if !b.next.is_empty() {
                ui.add_space(4.0);
                let width = ui.available_width();
                w::progress(ui, t, b.progress, width);
                ui.label(w::text(&b.next, 11.5, t.text_faint));
            }
            if !b.items.is_empty() {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(w::text_bold(t!("Badges"), 14.0, t.text));
                    ui.label(w::text(b.items.len().to_string(), 13.0, t.text_dim));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.link(w::text(t!("See all"), 12.5, t.accent)).clicked() {
                            self.all_badges = Some(id);
                        }
                    });
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for badge in b.items.iter().take(8) {
                        badge_icon(ui, t, badge, 54.0);
                    }
                });
            }
        });
        ui.add_space(12.0);
        if self.all_badges == Some(id) {
            let mut close = false;
            egui::Modal::new(egui::Id::unique(("all-badges", id))).show(ui.ctx(), |ui| {
                ui.set_width(520.0);
                ui.horizontal(|ui| {
                    ui.label(w::text_bold(t!("Badges"), 20.0, t.text));
                    ui.label(w::text(b.items.len().to_string(), 15.0, t.text_dim));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::icon_button(ui, t, icons::X, t!("Close"), false, 0).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(520.0)
                    .show(ui, |ui| {
                        for badge in &b.items {
                            let (r, resp) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 70.0),
                                Sense::click(),
                            );
                            if resp.hovered() {
                                ui.painter()
                                    .rect_filled(r, t.corner_small(), t.surface_hover);
                            }
                            let img = Rect::from_min_size(r.min + vec2(6.0, 5.0), vec2(60.0, 60.0));
                            match &badge.icon {
                                Some(u) => {
                                    if !w::paint_contain(ui, img, u, Align2::CENTER_CENTER) {
                                        w::skeleton(ui, t, img, CornerRadius::same(6));
                                    }
                                }
                                None => w::placeholder(ui, t, img, "", CornerRadius::same(6)),
                            }
                            let x = img.right() + 12.0;
                            ui.painter().text(
                                pos2(x, r.top() + 18.0),
                                Align2::LEFT_CENTER,
                                &badge.title,
                                w::font_semibold(14.0),
                                t.text,
                            );
                            ui.painter().text(
                                pos2(x, r.top() + 36.0),
                                Align2::LEFT_CENTER,
                                &badge.detail,
                                w::font(12.5),
                                t.accent,
                            );
                            ui.painter().text(
                                pos2(x, r.top() + 53.0),
                                Align2::LEFT_CENTER,
                                &badge.unlocked,
                                w::font(11.5),
                                t.text_faint,
                            );
                            if !badge.url.is_empty()
                                && resp
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(t!("Opens in your browser"))
                                    .clicked()
                            {
                                let _ = open::that_detached(&badge.url);
                            }
                        }
                    });
            });
            if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.all_badges = None;
            }
        }
    }

    /// Profile comments, with posting. Respects profiles that hide or turn
    /// off comments (Steam answers with the reason, which is shown).
    fn comments(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, mine: bool) {
        self.request_social(ctx, id);
        panel(ui, t, |ui| {
            let total = match ctx.state.comments.get(&id) {
                Some(Ok(c)) => c.total,
                _ => 0,
            };
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("Comments"), 17.0, t.text));
                if total > 0 {
                    ui.label(w::text(total.to_string(), 14.0, t.text_dim));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::icon_button(ui, t, icons::REFRESH, t!("Refresh"), false, 0).clicked() {
                        self.comment_start.remove(&id);
                        ctx.send(Command::LoadComments {
                            steam_id: id,
                            start: 0,
                        });
                    }
                });
            });
            ui.add_space(6.0);
            let data = match ctx.state.comments.get(&id) {
                None => {
                    for _ in 0..2 {
                        let (r, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
                        w::skeleton(ui, t, r.shrink2(vec2(0.0, 4.0)), t.corner_small());
                    }
                    return;
                }
                Some(Err(e)) => {
                    let why = if e.to_lowercase().contains("private") {
                        t!("This profile's comments are private.").to_string()
                    } else {
                        t!(
                            "Comments aren't available on this profile: {reason}",
                            reason = i18n::tr_dyn(e)
                        )
                    };
                    ui.horizontal(|ui| {
                        ui.label(w::text(icons::LOCK_SIMPLE, 16.0, t.text_faint));
                        ui.label(w::text(why, 13.0, t.text_dim));
                    });
                    return;
                }
                Some(Ok(c)) => c.clone(),
            };
            // Composer.
            let hint = if mine {
                t!("Write a comment on your profile…")
            } else {
                t!("Add a comment…")
            };
            ui.add(
                egui::TextEdit::multiline(&mut self.comment_draft)
                    .hint_text(hint)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .margin(vec2(10.0, 8.0)),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let text = self.comment_draft.trim().to_string();
                ui.add_enabled_ui(!text.is_empty() && text.chars().count() <= 1000, |ui| {
                    if w::button(
                        ui,
                        t,
                        Some(icons::SEND),
                        t!("Post comment"),
                        w::Kind::Primary,
                        w::Size::Sm,
                    )
                    .clicked()
                    {
                        ctx.send(Command::PostComment { steam_id: id, text });
                    }
                });
                ui.label(w::text(
                    format!("{}/1000", self.comment_draft.chars().count()),
                    11.5,
                    t.text_faint,
                ));
            });
            ui.add_space(8.0);
            if data.items.is_empty() {
                ui.label(w::text(t!("No comments yet."), 13.0, t.text_faint));
            }
            for c in &data.items {
                ui.separator();
                ui.horizontal_top(|ui| {
                    let (av, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                    match &c.avatar {
                        Some(u) => {
                            if !w::paint_cover(ui, av, u, CornerRadius::same(4)) {
                                w::skeleton(ui, t, av, CornerRadius::same(4));
                            }
                        }
                        None => w::placeholder(ui, t, av, "", CornerRadius::same(4)),
                    }
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        ctx.navigate("profile", Some(c.author.to_string()));
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Label::new(w::text_semibold(&c.name, 13.5, t.accent))
                                        .sense(Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                ctx.navigate("profile", Some(c.author.to_string()));
                            }
                            if c.time > 0 {
                                ui.label(w::text(ui::ago(c.time), 11.5, t.text_faint));
                            }
                            // Steam lets you delete any comment on your profile, and your own anywhere.
                            if mine || c.author == ctx.state.steam_id {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if w::icon_button(
                                        ui,
                                        t,
                                        icons::TRASH,
                                        t!("Delete comment"),
                                        false,
                                        0,
                                    )
                                    .clicked()
                                    {
                                        ctx.send(Command::DeleteComment {
                                            steam_id: id,
                                            comment_id: c.id,
                                        });
                                    }
                                });
                            }
                        });
                        w::markup_label(ui, c.html.trim(), 13.5, t.text);
                    });
                });
            }
            // Older and newer pages, 20 at a time (like Steam's comment pages).
            let start = self.comment_start.get(&id).copied().unwrap_or(0);
            if data.total as usize > data.items.len() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let mut go = None;
                    ui.add_enabled_ui(start > 0, |ui| {
                        if w::button(
                            ui,
                            t,
                            Some(icons::CARET_LEFT),
                            t!("Newer"),
                            w::Kind::Ghost,
                            w::Size::Sm,
                        )
                        .clicked()
                        {
                            go = Some(start.saturating_sub(20));
                        }
                    });
                    let page = start / 20 + 1;
                    let pages = data.total.div_ceil(20).max(1);
                    ui.label(w::text(
                        t!("Page {page} of {pages}", page = page, pages = pages),
                        12.5,
                        t.text_dim,
                    ));
                    ui.add_enabled_ui(start + 20 < data.total, |ui| {
                        if w::button(
                            ui,
                            t,
                            Some(icons::CARET_RIGHT),
                            t!("Older"),
                            w::Kind::Ghost,
                            w::Size::Sm,
                        )
                        .clicked()
                        {
                            go = Some(start + 20);
                        }
                    });
                    if let Some(s) = go {
                        self.comment_start.insert(id, s);
                        ctx.send(Command::LoadComments {
                            steam_id: id,
                            start: s,
                        });
                    }
                });
            }
        });
    }

    /// A few items from the Steam inventory, like Steam's item showcase.
    fn showcase(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId) {
        if self.showcase_requested.insert(id) {
            ctx.send(Command::LoadInventory {
                steam_id: id,
                app_id: 753,
                context_id: 6,
                language: i18n::language().steam_name().into(),
            });
        }
        let Some(inv) = ctx.state.inventories.get(&(id, 753, 6)) else {
            return;
        };
        if inv.items.is_empty() || inv.error.is_some() {
            return;
        }
        // Prefer backgrounds and emoticons, then anything colorful.
        let mut items: Vec<&model::InventoryItem> = inv.items.iter().collect();
        items.sort_by_key(|i| {
            std::cmp::Reverse((
                i.kind.to_lowercase().contains("background") as u8,
                i.marketable as u8,
            ))
        });
        panel(ui, t, |ui| {
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("Item showcase"), 17.0, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .link(w::text(t!("Open inventory"), 12.5, t.accent))
                        .clicked()
                    {
                        ctx.navigate("inventory", Some(id.to_string()));
                    }
                });
            });
            ui.add_space(8.0);
            let size = 84.0;
            let cols = (((ui.available_width() + 8.0) / (size + 8.0)).floor() as usize).max(1);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for item in items.iter().take(cols) {
                    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                    ui.painter().rect_filled(r, t.corner_small(), t.raised);
                    if !w::paint_contain(
                        ui,
                        r.shrink(6.0),
                        &art::econ(&item.icon, 192),
                        Align2::CENTER_CENTER,
                    ) {
                        w::skeleton(ui, t, r.shrink(6.0), CornerRadius::same(4));
                    }
                    if let Some(c) = crate::builtin::inventory::rarity_color(item) {
                        let bar = Rect::from_min_max(
                            pos2(r.left() + 8.0, r.bottom() - 5.0),
                            pos2(r.right() - 8.0, r.bottom() - 2.5),
                        );
                        ui.painter().rect_filled(bar, CornerRadius::same(2), c);
                    }
                    if resp
                        .on_hover_text(format!("{}\n{}", item.name, item.kind))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        ctx.navigate("inventory", Some(format!("{id}/753")));
                    }
                }
            });
        });
    }

    /// Most played games by total hours.
    fn most_played(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId) {
        let mut games: Vec<(model::AppId, String, u64)> = match ctx.state.player_games.get(&id) {
            Some(Ok(g)) if !g.is_empty() => g
                .iter()
                .map(|g| (g.app_id, g.name.clone(), g.playtime_minutes))
                .collect(),
            _ if id == ctx.state.steam_id => ctx
                .local
                .games
                .iter()
                .map(|g| (g.app_id, g.name.clone(), g.playtime_minutes.unwrap_or(0)))
                .collect(),
            _ => return,
        };
        games.retain(|g| g.2 > 0);
        if games.is_empty() {
            return;
        }
        games.sort_by_key(|g| std::cmp::Reverse(g.2));
        let max = games[0].2.max(1) as f32;
        panel(ui, t, |ui| {
            ui.label(w::text_bold(t!("Most played"), 17.0, t.text));
            ui.add_space(6.0);
            for (app, name, minutes) in games.iter().take(6) {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(68.0, 32.0), Sense::hover());
                    w::image_or_skeleton(ui, t, r, &art::header(*app), CornerRadius::same(4));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(w::text_medium(name, 13.5, t.text)).truncate());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(w::text(ui::playtime(*minutes), 12.5, t.text_dim));
                            });
                        });
                        let width = ui.available_width();
                        w::progress(ui, t, *minutes as f32 / max, width);
                    });
                });
                ui.add_space(4.0);
            }
        });
    }

    /// Recently played games on someone else's profile, like Steam's.
    fn recent_games(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId) {
        let Some(result) = ctx.state.player_games.get(&id).cloned() else {
            panel(ui, t, |ui| {
                ui.label(w::text_bold(t!("Recent activity"), 17.0, t.text));
                for _ in 0..3 {
                    let (r, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
                    w::skeleton(ui, t, r.shrink2(vec2(0.0, 4.0)), t.corner_small());
                }
            });
            return;
        };
        let Ok(mut games) = result else { return };
        // Your own profile: fall back to this PC's play history.
        if games.is_empty() && id == ctx.state.steam_id {
            games = ctx
                .local
                .games
                .iter()
                .map(|g| model::PlayerGame {
                    app_id: g.app_id,
                    name: g.name.clone(),
                    playtime_minutes: g.playtime_minutes.unwrap_or(0),
                    playtime_2weeks: 0,
                    last_played: g.last_played,
                    icon: None,
                })
                .collect();
        }
        let total_hours: u64 = games.iter().map(|g| g.playtime_minutes).sum::<u64>() / 60;
        let recent_minutes: u64 = games.iter().map(|g| g.playtime_2weeks).sum();
        games.sort_by_key(|g| std::cmp::Reverse((g.playtime_2weeks, g.last_played)));
        panel(ui, t, |ui| {
            ui.horizontal(|ui| {
                ui.label(w::text_bold(t!("Recent activity"), 17.0, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(w::text(
                        t!(
                            "{hours} hours past 2 weeks · {total} hours total",
                            hours = format!("{:.1}", recent_minutes as f64 / 60.0),
                            total = total_hours
                        ),
                        12.5,
                        t.text_dim,
                    ));
                });
            });
            ui.add_space(6.0);
            let shown: Vec<_> = games
                .iter()
                .filter(|g| g.last_played.is_some() || g.playtime_2weeks > 0)
                .take(4)
                .collect();
            if shown.is_empty() {
                ui.label(w::text(t!("No recent games."), 13.5, t.text_faint));
            }
            for g in shown {
                game_row(ui, t, ctx, g);
            }
            ui.add_space(6.0);
            if ui
                .link(w::text(
                    t!("See all {n} games", n = games.len()),
                    13.0,
                    t.accent,
                ))
                .clicked()
            {
                ctx.navigate("profile", Some(format!("{id}/games")));
            }
        });
    }

    /// Full games list for a profile.
    fn games_page(
        &mut self,
        ui: &mut Ui,
        t: &Theme,
        ctx: &mut Ctx,
        id: SteamId,
        name: &str,
        mine: bool,
    ) {
        ui.horizontal(|ui| {
            if w::button(
                ui,
                t,
                Some(icons::ARROW_LEFT),
                t!("Back to profile"),
                w::Kind::Ghost,
                w::Size::Sm,
            )
            .clicked()
            {
                ctx.navigate("profile", Some(id.to_string()));
            }
        });
        ui.add_space(8.0);
        panel(ui, t, |ui| {
            let title = if mine {
                t!("Your games").to_string()
            } else {
                t!("{name}'s games", name = name)
            };
            ui.label(w::text_bold(title, 24.0, t.text));
            let games = match ctx.state.player_games.get(&id).cloned() {
                None => {
                    for _ in 0..8 {
                        let (r, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
                        w::skeleton(ui, t, r.shrink2(vec2(0.0, 4.0)), t.corner_small());
                    }
                    return;
                }
                Some(Err(e)) => {
                    let body = if e == "private" {
                        t!("Its owner has chosen not to show their games.").to_string()
                    } else {
                        e
                    };
                    w::empty(
                        ui,
                        t,
                        icons::LOCK_SIMPLE,
                        t!("These games are hidden"),
                        &body,
                    );
                    return;
                }
                Some(Ok(g)) => g,
            };
            let hours: u64 = games.iter().map(|g| g.playtime_minutes).sum::<u64>() / 60;
            let shared = if mine {
                0
            } else {
                games
                    .iter()
                    .filter(|g| {
                        ctx.state
                            .apps
                            .as_ref()
                            .is_some_and(|a| a.contains_key(&g.app_id))
                    })
                    .count()
            };
            let mut sub = t!(
                "{n} games · {hours} hours on record",
                n = games.len(),
                hours = hours
            );
            if !mine && shared > 0 {
                sub = format!("{sub} · {}", t!("{n} in common with you", n = shared));
            }
            ui.label(w::text(sub, 13.5, t.text_dim));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                w::search(ui, t, &mut self.games_search, &t!("Search games"), 280.0);
                ui.add_space(8.0);
                w::segmented(
                    ui,
                    t,
                    &mut self.games_sort,
                    &[
                        (GameSort::Recent, &t!("Recently played")),
                        (GameSort::Playtime, &t!("Playtime")),
                        (GameSort::Name, &t!("Name")),
                    ],
                );
            });
            ui.add_space(10.0);
            let q = self.games_search.trim().to_lowercase();
            let mut list: Vec<_> = games
                .iter()
                .filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q))
                .collect();
            match self.games_sort {
                GameSort::Recent => {
                    list.sort_by_key(|g| std::cmp::Reverse(g.last_played.unwrap_or(0)))
                }
                GameSort::Playtime => list.sort_by_key(|g| std::cmp::Reverse(g.playtime_minutes)),
                GameSort::Name => {
                    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                }
            }
            if list.is_empty() {
                ui.label(w::text(
                    t!("No games match your search."),
                    13.5,
                    t.text_faint,
                ));
            }
            let row_h = 62.0;
            egui::ScrollArea::vertical()
                .id_salt(("profile-games", id))
                .max_height(ui.ctx().content_rect().height() - 260.0)
                .auto_shrink([false, true])
                .show_rows(ui, row_h, list.len(), |ui, range| {
                    for g in &list[range] {
                        game_row(ui, t, ctx, g);
                    }
                });
        });
    }
}

/// One game with playtime; opens it in your library if you own it, else the store.
fn game_row(ui: &mut Ui, t: &Theme, ctx: &mut Ctx, g: &model::PlayerGame) {
    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    ui.painter().rect_filled(
        row,
        t.corner_small(),
        w::mix(Color32::TRANSPARENT, t.surface_hover, hover),
    );
    let img = Rect::from_min_size(row.min + vec2(6.0, 8.0), vec2(98.0, 46.0));
    w::image_or_skeleton(ui, t, img, &art::header(g.app_id), t.corner_small());
    let owned = ctx
        .state
        .apps
        .as_ref()
        .is_some_and(|a| a.contains_key(&g.app_id));
    let p = ui.painter();
    p.text(
        pos2(img.right() + 12.0, row.top() + 20.0),
        Align2::LEFT_CENTER,
        &g.name,
        w::font_semibold(14.0),
        t.text,
    );
    let mut sub = t!("{time} on record", time = ui::playtime(g.playtime_minutes));
    if g.playtime_2weeks > 0 {
        sub = format!(
            "{sub} · {}",
            t!(
                "{time} past 2 weeks",
                time = ui::playtime(g.playtime_2weeks)
            )
        );
    }
    if let Some(when) = g.last_played {
        sub = format!("{sub} · {}", t!("last played {when}", when = ui::ago(when)));
    }
    p.text(
        pos2(img.right() + 12.0, row.top() + 42.0),
        Align2::LEFT_CENTER,
        sub,
        w::font(12.0),
        t.text_dim,
    );
    if owned {
        p.text(
            pos2(row.right() - 12.0, row.center().y),
            Align2::RIGHT_CENTER,
            format!("{}  {}", icons::CHECK, t!("In your library")),
            w::font_semibold(12.0),
            t.online,
        );
    }
    if resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        if owned {
            ctx.navigate("library", Some(g.app_id.to_string()));
        } else {
            ctx.navigate("store", Some(format!("app:{}", g.app_id)));
        }
    }
}

/// Translucent panel so the profile background shows through.
fn panel(ui: &mut Ui, t: &Theme, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(t.chrome.gamma_multiply(0.85))
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(t.corner())
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// Steam-style level circle; color steps every 10 levels.
fn level_badge(ui: &mut Ui, t: &Theme, level: u32) {
    let colors = [
        0x9B9B9B, 0xC02942, 0xD95B43, 0xFECC23, 0x467A3C, 0x4E8DDB, 0x7652C9, 0xC252C9, 0x542437,
        0x997C52,
    ];
    let c = colors[(level as usize / 10) % colors.len()];
    let color = Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let (r, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
    ui.painter()
        .circle_stroke(r.center(), 16.0, Stroke::new(2.5, color));
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        level.to_string(),
        w::font_bold(13.5),
        t.text,
    );
}

/// A badge picture with its name and details on hover.
fn badge_icon(ui: &mut Ui, t: &Theme, badge: &model::Badge, size: f32) {
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    match &badge.icon {
        Some(u) => {
            if !w::paint_contain(ui, r, u, Align2::CENTER_CENTER) {
                w::skeleton(ui, t, r, CornerRadius::same(6));
            }
        }
        None => w::placeholder(ui, t, r, "", CornerRadius::same(6)),
    }
    let tip = [
        badge.title.as_str(),
        badge.detail.as_str(),
        badge.unlocked.as_str(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    if resp
        .on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && !badge.url.is_empty()
    {
        let _ = open::that_detached(&badge.url);
    }
}

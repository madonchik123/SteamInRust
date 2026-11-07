//! Shared files: the hub's All / Screenshots / Artwork / Videos / News tabs,
//! the guide list, and the page of one Workshop item, screenshot or guide.

use super::{FILM, HubTab, HubView, IMAGES, Io, ListState, NEWSPAPER, THUMBS_DOWN, View, author, columns, error, file_query, pager, rich, stars};
use client::Command;
use client::hub::{self, FileEntry, FileKind, Request};
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, vec2};
use i18n::t;
use model::AppId;
use ui::{Theme, icons, widgets as w};

/// Steam's standard guide categories (the tag values stay in English).
const GUIDE_CATEGORIES: [&str; 17] = [
    "Achievements",
    "Characters",
    "Classes",
    "Co-op",
    "Crafting",
    "Game Modes",
    "Gameplay Basics",
    "Loot",
    "Maps or Levels",
    "Modding or Configuration",
    "Multiplayer",
    "Secrets",
    "Story or Lore",
    "Trading",
    "Walkthroughs",
    "Weapons",
    "Workshop",
];

fn category_label(tag: &str) -> String {
    match tag {
        "Achievements" => t!("Achievements"),
        "Characters" => t!("Characters"),
        "Classes" => t!("Classes"),
        "Co-op" => t!("Co-op"),
        "Crafting" => t!("Crafting"),
        "Game Modes" => t!("Game Modes"),
        "Gameplay Basics" => t!("Gameplay Basics"),
        "Loot" => t!("Loot"),
        "Maps or Levels" => t!("Maps or Levels"),
        "Modding or Configuration" => t!("Modding or Configuration"),
        "Multiplayer" => t!("Multiplayer"),
        "Secrets" => t!("Secrets"),
        "Story or Lore" => t!("Story or Lore"),
        "Trading" => t!("Trading"),
        "Walkthroughs" => t!("Walkthroughs"),
        "Weapons" => t!("Weapons"),
        "Workshop" => t!("Workshop"),
        other => return other.to_string(),
    }
    .to_string()
}

fn kind_label(kind: FileKind) -> String {
    match kind {
        FileKind::Screenshots => t!("Screenshot"),
        FileKind::Art => t!("Artwork"),
        FileKind::Videos => t!("Video"),
        FileKind::Guides => t!("Guide"),
        FileKind::Items | FileKind::Collections => t!("Workshop"),
    }
    .to_string()
}

fn tab_kind(tab: HubTab) -> FileKind {
    match tab {
        HubTab::Artwork => FileKind::Art,
        HubTab::Videos => FileKind::Videos,
        HubTab::Guides => FileKind::Guides,
        HubTab::Workshop => FileKind::Items,
        _ => FileKind::Screenshots,
    }
}

/// Load `pages` pages of a query (for "load more" lists); returns what's loaded.
fn load_pages(hv: &mut HubView, io: &mut Io, app: AppId, kind: FileKind, l: &ListState, per_page: u32, pages: u32) -> (Vec<FileEntry>, u64, bool, Option<String>) {
    let mut items = Vec::new();
    let mut total = 0;
    let mut loading = false;
    let mut err = None;
    for page in 1..=pages.max(1) {
        let q = file_query(app, kind, l, per_page, page, io.lang());
        let key = q.key();
        match io.state.hub.files.get(&key) {
            Some(Ok(p)) => {
                total = p.total;
                items.extend(p.items.iter().cloned());
            }
            Some(Err(e)) => err = Some(e.clone()),
            None => {
                loading = true;
                hv.ask(io, key, false, || Request::Files(q));
            }
        }
    }
    (items, total, loading, err)
}

pub(super) fn hub_tab(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId, tab: HubTab) {
    match tab {
        HubTab::All => all(hv, ui, t, io, app),
        HubTab::News => news(hv, ui, t, io, app),
        HubTab::Screenshots | HubTab::Artwork | HubTab::Videos => media_tab(hv, ui, t, io, app, tab),
        // These have their own pages.
        HubTab::Guides => guides(hv, ui, t, io, app),
        HubTab::Discussions | HubTab::Workshop => {}
    }
}

/// The hub's front: this week's most popular of everything, mixed like Steam's.
fn all(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId) {
    let l = ListState::default();
    let sets = [(FileKind::Screenshots, 12), (FileKind::Art, 8), (FileKind::Guides, 4), (FileKind::Videos, 4), (FileKind::Items, 4)];
    let mut lists: Vec<(FileKind, Vec<FileEntry>)> = Vec::new();
    let mut loading = false;
    for (kind, n) in sets {
        let (items, _, busy, _) = load_pages(hv, io, app, kind, &l, n, 1);
        loading |= busy;
        lists.push((kind, items));
    }
    // Interleave: two screenshots, then one of the others, and so on.
    let mut mixed: Vec<(FileKind, FileEntry)> = Vec::new();
    let mut idx = vec![0usize; lists.len()];
    loop {
        let before = mixed.len();
        for (i, take) in [(0, 2), (1, 1), (2, 1), (0, 1), (3, 1), (1, 1), (4, 1)] {
            for _ in 0..take {
                if let Some(e) = lists[i].1.get(idx[i]) {
                    mixed.push((lists[i].0, e.clone()));
                    idx[i] += 1;
                }
            }
        }
        if mixed.len() == before {
            break;
        }
    }
    ui.horizontal(|ui| {
        ui.label(w::text_bold(t!("MOST POPULAR THIS WEEK"), 12.0, t.text_faint));
    });
    ui.add_space(4.0);
    if mixed.is_empty() {
        if loading {
            w::loading(ui, t, &t!("Loading…"));
        } else {
            w::empty(ui, t, icons::USERS, t!("Nothing here yet"), t!("This game's community hasn't shared anything yet."));
        }
        return;
    }
    grid(hv, ui, t, io, &mixed, 300.0);
}

fn grid(_hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, items: &[(FileKind, FileEntry)], card_w: f32) {
    let gap = 14.0;
    let cols = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
    let cw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    for row in items.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (kind, e) in row {
                if ui.vertical(|ui| card(ui, t, io, *kind, e, cw)).inner {
                    io.go(View::Item(e.id));
                }
            }
        });
        ui.add_space(gap);
    }
}

/// One tile of shared content: picture, title, author, votes.
fn card(ui: &mut Ui, t: &Theme, io: &mut Io, kind: FileKind, e: &FileEntry, width: f32) -> bool {
    let mut open = false;
    egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).show(ui, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 4.0;
        let (img, resp) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), Sense::click());
        let radius = CornerRadius { nw: t.corner_small().nw, ne: t.corner_small().ne, sw: 0, se: 0 };
        let url = e.preview.clone().or_else(|| e.image.clone()).or_else(|| e.youtube.as_ref().map(|y| format!("https://img.youtube.com/vi/{y}/hqdefault.jpg")));
        match &url {
            Some(u) => w::image_or_skeleton(ui, t, img, u, radius),
            None => w::placeholder(ui, t, img, &e.title, radius),
        }
        if e.spoiler {
            ui.painter().rect_filled(img, radius, Color32::from_black_alpha(230));
            ui.painter().text(img.center(), Align2::CENTER_CENTER, t!("Spoiler"), w::font_semibold(15.0), t.text_dim);
        }
        if kind == FileKind::Videos {
            ui.painter().circle_filled(img.center(), 22.0, Color32::from_black_alpha(170));
            ui.painter().text(img.center(), Align2::CENTER_CENTER, icons::PLAY, w::font(18.0), Color32::WHITE);
        }
        // What it is, top left.
        let label = kind_label(kind);
        let g = ui.painter().layout_no_wrap(label.to_uppercase(), w::font_bold(10.0), Color32::WHITE);
        let chip = Rect::from_min_size(img.min + vec2(8.0, 8.0), g.size() + vec2(12.0, 6.0));
        ui.painter().rect_filled(chip, CornerRadius::same(3), Color32::from_black_alpha(170));
        ui.painter().galley(chip.min + vec2(6.0, 3.0), g, Color32::WHITE);
        if resp.hovered() {
            ui.painter().rect_stroke(img, radius, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
        open |= resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 4, bottom: 10 }).show(ui, |ui| {
            ui.set_width(width - 20.0);
            let title = if e.title.trim().is_empty() { e.description.lines().next().unwrap_or_default().to_string() } else { e.title.clone() };
            if !title.trim().is_empty() && ui.add(egui::Label::new(w::text_semibold(title, 13.5, t.text)).truncate().sense(Sense::click())).clicked() {
                open = true;
            }
            if kind == FileKind::Guides || kind == FileKind::Items {
                stars(ui, t, e.votes_up, e.votes_down, 11.5);
            }
            ui.horizontal(|ui| {
                author(ui, t, io, e.creator, "", 20.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if e.comments > 0 {
                        ui.label(w::text(format!("{}  {}", icons::CHAT, e.comments), 11.5, t.text_faint));
                    }
                    ui.label(w::text(format!("{}  {}", icons::THUMBS_UP, e.votes_up), 11.5, t.text_faint));
                });
            });
        });
    });
    open
}

/// Popular / newest controls shared by the media tabs and guides.
fn sort_bar(ui: &mut Ui, t: &Theme, l: &mut ListState, sorts: &[(u32, String)]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (sort, label) in sorts {
            if w::chip(ui, t, label, l.sort == *sort && l.submitted.is_empty()).clicked() {
                l.sort = *sort;
                l.submitted.clear();
                l.search.clear();
                changed = true;
            }
        }
        if l.sort == 3 && l.submitted.is_empty() {
            ui.add_space(6.0);
            let before = l.days;
            w::segmented(ui, t, &mut l.days, &[(1, &t!("Today")), (7, &t!("Week")), (30, &t!("Month")), (90, &t!("3 months")), (365, &t!("Year"))]);
            changed |= before != l.days;
        }
    });
    changed
}

fn media_tab(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId, tab: HubTab) {
    let kind = tab_kind(tab);
    let mut l = hv.lists.entry((app, tab)).or_default().clone();
    let sorts = [(3, t!("Most popular").to_string()), (1, t!("Most recent").to_string()), (0, t!("Most liked").to_string())];
    if sort_bar(ui, t, &mut l, &sorts) {
        l.page = 1;
    }
    hv.lists.insert((app, tab), l.clone());
    ui.add_space(10.0);
    let per = 24;
    let pages = l.page.max(1);
    let (items, total, loading, err) = load_pages(hv, io, app, kind, &l, per, pages);
    if let (true, Some(e)) = (items.is_empty(), &err) {
        if error(ui, t, e) {
            let q = file_query(app, kind, &l, per, 1, io.lang());
            hv.retry(io, Request::Files(q));
        }
        return;
    }
    if items.is_empty() {
        if loading {
            w::loading(ui, t, &t!("Loading…"));
        } else {
            let icon = match kind {
                FileKind::Videos => FILM,
                FileKind::Art => IMAGES,
                _ => icons::CAMERA,
            };
            w::empty(ui, t, icon, t!("Nothing here yet"), t!("This game's community hasn't shared anything yet."));
        }
        return;
    }
    let mixed: Vec<(FileKind, FileEntry)> = items.into_iter().map(|e| (kind, e)).collect();
    grid(hv, ui, t, io, &mixed, 280.0);
    if (mixed.len() as u64) < total {
        ui.vertical_centered(|ui| {
            if loading {
                ui.add(egui::Spinner::new().color(t.accent));
            } else if w::button(ui, t, Some(icons::CARET_DOWN), t!("Show more"), w::Kind::Secondary, w::Size::Md).clicked() {
                if let Some(l) = hv.lists.get_mut(&(app, tab)) {
                    l.page += 1;
                }
            }
        });
    }
}

/// The game's news, newest first; a post opens right here.
fn news(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId) {
    let have = io.state.hub.news.contains_key(&app);
    hv.ask(io, format!("news{app}"), have, || Request::News { app_id: app, count: 20 });
    let list = match io.state.hub.news.get(&app) {
        None => return w::loading(ui, t, &t!("Loading…")),
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, Request::News { app_id: app, count: 20 });
            }
            return;
        }
        Some(Ok(l)) if l.is_empty() => return w::empty(ui, t, NEWSPAPER, t!("No news yet"), t!("This game hasn't posted any news.")),
        Some(Ok(l)) => l.clone(),
    };
    for n in &list {
        let open = hv.news_open.as_deref() == Some(n.id.as_str());
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let head = ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(w::text_bold(n.feed.to_uppercase(), 10.5, t.accent));
                    ui.add(egui::Label::new(w::text_bold(&n.title, 17.0, t.text)).wrap());
                    let by = if n.author.is_empty() { ui::date(n.time) } else { format!("{} · {}", ui::date(n.time), n.author) };
                    ui.label(w::text(by, 12.0, t.text_faint));
                });
            });
            let toggle = head.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            ui.add_space(6.0);
            let doc = rich::parse(&n.contents);
            if open {
                if let Some(link) = rich::show(ui, t, &doc, 14.0) {
                    hv.follow(io, &link);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if w::button(ui, t, Some(icons::CARET_UP), t!("Show less"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        hv.news_open = None;
                    }
                    if n.external && !n.url.is_empty() && w::button(ui, t, Some(icons::EXTERNAL), t!("Read on the original site"), w::Kind::Ghost, w::Size::Sm).clicked() {
                        let _ = open::that_detached(&n.url);
                    }
                });
            } else {
                // First picture and a few lines, like Steam's news list.
                let img = doc.blocks.iter().find_map(|b| if let rich::Block::Image(u) = b { Some(u.clone()) } else { None });
                ui.horizontal_top(|ui| {
                    if let Some(u) = &img {
                        let (r, _) = ui.allocate_exact_size(vec2(200.0, 112.0), Sense::hover());
                        w::image_or_skeleton(ui, t, r, u, CornerRadius::same(4));
                        ui.add_space(10.0);
                    }
                    let plain = hub::strip_tags(&n.contents.replace("[", "<").replace("]", ">"));
                    let short: String = plain.chars().take(320).collect();
                    ui.add(egui::Label::new(w::text(if plain.len() > short.len() { format!("{short}…") } else { short }, 13.0, t.text_dim)).wrap());
                });
                ui.add_space(6.0);
                if w::button(ui, t, Some(icons::CARET_DOWN), t!("Read more"), w::Kind::Ghost, w::Size::Sm).clicked() || toggle {
                    hv.news_open = Some(n.id.clone());
                }
            }
        });
        ui.add_space(10.0);
    }
}

/// A game's guides: sort, time range, search, categories, like Steam's.
pub(super) fn guides(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, app: AppId) {
    let mut l = hv.lists.entry((app, HubTab::Guides)).or_default().clone();
    let mut changed = false;
    let sorts = [(3, t!("Most popular").to_string()), (0, t!("Top rated").to_string()), (1, t!("Most recent").to_string()), (21, t!("Recently updated").to_string())];
    let search = |ui: &mut Ui, l: &mut ListState, width: f32| {
        let r = w::search(ui, t, &mut l.search, &t!("Search guides"), width);
        r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
    };
    // Search beside the sorts when there's room, under them otherwise.
    if ui.available_width() >= 1150.0 {
        ui.horizontal(|ui| {
            changed |= sort_bar(ui, t, &mut l, &sorts);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if search(ui, &mut l, 260.0) {
                    l.submitted = l.search.trim().to_string();
                    changed = true;
                }
            });
        });
    } else {
        changed |= sort_bar(ui, t, &mut l, &sorts);
        ui.add_space(4.0);
        if search(ui, &mut l, 300.0) {
            l.submitted = l.search.trim().to_string();
            changed = true;
        }
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.label(w::text_bold(t!("CATEGORY"), 11.0, t.text_faint));
        for tag in GUIDE_CATEGORIES {
            let on = l.tags.contains(tag);
            if w::chip(ui, t, &category_label(tag), on).clicked() {
                if on {
                    l.tags.remove(tag);
                } else {
                    l.tags.insert(tag.to_string());
                }
                changed = true;
            }
        }
        ui.add_space(8.0);
        if w::toggle_chip(ui, t, &t!("Only in my language"), &mut l.my_language).clicked() {
            changed = true;
        }
    });
    if changed {
        l.page = 1;
    }
    hv.lists.insert((app, HubTab::Guides), l.clone());
    ui.add_space(12.0);
    let per = 20;
    let q = file_query(app, FileKind::Guides, &l, per, l.page, io.lang());
    let key = q.key();
    let have = io.state.hub.files.contains_key(&key);
    hv.ask(io, key.clone(), have, || Request::Files(q.clone()));
    let page = match io.state.hub.files.get(&key) {
        None => return w::loading(ui, t, &t!("Loading…")),
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, Request::Files(q));
            }
            return;
        }
        Some(Ok(p)) => p.clone(),
    };
    if page.items.is_empty() {
        w::empty(ui, t, icons::BOOKS, t!("No guides found"), t!("Try another category or search."));
        return;
    }
    let gap = 14.0;
    let card_w = 240.0;
    let cols = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
    let cw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    for row in page.items.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for g in row {
                if ui.vertical(|ui| guide_tile(ui, t, io, g, cw)).inner {
                    io.go(View::Item(g.id));
                }
            }
        });
        ui.add_space(gap);
    }
    let pages = (page.total.div_ceil(per as u64) as u32).clamp(1, 1000);
    if let Some(p) = pager(ui, t, l.page, pages) {
        if let Some(l) = hv.lists.get_mut(&(app, HubTab::Guides)) {
            l.page = p;
        }
    }
}

fn guide_tile(ui: &mut Ui, t: &Theme, io: &mut Io, g: &FileEntry, width: f32) -> bool {
    let mut open = false;
    let r = egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).show(ui, |ui| {
        ui.set_width(width);
        ui.set_min_height(width * 0.56 + 128.0);
        let (img, resp) = ui.allocate_exact_size(vec2(width, width * 0.56), Sense::click());
        let radius = CornerRadius { nw: t.corner_small().nw, ne: t.corner_small().ne, sw: 0, se: 0 };
        match &g.preview {
            Some(u) => w::image_or_skeleton(ui, t, img, u, radius),
            None => w::placeholder(ui, t, img, &g.title, radius),
        }
        open |= resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.spacing_mut().item_spacing.y = 3.0;
            if ui.add(egui::Label::new(w::text_semibold(&g.title, 14.0, t.text)).truncate().sense(Sense::click())).clicked() {
                open = true;
            }
            author(ui, t, io, g.creator, "", 18.0);
            stars(ui, t, g.votes_up, g.votes_down, 12.0);
            let desc: String = g.description.chars().take(140).collect();
            ui.add(egui::Label::new(w::text(desc, 12.0, t.text_faint)).wrap());
        });
    });
    if r.response.hovered() {
        ui.painter().rect_stroke(r.response.rect, t.corner_small(), Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    }
    open
}

// ---------- one item ----------

pub(super) fn item(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, id: u64) {
    let have = io.state.hub.items.contains_key(&id);
    let lang = io.lang();
    hv.ask(io, format!("item{id}"), have, || Request::Item { file_id: id, language: lang.clone() });
    let it = match io.state.hub.items.get(&id) {
        None => {
            ui.add_space(20.0);
            return w::loading(ui, t, &t!("Loading…"));
        }
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, Request::Item { file_id: id, language: lang });
            }
            return;
        }
        Some(Ok(it)) => it.clone(),
    };
    let e = &it.entry;
    ui.add_space(14.0);
    // Mature content stays hidden until asked for.
    if e.mature && !hv.mature_ok.contains(&id) {
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            ui.label(w::text(icons::EYE_SLASH, 40.0, t.text_faint));
            ui.label(w::text_semibold(t!("This item may contain mature content"), 18.0, t.text));
            ui.label(w::text(t!("Steam flagged it as possibly showing nudity or sexual content."), 13.5, t.text_dim));
            ui.add_space(10.0);
            if w::button(ui, t, Some(icons::EYE), t!("Show it anyway"), w::Kind::Secondary, w::Size::Md).clicked() {
                hv.mature_ok.insert(id);
            }
        });
        return;
    }
    let guide = matches!(it.file_type, 9 | 10);
    // Breadcrumb: game › section.
    ui.horizontal(|ui| {
        let game = if it.app_name.is_empty() { io.state.app_name(e.app_id).unwrap_or_default().to_string() } else { it.app_name.clone() };
        if ui.link(w::text(&game, 13.0, t.accent)).clicked() {
            io.go(View::Hub { app: e.app_id, tab: HubTab::All });
        }
        ui.label(w::text("›", 13.0, t.text_faint));
        let (label, to) = match it.file_type {
            9 | 10 => (t!("Guides").to_string(), View::Guides(e.app_id)),
            5 => (t!("Screenshots").to_string(), View::Hub { app: e.app_id, tab: HubTab::Screenshots }),
            3 => (t!("Artwork").to_string(), View::Hub { app: e.app_id, tab: HubTab::Artwork }),
            4 => (t!("Videos").to_string(), View::Hub { app: e.app_id, tab: HubTab::Videos }),
            _ => (t!("Workshop").to_string(), View::Page("workshop".into(), Some(e.app_id.to_string()))),
        };
        if ui.link(w::text(label, 13.0, t.accent)).clicked() {
            io.go(to);
        }
    });
    ui.add_space(2.0);
    let title = if e.title.is_empty() { kind_label(match it.file_type { 5 => FileKind::Screenshots, 3 => FileKind::Art, _ => FileKind::Items }) } else { e.title.clone() };
    ui.add(egui::Label::new(w::text_bold(title, if io.overlay { 20.0 } else { 26.0 }, t.text)).wrap());
    ui.horizontal_wrapped(|ui| {
        author(ui, t, io, e.creator, "", 26.0);
        ui.label(w::text(format!("·  {}", t!("Posted {date}", date = ui::date(e.created))), 12.5, t.text_faint));
        if e.updated > e.created + 60 {
            ui.label(w::text(format!("·  {}", t!("Updated {date}", date = ui::date(e.updated))), 12.5, t.text_faint));
        }
    });
    ui.add_space(12.0);
    if guide {
        return guide_body(hv, ui, t, io, &it);
    }
    let side = if io.overlay { 260.0 } else { 320.0 };
    columns(
        ui,
        side,
        hv,
        io,
        |ui, hv, io| {
            media(hv, ui, t, io, &it);
            if !it.description.trim().is_empty() {
                ui.add_space(14.0);
                ui.label(w::text_bold(t!("DESCRIPTION"), 12.0, t.text_faint));
                ui.add_space(4.0);
                let doc = rich::parse(&it.description);
                if let Some(link) = rich::show(ui, t, &doc, 14.0) {
                    hv.follow(io, &link);
                }
            }
            if !it.change_notes.is_empty() {
                ui.add_space(18.0);
                change_notes(hv, ui, t, &it);
            }
            ui.add_space(18.0);
            comments(hv, ui, t, io, e.id, e.creator, e.comments);
        },
        |ui, hv, io| item_side(hv, ui, t, io, &it),
    );
}

/// Big picture or video with a strip of thumbnails, like Steam's item page.
fn media(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, it: &hub::Item) {
    let mut previews = it.previews.clone();
    if previews.is_empty() {
        if let Some(img) = it.entry.image.clone().or(it.entry.preview.clone()) {
            previews.push(hub::Preview { url: Some(img), youtube: None });
        }
        if let Some(y) = &it.entry.youtube {
            previews.push(hub::Preview { url: None, youtube: Some(y.clone()) });
        }
    }
    // Screenshots and artwork: the full-size picture first.
    if matches!(it.file_type, 3 | 5) {
        if let Some(full) = &it.entry.image {
            previews.retain(|p| p.url.as_ref() != it.entry.preview.as_ref());
            previews.insert(0, hub::Preview { url: Some(full.clone()), youtube: None });
        }
    }
    if previews.is_empty() {
        return;
    }
    let sel = hv.media.get(&it.entry.id).copied().unwrap_or(0).min(previews.len() - 1);
    let wdt = ui.available_width();
    let (big, resp) = ui.allocate_exact_size(vec2(wdt, (wdt * 9.0 / 16.0).min(560.0)), Sense::click());
    ui.painter().rect_filled(big, t.corner_small(), Color32::BLACK);
    let p = &previews[sel];
    match (&p.youtube, &p.url) {
        (Some(y), _) => {
            w::paint_contain(ui, big, &format!("https://img.youtube.com/vi/{y}/hqdefault.jpg"), Align2::CENTER_CENTER);
            ui.painter().circle_filled(big.center(), 34.0, if resp.hovered() { t.accent } else { Color32::from_black_alpha(170) });
            ui.painter().text(big.center(), Align2::CENTER_CENTER, icons::PLAY, w::font(28.0), Color32::WHITE);
            if resp.on_hover_text(t!("Watch on YouTube")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                let _ = open::that_detached(format!("https://www.youtube.com/watch?v={y}"));
            }
        }
        (None, Some(u)) => {
            if !w::paint_contain(ui, big, u, Align2::CENTER_CENTER) {
                w::skeleton(ui, t, big, t.corner_small());
            }
            if resp.on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
                hv.lightbox = Some(u.clone());
            }
        }
        _ => {}
    }
    if previews.len() > 1 {
        ui.add_space(6.0);
        egui::ScrollArea::horizontal().id_salt(("media-strip", it.entry.id)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (i, p) in previews.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(112.0, 63.0), Sense::click());
                    let thumb = p.youtube.as_ref().map(|y| format!("https://img.youtube.com/vi/{y}/default.jpg")).or(p.url.clone()).unwrap_or_default();
                    w::image_or_skeleton(ui, t, r, &thumb, CornerRadius::same(3));
                    if p.youtube.is_some() {
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::PLAY, w::font(18.0), Color32::WHITE);
                    }
                    if i == sel {
                        ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                    } else if resp.hovered() {
                        ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        hv.media.insert(it.entry.id, i);
                    }
                }
            });
        });
    }
    let _ = io;
}

fn change_notes(hv: &mut HubView, ui: &mut Ui, t: &Theme, it: &hub::Item) {
    ui.horizontal(|ui| {
        ui.label(w::text_bold(t!("CHANGE NOTES"), 12.0, t.text_faint));
        ui.label(w::text(t!("{n} in total", n = it.change_total.max(it.change_notes.len() as u32)), 11.5, t.text_faint));
    });
    ui.add_space(4.0);
    let shown = if hv.show_all_notes { it.change_notes.len() } else { 3 };
    for n in it.change_notes.iter().take(shown) {
        egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(w::text_semibold(t!("Update: {date}", date = ui::date(n.time)), 13.0, t.text));
            if !n.text.trim().is_empty() {
                let doc = rich::parse(&n.text);
                rich::show(ui, t, &doc, 13.0);
            }
        });
        ui.add_space(6.0);
    }
    if it.change_notes.len() > 3 {
        let label = if hv.show_all_notes { t!("Show less") } else { t!("Show all") };
        if w::button(ui, t, None, label, w::Kind::Ghost, w::Size::Sm).clicked() {
            hv.show_all_notes = !hv.show_all_notes;
        }
    }
}

/// Subscribe, rate, favorite, stats, tags and required items.
fn item_side(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, it: &hub::Item) {
    let e = &it.entry;
    let workshop = it.file_type == 0;
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if workshop {
            let installed = io.local.games.iter().find(|g| g.app_id == e.app_id).map(|g| local::manage::workshop_items(g)).unwrap_or_default();
            let local_item = installed.iter().find(|i| i.id == e.id).cloned();
            let on = super::subscribed(e.id).unwrap_or(it.subscribed || local_item.is_some());
            if on {
                ui.horizontal(|ui| {
                    w::pill(ui, &t!("Subscribed"), t.play, t.play.gamma_multiply(0.15));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::button(ui, t, Some(icons::TRASH), t!("Unsubscribe"), w::Kind::Danger, w::Size::Sm).on_hover_text(t!("Remove it from this game and delete its files")).clicked() {
                            io.send(Command::UnsubscribeWorkshop { app_id: e.app_id, file_id: e.id });
                            if let Some(li) = &local_item {
                                let _ = local::manage::delete_workshop_files(li);
                            }
                        }
                    });
                });
            } else {
                let r = ui.add_sized(vec2(ui.available_width(), 40.0), egui::Button::new(w::text_semibold(format!("{}  {}", icons::PLUS, t!("Subscribe")), 15.0, t.on_accent)).fill(t.play));
                if r.on_hover_text(t!("Steam downloads it with the game")).clicked() {
                    io.send(Command::WorkshopSubscribe { app_id: e.app_id, file_id: e.id });
                }
            }
            ui.add_space(10.0);
        }
        rating(ui, t, io, it);
        ui.add_space(8.0);
        let fav = io.state.hub.favorites.get(&e.id).copied().unwrap_or(false);
        let label = if fav { t!("Remove from favorites") } else { t!("Favorite") };
        if w::button(ui, t, Some(icons::HEART), label, if fav { w::Kind::Secondary } else { w::Kind::Ghost }, w::Size::Sm).clicked() {
            io.send(Command::Hub(Request::Favorite { app_id: e.app_id, file_id: e.id, add: !fav }));
        }
    });
    ui.add_space(12.0);
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Grid::new(("item-stats", e.id)).num_columns(2).spacing(vec2(14.0, 5.0)).show(ui, |ui| {
            let mut row = |k: String, v: String| {
                ui.label(w::text(k, 12.5, t.text_faint));
                ui.label(w::text_semibold(v, 12.5, t.text));
                ui.end_row();
            };
            if e.size > 0 {
                row(t!("File size").to_string(), ui::bytes(e.size));
            }
            row(t!("Posted").to_string(), ui::date(e.created));
            if e.updated > e.created {
                row(t!("Updated").to_string(), ui::date(e.updated));
            }
            if e.views > 0 {
                row(t!("Unique visitors").to_string(), group(e.views));
            }
            if workshop {
                row(t!("Current subscribers").to_string(), group(e.subscriptions));
                if it.lifetime_subscriptions > 0 {
                    row(t!("Total unique subscribers").to_string(), group(it.lifetime_subscriptions));
                }
            }
            row(t!("Current favorites").to_string(), group(e.favorited));
        });
    });
    if !e.tags.is_empty() {
        ui.add_space(12.0);
        ui.label(w::text_bold(t!("TAGS"), 11.0, t.text_faint));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
            for tag in &e.tags {
                w::pill(ui, tag, t.text_dim, t.raised);
            }
        });
    }
    if !it.required.is_empty() {
        ui.add_space(12.0);
        ui.label(w::text_bold(t!("REQUIRED ITEMS"), 11.0, t.text_faint));
        ui.label(w::text(t!("This item needs all of these to work."), 12.0, t.text_dim));
        ui.add_space(4.0);
        for r in &it.required {
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(row, t.corner_small(), t.surface_hover);
            }
            let img = Rect::from_min_size(row.min + vec2(4.0, 4.0), vec2(38.0, 38.0));
            match &r.preview {
                Some(u) => w::image_or_skeleton(ui, t, img, u, CornerRadius::same(3)),
                None => w::placeholder(ui, t, img, "", CornerRadius::same(3)),
            }
            let galley = ui.painter().layout(r.title.clone(), w::font_medium(13.0), t.text, row.width() - 56.0);
            ui.painter().galley(egui::pos2(img.right() + 8.0, row.center().y - galley.size().y / 2.0), galley, t.text);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                io.go(View::Item(r.id));
            }
        }
    }
    let _ = hv;
}

/// Stars, rating count and your thumbs up / down.
fn rating(ui: &mut Ui, t: &Theme, io: &mut Io, it: &hub::Item) {
    let e = &it.entry;
    stars(ui, t, e.votes_up, e.votes_down, 15.0);
    ui.label(w::text(t!("{n} ratings", n = group((e.votes_up + e.votes_down) as u64)), 12.0, t.text_faint));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(w::text(t!("Rate:"), 12.5, t.text_dim));
        let up = it.my_vote == Some(true);
        let down = it.my_vote == Some(false);
        if w::icon_button(ui, t, icons::THUMBS_UP, &t!("Rate up"), up, 0).clicked() && !up {
            io.send(Command::Hub(Request::Vote { file_id: e.id, up: true }));
        }
        if w::icon_button(ui, t, THUMBS_DOWN, &t!("Rate down"), down, 0).clicked() && !down {
            io.send(Command::Hub(Request::Vote { file_id: e.id, up: false }));
        }
    });
}

/// 12,345 style numbers.
pub(super) fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Comments under an item or guide, ten at a time.
fn comments(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, id: u64, creator: model::SteamId, count: u32) {
    let key = format!("comments{id}");
    let start = hv.pages.get(&key).copied().unwrap_or(0);
    let have = io.state.hub.comments.contains_key(&id);
    hv.ask(io, format!("{key}@{start}"), have && start == 0, || Request::FileComments { file_id: id, creator, start });
    ui.label(w::text_bold(t!("COMMENTS"), 12.0, t.text_faint));
    ui.add_space(4.0);
    let data = match io.state.hub.comments.get(&id) {
        None => {
            if count > 0 {
                ui.add(egui::Spinner::new().color(t.accent));
            } else {
                ui.label(w::text(t!("No comments yet."), 13.0, t.text_faint));
            }
            return;
        }
        Some(Err(e)) => {
            ui.label(w::text(i18n::tr_dyn(e), 13.0, t.text_faint));
            return;
        }
        Some(Ok(c)) => c.clone(),
    };
    if data.items.is_empty() {
        ui.label(w::text(t!("No comments yet."), 13.0, t.text_faint));
        return;
    }
    for c in &data.items {
        ui.horizontal_top(|ui| {
            match &c.avatar {
                Some(u) => w::avatar(ui, t, u, 34.0, None),
                None => w::avatar(ui, t, &model::art::avatar(None), 34.0, None),
            };
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.horizontal(|ui| {
                    if ui.add(egui::Label::new(w::text_semibold(&c.name, 13.0, t.text)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        io.go(View::Page("profile".into(), Some(c.author.to_string())));
                    }
                    ui.label(w::text(ui::ago(c.time), 11.5, t.text_faint));
                });
                let doc = rich::parse(&c.html);
                if let Some(link) = rich::show(ui, t, &doc, 13.0) {
                    hv.follow(io, &link);
                }
            });
        });
        ui.add_space(8.0);
    }
    let pages = data.total.div_ceil(10).max(1);
    if let Some(p) = pager(ui, t, start / 10 + 1, pages) {
        let s = (p - 1) * 10;
        hv.pages.insert(key, s);
        io.send(Command::Hub(Request::FileComments { file_id: id, creator, start: s }));
    }
}

// ---------- guides ----------

fn guide_body(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, it: &hub::Item) {
    let id = it.entry.id;
    let have = io.state.hub.guides.contains_key(&id);
    let lang = io.lang();
    hv.ask(io, format!("guide{id}"), have, || Request::Guide { file_id: id, language: lang.clone() });
    let guide = match io.state.hub.guides.get(&id) {
        None => None,
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, Request::Guide { file_id: id, language: lang });
            }
            return;
        }
        Some(Ok(g)) => Some(g.clone()),
    };
    let side = if io.overlay { 230.0 } else { 290.0 };
    let mut jump = None;
    columns(
        ui,
        side,
        hv,
        io,
        |ui, hv, io| {
            // Overview, like the top of Steam's guide page.
            egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    if let Some(p) = &it.entry.preview {
                        let (r, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
                        w::image_or_skeleton(ui, t, r, p, CornerRadius::same(4));
                        ui.add_space(8.0);
                    }
                    ui.vertical(|ui| {
                        stars(ui, t, it.entry.votes_up, it.entry.votes_down, 14.0);
                        let overview = guide.as_ref().map(|g| g.overview.clone()).filter(|o| !o.is_empty()).unwrap_or_else(|| it.description.clone());
                        let doc = rich::parse(&overview);
                        if let Some(link) = rich::show(ui, t, &doc, 14.0) {
                            hv.follow(io, &link);
                        }
                    });
                });
            });
            ui.add_space(14.0);
            let Some(g) = &guide else {
                w::loading(ui, t, &t!("Loading…"));
                return;
            };
            for (i, s) in g.sections.iter().enumerate() {
                let r = egui::Frame::new().fill(t.surface).corner_radius(t.corner_small()).inner_margin(egui::Margin::same(16)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if !s.title.is_empty() {
                        ui.add(egui::Label::new(w::text_bold(&s.title, 19.0, t.text)).wrap());
                        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
                        ui.add_space(6.0);
                    }
                    let doc = rich::parse(&s.html);
                    if let Some(link) = rich::show(ui, t, &doc, 14.5) {
                        hv.follow(io, &link);
                    }
                });
                if hv.scroll_to == Some(i) {
                    r.response.scroll_to_me(Some(Align::TOP));
                    hv.scroll_to = None;
                }
                ui.add_space(12.0);
            }
            ui.add_space(8.0);
            comments(hv, ui, t, io, id, it.entry.creator, it.entry.comments);
        },
        |ui, _hv, io| {
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                rating(ui, t, io, it);
                ui.add_space(8.0);
                let fav = io.state.hub.favorites.get(&id).copied().unwrap_or(false);
                if w::button(ui, t, Some(icons::HEART), if fav { t!("Remove from favorites") } else { t!("Favorite") }, w::Kind::Ghost, w::Size::Sm).clicked() {
                    io.send(Command::Hub(Request::Favorite { app_id: it.entry.app_id, file_id: id, add: !fav }));
                }
            });
            ui.add_space(12.0);
            if let Some(g) = &guide {
                if !g.sections.is_empty() {
                    w::card(t).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(w::text_bold(t!("GUIDE INDEX"), 11.5, t.text_faint));
                        ui.add_space(4.0);
                        for (i, s) in g.sections.iter().enumerate() {
                            let r = ui.add(egui::Label::new(w::text(&s.title, 13.0, t.text_dim)).truncate().sense(Sense::click()));
                            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                jump = Some(i);
                            }
                        }
                    });
                    ui.add_space(12.0);
                }
            }
            w::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new(("guide-stats", id)).num_columns(2).spacing(vec2(14.0, 5.0)).show(ui, |ui| {
                    let e = &it.entry;
                    for (k, v) in [
                        (t!("Unique visitors").to_string(), group(e.views)),
                        (t!("Current favorites").to_string(), group(e.favorited)),
                        (t!("Posted").to_string(), ui::date(e.created)),
                        (t!("Updated").to_string(), ui::date(e.updated)),
                    ] {
                        ui.label(w::text(k, 12.5, t.text_faint));
                        ui.label(w::text_semibold(v, 12.5, t.text));
                        ui.end_row();
                    }
                });
            });
            if !it.entry.tags.is_empty() {
                ui.add_space(12.0);
                ui.label(w::text_bold(t!("CATEGORY"), 11.0, t.text_faint));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                    for tag in &it.entry.tags {
                        w::pill(ui, &category_label(tag), t.text_dim, t.raised);
                    }
                });
            }
        },
    );
    if jump.is_some() {
        hv.scroll_to = jump;
    }
}

// ---------- someone's files ----------

pub(super) fn kind_title(kind: FileKind) -> String {
    match kind {
        FileKind::Screenshots => t!("Screenshots"),
        FileKind::Art => t!("Artwork"),
        FileKind::Videos => t!("Videos"),
        FileKind::Guides => t!("Guides"),
        FileKind::Items | FileKind::Collections => t!("Workshop items"),
    }
    .to_string()
}

/// What one person shared, newest first, like the links on their profile.
pub(super) fn user_files(hv: &mut HubView, ui: &mut Ui, t: &Theme, io: &mut Io, user: model::SteamId, kind: FileKind) {
    let p = io.state.persona(user);
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        if w::avatar(ui, t, &model::art::avatar(p.avatar_hash.as_deref()), 48.0, None).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            io.go(View::Page("profile".into(), Some(user.to_string())));
        }
        ui.vertical(|ui| {
            ui.label(w::text_bold(&p.name, 22.0, t.text));
            ui.label(w::text(kind_title(kind), 13.5, t.text_dim));
        });
    });
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        for k in [FileKind::Screenshots, FileKind::Art, FileKind::Videos, FileKind::Items, FileKind::Guides] {
            if w::nav_tab(ui, t, &kind_title(k), k == kind, 32.0).clicked() && k != kind {
                io.go(View::UserFiles { user, kind: k });
            }
        }
    });
    ui.add_space(10.0);
    let page_key = format!("user{user}/{kind:?}");
    let page = hv.pages.get(&page_key).copied().unwrap_or(1);
    let key = hub::user_files_key(user, kind, page);
    let lang = io.lang();
    let req = || Request::UserFiles { steam_id: user, kind, page, language: lang.clone() };
    let have = io.state.hub.files.contains_key(&key);
    hv.ask(io, key.clone(), have, req);
    let data = match io.state.hub.files.get(&key) {
        None => return w::loading(ui, t, &t!("Loading…")),
        Some(Err(e)) if e == "private" => return w::empty(ui, t, icons::LOCK_SIMPLE, t!("This profile is private"), t!("Only their friends can see this.")),
        Some(Err(e)) => {
            if error(ui, t, e) {
                hv.retry(io, req());
            }
            return;
        }
        Some(Ok(d)) => d.clone(),
    };
    if data.items.is_empty() {
        return w::empty(ui, t, icons::CAMERA, t!("Nothing here yet"), t!("Nothing shared here yet."));
    }
    let mixed: Vec<(FileKind, FileEntry)> = data.items.into_iter().map(|e| (kind, e)).collect();
    grid(hv, ui, t, io, &mixed, 280.0);
    let pages = (data.total.div_ceil(hub::USER_PAGE as u64) as u32).max(1);
    if let Some(p) = pager(ui, t, page, pages) {
        hv.pages.insert(page_key, p);
    }
}

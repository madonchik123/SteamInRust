//! What someone shared (Figma "screenshots" `457:12553`): screenshots,
//! artwork, videos, Workshop items and guides, in the profile's main
//! column. The profile page and the community hub's "user files" view both
//! draw it with [`files_list`].

use client::Command;
use client::hub::{self, FileEntry, FileKind, Request};
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, Ui, vec2};
use extensions::Ctx;
use i18n::t;
use model::SteamId;
use ui::{Theme, icons, widgets as w};

/// What the list asked for.
pub(crate) enum FilesClick {
    /// Open one thing (its page in the community hub).
    Item(u64),
    /// Show another kind.
    Kind(FileKind),
    /// Another page.
    Page(u32),
    /// Ask Steam again after an error.
    Retry(Request),
}

/// The kinds a profile links to, with their names.
pub(crate) fn kinds() -> Vec<(FileKind, String)> {
    [
        (FileKind::Screenshots, t!("Screenshots")),
        (FileKind::Art, t!("Artworks")),
        (FileKind::Videos, t!("Videos")),
        (FileKind::Items, t!("Workshop Items")),
        (FileKind::Guides, t!("Guides")),
    ]
    .into_iter()
    .map(|(k, l)| (k, l.to_string()))
    .collect()
}

/// The request for a page of someone's files, and its key in the state.
pub(crate) fn request(user: SteamId, kind: FileKind, page: u32) -> (String, Request) {
    (hub::user_files_key(user, kind, page), Request::UserFiles { steam_id: user, kind, page, language: i18n::language().steam_name().into() })
}

/// The bar (which kind) and the grid of what someone shared, two to a row,
/// with the pages under it. `want` is set when the page isn't loaded yet.
pub(crate) fn files_list(ui: &mut Ui, t: &Theme, state: &client::State, user: SteamId, kind: FileKind, page: u32, want: &mut Option<Request>) -> Option<FilesClick> {
    let mut click = None;
    // The bar: what to show.
    w::component_frame(t, &t.components.segmented).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let mut pick = if kind == FileKind::Collections { FileKind::Items } else { kind };
            let width = ui.available_width().min(220.0);
            if w::select(ui, t, &mut pick, &kinds(), width) {
                click = Some(FilesClick::Kind(pick));
            }
        });
    });
    let (key, req) = request(user, kind, page);
    let data = match state.hub.files.get(&key) {
        None => {
            *want = Some(req);
            super::loading_rows(ui, t, 2, t.dense(220.0));
            return click;
        }
        Some(Err(e)) if e == "private" => {
            w::empty(ui, t, icons::LOCK_SIMPLE, t!("This profile is private"), t!("Only their friends can see this."));
            return click;
        }
        Some(Err(e)) => {
            ui.vertical_centered(|ui| {
                ui.label(w::body_m(t, i18n::tr_dyn(e), t.text_dim));
                if w::button(ui, t, Some(icons::REFRESH), t!("Try again"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    click = Some(FilesClick::Retry(req));
                }
            });
            return click;
        }
        Some(Ok(d)) => d,
    };
    if data.items.is_empty() {
        w::empty(ui, t, icons::CAMERA, t!("Nothing here yet"), t!("Nothing shared here yet."));
        return click;
    }
    let gap = t.spacing.md;
    let cols = if ui.available_width() > 520.0 { 2 } else { 1 };
    let cw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    for row in data.items.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for e in row {
                if card(ui, t, kind, e, cw) {
                    click = Some(FilesClick::Item(e.id));
                }
            }
        });
    }
    let pages = (data.total.div_ceil(hub::USER_PAGE as u64) as usize).max(1);
    if let Some(p) = super::pager(ui, t, page.saturating_sub(1) as usize, pages) {
        click = Some(FilesClick::Page(p as u32 + 1));
    }
    click
}

/// One thing someone shared, as the design's screenshot cards: the
/// picture, then its title with votes and comments. True when clicked.
fn card(ui: &mut Ui, t: &Theme, kind: FileKind, e: &FileEntry, width: f32) -> bool {
    let c = &t.components.profile_inset;
    let pad = t.dense(8.0);
    let img_h = (width - pad * 2.0) * 9.0 / 16.0;
    let line = t.typography.body_l.size + t.spacing.md * 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, img_h + pad * 2.0 + line), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp.clicked();
    }
    let look = t.look(c, w::interact(ui, &resp, false));
    w::paint_box(ui, t, rect, c, &look, 0.0);
    let img = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(width - pad * 2.0, img_h));
    let url = e
        .preview
        .as_deref()
        .and_then(|u| crate::links::validate_http_url(u).map(|_| u.to_string()))
        .or_else(|| e.image.as_deref().and_then(|u| crate::links::validate_http_url(u).map(|_| u.to_string())))
        .or_else(|| e.youtube.as_deref().and_then(crate::links::valid_youtube_id).map(|y| format!("https://img.youtube.com/vi/{y}/hqdefault.jpg")));
    match &url {
        Some(u) => w::image_or_skeleton(ui, t, img, u, CornerRadius::ZERO),
        None => w::placeholder(ui, t, img, &e.title, CornerRadius::ZERO),
    }
    let p = ui.painter();
    if e.spoiler {
        p.rect_filled(img, CornerRadius::ZERO, t.overlay_dark(3));
        p.text(img.center(), Align2::CENTER_CENTER, t!("Spoiler"), t.typography.body_l.font(), t.text_dim);
    }
    if kind == FileKind::Videos {
        p.circle_filled(img.center(), t.dense(22.0), t.overlay_dark(2));
        p.text(img.center(), Align2::CENTER_CENTER, icons::PLAY, t.typography.heading_s.font(), t.text);
    }
    if resp.hovered() {
        p.rect_stroke(img, CornerRadius::ZERO, Stroke::new(t.dense(2.0), t.accent), egui::StrokeKind::Inside);
    }
    // Votes and comments in small boxes on the right, the title before them.
    let y = img.bottom() + pad + line / 2.0 - t.spacing.xs;
    let small = t.typography.body_s.font();
    let mut x = rect.right() - pad;
    for (icon, n) in [(icons::CHAT, e.comments), (icons::THUMBS_UP, e.votes_up)] {
        let g = p.layout_no_wrap(format!("{icon}  {n}"), small.clone(), t.text);
        let chip = Rect::from_min_max(egui::pos2(x - g.size().x - t.spacing.sm * 2.0, y - line * 0.32), egui::pos2(x, y + line * 0.32));
        p.rect_filled(chip, t.corner_small(), t.raised);
        p.galley(egui::pos2(chip.left() + t.spacing.sm, y - g.size().y / 2.0), g, t.text);
        x = chip.left() - t.spacing.xs;
    }
    let title = if e.title.trim().is_empty() { e.description.lines().next().unwrap_or_default().to_string() } else { e.title.clone() };
    let mut job = egui::text::LayoutJob::simple_singleline(title, t.typography.body_l.font(), t.accent);
    job.wrap = egui::text::TextWrapping::truncate_at_width((x - img.left() - t.spacing.sm).max(20.0));
    let g = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(img.left(), y - g.size().y / 2.0), g, t.accent);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

impl super::Profile {
    /// Someone's screenshots (or other files) on their profile.
    pub(super) fn files_page(&mut self, ui: &mut Ui, t: &Theme, ctx: &mut Ctx, id: SteamId, kind: FileKind) {
        let mut want = None;
        let page = self.files_page.max(1);
        let click = super::section(ui, t, "", "", |ui| files_list(ui, t, ctx.state, id, kind, page, &mut want));
        if let Some(req) = want {
            let (key, _) = request(id, kind, page);
            if self.files_asked.insert(key) {
                ctx.send(Command::Hub(req));
            }
        }
        match click {
            Some(FilesClick::Item(item)) => ctx.navigate("workshop_item", Some(item.to_string())),
            Some(FilesClick::Kind(k)) => ctx.navigate("profile", Some(format!("{id}/{}", crate::builtin::hub::kind_slug(k)))),
            Some(FilesClick::Page(p)) => self.files_page = p,
            Some(FilesClick::Retry(req)) => ctx.send(Command::Hub(req)),
            None => {}
        }
    }
}

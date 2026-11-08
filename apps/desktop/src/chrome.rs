//! Our own window frame (the Windows title bar is turned off): a drag area,
//! minimize / maximize / close buttons, and resize handles on the edges.

use egui::{
    Align2, Color32, CursorIcon, Id, Rect, ResizeDirection, Sense, Ui, Vec2, ViewportCommand, vec2,
};
use ui::{Theme, icons, widgets as w};

/// Make `rect` move the window when dragged (double-click maximizes).
/// Call it before adding widgets on top, so they still get their clicks.
pub fn drag_area(ui: &mut Ui, rect: Rect, can_maximize: bool) {
    // One id per drag area (a window can have several, e.g. Friends & Chat).
    let resp = ui.interact(
        rect,
        Id::unique(("window-drag", rect.min.x as i32, rect.min.y as i32)),
        Sense::click_and_drag(),
    );
    if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if can_maximize && resp.double_clicked() {
        toggle_maximized(ui.ctx());
    }
}

pub fn toggle_maximized(ctx: &egui::Context) {
    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
}

/// Minimize, maximize/restore and close, right to left (call inside a
/// right-to-left layout). `height` is the bar height.
pub fn window_buttons(ui: &mut Ui, t: &Theme, height: f32, can_maximize: bool) {
    // In their own scope: buttons touch, like Windows', with no dead gaps,
    // and the spacing change doesn't leak into the rest of the bar.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let ctx = ui.ctx().clone();
        if button(ui, t, icons::X, height, true)
            .on_hover_text(i18n::tr_dyn("Close"))
            .clicked()
        {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if can_maximize {
            let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            let (icon, tip) = if max {
                (icons::RESTORE, "Restore")
            } else {
                (icons::SQUARE, "Maximize")
            };
            if button(ui, t, icon, height, false)
                .on_hover_text(i18n::tr_dyn(tip))
                .clicked()
            {
                toggle_maximized(&ctx);
            }
        }
        if button(ui, t, icons::MINUS, height, false)
            .on_hover_text(i18n::tr_dyn("Minimize"))
            .clicked()
        {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
    });
}

fn button(ui: &mut Ui, t: &Theme, icon: &str, height: f32, close: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(46.0, height), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.08);
    let bg = if close {
        Color32::from_rgb(0xC4, 0x2B, 0x1C)
    } else {
        t.surface_hover
    };
    if hover > 0.0 {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::ZERO, bg.gamma_multiply(hover));
    }
    let fg = if close && hover > 0.5 {
        Color32::WHITE
    } else {
        w::mix(t.text_dim, t.text, hover)
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        w::font(14.0),
        fg,
    );
    resp
}

/// Invisible handles on the window edges for resizing (not when maximized).
pub fn resize_handles(ctx: &egui::Context) {
    if ctx.input(|i| {
        i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
    }) {
        return;
    }
    let screen = ctx.content_rect();
    let (e, c) = (5.0, 12.0);
    let (l, r, tp, b) = (screen.left(), screen.right(), screen.top(), screen.bottom());
    // Keep clear of the minimize / maximize / close buttons in the top-right.
    let buttons = 3.0 * 46.0 + 8.0;
    let handles = [
        (
            Rect::from_min_max(egui::pos2(l + c, tp), egui::pos2(r - buttons, tp + e)),
            ResizeDirection::North,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(egui::pos2(l + c, b - e), egui::pos2(r - c, b)),
            ResizeDirection::South,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(egui::pos2(l, tp + c), egui::pos2(l + e, b - c)),
            ResizeDirection::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(egui::pos2(r - e, tp + 64.0), egui::pos2(r, b - c)),
            ResizeDirection::East,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_size(egui::pos2(l, tp), Vec2::splat(c)),
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_size(egui::pos2(l, b - c), Vec2::splat(c)),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(egui::pos2(r - c, b - c), Vec2::splat(c)),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeNwSe,
        ),
    ];
    for (i, (rect, dir, cursor)) in handles.into_iter().enumerate() {
        egui::Area::new(Id::unique(("resize-handle", i)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                let resp =
                    ui.allocate_rect(Rect::from_min_size(rect.min, rect.size()), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(cursor);
                }
                if resp.drag_started() {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::BeginResize(dir));
                }
            });
    }
}

/// A thin border, since a frameless window has none of its own.
pub fn border(ctx: &egui::Context, t: &Theme) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let rect = ctx.content_rect();
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::unique(("window-border", ctx.viewport_id())),
    ))
    .rect_stroke(
        rect,
        egui::CornerRadius::ZERO,
        egui::Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
}

/// Resize handles drawn into `ui` itself (for child windows such as Friends &
/// Chat, where separate areas end up swallowing clicks). Call it last, so the
/// thin edge strips sit on top of the window's content.
pub fn resize_edges(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if ctx.input(|i| {
        i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
    }) {
        return;
    }
    let screen = ctx.content_rect();
    let (e, c) = (5.0, 12.0);
    let (l, r, tp, b) = (screen.left(), screen.right(), screen.top(), screen.bottom());
    let buttons = 3.0 * 46.0 + 8.0;
    let handles = [
        (
            Rect::from_min_max(egui::pos2(l + c, tp), egui::pos2(r - buttons, tp + e)),
            ResizeDirection::North,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(egui::pos2(l + c, b - e), egui::pos2(r - c, b)),
            ResizeDirection::South,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(egui::pos2(l, tp + c), egui::pos2(l + e, b - c)),
            ResizeDirection::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(egui::pos2(r - e, tp + 40.0), egui::pos2(r, b - c)),
            ResizeDirection::East,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_size(egui::pos2(l, b - c), Vec2::splat(c)),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(egui::pos2(r - c, b - c), Vec2::splat(c)),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeNwSe,
        ),
    ];
    for (i, (rect, dir, cursor)) in handles.into_iter().enumerate() {
        let resp = ui.interact(
            rect,
            Id::unique(("resize-edge", ctx.viewport_id(), i)),
            Sense::drag(),
        );
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

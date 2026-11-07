//! A Steam-like player: click to pause, a seek bar with the buffered range,
//! time, volume + mute (remembered), quality, and full screen in its own
//! borderless window. Controls hide while playing unless the mouse moves.

use crate::{Player, Status};
use egui::{Align2, Color32, CornerRadius, Id, Key, Pos2, Rect, Sense, Ui, pos2, vec2};
use i18n::t;
use ui::{Theme, widgets as w};

const PAUSE: &str = "\u{E39E}";
const PLAY: &str = "\u{E3D0}";
const SPEAKER_HIGH: &str = "\u{E44A}";
const SPEAKER_LOW: &str = "\u{E44C}";
const SPEAKER_X: &str = "\u{E45C}";
const FULLSCREEN: &str = "\u{E0A2}";
const EXIT_FULLSCREEN: &str = "\u{E09A}";
const GEAR: &str = "\u{E272}";
const REPLAY: &str = "\u{E036}";
const CHECK: &str = "\u{E182}";

/// Per-player UI state.
#[derive(Default)]
pub(crate) struct UiState {
    /// When the pointer last moved over the player.
    moved: f64,
    last_pointer: Option<Pos2>,
    /// Seek position while dragging the bar.
    drag: Option<f64>,
    menu: bool,
    /// When this player was first shown.
    shown: Option<f64>,
    fs_logged: bool,
    fs_focused: bool,
}

/// What happened this frame.
#[derive(Default)]
pub struct Shown {
    /// Playback reached the end (once).
    pub ended: bool,
}

fn prefs_id() -> Id {
    Id::unique("video-player-prefs")
}

/// Remembered (volume, muted). Trailers start muted until the user turns the
/// sound on once, like Steam's store.
pub fn prefs(ctx: &egui::Context) -> (f32, bool) {
    ctx.data_mut(|d| d.get_persisted::<(f32, bool)>(prefs_id())).unwrap_or((0.7, true))
}

fn save_prefs(ctx: &egui::Context, volume: f32, muted: bool) {
    ctx.data_mut(|d| d.insert_persisted(prefs_id(), (volume, muted)));
}

/// Options for a trailer with the remembered sound settings.
pub fn options(ctx: &egui::Context) -> crate::Options {
    let (volume, muted) = prefs(ctx);
    crate::Options { volume, muted, ..Default::default() }
}

fn time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Show `p` in `rect` (and its full-screen window when asked).
pub fn show(ui: &mut Ui, t: &Theme, rect: Rect, p: &mut Player, poster: &str, title: &str) -> Shown {
    let ctx = ui.ctx().clone();
    if p.fullscreen {
        // The page keeps a still in place of the video.
        if !w::paint_cover(ui, rect, poster, t.corner()) {
            ui.painter().rect_filled(rect, t.corner(), Color32::BLACK);
        }
        ui.painter().rect_filled(rect, t.corner(), Color32::from_black_alpha(150));
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, t!("Playing in full screen"), w::font(14.0), Color32::from_white_alpha(220));
        let back = ui.interact(rect, ui.scope_id().with("video-fs-back"), Sense::click());
        if back.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            p.fullscreen = false;
        }
        let main = ctx.input(|i| i.viewport().outer_rect);
        if p.ui.shown.is_some() && !p.ui.fs_logged {
            tracing::debug!("video: full screen");
            p.ui.fs_logged = true;
        }
        // Like a web player's full screen: a borderless window covering the
        // whole monitor the app is on (taskbar included).
        let ppp = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0);
        let area = main.and_then(|r| monitor_at(r.center(), ppp)).or(main);
        let mut builder = egui::ViewportBuilder::default().with_title(title).with_decorations(false).with_resizable(false).with_taskbar(false);
        if let Some(r) = area {
            builder = builder.with_position(r.min).with_inner_size(r.size());
        }
        // The full-screen window is painted during the main window's pass:
        // keep both going (the player paces itself at the video's frame rate).
        ctx.request_repaint_after_for(std::time::Duration::from_millis(100), egui::ViewportId::ROOT);
        let mut shown = Shown::default();
        let mut close = false;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("video-fullscreen"), builder, |ui, _| {
            if !p.ui.fs_focused {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                p.ui.fs_focused = true;
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ui, |ui| {
                let r = ui.max_rect();
                shown = draw(ui, t, r, p, poster, true);
            });
            if ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape)) {
                close = true;
            }
        });
        if close {
            p.fullscreen = false;
            p.ui.fs_focused = false;
        }
        return shown;
    }
    draw(ui, t, rect, p, poster, false)
}

/// The bounds of the monitor showing `p` (in points), from Windows.
#[cfg(windows)]
fn monitor_at(p: Pos2, ppp: f32) -> Option<Rect> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
    unsafe {
        let m = MonitorFromPoint(POINT { x: (p.x * ppp) as i32, y: (p.y * ppp) as i32 }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(m, &mut info).as_bool() {
            return None;
        }
        let r = info.rcMonitor;
        Some(Rect::from_min_max(pos2(r.left as f32, r.top as f32) / ppp, pos2(r.right as f32, r.bottom as f32) / ppp))
    }
}

#[cfg(not(windows))]
fn monitor_at(_: Pos2, _: f32) -> Option<Rect> {
    None
}

fn draw(ui: &mut Ui, t: &Theme, rect: Rect, p: &mut Player, poster: &str, fullscreen: bool) -> Shown {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let radius = if fullscreen { CornerRadius::ZERO } else { t.corner() };
    // Prefer a sharper stream than the box: video scaled down looks better.
    p.set_display_width((rect.width() * ctx.pixels_per_point() * 1.25) as u32);
    p.poll(&ctx);
    let status = p.status();
    let first_shown = *p.ui.shown.get_or_insert(now);

    // Picture, letterboxed; the poster until the first frame.
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, radius, Color32::BLACK);
    match p.texture() {
        Some((id, size)) if size.x > 0.0 && size.y > 0.0 => {
            let s = (rect.width() / size.x).min(rect.height() / size.y);
            let r = Rect::from_center_size(rect.center(), size * s);
            painter.add(egui::epaint::RectShape::filled(r, if r.width() >= rect.width() - 1.0 { radius } else { CornerRadius::ZERO }, Color32::WHITE).with_texture(id, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
        }
        _ => {
            w::paint_cover(ui, rect, poster, radius);
        }
    }

    let id = ui.scope_id().with(("video", fullscreen));
    let area = ui.interact(rect, id, Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let st = &mut p.ui;
    if hovered && pointer != st.last_pointer {
        st.moved = now;
    }
    st.last_pointer = pointer;
    let show_controls = status != Status::Playing || st.drag.is_some() || st.menu || (hovered && now - st.moved < 2.5) || now - first_shown < 3.0;
    if fullscreen && !show_controls {
        ctx.set_cursor_icon(egui::CursorIcon::None);
    }
    if status == Status::Playing && hovered && now - st.moved < 2.6 {
        ctx.request_repaint_after_for(std::time::Duration::from_millis(300), egui::ViewportId::ROOT);
    }

    // Keyboard, when hovered or full screen.
    if hovered || fullscreen {
        let (space, m, f, left, right) = ctx.input(|i| (i.key_pressed(Key::Space), i.key_pressed(Key::M), i.key_pressed(Key::F), i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight)));
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing {
            if space {
                p.toggle();
            }
            if m {
                let muted = !p.muted();
                p.set_muted(muted);
                save_prefs(&ctx, p.volume(), muted);
            }
            if f {
                p.fullscreen = !fullscreen;
            }
            if left || right {
                let d = if left { -5.0 } else { 5.0 };
                p.seek((p.position() + d).max(0.0));
            }
        }
    }

    // Center: spinner, big play / replay button, or an error.
    let st = &mut p.ui;
    let busy = matches!(status, Status::Loading | Status::Buffering) && now - first_shown > 0.2;
    if busy {
        egui::Spinner::new().size(36.0).color(Color32::WHITE).paint_at(ui, Rect::from_center_size(rect.center(), vec2(36.0, 36.0)));
    } else if matches!(status, Status::Paused | Status::Ended) && st.drag.is_none() {
        painter.circle_filled(rect.center(), 34.0, Color32::from_black_alpha(160));
        let (icon, dx) = if status == Status::Ended { (REPLAY, 0.0) } else { (PLAY, 3.0) };
        painter.text(rect.center() + vec2(dx, 0.0), Align2::CENTER_CENTER, icon, w::font(30.0), Color32::WHITE);
    } else if status == Status::Failed {
        let msg = p.error().unwrap_or_default();
        painter.rect_filled(rect, radius, Color32::from_black_alpha(150));
        painter.text(rect.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, t!("This video can't be played"), w::font_semibold(15.0), Color32::WHITE);
        painter.text(rect.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, msg, w::font(11.5), Color32::from_white_alpha(160));
    }

    let out = Shown { ended: p.take_ended() };
    if area.clicked() {
        p.toggle();
    }
    if area.double_clicked() {
        p.fullscreen = !fullscreen;
    }

    // "Sound off" hint while a trailer plays muted.
    if p.muted() && p.has_audio() && matches!(status, Status::Playing | Status::Buffering) {
        let text = format!("{SPEAKER_X}  {}", t!("Sound off"));
        let galley = painter.layout_no_wrap(text, w::font(12.5), Color32::WHITE);
        let r = Rect::from_min_size(pos2(rect.right() - galley.size().x - 34.0, rect.top() + 12.0), galley.size() + vec2(20.0, 10.0));
        let b = ui.interact(r, id.with("unmute"), Sense::click());
        painter.rect_filled(r, CornerRadius::same(12), Color32::from_black_alpha(if b.hovered() { 220 } else { 150 }));
        painter.galley(r.min + vec2(10.0, 5.0), galley, Color32::WHITE);
        if b.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            unmute(&ctx, p);
        }
    }

    if show_controls && status != Status::Failed {
        controls(ui, t, rect, p, fullscreen, id);
    }
    out
}

fn unmute(ctx: &egui::Context, p: &mut Player) {
    p.set_muted(false);
    if p.volume() < 0.05 {
        p.set_volume(0.5);
    }
    save_prefs(ctx, p.volume(), false);
}

/// Vertical fade from transparent to `bottom` color.
fn gradient(painter: &egui::Painter, r: Rect, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    let top = Color32::TRANSPARENT;
    mesh.colored_vertex(r.left_top(), top);
    mesh.colored_vertex(r.right_top(), top);
    mesh.colored_vertex(r.left_bottom(), bottom);
    mesh.colored_vertex(r.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(mesh);
}

fn icon_button(ui: &Ui, painter: &egui::Painter, r: Rect, id: Id, icon: &str, size: f32, tip: &str) -> egui::Response {
    let b = ui.interact(r, id, Sense::click());
    if b.hovered() {
        painter.rect_filled(r, CornerRadius::same(5), Color32::from_white_alpha(28));
    }
    painter.text(r.center(), Align2::CENTER_CENTER, icon, w::font(size), Color32::WHITE);
    b.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

fn controls(ui: &mut Ui, t: &Theme, rect: Rect, p: &mut Player, fullscreen: bool, id: Id) {
    let ctx = ui.ctx().clone();
    let painter = ui.painter_at(rect);
    let status = p.status();
    let pad = if fullscreen { 24.0 } else { 12.0 };
    gradient(&painter, Rect::from_min_max(pos2(rect.left(), rect.bottom() - 90.0), rect.right_bottom()), Color32::from_black_alpha(190));

    // Seek bar with the buffered range.
    let dur = p.duration();
    let bar = Rect::from_min_max(pos2(rect.left() + pad, rect.bottom() - 46.0), pos2(rect.right() - pad, rect.bottom() - 32.0));
    let seek = ui.interact(bar, id.with("seek"), Sense::click_and_drag());
    let frac_at = |x: f32| ((x - bar.left()) / bar.width()).clamp(0.0, 1.0) as f64;
    if dur > 0.0 {
        if let Some(pos) = seek.interact_pointer_pos() {
            if seek.dragged() || seek.is_pointer_button_down_on() {
                p.ui.drag = Some(frac_at(pos.x) * dur);
            }
        }
        if seek.drag_stopped() || seek.clicked() {
            if let Some(tgt) = p.ui.drag.take().or_else(|| seek.interact_pointer_pos().map(|q| frac_at(q.x) * dur)) {
                p.seek(tgt);
            }
        }
    }
    let shown_pos = p.ui.drag.unwrap_or(p.position());
    let thick = if seek.hovered() || p.ui.drag.is_some() { 6.0 } else { 4.0 };
    let line = Rect::from_center_size(bar.center(), vec2(bar.width(), thick));
    let x_of = |s: f64| line.left() + line.width() * if dur > 0.0 { (s / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
    painter.rect_filled(line, CornerRadius::same(3), Color32::from_white_alpha(50));
    let buf = p.buffered();
    painter.rect_filled(Rect::from_min_max(line.min, pos2(x_of(buf), line.max.y)), CornerRadius::same(3), Color32::from_white_alpha(90));
    painter.rect_filled(Rect::from_min_max(line.min, pos2(x_of(shown_pos), line.max.y)), CornerRadius::same(3), t.accent);
    if seek.hovered() || p.ui.drag.is_some() {
        painter.circle_filled(pos2(x_of(shown_pos), line.center().y), 7.0, Color32::WHITE);
        if let Some(h) = seek.hover_pos().or(seek.interact_pointer_pos()) {
            if dur > 0.0 {
                let s = time(frac_at(h.x) * dur);
                let g = painter.layout_no_wrap(s, w::font(12.0), Color32::WHITE);
                let r = Rect::from_center_size(pos2(h.x.clamp(bar.left() + 24.0, bar.right() - 24.0), bar.top() - 14.0), g.size() + vec2(12.0, 6.0));
                painter.rect_filled(r, CornerRadius::same(4), Color32::from_black_alpha(200));
                painter.galley(r.min + vec2(6.0, 3.0), g, Color32::WHITE);
            }
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // Buttons.
    let cy = rect.bottom() - 16.0;
    let mut x = rect.left() + pad - 6.0;
    let bs = 30.0;
    let mut slot = |w: f32| {
        let r = Rect::from_min_size(pos2(x, cy - bs / 2.0), vec2(w, bs));
        x += w + 4.0;
        r
    };
    let playing = matches!(status, Status::Playing | Status::Buffering);
    let (icon, tip) = if playing { (PAUSE, t!("Pause")) } else if status == Status::Ended { (REPLAY, t!("Replay")) } else { (PLAY, t!("Play video")) };
    if icon_button(ui, &painter, slot(bs), id.with("play"), icon, 18.0, &tip).clicked() {
        p.toggle();
    }
    if p.has_audio() {
        let (vol, muted) = (p.volume(), p.muted());
        let icon = if muted || vol <= 0.0 { SPEAKER_X } else if vol < 0.5 { SPEAKER_LOW } else { SPEAKER_HIGH };
        let tip = if muted { t!("Unmute") } else { t!("Mute") };
        if icon_button(ui, &painter, slot(bs), id.with("mute"), icon, 18.0, &tip).clicked() {
            if muted {
                unmute(&ctx, p);
            } else {
                p.set_muted(true);
                save_prefs(&ctx, vol, true);
            }
        }
        // Volume slider.
        let vr = slot(72.0);
        let vs = ui.interact(vr, id.with("volume"), Sense::click_and_drag());
        if let Some(q) = vs.interact_pointer_pos() {
            if vs.dragged() || vs.clicked() || vs.is_pointer_button_down_on() {
                let v = ((q.x - vr.left() - 4.0) / (vr.width() - 8.0)).clamp(0.0, 1.0);
                p.set_volume(v);
                p.set_muted(v <= 0.0);
                save_prefs(&ctx, v, v <= 0.0);
            }
        }
        if let Some(scroll) = vs.hover_pos().map(|_| ctx.input(|i| i.smooth_scroll_delta.y)).filter(|d| *d != 0.0) {
            let v = (p.volume() + scroll / 400.0).clamp(0.0, 1.0);
            p.set_volume(v);
            p.set_muted(v <= 0.0);
            save_prefs(&ctx, v, v <= 0.0);
        }
        let shown_vol = if p.muted() { 0.0 } else { p.volume() };
        let track = Rect::from_center_size(vr.center(), vec2(vr.width() - 8.0, 4.0));
        painter.rect_filled(track, CornerRadius::same(2), Color32::from_white_alpha(60));
        let knob_x = track.left() + track.width() * shown_vol;
        painter.rect_filled(Rect::from_min_max(track.min, pos2(knob_x, track.max.y)), CornerRadius::same(2), Color32::WHITE);
        painter.circle_filled(pos2(knob_x, track.center().y), if vs.hovered() { 6.0 } else { 4.5 }, Color32::WHITE);
        if vs.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    let tr = slot(0.0);
    let label = if dur > 0.0 { format!("{} / {}", time(shown_pos), time(dur)) } else { time(shown_pos) };
    painter.text(pos2(tr.left() + 6.0, cy), Align2::LEFT_CENTER, label, w::font(12.5), Color32::from_white_alpha(230));

    // Right side: quality, full screen.
    let fs = Rect::from_min_size(pos2(rect.right() - pad + 6.0 - bs, cy - bs / 2.0), vec2(bs, bs));
    let (icon, tip) = if fullscreen { (EXIT_FULLSCREEN, t!("Exit full screen")) } else { (FULLSCREEN, t!("Full screen")) };
    if icon_button(ui, &painter, fs, id.with("fs"), icon, 18.0, &tip).clicked() {
        p.fullscreen = !fullscreen;
    }
    let qualities = p.qualities();
    if qualities.len() > 1 {
        let (cur, manual) = p.quality();
        let gr = fs.translate(vec2(-bs - 4.0, 0.0));
        let label = qualities.get(cur).map(|q| format!("{}p", q.height)).unwrap_or_default();
        painter.text(pos2(gr.left() - 4.0, cy), Align2::RIGHT_CENTER, &label, w::font(11.5), Color32::from_white_alpha(170));
        let gear = icon_button(ui, &painter, gr, id.with("gear"), GEAR, 17.0, &t!("Quality"));
        if gear.clicked() {
            p.ui.menu = !p.ui.menu;
        }
        if p.ui.menu {
            let mut pick: Option<Option<usize>> = None;
            let resp = egui::Area::new(id.with("quality-menu")).order(egui::Order::Foreground).pivot(Align2::RIGHT_BOTTOM).fixed_pos(gr.right_top() + vec2(0.0, -8.0)).show(&ctx, |ui| {
                egui::Frame::new().fill(Color32::from_rgba_unmultiplied(20, 22, 28, 240)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                    ui.set_width(140.0);
                    let row = |ui: &mut Ui, text: String, on: bool| -> bool {
                        let (r, b) = ui.allocate_exact_size(vec2(140.0, 26.0), Sense::click());
                        if b.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(4), Color32::from_white_alpha(25));
                        }
                        if on {
                            ui.painter().text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, CHECK, w::font(13.0), t.accent);
                        }
                        ui.painter().text(r.left_center() + vec2(28.0, 0.0), Align2::LEFT_CENTER, text, w::font(13.0), Color32::WHITE);
                        b.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
                    };
                    let auto = if manual { t!("Auto").to_string() } else { format!("{} ({label})", t!("Auto")) };
                    if row(ui, auto, !manual) {
                        pick = Some(None);
                    }
                    for (i, q) in qualities.iter().enumerate().rev() {
                        if row(ui, format!("{}p", q.height), manual && i == cur) {
                            pick = Some(Some(i));
                        }
                    }
                });
            });
            if let Some(q) = pick {
                p.set_quality(q);
                p.ui.menu = false;
            } else if resp.response.clicked_elsewhere() && !gear.clicked() {
                p.ui.menu = false;
            }
        }
    }
}

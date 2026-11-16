//! The power menu: slides in from the right over a dimmed screen.

use super::focus::{Dir, Focus, Place, ease};
use super::input::Press;
use super::{BigPicture, Frame, Page, paint};
use egui::{Align2, Color32, Id, Rect, Ui, pos2, vec2};
use i18n::t;
use ui::{icons, widgets as w};

const OPEN_SECS: f32 = 0.28;

pub struct Menu {
    focus: Focus,
    opened: f64,
    closing: Option<f64>,
    dir: Option<Dir>,
    press: bool,
}

#[derive(PartialEq, Eq)]
pub enum Outcome {
    None,
    LeaveBigPicture,
}

impl Menu {
    pub fn new(now: f64) -> Self {
        Self { focus: Focus::default(), opened: now, closing: None, dir: None, press: false }
    }

    pub fn input(&mut self, presses: &[(Press, bool)], now: f64) -> Outcome {
        if self.closing.is_some() {
            return Outcome::None;
        }
        for (p, _) in presses {
            match p {
                Press::Move(d) => self.dir = Some(*d),
                Press::A => self.press = true,
                Press::B | Press::Start => self.closing = Some(now),
                // Guide twice: out of Big Picture.
                Press::Guide => return Outcome::LeaveBigPicture,
                _ => {}
            }
        }
        Outcome::None
    }
}

/// Draw the menu and do what was chosen.
pub fn show(bp: &mut BigPicture, ui: &mut Ui, f: &mut Frame, screen: Rect) {
    let Some(mut m) = bp.menu.take() else { return };
    let (t, s, now) = (f.t.clone(), f.s, f.now);
    let e = match m.closing {
        Some(at) => 1.0 - ease(((now - at) as f32 / OPEN_SECS).min(1.0)),
        None => ease(((now - m.opened) as f32 / OPEN_SECS).min(1.0)),
    };
    if e < 1.0 {
        ui.ctx().request_repaint();
    }
    if m.closing.is_some_and(|at| now - at > OPEN_SECS as f64) {
        return; // closed
    }
    let p = ui.painter().clone();
    p.rect_filled(screen, 0.0, Color32::from_black_alpha((150.0 * e) as u8));
    // Clicking outside closes it.
    let outside = ui.interact(screen, Id::unique("bp-menu-backdrop"), egui::Sense::click());

    let width = 560.0 * s;
    let panel = Rect::from_min_size(pos2(screen.right() - width * e, screen.top()), vec2(width, screen.height()));
    paint::shadow(&p, panel, 0.0, s, 1.0, Color32::from_black_alpha(200));
    p.rect_filled(panel, 0.0, t.chrome);
    let edge = Rect::from_min_size(panel.min, vec2(3.0 * s, panel.height()));
    w::gradient(&p, edge, t.accent, t.accent_2, false);
    let blocker = ui.interact(panel, Id::unique("bp-menu-panel"), egui::Sense::click());
    if outside.clicked() && !blocker.hovered() {
        m.closing = Some(now);
    }

    // Who's signed in.
    let pad = 48.0 * s;
    let me = f.app.state.me().cloned();
    let av = Rect::from_min_size(panel.min + vec2(pad, 70.0 * s), vec2(84.0 * s, 84.0 * s));
    let url = model::art::avatar_full(me.as_ref().and_then(|x| x.avatar_hash.as_deref()));
    if !w::paint_cover(ui, av, &url, paint::radius(42.0 * s)) {
        p.circle_filled(av.center(), 42.0 * s, t.raised);
    }
    let status = me.as_ref().map(|x| crate::shell::status_text(&f.app.state, x)).unwrap_or_default();
    let color = me.as_ref().map(|x| crate::shell::status_color(&t, x)).unwrap_or(t.text_dim);
    p.circle_stroke(av.center(), 44.0 * s, egui::Stroke::new(3.0 * s, color));
    paint::line(&p, pos2(av.right() + 22.0 * s, av.center().y - 4.0 * s), Align2::LEFT_BOTTOM, &f.app.state.persona_name, w::font_bold(30.0 * s), t.text, panel.right() - av.right() - pad - 22.0 * s);
    paint::line(&p, pos2(av.right() + 22.0 * s, av.center().y + 4.0 * s), Align2::LEFT_TOP, &status, w::font_medium(20.0 * s), color, panel.right() - av.right() - pad - 22.0 * s);

    let items = [
        (icons::DESKTOP, t!("Exit Big Picture"), false),
        (icons::GEAR, t!("Settings"), false),
        (icons::X, t!("Close menu"), false),
        (icons::POWER, t!("Quit SteamInRust"), true),
    ];
    let dir = if m.closing.is_none() { m.dir.take() } else { None };
    let press = std::mem::take(&mut m.press) && m.closing.is_none();
    m.focus.begin(ui, dir, press);
    let row_h = 84.0 * s;
    let mut y = av.bottom() + 60.0 * s;
    let mut chosen = None;
    for (i, (icon, label, danger)) in items.iter().enumerate() {
        // Rows arrive one after another as the panel opens.
        let k = ease(((now - m.opened) as f32 - i as f32 * 0.04) / OPEN_SECS);
        let dx = (1.0 - k) * 60.0 * s;
        let r = Rect::from_min_size(pos2(panel.left() + pad * 0.6 + dx, y), vec2(width - pad * 1.2, row_h));
        let hit = m.focus.item(ui, Id::unique(("bp-menu", i)), r, screen, Place::Page);
        let fill = w::mix(Color32::TRANSPARENT, Color32::from_white_alpha(22), hit.glow);
        p.rect_filled(r, paint::radius(14.0 * s), fill);
        let c = if *danger { t.danger } else { t.text };
        p.text(pos2(r.left() + 26.0 * s, r.center().y), Align2::LEFT_CENTER, *icon, w::font(30.0 * s), c);
        p.text(pos2(r.left() + 76.0 * s, r.center().y), Align2::LEFT_CENTER, *label, w::font_semibold(26.0 * s), c);
        if hit.activated {
            chosen = Some(i);
        }
        y += row_h + 8.0 * s;
        // A line before the quit row.
        if i == 2 {
            p.hline(r.x_range(), y + 4.0 * s, egui::Stroke::new(1.0, Color32::from_white_alpha(20)));
            y += 16.0 * s;
        }
    }
    let ids: Vec<Id> = (0..items.len()).map(|i| Id::unique(("bp-menu", i))).collect();
    m.focus.end(&ids[..1], ids[0]);
    m.focus.paint_ring(ui, &t, s, 14.0 * s);
    let tip = if bp.device == super::input::Device::Pad { t!("Press Guide again to leave Big Picture") } else { t!("Press Home to leave Big Picture") };
    p.text(pos2(panel.left() + pad, panel.bottom() - 40.0 * s), Align2::LEFT_BOTTOM, tip, w::font(17.0 * s), t.text_faint);

    match chosen {
        Some(0) => {
            bp.leave(ui.ctx());
            return;
        }
        Some(1) => {
            bp.open(Page::Settings, now);
            m.closing = Some(now);
        }
        Some(2) => m.closing = Some(now),
        Some(3) => {
            m.closing = Some(now);
            f.app.request_exit(ui.ctx());
        }
        _ => {}
    }
    bp.menu = Some(m);
}

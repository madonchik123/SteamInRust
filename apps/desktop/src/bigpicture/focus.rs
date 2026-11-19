//! Focus for a couch: one thing is always focused, arrows/D-pad move it to
//! the nearest thing in that direction, and a glowing ring glides after it.

use egui::{Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui};
use ui::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Where an item lives: in the scrolling page or in the fixed bars.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Page,
    Bar,
}

#[derive(Clone, Copy)]
struct Item {
    id: Id,
    rect: Rect,
    place: Place,
}

#[derive(Default)]
pub struct Focus {
    pub current: Option<Id>,
    /// The focus got where it is by the mouse pointing at it (not keys, a
    /// controller or a click). Tabs only switch on a click then.
    pub by_pointer: bool,
    items: Vec<Item>,
    last: Vec<Item>,
    press: bool,
    pointer_moved: bool,
    /// The focus moved by D-pad/keys this frame (scroll it into view).
    pub moved: bool,
    /// Ring position (animated) and where it's heading.
    ring: Option<Rect>,
    ring_target: Option<(Rect, f32)>,
    /// No ring this frame (a full-page viewer: a box around the whole page
    /// only gets in the way).
    no_ring: bool,
    /// A move with nowhere to go: the ring nudges that way.
    bump: Option<(Dir, f64)>,
    /// Focus last frame, and when it last changed (for the tile sheen).
    seen: Option<Id>,
    pub changed_at: f64,
    /// When this focus scope was first used.
    opened: f64,
    last_pointer: Option<egui::Pos2>,
}

pub struct Hit {
    pub focused: bool,
    pub activated: bool,
    /// 0..1, eases in and out with focus.
    pub glow: f32,
}

impl Focus {
    /// Start a frame: apply a move using last frame's item positions.
    pub fn begin(&mut self, ui: &Ui, dir: Option<Dir>, press: bool) {
        if self.opened == 0.0 {
            self.opened = ui.input(|i| i.time).max(0.001);
        }
        self.last = std::mem::take(&mut self.items);
        self.press = press;
        self.moved = false;
        self.ring_target = None;
        self.no_ring = false;
        let now = ui.input(|i| i.time);
        // The mouse takes focus only when it really moves (not when a page
        // scrolls under it, nor right as the screen opens).
        // The first position seen is just where the cursor already was (the
        // window appeared under it): only a move after that counts.
        let pos = ui.input(|i| i.pointer.latest_pos());
        self.pointer_moved = matches!((self.last_pointer, pos), (Some(a), Some(b)) if (a - b).length() > 1.0) && now - self.opened > 0.6;
        if pos.is_some() {
            self.last_pointer = pos;
        }
        if let Some(d) = dir {
            self.step(d, now);
        }
        if self.current != self.seen {
            self.seen = self.current;
            self.changed_at = now;
        }
    }

    fn step(&mut self, dir: Dir, now: f64) {
        self.by_pointer = false;
        let Some(from) = self.current.and_then(|c| self.last.iter().find(|i| i.id == c)).copied() else {
            self.current = self.last.first().map(|i| i.id);
            self.moved = true;
            return;
        };
        match nearest(&self.last, from.rect, dir) {
            Some(id) => {
                self.current = Some(id);
                self.moved = true;
            }
            None => self.bump = Some((dir, now)),
        }
    }

    /// Register a focusable item. `clip` is the visible area (mouse only
    /// reaches what's visible; the D-pad reaches everything).
    pub fn item(&mut self, ui: &Ui, id: Id, rect: Rect, clip: Rect, place: Place) -> Hit {
        self.items.push(Item { id, rect, place });
        let mut activated = false;
        let visible = rect.intersect(clip);
        if visible.is_positive() {
            let r = ui.interact(visible, id, Sense::click());
            if r.hovered() && self.pointer_moved && self.current != Some(id) {
                self.current = Some(id);
                self.by_pointer = true;
            }
            if r.clicked() {
                self.current = Some(id);
                self.by_pointer = false;
                activated = true;
            }
        }
        let focused = self.current == Some(id);
        if focused && self.press {
            activated = true;
            self.press = false;
        }
        let glow = ui.ctx().animate_bool_with_time(id.with("bp-focus"), focused, 0.18);
        if focused {
            self.ring_target = Some((rect, 0.0));
        }
        Hit { focused, activated, glow }
    }

    /// Ring around a different shape than the item (a grown tile, a pill).
    /// Hide the ring this frame.
    pub fn hide_ring(&mut self) {
        self.no_ring = true;
    }

    pub fn ring_around(&mut self, rect: Rect, radius: f32) {
        self.ring_target = Some((rect, radius));
    }

    pub fn exists(&self, id: Id) -> bool {
        self.items.iter().any(|i| i.id == id)
    }

    /// End a frame: if the focused thing is gone, focus the first of
    /// `first` that exists, else the first page item, else `last`, else
    /// anything. So something is always focused.
    pub fn end(&mut self, first: &[Id], last: Id) {
        if self.current.is_some_and(|c| self.exists(c)) {
            return;
        }
        self.current = first
            .iter()
            .copied()
            .find(|f| self.exists(*f))
            .or_else(|| self.items.iter().find(|i| i.place == Place::Page).map(|i| i.id))
            .or_else(|| Some(last).filter(|l| self.exists(*l)))
            .or_else(|| self.items.first().map(|i| i.id));
        self.ring_target = None;
    }

    /// The focused item's rect and place this frame.
    pub fn focused(&self) -> Option<(Rect, Place)> {
        let c = self.current?;
        self.items.iter().find(|i| i.id == c).map(|i| (i.rect, i.place))
    }

    /// Paint the ring: glides to the focused item, glows and breathes.
    pub fn paint_ring(&mut self, ui: &Ui, t: &Theme, s: f32, radius: f32) {
        if self.no_ring {
            self.ring = None;
            return;
        }
        let Some((target, r_override)) = self.ring_target.or_else(|| self.focused().map(|f| (f.0, 0.0))) else {
            self.ring = None;
            return;
        };
        let (dt, now) = ui.input(|i| (i.stable_dt.min(0.1), i.time));
        let k = 1.0 - (-dt * 16.0).exp();
        let ring = match self.ring {
            // Far jumps (another screen) snap closer quickly but still glide.
            Some(r) => Rect::from_min_max(r.min + (target.min - r.min) * k, r.max + (target.max - r.max) * k),
            None => target,
        };
        let settled = (ring.min - target.min).length() < 0.5 && (ring.max - target.max).length() < 0.5;
        self.ring = Some(if settled { target } else { ring });
        if !settled {
            ui.ctx().request_repaint();
        }
        let mut rect = self.ring.unwrap();
        // Nudge when there's nowhere to go.
        if let Some((dir, at)) = self.bump {
            let e = (now - at) as f32 / 0.25;
            if e >= 1.0 {
                self.bump = None;
            } else {
                let amount = (e * std::f32::consts::PI).sin() * 10.0 * s;
                let v = match dir {
                    Dir::Up => egui::vec2(0.0, -amount),
                    Dir::Down => egui::vec2(0.0, amount),
                    Dir::Left => egui::vec2(-amount, 0.0),
                    Dir::Right => egui::vec2(amount, 0.0),
                };
                rect = rect.translate(v);
                ui.ctx().request_repaint();
            }
        }
        let radius = if r_override > 0.0 { r_override } else { radius };
        let pulse = ((now * 2.6).sin() as f32) * 0.5 + 0.5;
        let p = ui.painter();
        // Soft outer glow, wider and fainter outward.
        for i in 1..=5 {
            let grow = i as f32 * 3.0 * s + pulse * 2.0 * s;
            let a = (0.16 - i as f32 * 0.025).max(0.0) * (0.75 + 0.25 * pulse);
            p.rect_stroke(rect.expand(grow), CornerRadius::same((radius + grow).min(255.0) as u8), Stroke::new(3.0 * s, t.accent.gamma_multiply(a)), StrokeKind::Outside);
        }
        let ring_color = ui::widgets::mix(t.accent, Color32::WHITE, 0.25 + 0.2 * pulse);
        p.rect_stroke(rect.expand(2.0 * s), CornerRadius::same((radius + 2.0 * s).min(255.0) as u8), Stroke::new(3.5 * s, ring_color), StrokeKind::Outside);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }

    /// Forget the ring so it appears without gliding (opening an overlay).
    pub fn reset_ring(&mut self) {
        self.ring = None;
    }
}

/// The best item in `dir` from `from`: close along the way, and lined up
/// across it (items in the same row or column win over diagonal ones).
fn nearest(items: &[Item], from: Rect, dir: Dir) -> Option<Id> {
    let c = from.center();
    let mut best: Option<(f32, Id)> = None;
    for it in items {
        let r = it.rect;
        if r == from {
            continue;
        }
        let rc = r.center();
        // Gap along the move, and how far off-line across it.
        let (ahead, gap, off, off_center) = match dir {
            Dir::Down => (rc.y > c.y + 1.0 && r.bottom() > from.bottom() - 1.0, r.top() - from.bottom(), span_gap(from.x_range(), r.x_range()), (rc.x - c.x).abs()),
            Dir::Up => (rc.y < c.y - 1.0 && r.top() < from.top() + 1.0, from.top() - r.bottom(), span_gap(from.x_range(), r.x_range()), (rc.x - c.x).abs()),
            Dir::Right => (rc.x > c.x + 1.0 && r.right() > from.right() - 1.0, r.left() - from.right(), span_gap(from.y_range(), r.y_range()), (rc.y - c.y).abs()),
            Dir::Left => (rc.x < c.x - 1.0 && r.left() < from.left() + 1.0, from.left() - r.right(), span_gap(from.y_range(), r.y_range()), (rc.y - c.y).abs()),
        };
        // Sideways moves stay in the row: the end of a shelf stops there
        // instead of jumping to a shelf above or below.
        if !ahead || (matches!(dir, Dir::Left | Dir::Right) && off > 0.0) {
            continue;
        }
        let score = gap.max(0.0) + off * 4.0 + off_center * 0.05;
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, it.id));
        }
    }
    best.map(|b| b.1)
}

/// Distance between two ranges, 0 when they overlap.
fn span_gap(a: egui::Rangef, b: egui::Rangef) -> f32 {
    if a.max < b.min {
        b.min - a.max
    } else if b.max < a.min {
        a.min - b.max
    } else {
        0.0
    }
}

/// Smooth scrolling toward a target that follows the focus.
#[derive(Default, Clone, Copy, Debug)]
pub struct Scroller {
    pub pos: f32,
    pub target: f32,
}

impl Scroller {
    /// Bring `start..end` (content coordinates) into a view of `view` length,
    /// keeping `margin` of room around it.
    pub fn reveal(&mut self, start: f32, end: f32, view: f32, margin: f32) {
        let margin = margin.min(((view - (end - start)) / 2.0).max(0.0));
        if start - margin < self.target {
            self.target = start - margin;
        } else if end + margin > self.target + view {
            self.target = end + margin - view;
        }
    }

    pub fn clamp(&mut self, content: f32, view: f32) {
        self.target = self.target.clamp(0.0, (content - view).max(0.0));
    }

    /// Move toward the target; true while still moving.
    pub fn step(&mut self, ui: &Ui) -> bool {
        let dt = ui.input(|i| i.stable_dt.min(0.1));
        let k = 1.0 - (-dt * 12.0).exp();
        self.pos += (self.target - self.pos) * k;
        if (self.target - self.pos).abs() < 0.5 {
            self.pos = self.target;
            false
        } else {
            ui.ctx().request_repaint();
            true
        }
    }
}

/// Ease-out cubic.
pub fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// 0→1 over `secs` since `start`, eased (and keeps painting until done).
pub fn since(ui: &Ui, start: f64, secs: f32) -> f32 {
    let e = ((ui.input(|i| i.time) - start) as f32 / secs).clamp(0.0, 1.0);
    if e < 1.0 {
        ui.ctx().request_repaint();
    }
    ease(e)
}

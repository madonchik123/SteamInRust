//! The overlay drawn inside the game (see `crates/steamclient/src/overlay`).
//!
//! When a game we started draws the overlay itself, its DLL connects over a
//! pipe. We run the overlay's UI (the same code as the window overlay in
//! `overlay.rs`) in a separate egui context with the game's mouse and
//! keyboard as input, and send the resulting triangles and textures back for
//! the DLL to draw on top of the game's frames.

use client::ingame::Conn;
use client::ingame::wire::{self, FromGame, Input, ToGame};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Session {
    pub conn: Conn,
    pub ctx: egui::Context,
    size: (u32, u32),
    /// What the DLL last heard: overlay open (it has the input).
    pub game_open: bool,
    events: Vec<Event>,
    modifiers: Modifiers,
    started: Instant,
    /// When the overlay's UI wants to run again (egui's repaint requests).
    wake_at: Arc<Mutex<Option<Instant>>>,
    last_run: Option<Instant>,
    /// The last frame sent drew nothing.
    sent_empty: bool,
    pixels_per_point: f32,
    theme: Option<ui::Theme>,
}

impl Session {
    pub fn new(conn: Conn, main: &egui::Context, rt: &tokio::runtime::Runtime, theme: &ui::Theme) -> Self {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        ui::install_fonts(&ctx);
        crate::image_cache::ImageCache::install(&ctx, rt.handle().clone());
        theme.apply(&ctx);
        no_animations(&ctx);
        let wake_at: Arc<Mutex<Option<Instant>>> = Arc::default();
        let (w, main) = (wake_at.clone(), main.clone());
        // Images loading, animations, the clock: run again when it asks.
        ctx.set_request_repaint_callback(move |info| {
            let at = Instant::now() + info.delay;
            let mut slot = w.lock().unwrap();
            if slot.is_none_or(|s| at < s) {
                *slot = Some(at);
            }
            wake(&main, info.delay);
        });
        let size = conn.size;
        Self {
            conn,
            ctx,
            size,
            game_open: false,
            events: Vec::new(),
            modifiers: Modifiers::default(),
            started: Instant::now(),
            wake_at,
            last_run: None,
            sent_empty: false,
            pixels_per_point: 1.0,
            theme: Some(theme.clone()),
        }
    }

    /// Read what the game sent. Returns `Some(open)` when the game opened or
    /// closed the overlay (Shift+Tab), and false once the game is gone.
    pub fn poll(&mut self) -> (bool, Option<bool>) {
        let mut opened = None;
        loop {
            match self.conn.from_game.try_recv() {
                Ok(FromGame::Opened(open)) => {
                    self.game_open = open;
                    opened = Some(open);
                    self.wake_now();
                }
                Ok(FromGame::Resize { width, height }) => {
                    self.size = (width, height);
                    self.wake_now();
                }
                Ok(FromGame::Input { modifiers, events }) => {
                    self.input(modifiers, events);
                    self.wake_now();
                }
                Ok(FromGame::Hello { .. }) => {}
                Err(std::sync::mpsc::TryRecvError::Empty) => return (true, opened),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return (false, opened),
            }
        }
    }

    /// Points per back-buffer pixel: sized as if the game's resolution were
    /// a 1080p screen.
    fn native(&self) -> f32 {
        (self.size.1.max(1) as f32 / 1080.0).clamp(0.75, 3.0)
    }

    /// Keep the overlay's look in step with the app's theme.
    pub fn theme(&mut self, theme: &ui::Theme) {
        if self.theme.as_ref() != Some(theme) {
            theme.apply(&self.ctx);
            no_animations(&self.ctx);
            self.theme = Some(theme.clone());
            self.wake_now();
        }
    }

    fn wake_now(&self) {
        *self.wake_at.lock().unwrap() = Some(Instant::now());
    }

    /// Tell the game to open or close (the overlay's close button, Esc, a
    /// game asking for a dialog).
    pub fn set_open(&mut self, open: bool) {
        if self.game_open != open {
            self.game_open = open;
            self.conn.send(&ToGame::SetOpen(open));
            self.wake_now();
        }
    }

    fn input(&mut self, mods: u8, events: Vec<Input>) {
        let m = Modifiers {
            alt: mods & wire::MOD_ALT != 0,
            ctrl: mods & wire::MOD_CTRL != 0,
            shift: mods & wire::MOD_SHIFT != 0,
            mac_cmd: false,
            command: mods & wire::MOD_CTRL != 0,
        };
        if m != self.modifiers {
            self.modifiers = m;
            self.events.push(Event::ModifiersChanged(m));
        }
        let ppp = self.native();
        let pos = |x: f32, y: f32| Pos2::new(x / ppp, y / ppp);
        for e in events {
            match e {
                Input::Move { x, y } => self.events.push(Event::PointerMoved(pos(x, y))),
                Input::Button { button, down, x, y } => {
                    let button = match button {
                        0 => PointerButton::Primary,
                        1 => PointerButton::Secondary,
                        2 => PointerButton::Middle,
                        3 => PointerButton::Extra1,
                        _ => PointerButton::Extra2,
                    };
                    self.events.push(Event::PointerMoved(pos(x, y)));
                    self.events.push(Event::PointerButton { pos: pos(x, y), button, pressed: down, modifiers: m });
                }
                Input::Wheel { x, y } => self.events.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: Vec2::new(x, y),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    modifiers: m,
                }),
                Input::Key { vk, down, repeat } => {
                    if down && m.ctrl {
                        match vk {
                            0x43 => self.events.push(Event::Copy),
                            0x58 => self.events.push(Event::Cut),
                            // Ctrl+V comes as Paste with the text.
                            0x56 => continue,
                            _ => {}
                        }
                    }
                    if let Some(key) = key_from_vk(vk) {
                        self.events.push(Event::Key { key, physical_key: None, pressed: down, repeat, modifiers: m });
                    }
                }
                Input::Text(t) => self.events.push(Event::Text(t)),
                Input::Paste(t) => self.events.push(Event::Paste(t)),
                Input::Leave => self.events.push(Event::PointerGone),
            }
        }
    }

    /// Whether the UI should run now; if not yet, asks the main window to
    /// come back in time.
    fn due(&self, main: &egui::Context) -> bool {
        let Some(at) = *self.wake_at.lock().unwrap() else { return false };
        let now = Instant::now();
        // At most ~60 frames a second.
        let earliest = self.last_run.map(|t| t + Duration::from_millis(15)).unwrap_or(now);
        let at = at.max(earliest);
        if at <= now {
            return true;
        }
        wake(main, at - now);
        false
    }

    /// Run the overlay UI with the game's input and send the frame. `draw`
    /// is false when nothing should show (closed, nothing pinned).
    pub fn run(&mut self, main: &egui::Context, draw: bool, ui: impl FnMut(&mut egui::Ui)) {
        if !draw {
            if !self.sent_empty {
                self.conn.send(&ToGame::Frame { pixels_per_point: self.pixels_per_point, cursor: wire::cursor::ARROW, set: vec![], free: vec![], meshes: vec![] });
                self.sent_empty = true;
            }
            self.events.clear();
            return;
        }
        if self.sent_empty {
            self.sent_empty = false;
            self.wake_now();
        }
        if !self.due(main) {
            return;
        }
        *self.wake_at.lock().unwrap() = None;
        self.last_run = Some(Instant::now());
        let (w, h) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
        // Sized for the game's resolution, like it was a 1080p screen.
        let native = self.native();
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h) / native)),
            time: Some(self.started.elapsed().as_secs_f64()),
            predicted_dt: 1.0 / 60.0,
            events: std::mem::take(&mut self.events),
            focused: true,
            max_texture_side: Some(8192),
            ..Default::default()
        };
        let vp = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        vp.native_pixels_per_point = Some(native);
        vp.focused = Some(true);
        vp.monitor_size = Some(Vec2::new(w, h) / native);
        let mut out = self.ctx.run_ui(raw, ui);
        self.pixels_per_point = out.pixels_per_point;
        for c in &out.platform_output.commands {
            match c {
                egui::OutputCommand::CopyText(t) => main.copy_text(t.clone()),
                egui::OutputCommand::OpenUrl(u) => main.open_url(u.clone()),
                _ => {}
            }
        }
        if let Some(info) = out.viewport_output.get(&egui::ViewportId::ROOT) {
            if info.repaint_delay < Duration::from_secs(3600) {
                let at = Instant::now() + info.repaint_delay;
                let mut slot = self.wake_at.lock().unwrap();
                if slot.is_none_or(|s| at < s) {
                    *slot = Some(at);
                }
                wake(main, info.repaint_delay.max(Duration::from_millis(15)));
            }
        }
        let cursor = cursor(out.platform_output.cursor_icon);
        let mut textures = std::mem::take(&mut out.textures_delta);
        let set = textures
            .set
            .iter()
            .flat_map(|(id, deltas)| deltas.iter().map(move |d| (id, d)))
            .map(|(id, d)| {
                let egui::ImageData::Color(img) = &d.image;
                wire::TexSet {
                    id: tex_id(*id),
                    pos: d.pos.map(|[x, y]| [x as u32, y as u32]),
                    size: [img.size[0] as u32, img.size[1] as u32],
                    linear: d.options.magnification == egui::TextureFilter::Linear,
                    rgba: img.as_raw().to_vec(),
                }
            })
            .collect();
        let free = textures.free.iter().map(|id| tex_id(*id)).collect();
        // Handed to the game; egui checks that nobody dropped them unseen.
        textures.clear();
        let meshes = self
            .ctx
            .tessellate(out.shapes, out.pixels_per_point)
            .into_iter()
            .filter_map(|p| match p.primitive {
                egui::epaint::Primitive::Mesh(m) if !m.indices.is_empty() => {
                    let mut vertices = Vec::with_capacity(m.vertices.len() * wire::VERTEX_SIZE);
                    for v in &m.vertices {
                        for f in [v.pos.x, v.pos.y, v.uv.x, v.uv.y] {
                            vertices.extend_from_slice(&f.to_le_bytes());
                        }
                        vertices.extend_from_slice(&v.color.to_array());
                    }
                    let c = p.clip_rect;
                    Some(wire::Mesh { clip: [c.min.x, c.min.y, c.max.x, c.max.y], texture: tex_id(m.texture_id), vertices, indices: m.indices })
                }
                _ => None,
            })
            .collect();
        self.conn.send(&ToGame::Frame { pixels_per_point: out.pixels_per_point, cursor, set, free, meshes });
    }
}

fn tex_id(id: egui::TextureId) -> u64 {
    match id {
        egui::TextureId::Managed(n) => n << 1,
        egui::TextureId::User(n) => (n << 1) | 1,
    }
}

fn cursor(c: egui::CursorIcon) -> u8 {
    use egui::CursorIcon as C;
    use wire::cursor::*;
    match c {
        C::PointingHand => HAND,
        C::Text | C::VerticalText => TEXT,
        C::ResizeHorizontal | C::ResizeEast | C::ResizeWest | C::ResizeColumn => RESIZE_H,
        C::ResizeVertical | C::ResizeNorth | C::ResizeSouth | C::ResizeRow => RESIZE_V,
        C::ResizeNwSe | C::ResizeNorthWest | C::ResizeSouthEast => RESIZE_NWSE,
        C::ResizeNeSw | C::ResizeNorthEast | C::ResizeSouthWest => RESIZE_NESW,
        C::Move | C::Grab | C::Grabbing | C::AllScroll => MOVE,
        C::NotAllowed | C::NoDrop => NOT_ALLOWED,
        _ => ARROW,
    }
}

/// Windows virtual-key code to egui's key.
fn key_from_vk(vk: u32) -> Option<Key> {
    Some(match vk {
        0x08 => Key::Backspace,
        0x09 => Key::Tab,
        0x0D => Key::Enter,
        0x1B => Key::Escape,
        0x20 => Key::Space,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x23 => Key::End,
        0x24 => Key::Home,
        0x25 => Key::ArrowLeft,
        0x26 => Key::ArrowUp,
        0x27 => Key::ArrowRight,
        0x28 => Key::ArrowDown,
        0x2D => Key::Insert,
        0x2E => Key::Delete,
        0x30..=0x39 => Key::from_name(&char::from_u32(vk)?.to_string())?,
        0x41..=0x5A => Key::from_name(&char::from_u32(vk)?.to_string())?,
        0x70..=0x7B => Key::from_name(&format!("F{}", vk - 0x6F))?,
        0xBA => Key::Semicolon,
        0xBB => Key::Equals,
        0xBC => Key::Comma,
        0xBD => Key::Minus,
        0xBE => Key::Period,
        0xBF => Key::Slash,
        0xC0 => Key::Backtick,
        0xDB => Key::OpenBracket,
        0xDC => Key::Backslash,
        0xDD => Key::CloseBracket,
        0xDE => Key::Quote,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(key_from_vk(0x41), Some(Key::A));
        assert_eq!(key_from_vk(0x35), Some(Key::Num5));
        assert_eq!(key_from_vk(0x7B), Some(Key::F12));
        assert_eq!(key_from_vk(0x1B), Some(Key::Escape));
        assert_eq!(key_from_vk(0xFF), None);
    }
}

/// The in-game UI only gets frames when the app's main window repaints, so
/// fades and smooth scrolling stutter there. Hover and scroll change at once.
fn no_animations(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.animation_time = 0.0;
        s.scroll_animation = egui::style::ScrollAnimation::none();
    });
}

/// The helper window whose frames run the in-game overlay (see
/// `overlay::keep_ticking`).
pub fn tick() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("steaminrust-ingame-tick")
}

/// Ask for an in-game overlay frame after `delay`: on the helper window, and
/// on the main window (which runs it when the helper can't).
pub fn wake(main: &egui::Context, delay: Duration) {
    main.request_repaint_after_for(delay, tick());
    main.request_repaint_after_for(delay, egui::ViewportId::ROOT);
}

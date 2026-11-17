//! Controller and keyboard input for Big Picture, turned into the same few
//! presses: a direction, A (select), B (back), X, Y, LB/RB (tabs), Start
//! (menu) and Guide.

use egui::{Key, Modifiers};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::focus::Dir;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Press {
    Move(Dir),
    A,
    B,
    X,
    Y,
    Lb,
    Rb,
    Start,
    Guide,
}

/// What the last input came from: hints show buttons or keys to match.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Device {
    #[default]
    Keyboard,
    Pad,
}

pub enum PadEvent {
    Press(Press),
    /// How many controllers are connected now.
    Connected(usize),
}

/// Start reading controllers on a background thread. Each press wakes the UI.
pub fn spawn(ctx: egui::Context) -> mpsc::Receiver<PadEvent> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("gamepads".into()).spawn(move || read_pads(tx, ctx));
    if let Err(e) = spawned {
        tracing::warn!("controllers: {e}");
    }
    rx
}

/// A held direction repeats like a held key: once, then after a pause, steadily.
const FIRST_REPEAT: Duration = Duration::from_millis(380);
const REPEAT: Duration = Duration::from_millis(120);
/// Stick thresholds (with a gap so a stick near the edge doesn't flicker).
const STICK_ON: f32 = 0.55;
const STICK_OFF: f32 = 0.35;

/// On Windows the gamepad library reads every pad 125 times a second, even
/// with none plugged in, which was nearly all of the app's CPU use while
/// idle. So it only runs while a controller is connected: Windows says when
/// one is plugged in, and Guide then opens Big Picture as before.
fn read_pads(tx: mpsc::Sender<PadEvent>, ctx: egui::Context) {
    #[cfg(windows)]
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    #[cfg(windows)]
    let _watch = plugged::on_added(move || {
        let _ = wake_tx.send(());
    });
    #[cfg(windows)]
    let mut announced = None;
    loop {
        #[cfg(windows)]
        if plugged::count() == 0 {
            if announced != Some(0) {
                announced = Some(0);
                if tx.send(PadEvent::Connected(0)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            // Sleep until one is plugged in (and look again now and then).
            let _ = wake_rx.recv_timeout(Duration::from_secs(10));
            while wake_rx.try_recv().is_ok() {}
            continue;
        }
        #[cfg(windows)]
        {
            announced = None;
        }
        let gilrs = match gilrs::Gilrs::new() {
            Ok(g) => g,
            Err(gilrs::Error::NotImplemented(g)) => g,
            Err(e) => {
                tracing::warn!("controllers unavailable: {e}");
                return;
            }
        };
        if !read_connected(gilrs, &tx, &ctx) {
            return;
        }
    }
}

/// Windows' own list of game controllers, for the cheap "is one plugged in?".
#[cfg(windows)]
mod plugged {
    use windows::Foundation::EventHandler;
    use windows::Gaming::Input::RawGameController;

    pub fn count() -> u32 {
        RawGameController::RawGameControllers().and_then(|list| list.Size()).unwrap_or(0)
    }

    /// Calls `wake` whenever a controller is plugged in, for as long as the
    /// returned value is kept.
    pub fn on_added(wake: impl Fn() + Send + Sync + 'static) -> Option<Watch> {
        let handler = EventHandler::<RawGameController>::new(move |_, _| {
            wake();
            Ok(())
        });
        RawGameController::RawGameControllerAdded(&handler).ok().map(Watch)
    }

    pub struct Watch(i64);

    impl Drop for Watch {
        fn drop(&mut self) {
            let _ = RawGameController::RemoveRawGameControllerAdded(self.0);
        }
    }
}

/// Read pads until the app is gone (`false`) or, on Windows, the last pad is
/// unplugged (`true`).
fn read_connected(mut gilrs: gilrs::Gilrs, tx: &mpsc::Sender<PadEvent>, ctx: &egui::Context) -> bool {
    use gilrs::{Axis, Button, EventType};
    let send = |e: PadEvent| {
        let ok = tx.send(e).is_ok();
        ctx.request_repaint();
        ok
    };
    let dirs = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];
    let mut dpad = [false; 4];
    let mut stick = [false; 4];
    // When each held direction fires next.
    let mut next: [Option<Instant>; 4] = [None; 4];
    let mut connected = gilrs.gamepads().count();
    if !send(PadEvent::Connected(connected)) {
        return false;
    }
    loop {
        while let Some(ev) = gilrs.next_event_blocking(Some(Duration::from_millis(30))) {
            let press = match ev.event {
                EventType::ButtonPressed(b, _) => match b {
                    Button::South => Some(Press::A),
                    Button::East => Some(Press::B),
                    Button::West => Some(Press::X),
                    Button::North => Some(Press::Y),
                    Button::LeftTrigger => Some(Press::Lb),
                    Button::RightTrigger => Some(Press::Rb),
                    Button::Start => Some(Press::Start),
                    Button::Mode => Some(Press::Guide),
                    Button::DPadUp => {
                        dpad[0] = true;
                        None
                    }
                    Button::DPadDown => {
                        dpad[1] = true;
                        None
                    }
                    Button::DPadLeft => {
                        dpad[2] = true;
                        None
                    }
                    Button::DPadRight => {
                        dpad[3] = true;
                        None
                    }
                    _ => None,
                },
                EventType::ButtonReleased(b, _) => {
                    match b {
                        Button::DPadUp => dpad[0] = false,
                        Button::DPadDown => dpad[1] = false,
                        Button::DPadLeft => dpad[2] = false,
                        Button::DPadRight => dpad[3] = false,
                        _ => {}
                    }
                    None
                }
                EventType::AxisChanged(axis, v, _) => {
                    // Past the "on" line to start, back inside the "off" line to stop.
                    let edge = |held: bool| if held { STICK_OFF } else { STICK_ON };
                    match axis {
                        // Up is positive on the stick's Y axis.
                        Axis::LeftStickY => {
                            stick[0] = v > edge(stick[0]);
                            stick[1] = -v > edge(stick[1]);
                        }
                        Axis::LeftStickX => {
                            stick[2] = -v > edge(stick[2]);
                            stick[3] = v > edge(stick[3]);
                        }
                        _ => {}
                    }
                    None
                }
                EventType::Connected | EventType::Disconnected => {
                    let now = gilrs.gamepads().count();
                    if now != connected {
                        connected = now;
                        if !send(PadEvent::Connected(now)) {
                            return false;
                        }
                    }
                    if cfg!(windows) && now == 0 {
                        // Back to waiting for Windows to say one is plugged in.
                        return true;
                    }
                    None
                }
                _ => None,
            };
            if let Some(p) = press {
                if !send(PadEvent::Press(p)) {
                    return false;
                }
            }
        }
        // Held directions: fire on press, then repeat.
        let now = Instant::now();
        for i in 0..4 {
            let held = dpad[i] || stick[i];
            match (held, next[i]) {
                (true, None) => {
                    next[i] = Some(now + FIRST_REPEAT);
                    if !send(PadEvent::Press(Press::Move(dirs[i]))) {
                        return false;
                    }
                }
                (true, Some(at)) if now >= at => {
                    next[i] = Some(now + REPEAT);
                    if !send(PadEvent::Press(Press::Move(dirs[i]))) {
                        return false;
                    }
                }
                (false, Some(_)) => next[i] = None,
                _ => {}
            }
        }
    }
}

/// This frame's keyboard presses (taken, so egui doesn't also act on them).
/// `typing`: letters go to a text box, so only keys that aren't text count.
pub fn keyboard(ctx: &egui::Context, typing: bool) -> Vec<Press> {
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        let mut take = |key: Key, press: Press| {
            if i.consume_key(Modifiers::NONE, key) {
                out.push(press);
            }
        };
        take(Key::ArrowUp, Press::Move(Dir::Up));
        take(Key::ArrowDown, Press::Move(Dir::Down));
        take(Key::ArrowLeft, Press::Move(Dir::Left));
        take(Key::ArrowRight, Press::Move(Dir::Right));
        take(Key::Enter, Press::A);
        take(Key::Escape, Press::B);
        take(Key::PageUp, Press::Lb);
        take(Key::PageDown, Press::Rb);
        take(Key::F1, Press::Start);
        take(Key::Home, Press::Guide);
        if !typing {
            take(Key::Space, Press::A);
            take(Key::Backspace, Press::B);
            take(Key::Q, Press::Lb);
            take(Key::E, Press::Rb);
            take(Key::X, Press::X);
            take(Key::Y, Press::Y);
            take(Key::M, Press::Start);
            // WASD moves too.
            take(Key::W, Press::Move(Dir::Up));
            take(Key::S, Press::Move(Dir::Down));
            take(Key::A, Press::Move(Dir::Left));
            take(Key::D, Press::Move(Dir::Right));
        }
        if i.consume_key(Modifiers::SHIFT, Key::Tab) {
            out.push(Press::Lb);
        }
        if i.consume_key(Modifiers::NONE, Key::Tab) {
            out.push(Press::Rb);
        }
    });
    out
}

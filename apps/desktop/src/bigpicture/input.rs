//! Controller and keyboard input for Big Picture, turned into the same few
//! presses: a direction, A (select), B (back), X, Y, LB/RB (tabs), Start
//! (menu) and Guide.

use egui::{Key, Modifiers};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
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

/// The controller settings, read by the thread that reads the pads.
static SWAP_AB: AtomicBool = AtomicBool::new(false);
static REPEAT_LEVEL: AtomicU8 = AtomicU8::new(1);
static STICK_LEVEL: AtomicU8 = AtomicU8::new(1);

/// Apply the controller settings.
pub fn tune(swap_ab: bool, repeat: u8, stick: u8) {
    SWAP_AB.store(swap_ab, Ordering::Relaxed);
    REPEAT_LEVEL.store(repeat, Ordering::Relaxed);
    STICK_LEVEL.store(stick, Ordering::Relaxed);
}

/// A held direction repeats like a held key: once, then after a pause,
/// steadily. (First repeat, then every…) for slow, normal and fast.
fn repeat_times(level: u8) -> (Duration, Duration) {
    let (first, every) = match level {
        0 => (500, 200),
        2 => (280, 70),
        _ => (380, 120),
    };
    (Duration::from_millis(first), Duration::from_millis(every))
}

/// How far the stick goes to start a move, and to end it (with a gap so a
/// stick near the edge doesn't flicker), for low, normal and high sensitivity.
fn stick_edges(level: u8) -> (f32, f32) {
    match level {
        0 => (0.72, 0.5),
        2 => (0.4, 0.25),
        _ => (0.55, 0.35),
    }
}

/// What a face or menu button means. `swap`: for pads with A and B the other
/// way round, the right button confirms and the bottom one goes back.
fn button_press(b: gilrs::Button, swap: bool) -> Option<Press> {
    use gilrs::Button;
    Some(match b {
        Button::South if swap => Press::B,
        Button::East if swap => Press::A,
        Button::South => Press::A,
        Button::East => Press::B,
        Button::West => Press::X,
        Button::North => Press::Y,
        Button::LeftTrigger => Press::Lb,
        Button::RightTrigger => Press::Rb,
        Button::Start => Press::Start,
        Button::Mode => Press::Guide,
        _ => return None,
    })
}

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
                    other => button_press(other, SWAP_AB.load(Ordering::Relaxed)),
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
                    let (on, off) = stick_edges(STICK_LEVEL.load(Ordering::Relaxed));
                    let edge = |held: bool| if held { off } else { on };
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
        let (first_repeat, repeat) = repeat_times(REPEAT_LEVEL.load(Ordering::Relaxed));
        let now = Instant::now();
        for i in 0..4 {
            let held = dpad[i] || stick[i];
            match (held, next[i]) {
                (true, None) => {
                    next[i] = Some(now + first_repeat);
                    if !send(PadEvent::Press(Press::Move(dirs[i]))) {
                        return false;
                    }
                }
                (true, Some(at)) if now >= at => {
                    next[i] = Some(now + repeat);
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

/// Which keys stand for which presses. `typing`: letters go to a text box,
/// so only keys that aren't text count. Back is Esc only, like Steam's Big
/// Picture: Backspace deletes in a text box (see [`backspace`]) and does
/// nothing anywhere else, so deleting can never leave a page.
pub fn key_map(typing: bool) -> Vec<(Key, Press)> {
    let mut map = vec![
        (Key::ArrowUp, Press::Move(Dir::Up)),
        (Key::ArrowDown, Press::Move(Dir::Down)),
        (Key::ArrowLeft, Press::Move(Dir::Left)),
        (Key::ArrowRight, Press::Move(Dir::Right)),
        (Key::Enter, Press::A),
        (Key::Escape, Press::B),
        (Key::PageUp, Press::Lb),
        (Key::PageDown, Press::Rb),
        (Key::F1, Press::Start),
        (Key::Home, Press::Guide),
    ];
    if !typing {
        map.extend([
            (Key::Space, Press::A),
            (Key::Q, Press::Lb),
            (Key::E, Press::Rb),
            (Key::X, Press::X),
            (Key::Y, Press::Y),
            (Key::M, Press::Start),
            // WASD moves too.
            (Key::W, Press::Move(Dir::Up)),
            (Key::S, Press::Move(Dir::Down)),
            (Key::A, Press::Move(Dir::Left)),
            (Key::D, Press::Move(Dir::Right)),
        ]);
    }
    map
}

/// Backspace in a text box: delete the last character (with Ctrl, the last
/// word). An empty box stays empty; it never means "back".
pub fn backspace(field: &mut String, word: bool) {
    if word {
        let kept = field.trim_end().len();
        field.truncate(kept);
        let cut = field.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        field.truncate(cut);
    } else {
        field.pop();
    }
}

/// This frame's keyboard presses (taken, so egui doesn't also act on them).
pub fn keyboard(ctx: &egui::Context, typing: bool) -> Vec<Press> {
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        for (key, press) in key_map(typing) {
            if i.consume_key(Modifiers::NONE, key) {
                out.push(press);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swapping_a_and_b() {
        use gilrs::Button;
        assert_eq!(button_press(Button::South, false), Some(Press::A));
        assert_eq!(button_press(Button::East, false), Some(Press::B));
        assert_eq!(button_press(Button::South, true), Some(Press::B));
        assert_eq!(button_press(Button::East, true), Some(Press::A));
        // Only A and B swap.
        assert_eq!(button_press(Button::West, true), Some(Press::X));
        assert_eq!(button_press(Button::Mode, true), Some(Press::Guide));
        assert_eq!(button_press(Button::DPadUp, false), None);
    }

    #[test]
    fn controller_levels_keep_their_order() {
        let (slow, normal, fast) = (repeat_times(0), repeat_times(1), repeat_times(2));
        assert!(slow.1 > normal.1 && normal.1 > fast.1);
        assert!(slow.0 > normal.0 && normal.0 > fast.0);
        for level in 0..3 {
            let (on, off) = stick_edges(level);
            assert!(on > off && off > 0.0 && on < 1.0, "the stick needs a gap between on and off");
        }
        assert!(stick_edges(0).0 > stick_edges(1).0 && stick_edges(1).0 > stick_edges(2).0);
    }

    #[test]
    fn backspace_never_goes_back() {
        for typing in [false, true] {
            let map = key_map(typing);
            assert!(!map.iter().any(|(k, _)| *k == Key::Backspace), "Backspace mapped (typing: {typing})");
            assert!(map.iter().any(|(k, p)| *k == Key::Escape && *p == Press::B), "Esc must be back (typing: {typing})");
        }
    }

    #[test]
    fn typing_keeps_letters_for_the_text_box() {
        let map = key_map(true);
        for key in [Key::Space, Key::Q, Key::E, Key::X, Key::Y, Key::M, Key::W, Key::A, Key::S, Key::D] {
            assert!(!map.iter().any(|(k, _)| *k == key), "{key:?} must type, not act");
        }
    }

    #[test]
    fn backspace_deletes_and_stops_at_empty() {
        let mut s = String::from("hi");
        backspace(&mut s, false);
        assert_eq!(s, "h");
        backspace(&mut s, false);
        backspace(&mut s, false);
        backspace(&mut s, false);
        assert_eq!(s, "");
        let mut s = String::from("see you tonight  ");
        backspace(&mut s, true);
        assert_eq!(s, "see you ");
        backspace(&mut s, true);
        assert_eq!(s, "see ");
        backspace(&mut s, true);
        assert_eq!(s, "");
        backspace(&mut s, true);
        assert_eq!(s, "");
    }
}

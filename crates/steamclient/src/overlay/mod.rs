//! The in-game overlay, drawn inside the game like Steam's.
//!
//! The app's old overlay was a separate always-on-top window. Exclusive
//! fullscreen games minimize when another window takes focus, so Shift+Tab
//! hid the game. Steam draws its overlay inside the game instead, and so do
//! we, for games SteamInRust starts with this DLL:
//!
//! - `d3d11`: hooks the game's `Present` and draws the overlay on top of
//!   each frame (D3D11 games; others keep the app's window overlay).
//! - `input`: Shift+Tab, and while the overlay is open, the game's mouse and
//!   keyboard go to the overlay, the cursor shows and the game can't lock it.
//! - this file: the pipe to the app. The app runs the overlay's UI (the same
//!   egui code as before) and sends triangles and textures to draw.
//!
//! Wire format: `wire.rs`, shared with the app.

pub mod wire;

#[cfg(windows)]
mod d3d11;
#[cfg(windows)]
mod input;

use wire::{Mesh, TexSet};

/// What the app last asked to draw.
pub struct Frame {
    pub pixels_per_point: f32,
    pub meshes: Vec<Mesh>,
}

pub enum TexOp {
    Set(TexSet),
    Free(u64),
}

/// Start the overlay for a game SteamInRust started (once per process).
pub fn start() {
    #[cfg(windows)]
    {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            if std::env::var_os("STEAMINRUST_NO_INGAME_OVERLAY").is_some() {
                crate::log!("overlay: turned off by STEAMINRUST_NO_INGAME_OVERLAY");
                return;
            }
            let Some((pipe, token)) = crate::ipc::target() else { return };
            let _ = std::thread::Builder::new().name("sir-overlay".into()).spawn(move || {
                if d3d11::install() {
                    shared::run(wire::pipe_name(&pipe), token);
                }
            });
        });
    }
}

/// The first game frame came through our hook.
#[cfg(windows)]
fn hooked(size: (u32, u32)) {
    shared::with(|s| {
        s.hooked = true;
        s.size = size;
    });
    input::set_size(size);
}

#[cfg(windows)]
fn resized(size: (u32, u32)) {
    shared::with(|s| {
        s.size = size;
        if s.connected {
            s.out.push(wire::FromGame::Resize { width: size.0, height: size.1 });
        }
    });
    input::set_size(size);
}

/// GameOverlayActivated_t, so games pause or show their cursor like they do
/// for Steam's overlay.
fn post_activated(open: bool, by_user: bool) {
    const GAME_OVERLAY_ACTIVATED: i32 = 331;
    let mut g = crate::state::lock();
    let mut data = vec![0u8; 12];
    data[0] = open as u8;
    data[1] = by_user as u8;
    data[4..8].copy_from_slice(&g.app_id().to_le_bytes());
    data[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    g.post(GAME_OVERLAY_ACTIVATED, data);
}

#[cfg(windows)]
mod shared {
    use super::wire::{self, FromGame, Input, ToGame};
    use super::{Frame, TexOp};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard};
    use std::time::Duration;

    #[derive(Default)]
    pub struct Shared {
        pub hooked: bool,
        pub connected: bool,
        pub open: bool,
        pub size: (u32, u32),
        pub frame: Option<Arc<Frame>>,
        pub tex_ops: Vec<TexOp>,
        pub out: Vec<FromGame>,
        pub input: Vec<Input>,
        pub modifiers: u8,
        pub cursor: u8,
    }

    static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
    static WAKE: Condvar = Condvar::new();

    fn lock() -> MutexGuard<'static, Option<Shared>> {
        SHARED.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
        let mut g = lock();
        f(g.get_or_insert_with(Default::default))
    }

    /// Queue input for the app and wake the pipe thread.
    pub fn push_input(modifiers: u8, e: Input) {
        with(|s| {
            s.modifiers = modifiers;
            s.input.push(e);
        });
        WAKE.notify_one();
    }

    pub fn cursor() -> u8 {
        with(|s| s.cursor)
    }

    pub fn take_for_present() -> (Vec<TexOp>, Option<Arc<Frame>>) {
        with(|s| (std::mem::take(&mut s.tex_ops), s.frame.clone()))
    }

    /// Open or close. `tell_app` is false when the app asked for it.
    pub fn set_open(open: bool, tell_app: bool, by_user: bool) {
        let changed = with(|s| {
            if s.open == open || (open && !s.connected) {
                return false;
            }
            s.open = open;
            if !open {
                // Back to the game at once, even if the app is slow to answer;
                // it sends pinned windows again in its next frame.
                s.frame = None;
            }
            if tell_app {
                s.out.push(FromGame::Opened(open));
            }
            true
        });
        if changed {
            crate::log!("overlay: {}", if open { "opened" } else { "closed" });
            super::input::capture(open);
            super::post_activated(open, by_user);
            WAKE.notify_one();
        }
    }

    /// Keep a connection to the app while the game runs.
    pub fn run(pipe: String, token: String) {
        loop {
            if !with(|s| s.hooked) {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            match std::fs::OpenOptions::new().read(true).write(true).open(&pipe) {
                Ok(f) => {
                    let r = serve(&f, &token);
                    crate::log!("overlay: app connection ended: {r:?}");
                    set_open(false, false, false);
                    with(|s| {
                        s.connected = false;
                        s.frame = None;
                        s.input.clear();
                        s.out.clear();
                    });
                }
                Err(e) => crate::log!("overlay: can't reach the app at {pipe}: {e}"),
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    fn available(f: &std::fs::File) -> std::io::Result<u32> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Pipes::PeekNamedPipe;
        let mut n = 0u32;
        unsafe { PeekNamedPipe(HANDLE(f.as_raw_handle()), None, 0, None, Some(&mut n), None) }.map_err(std::io::Error::other)?;
        Ok(n)
    }

    /// One connection. A synchronous pipe handle can't read and write at the
    /// same time from two threads, so one thread peeks for incoming data and
    /// writes what's queued.
    fn serve(f: &std::fs::File, token: &str) -> std::io::Result<()> {
        let (width, height) = with(|s| s.size);
        wire::write_message(&mut &*f, &FromGame::Hello { token: token.into(), pid: std::process::id(), width, height }.encode())?;
        with(|s| s.connected = true);
        crate::log!("overlay: connected to the app");
        loop {
            while available(f)? >= 4 {
                let msg = wire::read_message(&mut &*f)?;
                match ToGame::decode(&msg)? {
                    ToGame::Frame { pixels_per_point, cursor, set, free, meshes } => with(|s| {
                        s.tex_ops.extend(set.into_iter().map(TexOp::Set));
                        s.tex_ops.extend(free.into_iter().map(TexOp::Free));
                        s.cursor = cursor;
                        s.frame = Some(Arc::new(Frame { pixels_per_point, meshes }));
                    }),
                    ToGame::SetOpen(open) => set_open(open, false, false),
                }
            }
            let (out, input, modifiers) = with(|s| (std::mem::take(&mut s.out), std::mem::take(&mut s.input), s.modifiers));
            for m in out {
                wire::write_message(&mut &*f, &m.encode())?;
            }
            if !input.is_empty() {
                wire::write_message(&mut &*f, &FromGame::Input { modifiers, events: input }.encode())?;
            }
            // Wake right away for input, else check the pipe every few ms.
            let g = lock();
            let open = g.as_ref().is_some_and(|s| s.open);
            let pending = g.as_ref().is_some_and(|s| !s.input.is_empty() || !s.out.is_empty());
            if !pending {
                let _ = WAKE.wait_timeout(g, Duration::from_millis(if open { 4 } else { 12 }));
            }
        }
    }
}

#[cfg(not(windows))]
mod shared {
    pub fn set_open(_: bool, _: bool, _: bool) {}
}

/// Open or close the overlay from game code (e.g. ActivateGameOverlay).
/// Returns false when no in-game overlay is drawing for this game.
#[allow(dead_code)]
pub fn set_open(open: bool) -> bool {
    #[cfg(windows)]
    {
        if !shared::with(|s| s.connected) {
            return false;
        }
    }
    shared::set_open(open, true, false);
    true
}

//! macOS: the permissions SteamInRust asks for, explained in one window at
//! start-up (until the player has answered once), with a button for each
//! that brings up the system's own question or its Settings page.
//!
//! Today that's Input Monitoring: without it macOS doesn't pass Shift+Tab
//! to SteamInRust while a game is in front, so the overlay can't open.

use egui::{Align, Layout, RichText};
use i18n::t;
use std::sync::Mutex;
use ui::{Theme, icons, widgets as w};

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
}

/// kIOHIDRequestTypeListenEvent: Input Monitoring.
const LISTEN_EVENT: u32 = 1;
/// kIOHIDAccessTypeGranted
const GRANTED: u32 = 0;
/// kIOHIDAccessTypeUnknown: never asked yet.
const UNKNOWN: u32 = 2;

#[derive(Default)]
struct State {
    /// Decided on the first frame after sign-in.
    started: bool,
    open: bool,
    /// Granted while running: the shortcut works after a restart.
    granted_now: bool,
}

static STATE: Mutex<State> = Mutex::new(State { started: false, open: false, granted_now: false });

fn answered_file() -> std::path::PathBuf {
    client::store::config_dir().join("macos-permissions-asked")
}

fn input_monitoring() -> u32 {
    unsafe { IOHIDCheckAccess(LISTEN_EVENT) }
}

/// Open the window (Settings → "macOS permissions").
pub fn open() {
    let mut s = STATE.lock().unwrap();
    s.started = true;
    s.open = true;
}

/// Something isn't allowed yet.
pub fn missing() -> bool {
    input_monitoring() != GRANTED
}

/// Call every frame while signed in.
pub fn show(ctx: &egui::Context, t: &Theme) {
    let mut s = STATE.lock().unwrap();
    if !s.started {
        s.started = true;
        // Only on its own when macOS has never asked: a refusal (or a
        // permission macOS forgot) is the player's call, reachable from
        // Settings → Interface.
        s.open = input_monitoring() == UNKNOWN && !answered_file().exists();
    }
    if !s.open {
        return;
    }
    let mut close = false;
    egui::Modal::new(egui::Id::unique("macos-permissions")).frame(egui::Frame::popup(&ctx.style()).inner_margin(24)).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(w::text_bold(t!("Allow SteamInRust to work with your games"), 19.0, t.text));
        ui.add(egui::Label::new(w::text(t!("macOS asks before an app can do some things. Here is what SteamInRust needs and why. You can change these later in System Settings → Privacy & Security."), 13.0, t.text_dim)).wrap());
        ui.add_space(12.0);

        let status = input_monitoring();
        egui::Frame::new().fill(t.surface).corner_radius(t.corner()).inner_margin(16).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::SHIELD).size(18.0).color(t.accent));
                ui.label(w::text_semibold(t!("Input Monitoring"), 15.0, t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if status == GRANTED {
                        w::pill(ui, &format!("{}  {}", icons::CHECK, t!("Allowed")), t.play, t.play.gamma_multiply(0.15));
                    } else {
                        w::pill(ui, t!("Not allowed"), t.text_dim, t.surface_hover);
                    }
                });
            });
            ui.add(egui::Label::new(w::text(t!("So Shift+Tab opens the overlay (friends, chat, achievements) while a game is in front. SteamInRust only listens for that shortcut."), 13.0, t.text_dim)).wrap());
            if status != GRANTED {
                ui.add_space(4.0);
                if status != UNKNOWN {
                    ui.add(egui::Label::new(w::text(t!("Already allowed it? macOS forgets it when SteamInRust is updated: switch SteamInRust off and on again in that list."), 12.0, t.text_faint)).wrap());
                }
                let label = if status == UNKNOWN { t!("Allow…") } else { t!("Open System Settings") };
                if w::button(ui, t, None, label, w::Kind::Primary, w::Size::Sm).clicked() {
                    // The first time macOS asks itself; after that only Settings can change it.
                    if status == UNKNOWN {
                        unsafe { IOHIDRequestAccess(LISTEN_EVENT) };
                    } else {
                        let _ = open::that_detached("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent");
                    }
                    s.granted_now = true;
                }
            }
            // macOS applies Input Monitoring to an app when it starts.
            if s.granted_now && status == GRANTED {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(w::text(t!("Restart SteamInRust so the shortcut starts working."), 13.0, t.text));
                    if w::button(ui, t, None, t!("Restart now"), w::Kind::Secondary, w::Size::Sm).clicked() {
                        restart();
                    }
                });
            }
        });
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let done = !missing();
            let (label, kind) = if done { (t!("Done"), w::Kind::Primary) } else { (t!("Not now"), w::Kind::Secondary) };
            if w::button(ui, t, None, label, kind, w::Size::Md).clicked() {
                close = true;
            }
        });
    });
    if close {
        s.open = false;
        let _ = std::fs::create_dir_all(client::store::config_dir());
        let _ = std::fs::write(answered_file(), b"1");
    }
}

/// Start a new copy (from the app bundle when there is one) and quit.
fn restart() {
    let exe = std::env::current_exe().unwrap_or_default();
    let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(|p| p.to_path_buf());
    let target = bundle.unwrap_or(exe);
    // After this copy has gone, so the new one doesn't hand itself over to it.
    let script = format!("sleep 1; open \"{}\"", target.display().to_string().replace('"', "\\\""));
    if std::process::Command::new("sh").args(["-c", &script]).spawn().is_ok() {
        std::process::exit(0);
    }
}

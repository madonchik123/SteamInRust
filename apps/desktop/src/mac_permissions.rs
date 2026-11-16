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
        s.open = missing() && !answered_file().exists();
    }
    if !s.open {
        return;
    }
    let mut close = false;
    egui::Modal::new(egui::Id::unique("macos-permissions")).show(ctx, |ui| {
        ui.set_width(540.0);
        ui.label(w::text_bold(t!("Allow SteamInRust to work with your games"), 20.0, t.text));
        ui.add_space(6.0);
        ui.label(w::text(t!("macOS asks before an app can do some things. Here is what SteamInRust needs and why. You can change these later in System Settings → Privacy & Security."), 13.0, t.text_dim));
        ui.add_space(14.0);

        let status = input_monitoring();
        w::card(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::SHIELD).size(22.0).color(t.accent));
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - 150.0);
                    ui.label(w::text_semibold(t!("Input Monitoring"), 14.5, t.text));
                    ui.add(egui::Label::new(w::text(t!("So Shift+Tab opens the overlay (friends, chat, achievements) while a game is in front. SteamInRust only listens for that shortcut."), 12.5, t.text_dim)).wrap());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if status == GRANTED {
                        ui.label(w::text_semibold(format!("{}  {}", icons::CHECK, t!("Allowed")), 13.0, t.play));
                    } else if w::button(ui, t, None, t!("Allow…"), w::Kind::Primary, w::Size::Sm).clicked() {
                        // The first time macOS asks itself; after a "Don't allow" only Settings can change it.
                        if status == UNKNOWN {
                            unsafe { IOHIDRequestAccess(LISTEN_EVENT) };
                        } else {
                            let _ = open::that_detached("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent");
                        }
                        s.granted_now = true;
                    }
                });
            });
        });
        // macOS applies Input Monitoring to an app when it starts.
        if s.granted_now && status == GRANTED {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(w::text(t!("Restart SteamInRust so the shortcut starts working."), 13.0, t.text));
                if w::button(ui, t, None, t!("Restart now"), w::Kind::Secondary, w::Size::Sm).clicked() {
                    restart();
                }
            });
        }
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            let label = if missing() { t!("Not now") } else { t!("Done") };
            if w::button(ui, t, None, label, w::Kind::Secondary, w::Size::Md).clicked() {
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

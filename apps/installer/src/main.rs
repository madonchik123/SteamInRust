//! SteamInRust Setup, laid out like Steam's own installer (a classic Windows
//! setup wizard): Welcome → Language → Install location → Installing (with
//! "Show details") → Finish ("Run SteamInRust"). The same program uninstalls
//! (`--uninstall`) and applies updates (`--update --dir <folder>`) in a small
//! dark window like Steam's "Updating Steam…".

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod wizard;
mod work;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use work::Progress;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Install,
    Update,
    Uninstall,
}

fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() -> eframe::Result {
    i18n::set_language(i18n::Lang::detect());
    let has = |f: &str| std::env::args().any(|a| a == f);
    let mode = if has("--uninstall") {
        Mode::Uninstall
    } else if has("--update") {
        Mode::Update
    } else {
        Mode::Install
    };
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default();
    let dir = match mode {
        Mode::Uninstall => here,
        Mode::Update => arg_value("--dir").map(PathBuf::from).or_else(work::installed_dir).unwrap_or_else(work::default_dir),
        Mode::Install => work::installed_dir().unwrap_or_else(work::default_dir),
    };

    // Quiet uninstall (Windows' "Apps & features" automation): no window.
    if mode == Mode::Uninstall && has("--quiet") {
        let p: work::Shared = Default::default();
        let _ = work::uninstall(&dir, false, &p);
        return Ok(());
    }

    let icon = Arc::new(egui::IconData { rgba: ui::brand::icon_rgba(256), width: 256, height: 256 });
    let viewport = match mode {
        // Steam's updater: a small borderless dark window.
        Mode::Update => egui::ViewportBuilder::default().with_title("Updating SteamInRust").with_inner_size([420.0, 150.0]).with_resizable(false).with_decorations(false).with_icon(icon),
        // The wizard: a normal window with the Windows title bar.
        _ => egui::ViewportBuilder::default()
            .with_title(if mode == Mode::Uninstall { "SteamInRust Uninstall" } else { "SteamInRust Setup" })
            .with_inner_size([500.0, 390.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(icon),
    };
    // Fixed sizes: never reuse a size remembered from an earlier run.
    let options = eframe::NativeOptions { viewport, centered: true, persist_window: false, ..Default::default() };
    let progress = Arc::new(Mutex::new(Progress::default()));
    eframe::run_native(
        "SteamInRust Setup",
        options,
        Box::new(move |cc| {
            wizard::fonts(&cc.egui_ctx);
            let mut w = wizard::Wizard::new(mode, dir, progress);
            if mode == Mode::Update {
                w.start(&cc.egui_ctx);
            }
            Ok(Box::new(w))
        }),
    )
}

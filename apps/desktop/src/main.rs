// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod builtin;
mod chrome;
mod demo;
mod friends_window;
mod image_cache;
mod ingame;
mod launch_popup;
mod links;
mod login;
mod offline_ui;
mod overlay;
mod shell;
mod tray;
mod updater;

fn main() -> eframe::Result {
    // A `steam://` link (or a plain second launch) goes to the running copy.
    let link = std::env::args().skip(1).find(|a| a.starts_with("steam://"));
    // macOS hands links over as events instead of arguments.
    #[cfg(target_os = "macos")]
    links::install_url_handler();
    // `--separate` runs a second copy (for previews and testing).
    let separate = std::env::args().any(|a| a == "--separate");
    if !separate && links::forward_to_running(link.as_deref().unwrap_or("show")) {
        return Ok(());
    }
    // An update downloaded last time installs now, before the app opens.
    if !separate && updater::apply_pending_on_start() {
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,eframe=warn,egui_glow=warn".into()),
        )
        .init();

    let options = eframe::NativeOptions {
        // Our own title bar (see chrome.rs) instead of the Windows one.
        viewport: chrome::frame(egui::ViewportBuilder::default())
            .with_title("SteamInRust")
            // Starts at sign-in size; the app resizes itself after sign-in.
            .with_inner_size([720.0, 470.0])
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: ui::brand::icon_rgba(256), width: 256, height: 256 })),
        ..Default::default()
    };
    eframe::run_native("SteamInRust", options, Box::new(move |cc| Ok(Box::new(app::SteamApp::new(cc, link)))))
}

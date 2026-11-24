// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod bigpicture;
mod builtin;
mod chrome;
mod demo;
mod friends_window;
mod image_cache;
mod sized_images;
mod ingame;
mod launch_popup;
mod launch_question;
mod links;
mod memlog;
mod login;
#[cfg(target_os = "macos")]
mod mac_permissions;
mod offline_ui;
mod overlay;
mod shell;
mod taskbar;
mod texture_cache;
mod tray;
mod updater;
mod window_icon;

fn main() -> eframe::Result {
    // The elevated helper for a game's first-time setup (install scripts):
    // runs the steps it was given and exits, no window.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--install-script-steps") {
        let code = args.get(2).map(|f| client::installscript::helper_main(std::path::Path::new(f))).unwrap_or(2);
        std::process::exit(code);
    }
    // A `steam://` link (or a plain second launch) goes to the running copy.
    let link = std::env::args().skip(1).find(|a| a.starts_with("steam://"));
    // macOS hands links over as events instead of arguments.
    #[cfg(target_os = "macos")]
    links::install_url_handler();
    // `--separate` runs a second copy (for previews and testing).
    let separate = std::env::args().any(|a| a == "--separate");
    // Only one copy runs: a later one (a desktop shortcut clicked twice, a
    // game started while the app is still opening) hands its link over and
    // exits instead of becoming a second app with a second tray icon. The
    // lock is held until the app quits.
    let _instance = if separate {
        None
    } else {
        match links::claim_instance(link.as_deref().unwrap_or("show")) {
            links::Instance::Primary(lock) => lock,
            links::Instance::Exit => return Ok(()),
        }
    };
    // An update downloaded last time installs now, before the app opens.
    if !separate && updater::apply_pending_on_start() {
        return Ok(());
    }

    let filter = || tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,eframe=warn,egui_glow=warn".into());
    // STEAMINRUST_LOG_FILE: the log goes to that file too (an app opened
    // from Finder or the Dock has no console to print to).
    let file = std::env::var_os("STEAMINRUST_LOG_FILE").and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    match file {
        Some(f) => tracing_subscriber::fmt().with_env_filter(filter()).with_ansi(false).with_writer(std::sync::Mutex::new(f)).init(),
        None => tracing_subscriber::fmt().with_env_filter(filter()).init(),
    }
    // A crash leaves its reason in the log too (an app started from Finder
    // has no console for the panic message).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        default_hook(info);
    }));

    // `--offscreen` (previews and screenshots): the window opens far off the
    // screen and stays there, so test copies never show up on the desktop.
    let mut viewport = egui::ViewportBuilder::default();
    if app::offscreen() {
        viewport = viewport.with_position(app::OFFSCREEN_POS);
    }
    let options = eframe::NativeOptions {
        // A test copy's off-screen window must never become the real app's.
        persist_window: !app::offscreen(),
        // Our own title bar (see chrome.rs) instead of the Windows one.
        viewport: chrome::frame(viewport)
            .with_title("SteamInRust")
            // Starts at sign-in size; the app resizes itself after sign-in.
            .with_inner_size([720.0, 470.0])
            // macOS: the GL setup is chosen with the first window; only a
            // see-through-capable one lets the overlay window be see-through
            // (the app paints its own background anyway).
            .with_transparent(cfg!(target_os = "macos"))
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: ui::brand::icon_rgba(256), width: 256, height: 256 })),
        ..Default::default()
    };
    eframe::run_native("SteamInRust", options, Box::new(move |cc| Ok(Box::new(app::SteamApp::new(cc, link)))))
}

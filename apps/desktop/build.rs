//! Keeps `target/` small: Rust's incremental cache grows with every variant
//! that gets built (app, tests, examples) and never shrinks on its own. On
//! each build of the app this deletes cache folders that haven't been used
//! for a few days. Nothing else is touched; Cargo rebuilds anything it needs.

use std::path::Path;
use std::time::{Duration, SystemTime};

const STALE_AFTER: Duration = Duration::from_secs(3 * 24 * 60 * 60);

fn main() {
    println!("cargo:rerun-if-changed=src");
    // OUT_DIR = target/<profile>/build/<pkg>/out -> target/<profile>
    let Some(out) = std::env::var_os("OUT_DIR") else { return };
    let Some(profile_dir) = Path::new(&out).ancestors().nth(3) else { return };
    trim(&profile_dir.join("incremental"));
    icon();
}

/// The app icon and version info in the .exe (Explorer, taskbar, Apps & features).
fn icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/icon.ico")
        .set("ProductName", "SteamInRust")
        .set("FileDescription", "SteamInRust")
        .set("CompanyName", "SteamInRust")
        .set("LegalCopyright", "MIT License. Not affiliated with Valve.");
    if let Err(e) = res.compile() {
        println!("cargo:warning=icon not embedded: {e}");
    }
}

fn trim(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let path = e.path();
        let used = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let old = used.and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > STALE_AFTER);
        if old && path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

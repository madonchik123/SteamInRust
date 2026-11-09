//! Updates from GitHub Releases, like a launcher: check on start and every few
//! hours, download the new setup in the background, verify its SHA-256, then
//! "Restart to update" (or it installs on the next start, like Steam).
//!
//! Uses GitHub's public API with no token: nothing secret ships in the app.
//! While the repository is private the check finds nothing and stays quiet.

use crate::app::SteamApp;
use egui::{Align, Layout, Ui};
use i18n::t;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use ui::{Theme, icons, widgets as w};

const REPO: &str = "madonchik123/SteamInRust";
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    setup_url: String,
    setup_name: String,
    sums_url: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release, f32),
    Ready(Release, PathBuf),
    Failed(String),
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// Download updates by themselves and install them on restart.
    pub auto: bool,
    /// A version the user chose to skip.
    pub skipped: Option<String>,
    /// Downloaded and verified, waiting for the next start.
    pub pending: Option<(String, PathBuf)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { auto: true, skipped: None, pending: None }
    }
}

struct Inner {
    status: Status,
    last_check: Option<Instant>,
    settings: Settings,
    /// The "update ready" window was shown for this version.
    prompted: Option<String>,
    /// The update window is open.
    open: bool,
}

static STATE: LazyLock<Arc<Mutex<Inner>>> = LazyLock::new(|| Arc::new(Mutex::new(Inner { status: Status::Idle, last_check: None, settings: load(), prompted: None, open: false })));

fn settings_path() -> PathBuf {
    client::store::config_dir().join("updater.json")
}

fn load() -> Settings {
    std::fs::read_to_string(settings_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(s: &Settings) {
    let _ = std::fs::create_dir_all(client::store::config_dir());
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(settings_path(), text);
    }
}

/// The folder the running app lives in.
fn install_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}

/// Installed with the setup (it leaves `uninstall.exe` next to the app).
/// Copies run from a build folder or a portable zip aren't replaced.
pub fn is_installed() -> bool {
    install_dir().is_some_and(|d| d.join("uninstall.exe").exists())
}

/// "v1.2.3" -> (1, 2, 3).
fn parse(v: &str) -> (u32, u32, u32) {
    let mut n = v.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map(|p| p.parse::<u32>().unwrap_or(0));
    (n.next().unwrap_or(0), n.next().unwrap_or(0), n.next().unwrap_or(0))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

/// On start: an update downloaded earlier gets installed now, before the app
/// opens (Steam does the same). Returns true when the app should quit.
pub fn apply_pending_on_start() -> bool {
    let mut s = load();
    let Some((version, setup)) = s.pending.take() else { return false };
    save(&s);
    if !is_installed() || !setup.exists() || !is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return false;
    }
    run_setup(&setup)
}

fn run_setup(setup: &std::path::Path) -> bool {
    let Some(dir) = install_dir() else { return false };
    std::process::Command::new(setup).args(["--update", "--dir"]).arg(&dir).spawn().is_ok()
}

/// Called every frame: schedules checks and shows the update window.
/// Built from source (`cargo run`, or anything in a cargo `target` folder):
/// updates come through git, so don't offer installers on our own.
fn is_dev_build() -> bool {
    cfg!(debug_assertions)
        || std::env::current_exe().is_ok_and(|p| p.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target")))
}

pub fn tick(app: &mut SteamApp, ctx: &egui::Context) {
    let state = STATE.clone();
    let mut g = state.lock().unwrap();
    // Dev builds only check when asked (Settings > Check for updates).
    let due = !is_dev_build() && g.last_check.is_none_or(|t| t.elapsed() > EVERY);
    if due && !matches!(g.status, Status::Checking | Status::Downloading(..)) {
        g.last_check = Some(Instant::now());
        g.status = Status::Checking;
        drop(g);
        check(ctx.clone(), false);
        g = state.lock().unwrap();
    }
    // Ready: offer a restart once per version.
    if let Status::Ready(r, _) = &g.status {
        if g.prompted.as_deref() != Some(&r.version) {
            g.prompted = Some(r.version.clone());
            g.open = true;
        }
    }
    let open = g.open;
    drop(g);
    if open {
        window(app, ctx);
    }
}

/// Look for a new release; with `manual` the result is always shown.
pub fn check(ctx: egui::Context, manual: bool) {
    let state = STATE.clone();
    if let Ok(mut g) = state.lock() {
        g.status = Status::Checking;
    }
    std::thread::spawn(move || {
        let result = block_on(latest());
        let mut g = state.lock().unwrap();
        g.status = match result {
            Ok(Some(r)) if is_newer(&r.version, env!("CARGO_PKG_VERSION")) => {
                let skipped = g.settings.skipped.as_deref() == Some(&r.version) && !manual;
                if skipped {
                    Status::UpToDate
                } else if g.settings.auto && is_installed() {
                    if manual {
                        g.open = true;
                    }
                    drop(g);
                    download(ctx.clone(), r);
                    return;
                } else {
                    if manual || g.prompted.as_deref() != Some(&r.version) {
                        g.prompted = Some(r.version.clone());
                        g.open = true;
                    }
                    Status::Available(r)
                }
            }
            Ok(_) => Status::UpToDate,
            Err(e) => {
                tracing::debug!("update check: {e}");
                if manual { Status::Failed(e) } else { Status::Idle }
            }
        };
        // Asked from the menu: always say how it went, like Steam does.
        if manual {
            g.open = true;
        }
        ctx.request_repaint();
    });
}

fn download(ctx: egui::Context, r: Release) {
    let state = STATE.clone();
    if let Ok(mut g) = state.lock() {
        g.status = Status::Downloading(r.clone(), 0.0);
    }
    std::thread::spawn(move || {
        let (s2, c2, r2) = (state.clone(), ctx.clone(), r.clone());
        let progress = move |f: f32| {
            if let Ok(mut g) = s2.lock() {
                g.status = Status::Downloading(r2.clone(), f);
            }
            c2.request_repaint();
        };
        let result = block_on(fetch_setup(&r, progress));
        let mut g = state.lock().unwrap();
        g.status = match result {
            Ok(path) => {
                g.settings.pending = Some((r.version.clone(), path.clone()));
                save(&g.settings);
                Status::Ready(r, path)
            }
            Err(e) => Status::Failed(e),
        };
        ctx.request_repaint();
    });
}

fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime").block_on(f)
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().user_agent(concat!("SteamInRust/", env!("CARGO_PKG_VERSION"), " (updater)")).timeout(Duration::from_secs(60)).build().unwrap_or_default()
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

async fn latest() -> Result<Option<Release>, String> {
    let resp = http().get(format!("https://api.github.com/repos/{REPO}/releases/latest")).header("Accept", "application/vnd.github+json").send().await.map_err(|e| e.to_string())?;
    // Private repository or no release yet.
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let rel: ApiRelease = resp.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    if rel.draft || rel.prerelease {
        return Ok(None);
    }
    let Some(setup) = rel.assets.iter().find(|a| a.name.starts_with("SteamInRust-Setup") && a.name.ends_with(".exe")) else { return Ok(None) };
    Ok(Some(Release {
        version: rel.tag_name.trim_start_matches('v').to_string(),
        notes: rel.body.unwrap_or_default(),
        page: rel.html_url,
        setup_url: setup.browser_download_url.clone(),
        setup_name: setup.name.clone(),
        sums_url: rel.assets.iter().find(|a| a.name == "SHA256SUMS.txt").map(|a| a.browser_download_url.clone()),
    }))
}

/// Download the setup into the cache and check it against SHA256SUMS.txt.
async fn fetch_setup(r: &Release, progress: impl Fn(f32)) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    let client = http();
    let sums_url = r.sums_url.as_ref().ok_or_else(|| t!("The release has no checksums; not installing it.").to_string())?;
    let sums = client.get(sums_url).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    let want = sums
        .lines()
        .find_map(|l| {
            let (hash, name) = l.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == r.setup_name).then(|| hash.to_lowercase())
        })
        .ok_or_else(|| t!("The release has no checksums; not installing it.").to_string())?;
    let mut resp = client.get(&r.setup_url).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    let total = resp.content_length().unwrap_or(0).max(1);
    let mut data = Vec::with_capacity(total as usize);
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        data.extend_from_slice(&chunk);
        progress(data.len() as f32 / total as f32);
    }
    let got: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        return Err(t!("The download was damaged (checksum mismatch). Try again later.").to_string());
    }
    let dir = std::env::temp_dir().join("SteamInRust-update");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(&r.setup_name);
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Close the app and let the setup replace it, then it starts again.
fn restart_to_update(app: &mut SteamApp, ctx: &egui::Context, setup: &std::path::Path) {
    if let Ok(mut g) = STATE.lock() {
        g.settings.pending = None;
        save(&g.settings);
    }
    if run_setup(setup) {
        app.exiting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// The update window: what's new, and install now or later.
fn window(app: &mut SteamApp, ctx: &egui::Context) {
    let t = app.prefs.theme.clone();
    let status = STATE.lock().unwrap().status.clone();
    let (release, ready) = match &status {
        Status::Available(r) | Status::Downloading(r, _) => (r.clone(), None),
        Status::Ready(r, p) => (r.clone(), Some(p.clone())),
        Status::Checking | Status::UpToDate | Status::Failed(_) => {
            // The answer to "Check for updates…".
            let mut close = false;
            egui::Modal::new(egui::Id::unique("update-check")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.horizontal(|ui| {
                    w::logo(ui, &t, 28.0);
                    ui.label(w::text_bold(t!("SteamInRust update"), 16.0, t.text));
                });
                ui.add_space(10.0);
                let text = match &status {
                    Status::Checking => t!("Checking for updates…").to_string(),
                    Status::UpToDate => t!("Your SteamInRust client is already up to date ({v}).", v = env!("CARGO_PKG_VERSION")),
                    Status::Failed(e) => t!("Couldn't check for updates: {error}", error = e),
                    _ => String::new(),
                };
                ui.add(egui::Label::new(w::text(text, 13.5, t.text_dim)).wrap());
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::button(ui, &t, None, t!("OK"), w::Kind::Primary, w::Size::Sm).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                STATE.lock().unwrap().open = false;
            }
            return;
        }
        _ => {
            STATE.lock().unwrap().open = false;
            return;
        }
    };
    let mut close = false;
    egui::Modal::new(egui::Id::unique("update-window")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            w::logo(ui, &t, 34.0);
            ui.vertical(|ui| {
                ui.label(w::text_bold(t!("SteamInRust {v} is available", v = release.version), 17.0, t.text));
                ui.label(w::text(t!("You have {v}", v = env!("CARGO_PKG_VERSION")), 12.5, t.text_dim));
            });
        });
        ui.add_space(10.0);
        ui.label(w::text_bold(t!("WHAT'S NEW"), 10.5, t.text_faint));
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| notes(ui, &t, &release.notes));
        ui.add_space(12.0);
        if let Status::Downloading(_, f) = &status {
            ui.label(w::text(t!("Downloading… {pct}%", pct = (f * 100.0) as u32), 12.5, t.text_dim));
            w::progress(ui, &t, *f, ui.available_width());
            ui.add_space(8.0);
        }
        // Small links on their own row, so long labels (Russian) never overlap.
        ui.horizontal_wrapped(|ui| {
            if ui.link(w::text(t!("Release page"), 12.5, t.accent)).clicked() {
                let _ = open::that_detached(&release.page);
            }
            if let (None, Status::Available(r)) = (&ready, &status) {
                ui.label(w::text("·", 12.5, t.text_faint));
                if ui.link(w::text(t!("Skip this version"), 12.5, t.text_dim)).clicked() {
                    if let Ok(mut g) = STATE.lock() {
                        g.settings.skipped = Some(r.version.clone());
                        save(&g.settings);
                    }
                    close = true;
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match (&ready, &status) {
                    (Some(setup), _) => {
                        if w::button(ui, &t, Some(icons::REFRESH), t!("Restart and update"), w::Kind::Primary, w::Size::Md).clicked() {
                            restart_to_update(app, ctx, setup);
                            close = true;
                        }
                        if w::button(ui, &t, None, t!("On next start"), w::Kind::Ghost, w::Size::Md).on_hover_text(t!("It installs the next time you open SteamInRust")).clicked() {
                            close = true;
                        }
                    }
                    (None, Status::Available(r)) => {
                        if is_installed() {
                            if w::button(ui, &t, Some(icons::DOWNLOAD), t!("Download and install"), w::Kind::Primary, w::Size::Md).clicked() {
                                download(ctx.clone(), r.clone());
                            }
                        } else if w::button(ui, &t, Some(icons::DOWNLOAD), t!("Get the installer"), w::Kind::Primary, w::Size::Md).on_hover_text(t!("This copy wasn't installed with the setup, so it can't replace itself")).clicked() {
                            let _ = open::that_detached(&r.page);
                        }
                        if w::button(ui, &t, None, t!("Later"), w::Kind::Ghost, w::Size::Md).clicked() {
                            close = true;
                        }
                    }
                    _ => {
                        if w::button(ui, &t, None, t!("Hide"), w::Kind::Ghost, w::Size::Md).clicked() {
                            close = true;
                        }
                    }
                }
            });
        });
    });
    if close {
        STATE.lock().unwrap().open = false;
    }
}

/// Release notes are Markdown; show headings, bullets and text.
fn notes(ui: &mut Ui, t: &Theme, text: &str) {
    if text.trim().is_empty() {
        ui.label(w::text(t!("Bug fixes and improvements."), 13.0, t.text_dim));
        return;
    }
    for line in text.lines() {
        let l = line.trim();
        if let Some(h) = l.strip_prefix('#') {
            let h = h.trim_start_matches('#').trim();
            // The version heading ("[0.4.0](compare link) (date)"): the title already says it.
            if h.starts_with('[') {
                continue;
            }
            let h = plain(h);
            let h = match h.as_str() {
                "New" | "Features" => t!("New").to_string(),
                "Fixes" | "Bug Fixes" => t!("Fixes").to_string(),
                "Faster" | "Performance Improvements" => t!("Faster").to_string(),
                _ => h,
            };
            ui.add_space(6.0);
            ui.label(w::text_semibold(h, 14.0, t.text));
        } else if let Some(b) = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(w::text("•", 13.0, t.accent));
                ui.add(egui::Label::new(w::text(plain(b), 13.0, t.text_dim)).wrap());
            });
        } else if !l.is_empty() {
            ui.add(egui::Label::new(w::text(plain(l), 13.0, t.text_dim)).wrap());
        }
    }
}

/// Release-note Markdown as plain text: "[#17](url)" -> "#17", commit links
/// ("([7562a6d](url))") dropped, bold and code marks removed.
fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|i| open + i) else { break };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else { break };
        out.push_str(&rest[..open]);
        let text = &rest[open + 1..close];
        // A bare commit hash: leave it out.
        let commit = text.len() >= 7 && text.chars().all(|c| c.is_ascii_hexdigit());
        if !commit {
            out.push_str(text);
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    let out = out.replace("**", "").replace('`', "").replace(" ()", "").replace("()", "");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The Updates card in Settings > About.
pub fn settings_ui(ui: &mut Ui, t: &Theme) {
    let state = STATE.clone();
    let (status, mut auto) = {
        let g = state.lock().unwrap();
        (g.status.clone(), g.settings.auto)
    };
    w::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(w::text_semibold(t!("Updates"), 15.0, t.text));
        ui.add_space(4.0);
        let line = match &status {
            Status::Idle => t!("Version {v}", v = env!("CARGO_PKG_VERSION")),
            Status::Checking => t!("Checking for updates…").to_string(),
            Status::UpToDate => t!("You're up to date ({v})", v = env!("CARGO_PKG_VERSION")),
            Status::Available(r) => t!("Version {v} is available", v = r.version),
            Status::Downloading(r, f) => t!("Downloading {v}… {pct}%", v = r.version, pct = (f * 100.0) as u32),
            Status::Ready(r, _) => t!("Version {v} is ready to install", v = r.version),
            Status::Failed(e) => t!("Couldn't check for updates: {error}", error = e),
        };
        ui.label(w::text(line, 13.0, t.text_dim));
        if !is_installed() {
            ui.label(w::text(t!("This copy wasn't installed with the setup, so updates are only announced, not installed."), 12.0, t.text_faint));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let busy = matches!(status, Status::Checking | Status::Downloading(..));
            if ui.add_enabled_ui(!busy, |ui| w::button(ui, t, Some(icons::REFRESH), t!("Check for updates"), w::Kind::Secondary, w::Size::Sm)).inner.clicked() {
                check(ui.ctx().clone(), true);
            }
            if matches!(status, Status::Available(_) | Status::Ready(..)) && w::button(ui, t, None, t!("Show"), w::Kind::Ghost, w::Size::Sm).clicked() {
                state.lock().unwrap().open = true;
            }
        });
        ui.add_space(4.0);
        if w::check(ui, t, &mut auto, t!("Download updates automatically and install them on restart")).changed() {
            let mut g = state.lock().unwrap();
            g.settings.auto = auto;
            save(&g.settings);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::plain;

    #[test]
    fn release_notes_become_plain_text() {
        let line = "compact Workshop filters ([#17](https://github.com/x/y/issues/17)) ([7562a6d](https://github.com/x/y/commit/7562a6d30e))";
        assert_eq!(plain(line), "compact Workshop filters (#17)");
        assert_eq!(plain("**bold** and `code`"), "bold and code");
        assert_eq!(plain("no links here"), "no links here");
    }

    #[test]
    fn compares_versions() {
        assert!(super::is_newer("v0.2.0", "0.1.9"));
        assert!(super::is_newer("1.0.0", "0.9.12"));
        assert!(!super::is_newer("v0.1.0", "0.1.0"));
        assert!(!super::is_newer("0.1.0-beta", "0.1.0"));
        assert!(super::is_newer("0.1.10", "0.1.9"));
    }
}

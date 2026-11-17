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
    /// The check failed.
    Failed(String),
    /// The check worked but the download didn't (it continues where it
    /// stopped next time).
    DownloadFailed(Release, String),
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
#[cfg(not(target_os = "macos"))]
fn install_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}

/// Installed with the setup (it leaves `uninstall.exe` next to the app).
/// Copies run from a build folder or a portable zip aren't replaced.
#[cfg(not(target_os = "macos"))]
pub fn is_installed() -> bool {
    install_dir().is_some_and(|d| d.join("uninstall.exe").exists())
}

/// macOS: running from an app bundle in a folder we may write to (so the
/// new version can take its place), not from a build folder.
#[cfg(target_os = "macos")]
pub fn is_installed() -> bool {
    let Some(app) = bundle() else { return false };
    let Some(dir) = app.parent() else { return false };
    let probe = dir.join(".steaminrust-update-check");
    let writable = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    writable && !is_dev_build()
}

/// The `SteamInRust.app` this copy runs from (macOS).
#[cfg(target_os = "macos")]
fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(PathBuf::from)
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
    let Some((version, setup)) = s.pending.take() else {
        // Installers for this version or older aren't needed any more; a
        // newer one (finished or half-downloaded) is kept to use or continue.
        clean_old_updates();
        return false;
    };
    save(&s);
    if !is_installed() || !setup.exists() || !is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return false;
    }
    run_setup(&setup)
}

/// macOS: after this copy quits, put the app from the downloaded disk image
/// in place of the running one and open it again. Arguments: our process
/// id, the .dmg, the installed .app. A failed copy leaves the old app.
#[cfg(target_os = "macos")]
const MAC_UPDATE: &str = r#"
while kill -0 "$1" 2>/dev/null; do sleep 0.3; done
mnt=$(mktemp -d /tmp/steaminrust-update.XXXXXX) || exit 1
hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$mnt" "$2" >/dev/null || exit 1
rm -rf "$3.new"
if ditto "$mnt/SteamInRust.app" "$3.new"; then
  rm -rf "$3.old"
  if mv "$3" "$3.old"; then
    if mv "$3.new" "$3"; then rm -rf "$3.old"; else mv "$3.old" "$3"; fi
  fi
fi
rm -rf "$3.new"
hdiutil detach "$mnt" -quiet
rmdir "$mnt"
[ -n "$4" ] || open "$3"
"#;

#[cfg(target_os = "macos")]
fn run_setup(dmg: &std::path::Path) -> bool {
    let Some(app) = bundle() else { return false };
    std::process::Command::new("/bin/sh").arg("-c").arg(MAC_UPDATE).arg("sh").arg(std::process::id().to_string()).arg(dmg).arg(&app).spawn().is_ok()
}

#[cfg(not(target_os = "macos"))]
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
    if due {
        // `check` itself skips it while a check or download is under way.
        g.last_check = Some(Instant::now());
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
        // Already checking, downloading or downloaded: never start a second
        // one (two downloads made the progress bar jump between them). Asked
        // from a menu, just show where it is.
        if matches!(g.status, Status::Checking | Status::Downloading(..) | Status::Ready(..)) {
            if manual {
                g.open = true;
                ctx.request_repaint();
            }
            return;
        }
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
        if matches!(g.status, Status::Downloading(..)) {
            return;
        }
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
            Err(e) => {
                tracing::warn!("update download: {e}");
                Status::DownloadFailed(r, e)
            }
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

/// For the installer itself: no limit on the whole download (it used to cut
/// off after 60 s, which slow connections hit as "error decoding response
/// body"), only on waiting for the next bytes.
fn http_download() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("SteamInRust/", env!("CARGO_PKG_VERSION"), " (updater)"))
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// Where downloaded updates are kept until they're installed.
fn update_dir() -> PathBuf {
    std::env::temp_dir().join("SteamInRust-update")
}

/// Delete everything in the update folder except files for `keep` (the
/// installer's name, finished or half-downloaded), so old installers never
/// pile up.
fn clean_update_dir(keep: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(update_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if keep.is_some_and(|k| name == k || name == format!("{k}.part")) {
            continue;
        }
        let path = e.path();
        let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
    }
}

/// Delete installers in the update folder for this version or older.
fn clean_old_updates() {
    let Ok(entries) = std::fs::read_dir(update_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if version_in(&name).is_none_or(|v| !is_newer(&v, env!("CARGO_PKG_VERSION"))) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The version in an installer's name: "SteamInRust-Setup-0.7.1.exe.part" -> "0.7.1".
fn version_in(name: &str) -> Option<String> {
    name.split(['-', '_'])
        .map(|part| part.trim_end_matches(".part").trim_end_matches(".exe").trim_end_matches(".dmg").trim_end_matches(".zip"))
        .find(|part| part.split('.').count() == 3 && part.split('.').all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())))
        .map(String::from)
}

fn sha256_file(path: &std::path::Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
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
    // Windows: the setup; macOS: the disk image (opened from the release page).
    let wanted = |n: &str| if cfg!(target_os = "macos") { n.starts_with("SteamInRust-") && n.ends_with("-macos.dmg") } else { n.starts_with("SteamInRust-Setup") && n.ends_with(".exe") };
    let sums = if cfg!(target_os = "macos") { "SHA256SUMS-macos.txt" } else { "SHA256SUMS.txt" };
    let Some(setup) = rel.assets.iter().find(|a| wanted(&a.name)) else { return Ok(None) };
    Ok(Some(Release {
        version: rel.tag_name.trim_start_matches('v').to_string(),
        notes: rel.body.unwrap_or_default(),
        page: rel.html_url,
        setup_url: setup.browser_download_url.clone(),
        setup_name: setup.name.clone(),
        sums_url: rel.assets.iter().find(|a| a.name == sums).map(|a| a.browser_download_url.clone()),
    }))
}

/// Download the setup and check it against SHA256SUMS.txt. It's written to
/// `<name>.part` as it arrives, so closing the app (or a dropped connection)
/// doesn't lose it: the next try continues from there. A finished, checked
/// file isn't downloaded again. Older installers are deleted.
async fn fetch_setup(r: &Release, progress: impl Fn(f32)) -> Result<PathBuf, String> {
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
    let dir = update_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    clean_update_dir(Some(&r.setup_name));
    let path = dir.join(&r.setup_name);
    if path.exists() {
        if sha256_file(&path).as_deref() == Some(want.as_str()) {
            tracing::info!("update download: already downloaded and checked");
            progress(1.0);
            return Ok(path);
        }
        let _ = std::fs::remove_file(&path);
    }
    let part = dir.join(format!("{}.part", r.setup_name));
    let download = http_download();
    // A dropped connection resumes from what's on disk, a few times; after
    // that the error is shown and the .part file stays for the next try.
    let mut tries = 0;
    loop {
        match fetch_part(&download, &r.setup_url, &part, &progress).await {
            Ok(()) => break,
            Err(e) => {
                tries += 1;
                tracing::warn!("update download interrupted ({tries}): {e}");
                if tries >= 4 {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    if sha256_file(&part).as_deref() != Some(want.as_str()) {
        let _ = std::fs::remove_file(&part);
        return Err(t!("The download was damaged (checksum mismatch). Try again later.").to_string());
    }
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Download `url` into `part`, continuing after what's already there.
async fn fetch_part(client: &reqwest::Client, url: &str, part: &std::path::Path, progress: &impl Fn(f32)) -> Result<(), String> {
    use std::io::Write;
    let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let mut req = client.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = req.send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    // 206: the rest of the file; 200: the server sent all of it again.
    let resumed = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let start = if resumed { have } else { 0 };
    tracing::info!("update download: {} at {} KB", if resumed { "continuing" } else { "starting" }, start / 1024);
    let total = start + resp.content_length().unwrap_or(0);
    let mut file = std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(part).map_err(|e| e.to_string())?;
    let mut done = start;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        progress(done as f32 / total.max(1) as f32);
    }
    file.flush().map_err(|e| e.to_string())?;
    if total > start && done < total {
        return Err("the download ended early".into());
    }
    Ok(())
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
        Status::Available(r) | Status::Downloading(r, _) | Status::DownloadFailed(r, _) => (r.clone(), None),
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
        if let Status::DownloadFailed(_, e) = &status {
            ui.add(egui::Label::new(w::text(t!("Couldn't download the update: {error}", error = e), 12.5, t.danger)).wrap());
            ui.label(w::text(t!("What's downloaded so far is kept; Retry continues from there."), 12.0, t.text_faint));
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
                    (None, Status::DownloadFailed(r, _)) => {
                        if w::button(ui, &t, Some(icons::REFRESH), t!("Retry"), w::Kind::Primary, w::Size::Md).clicked() {
                            if let Ok(mut g) = STATE.lock() {
                                g.status = Status::Available(r.clone());
                            }
                            download(ctx.clone(), r.clone());
                        }
                        if w::button(ui, &t, None, t!("Later"), w::Kind::Ghost, w::Size::Md).clicked() {
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
            Status::DownloadFailed(_, e) => t!("Couldn't download the update: {error}", error = e),
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
            if matches!(status, Status::Available(_) | Status::Ready(..) | Status::DownloadFailed(..)) && w::button(ui, t, None, t!("Show"), w::Kind::Ghost, w::Size::Sm).clicked() {
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
    use super::{plain, version_in};

    #[test]
    fn release_notes_become_plain_text() {
        let line = "compact Workshop filters ([#17](https://github.com/x/y/issues/17)) ([7562a6d](https://github.com/x/y/commit/7562a6d30e))";
        assert_eq!(plain(line), "compact Workshop filters (#17)");
        assert_eq!(plain("**bold** and `code`"), "bold and code");
        assert_eq!(plain("no links here"), "no links here");
    }

    #[test]
    fn finds_versions_in_installer_names() {
        assert_eq!(version_in("SteamInRust-Setup-0.7.1.exe").as_deref(), Some("0.7.1"));
        assert_eq!(version_in("SteamInRust-Setup-0.7.1.exe.part").as_deref(), Some("0.7.1"));
        assert_eq!(version_in("SteamInRust-0.10.2-macos.dmg").as_deref(), Some("0.10.2"));
        assert_eq!(version_in("notes.txt"), None);
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

#[cfg(all(test, target_os = "macos"))]
mod mac_tests {
    use std::process::Command;

    /// The update script puts the disk image's app in place of the old one.
    #[test]
    fn replaces_the_app_from_a_disk_image() {
        let dir = std::env::temp_dir().join(format!("sir-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stage = dir.join("stage");
        let new_app = stage.join("SteamInRust.app").join("Contents");
        std::fs::create_dir_all(&new_app).unwrap();
        std::fs::write(new_app.join("version"), b"new").unwrap();
        let installed = dir.join("Applications").join("SteamInRust.app");
        std::fs::create_dir_all(installed.join("Contents")).unwrap();
        std::fs::write(installed.join("Contents").join("version"), b"old").unwrap();
        let dmg = dir.join("update.dmg");
        let made = Command::new("hdiutil").args(["create", "-quiet", "-fs", "HFS+", "-format", "UDZO", "-srcfolder"]).arg(&stage).arg(&dmg).status().unwrap();
        assert!(made.success());
        // A process id that is gone (so it doesn't wait), and "no reopen".
        let ok = Command::new("/bin/sh").arg("-c").arg(super::MAC_UPDATE).arg("sh").arg("999999").arg(&dmg).arg(&installed).arg("test").status().unwrap();
        assert!(ok.success());
        assert_eq!(std::fs::read(installed.join("Contents").join("version")).unwrap(), b"new");
        assert!(!dir.join("Applications").join("SteamInRust.app.old").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

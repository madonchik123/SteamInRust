//! Plain data types shared by every crate. No I/O lives here, so extensions
//! can depend on it without pulling in networking.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type AppId = u32;
pub type SteamId = u64;

/// A game installed in one of the local Steam library folders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledGame {
    pub app_id: AppId,
    pub name: String,
    pub install_dir: PathBuf,
    pub library: PathBuf,
    pub size_on_disk: u64,
    /// Unix seconds.
    pub last_updated: Option<u64>,
    /// Bytes downloaded by the last update (from Steam's appmanifest).
    #[serde(default)]
    pub last_update_bytes: Option<u64>,
    /// Unix seconds, from the account's localconfig.
    pub last_played: Option<u64>,
    /// Total minutes played, from the account's localconfig.
    pub playtime_minutes: Option<u64>,
    pub state: InstallState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallState {
    Installed,
    UpdateRequired,
    Downloading,
    Unknown,
}

impl InstallState {
    /// Decode the `StateFlags` bitfield from an app manifest.
    pub fn from_flags(flags: u64) -> Self {
        const FULLY_INSTALLED: u64 = 4;
        const UPDATE_REQUIRED: u64 = 2;
        const DOWNLOADING: u64 = 1 << 20;
        if flags & DOWNLOADING != 0 {
            Self::Downloading
        } else if flags & UPDATE_REQUIRED != 0 {
            Self::UpdateRequired
        } else if flags & FULLY_INSTALLED != 0 {
            Self::Installed
        } else {
            Self::Unknown
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Installed => "Installed",
            Self::UpdateRequired => "Update required",
            Self::Downloading => "Downloading",
            Self::Unknown => "Unknown state",
        }
    }
}

/// A Steam account that has logged in on this machine (from loginusers.vdf).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAccount {
    pub steam_id: SteamId,
    pub account_name: String,
    pub persona_name: String,
    pub most_recent: bool,
    pub last_login: Option<u64>,
}

/// 32-bit account id used for `userdata/<id>` folders.
pub fn account_id(steam_id: SteamId) -> u32 {
    (steam_id & 0xFFFF_FFFF) as u32
}

/// An app the logged-in account owns, resolved through PICS.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OwnedApp {
    pub app_id: AppId,
    pub name: String,
    pub kind: String,
    pub launch: Vec<LaunchOption>,
    /// Icon hash for [`art::icon`].
    #[serde(default)]
    pub icon: Option<String>,
    /// For DLC: the game it belongs to.
    #[serde(default)]
    pub parent: Option<AppId>,
    /// Folder name under `steamapps/common`.
    #[serde(default)]
    pub install_dir: String,
    #[serde(default)]
    pub depots: Vec<DepotInfo>,
    #[serde(default)]
    pub branches: Vec<Branch>,
    /// Steam Cloud settings, when the game uses it.
    #[serde(default)]
    pub cloud: Option<CloudConfig>,
    /// Has store art (a store page). Steam leaves DLC without it, like
    /// Dota 2's "Vulkan support", off the game's library page.
    #[serde(default)]
    pub store_assets: bool,
    /// Art paths from app info in Steam's newer layout, e.g.
    /// "<hash>/header.jpg" (see [`art::remember`]).
    #[serde(default)]
    pub art: Vec<String>,
    /// Steam's feature categories (1 multi-player, 2 single-player, 9 co-op,
    /// 22 achievements, 23 cloud, 28 full controller support, 30 workshop…).
    #[serde(default)]
    pub categories: Vec<u32>,
    /// Steam genre ids (1 Action, 2 Strategy, 3 RPG, 23 Indie…).
    #[serde(default)]
    pub genres: Vec<u32>,
    /// Steam's store tags (user tags: 19 Action, 492 Indie, 1664 Puzzle…).
    #[serde(default)]
    pub tags: Vec<u32>,
    /// Steam Deck: 0 unknown, 1 unsupported, 2 playable, 3 verified.
    #[serde(default)]
    pub deck: u8,
    /// Steam Machine compatibility, on the same scale.
    #[serde(default)]
    pub machine: u8,
    /// Steam Frame compatibility, on the same scale.
    #[serde(default)]
    pub frame: u8,
    /// Supported languages, one bit per Steam language id (0 = not known).
    #[serde(default)]
    pub languages: u64,
    /// The subscription app this game comes with, like EA Play (0 = none).
    #[serde(default)]
    pub master_sub: u32,
    /// Systems the game runs on (`common/oslist`, e.g. "windows,macos");
    /// empty when app info doesn't say.
    #[serde(default)]
    pub os_list: String,
}

/// A game's Steam Cloud setup, from its app info (`ufs`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudConfig {
    /// Bytes allowed in the cloud.
    pub quota: u64,
    pub max_files: u32,
    /// Auto-Cloud: folders the game saves into by itself.
    pub save_files: Vec<SaveRule>,
}

/// One Auto-Cloud rule: files matching `pattern` under `root`/`path`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SaveRule {
    /// Steam's root name, e.g. "WinMyDocuments" or "gameinstall".
    pub root: String,
    pub path: String,
    pub pattern: String,
    pub recursive: bool,
}

/// What a Steam Cloud sync did, or found.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudReport {
    pub app_id: AppId,
    pub downloaded: u32,
    pub uploaded: u32,
    pub up_to_date: u32,
    /// Files changed both here and in the cloud since the last sync.
    pub conflicts: Vec<CloudConflict>,
    /// Files that couldn't be synced, with the reason.
    pub errors: Vec<(String, String)>,
    pub cloud_files: u32,
    pub cloud_bytes: u64,
    pub quota_bytes: u64,
    /// Unix seconds.
    pub finished: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudConflict {
    /// The file's cloud name, e.g. "%WinMyDocuments%My Games/X/save1.sav".
    pub name: String,
    pub local_time: u64,
    pub cloud_time: u64,
    pub local_size: u64,
    pub cloud_size: u64,
}

/// One depot (content package) of an app, from PICS.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DepotInfo {
    pub id: u32,
    /// "windows", "macos", "linux" or empty for all.
    pub os: String,
    /// "64", "32" or empty.
    pub arch: String,
    /// Empty = all languages.
    pub language: String,
    pub low_violence: bool,
    pub dlc_app: Option<AppId>,
    /// Shared depot owned by another app (e.g. redistributables).
    pub shared_install: bool,
    /// The app a shared depot comes from (228980 for Steam's redistributables).
    #[serde(default)]
    pub from_app: Option<AppId>,
    /// (branch, manifest gid, size in bytes)
    pub manifests: Vec<(String, u64, u64)>,
}

impl DepotInfo {
    pub fn manifest(&self, branch: &str) -> Option<(u64, u64)> {
        self.manifests.iter().find(|m| m.0 == branch).or_else(|| self.manifests.iter().find(|m| m.0 == "public")).map(|m| (m.1, m.2))
    }
}

/// A version branch (Steam's "Betas").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Branch {
    pub name: String,
    pub description: String,
    pub build_id: u64,
    /// Unix seconds.
    pub updated: u64,
    pub password: bool,
}

/// A Steam Workshop item, as listed in the Workshop browser.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkshopEntry {
    pub id: u64,
    pub title: String,
    pub preview: Option<String>,
    pub description: String,
    pub size: u64,
    pub subscriptions: u64,
    pub favorited: u64,
    /// Unix seconds.
    pub updated: u64,
    pub votes_up: u32,
    pub votes_down: u32,
    pub tags: Vec<String>,
}

/// A game's own Workshop tag group, e.g. "Addon Type" or "Heroes".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkshopTagCategory {
    pub name: String,
    /// Steam shows it as a dropdown: pick one.
    pub single: bool,
    /// (tag sent to Steam, name to show)
    pub tags: Vec<(String, String)>,
}

/// One line in the shopping cart.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CartLine {
    pub line_item_id: u64,
    pub package_id: u32,
    /// Formatted price as Steam shows it.
    pub price: String,
    /// Friend's account id when bought as a gift.
    pub gift_to: Option<u32>,
    /// A gift (Steam's "This is a gift"), with or without a recipient yet.
    #[serde(default)]
    pub is_gift: bool,
    /// A whole bundle rather than a package (`package_id` is then 0).
    #[serde(default)]
    pub bundle_id: u32,
}

/// Live state of one download, as shown on the Downloads page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DownloadStatus {
    pub app_id: AppId,
    pub phase: DownloadPhase,
    pub downloaded: u64,
    pub download_total: u64,
    pub disk_done: u64,
    pub disk_total: u64,
    /// Bytes per second over the last few seconds.
    pub speed: u64,
    pub paused: bool,
    pub error: Option<String>,
    /// Bytes saved by reusing unchanged chunks.
    pub reused: u64,
    /// Not errors, but worth showing (e.g. skipped content).
    #[serde(default)]
    pub notes: Vec<String>,
    /// Where the game is going (exists as soon as files start arriving).
    #[serde(default)]
    pub install_dir: Option<PathBuf>,
    /// Bytes per second landing on disk (downloaded, reused or copied).
    #[serde(default)]
    pub disk_speed: u64,
}

impl DownloadStatus {
    /// (done, total) for the progress bar, like Steam: the bytes this
    /// download fetches (an update shows only the update, e.g. 0 / 6 GB).
    /// While existing files are checked or verified, the check's progress.
    pub fn shown(&self) -> (u64, u64) {
        if self.phase == DownloadPhase::Verifying || self.download_total == 0 {
            (self.disk_done.min(self.disk_total), self.disk_total)
        } else {
            (self.downloaded.min(self.download_total), self.download_total)
        }
    }

    pub fn fraction(&self) -> f32 {
        let (done, total) = self.shown();
        (done as f32 / total.max(1) as f32).clamp(0.0, 1.0)
    }

    pub fn verifying(&self) -> bool {
        self.phase == DownloadPhase::Verifying
    }

    /// What it's doing right now, in words (an English key for translation):
    /// downloading, writing to disk, checking files…, like Steam's status.
    pub fn activity(&self) -> &'static str {
        match self.phase {
            DownloadPhase::Queued | DownloadPhase::Preparing => "Preparing…",
            DownloadPhase::Checking => "Checking existing files…",
            DownloadPhase::Verifying => "Verifying files…",
            DownloadPhase::Finishing => "Finishing…",
            DownloadPhase::Done => "Done",
            DownloadPhase::Failed => "Failed",
            _ if self.paused => "Paused",
            // More than ~50 KB/s over the network: downloading. Otherwise
            // data still landing on disk (reused chunks, buffered writes).
            _ if self.speed > 50_000 => "Downloading",
            _ if self.disk_speed > 0 => "Writing to disk",
            _ => "Waiting for content servers…",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DownloadPhase {
    #[default]
    Queued,
    Preparing,
    Checking,
    Downloading,
    Finishing,
    /// Checking an installed game's files (Verify integrity).
    Verifying,
    Done,
    Failed,
}

impl OwnedApp {
    pub fn is_dlc(&self) -> bool {
        self.kind == "dlc"
    }

    /// Can it be installed and played on this computer's system? Like
    /// Steam: from the app's own system list, else its launch options;
    /// unknown counts as yes.
    pub fn runs_on_this_os(&self) -> bool {
        self.runs_on(THIS_OS)
    }

    /// Does it have a version for `os` ("windows", "macos", "linux")?
    pub fn runs_on(&self, os: &str) -> bool {
        if !self.os_list.trim().is_empty() {
            return os_matches(&self.os_list, os);
        }
        let launch: Vec<&LaunchOption> = self.launch.iter().filter(|o| !o.executable.is_empty()).collect();
        launch.is_empty() || launch.iter().any(|o| os_matches(&o.os_list, os))
    }

    /// The systems it has versions for, in Steam's order (empty: unknown).
    pub fn systems(&self) -> Vec<&'static str> {
        let known = !self.os_list.trim().is_empty() || self.launch.iter().any(|o| !o.os_list.trim().is_empty());
        if !known {
            return Vec::new();
        }
        ["windows", "macos", "linux"].into_iter().filter(|os| self.runs_on(os)).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchOption {
    pub executable: String,
    pub arguments: String,
    pub working_dir: String,
    pub description: String,
    /// Empty means any OS.
    pub os_list: String,
}

impl LaunchOption {
    pub fn runs_on_this_os(&self) -> bool {
        os_matches(&self.os_list, THIS_OS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PersonaState {
    #[default]
    Offline,
    Online,
    Busy,
    Away,
    Snooze,
    LookingToTrade,
    LookingToPlay,
    Invisible,
}

impl PersonaState {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Online,
            2 => Self::Busy,
            3 => Self::Away,
            4 => Self::Snooze,
            5 => Self::LookingToTrade,
            6 => Self::LookingToPlay,
            7 => Self::Invisible,
            _ => Self::Offline,
        }
    }
    pub fn as_u32(self) -> u32 {
        self as u32
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Offline => "Offline",
            Self::Online => "Online",
            Self::Busy => "Busy",
            Self::Away => "Away",
            Self::Snooze => "Snooze",
            Self::LookingToTrade => "Looking to trade",
            Self::LookingToPlay => "Looking to play",
            Self::Invisible => "Invisible",
        }
    }
    /// States a user can pick for themselves.
    pub const SELECTABLE: [PersonaState; 4] =
        [Self::Online, Self::Away, Self::Busy, Self::Invisible];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Relationship {
    None,
    Blocked,
    RequestRecipient,
    Friend,
    RequestInitiator,
    Ignored,
    Other(u32),
}

impl Relationship {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Blocked,
            2 => Self::RequestRecipient,
            3 => Self::Friend,
            4 => Self::RequestInitiator,
            5 => Self::Ignored,
            o => Self::Other(o),
        }
    }
}

/// Everything known about one user's public presence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Persona {
    pub steam_id: SteamId,
    pub name: String,
    pub state: PersonaState,
    pub game_app_id: Option<AppId>,
    pub game_name: Option<String>,
    /// The game's own status line, when it's plain text (e.g. "In a match").
    #[serde(default)]
    pub rich_status: Option<String>,
    /// Hex avatar hash; see [`art::avatar`].
    pub avatar_hash: Option<String>,
    pub last_logoff: Option<u64>,
    /// The game server they're on ("ip:port"), when the game reports one.
    #[serde(default)]
    pub server: Option<String>,
    /// The game lobby they're in.
    #[serde(default)]
    pub lobby: Option<u64>,
    /// How to join them: the game's own launch arguments (rich presence "connect").
    #[serde(default)]
    pub connect: Option<String>,
    /// They're broadcasting their game on Steam.
    #[serde(default)]
    pub broadcasting: bool,
    /// Your nickname for them (Steam syncs these).
    #[serde(default)]
    pub nickname: Option<String>,
}

impl Persona {
    /// Your nickname for them, else their name, else their SteamID.
    pub fn display_name(&self) -> String {
        match (&self.nickname, self.name.is_empty()) {
            (Some(n), _) if !n.is_empty() => n.clone(),
            (_, false) => self.name.clone(),
            _ => self.steam_id.to_string(),
        }
    }

    /// Launch arguments that join them in their game, if Steam says how.
    pub fn join_args(&self) -> Option<String> {
        if let Some(tokens) = self.connect.as_deref().and_then(|c| safe_connect_tokens(c).ok()).filter(|t| !t.is_empty()) {
            return Some(tokens.join(" "));
        }
        if let Some(s) = self.server.as_deref().map(str::trim).filter(|s| is_safe_host_port(s)) {
            return Some(format!("+connect {s}"));
        }
        self.lobby.map(|l| format!("+connect_lobby {l}"))
    }

    pub fn in_game(&self) -> bool {
        self.game_app_id.is_some() || self.game_name.is_some()
    }
}

/// Safe `host` / `host:port` / `[ipv6]:port` token for `+connect`.
pub fn is_safe_host_port(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with(['-', '+'])
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'[' | b']' | b'-'))
}

fn is_u64_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u64>().is_ok()
}

fn is_safe_connect_password(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with(['-', '+'])
        && !s.contains("%command%")
        && !s.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '"' | '\'' | ';' | '\\'))
}

/// "-connect=<host>" with optional "-port=<digits>" and "-password=<pw>"
/// (Bohemia's engines: Arma, DayZ), each once and in any order.
fn dash_connect_tokens(tokens: &[&str]) -> Result<Vec<String>, &'static str> {
    let (mut host, mut port, mut password) = (None, None, None);
    for t in tokens {
        let (key, value) = t.split_once('=').ok_or("unsafe connect arguments")?;
        let slot = match key {
            "-connect" if is_safe_host_port(value) => &mut host,
            "-port" if is_u64_digits(value) && value.parse::<u16>().is_ok() => &mut port,
            "-password" if is_safe_connect_password(value) => &mut password,
            _ => return Err("unsafe connect arguments"),
        };
        if slot.replace(value).is_some() {
            return Err("unsafe connect arguments");
        }
    }
    let host = host.ok_or("unsafe connect arguments")?;
    let mut out = vec![format!("-connect={host}")];
    out.extend(port.map(|p| format!("-port={p}")));
    out.extend(password.map(|p| format!("-password={p}")));
    Ok(out)
}

/// Validate an untrusted remote join/invite `connect` string into argv tokens.
/// Empty or whitespace-only strings return `Ok(Vec::new())`.
pub fn safe_connect_tokens(connect: &str) -> Result<Vec<String>, &'static str> {
    if connect.chars().any(char::is_control) {
        return Err("unsafe control characters in connect arguments");
    }
    let trimmed = connect.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    match tokens.as_slice() {
        ["+connect_lobby", id] if is_u64_digits(id) => Ok(vec!["+connect_lobby".into(), (*id).into()]),
        ["+invite", id] if is_u64_digits(id) => Ok(vec!["+invite".into(), (*id).into()]),
        ["+connect", addr] if is_safe_host_port(addr) => Ok(vec!["+connect".into(), (*addr).into()]),
        ["+connect", addr, "+password", pw] if is_safe_host_port(addr) && is_safe_connect_password(pw) => {
            Ok(vec!["+connect".into(), (*addr).into(), "+password".into(), (*pw).into()])
        }
        // Arma / DayZ style: "-connect=host [-port=N] [-password=pw]", each
        // once, nothing else.
        _ if tokens.iter().all(|t| t.starts_with('-')) => dash_connect_tokens(&tokens),
        _ => Err("unsafe connect arguments"),
    }
}

/// Public profile details and equipped cosmetics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub steam_id: SteamId,
    /// Full URL of the profile background image.
    pub background: Option<String>,
    /// Full URL of the avatar frame (PNG).
    pub avatar_frame: Option<String>,
    pub real_name: String,
    pub summary: String,
    pub location: String,
    /// Unix seconds.
    pub member_since: Option<u64>,
    /// Who can see this profile's games, inventory and friends.
    pub visibility: Visibility,
    /// VAC bans on record (from the public profile).
    #[serde(default)]
    pub vac_banned: bool,
    /// An animated background's video (MP4), played behind the profile;
    /// `background` is its still picture.
    #[serde(default)]
    pub background_video: Option<String>,
    /// Whether the background fills the window (Steam's "Full Screen")
    /// rather than showing at its own size.
    #[serde(default)]
    pub background_full: bool,
    /// An animated avatar from the Points Shop (an animated GIF).
    #[serde(default)]
    pub animated_avatar: Option<String>,
    /// The mini profile's background: picture and video.
    #[serde(default)]
    pub mini_background: Option<String>,
    #[serde(default)]
    pub mini_background_video: Option<String>,
    /// The profile's Steam theme ("Summer", "DarkMode"…; "" for the default).
    #[serde(default)]
    pub theme: String,
}

/// Steam community visibility of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Visibility {
    /// Not loaded yet.
    #[default]
    Unknown,
    Private,
    FriendsOnly,
    Public,
}

/// One item (or stack) in a Steam inventory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InventoryItem {
    pub asset_id: u64,
    pub class_id: u64,
    pub amount: u64,
    pub name: String,
    /// The item's type line, e.g. "Mil-Spec Grade Rifle" or "Trading Card".
    pub kind: String,
    /// Hex colors without '#', as Steam sends them.
    pub name_color: Option<String>,
    pub background: Option<String>,
    /// Image hash; see [`art::econ`].
    pub icon: String,
    pub icon_large: Option<String>,
    pub tradable: bool,
    pub marketable: bool,
    pub market_hash_name: String,
    /// Description lines (may contain simple HTML) and their color.
    pub description: Vec<(String, Option<String>)>,
    /// (category, tag, color)
    pub tags: Vec<(String, String, Option<String>)>,
    /// Links Steam attaches, e.g. "Inspect in Game..." (name, url).
    pub actions: Vec<(String, String)>,
    pub context_id: u64,
    /// Notes only the owner sees, e.g. "Tradable After Nov 14, 2026".
    pub owner_notes: Vec<String>,
    /// Days of trade / market hold after getting the item (0 = none).
    pub trade_hold_days: u32,
    pub market_hold_days: u32,
    /// Interchangeable items (cards, cases) that the market sells by count.
    pub commodity: bool,
    /// The game's own market fee (e.g. 0.10); Steam's is separate.
    pub market_fee: Option<f64>,
    /// Actions only the owner has, e.g. "Turn into Gems..." (name, link).
    #[serde(default)]
    pub owner_actions: Vec<(String, String)>,
}

/// What an item sells for on the Community Market, in the wallet currency.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketPrice {
    /// Cheapest listing, as Steam formats it.
    pub lowest: Option<String>,
    pub median: Option<String>,
    /// Sold in the last 24 hours.
    pub volume: Option<u64>,
    /// In cents (what the buyer pays). From the order book, when loaded.
    pub lowest_sell: Option<u64>,
    pub highest_buy: Option<u64>,
    pub sell_count: Option<u64>,
    pub buy_count: Option<u64>,
    /// Fee rules for your wallet currency, when Steam sent them.
    pub fee_rules: Option<fees::Rules>,
    /// Steam currency code the cent values are in.
    pub currency: i32,
    /// Cheapest listings and highest buy orders: (price in cents, count).
    pub sell_ladder: Vec<(u64, u64)>,
    pub buy_ladder: Vec<(u64, u64)>,
    /// Daily median sale price: (unix time, price in currency units, sold).
    pub history: Vec<(u64, f64, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ListingStatus {
    #[default]
    Active,
    /// Waiting for confirmation in the Steam Mobile app (or email).
    NeedsConfirmation,
    OnHold,
}

/// One of your Community Market listings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MyListing {
    pub listing_id: u64,
    pub app_id: AppId,
    pub name: String,
    pub game: String,
    pub icon: String,
    pub name_color: Option<String>,
    /// Cents the buyer pays and you receive.
    pub buyer_pays: u64,
    pub you_receive: u64,
    pub currency: i32,
    pub amount: u64,
    /// Unix seconds.
    pub created: u64,
    pub status: ListingStatus,
    pub market_hash_name: String,
}

/// One of your buy orders.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuyOrder {
    pub order_id: u64,
    pub app_id: AppId,
    pub name: String,
    pub icon: String,
    pub price: u64,
    pub currency: i32,
    pub quantity: u64,
    pub remaining: u64,
    pub market_hash_name: String,
}

/// A Steam notification (the bell): gifts, items, comments, trade offers...
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Notification {
    pub id: u64,
    /// Steam's ESteamNotificationType (2 gift, 3 comment, 4 item, 5 friend
    /// invite, 8 wishlist, 9 trade offer, 11 support reply, ...).
    pub kind: i32,
    /// Unix seconds.
    pub time: u64,
    pub read: bool,
    /// Details Steam attached, when present.
    pub app_id: Option<AppId>,
    pub from: Option<SteamId>,
    pub text: Option<String>,
    /// New items: where the item is, and the game that dropped it.
    pub context_id: Option<u64>,
    pub asset_id: Option<u64>,
    pub source_app: Option<AppId>,
    /// Support replies: the request's reference code.
    pub ticket: Option<String>,
    /// The raw JSON Steam sent, for plugins and future types.
    pub data: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Notifications {
    pub items: Vec<Notification>,
    pub unread: u32,
    pub pending_gifts: u32,
    pub pending_friends: u32,
    /// Things waiting for confirmation in the Steam Mobile app.
    pub confirmations: u32,
}

/// A comment on a profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub author: SteamId,
    pub name: String,
    pub avatar: Option<String>,
    /// Unix seconds.
    pub time: u64,
    /// The comment as Steam's HTML.
    pub html: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Comments {
    pub items: Vec<Comment>,
    pub total: u32,
}

/// A badge on a profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Badge {
    pub title: String,
    pub icon: Option<String>,
    /// e.g. "340 XP" or "Level 2, 200 XP".
    pub detail: String,
    /// e.g. "Unlocked Oct 14 @ 10:06am".
    pub unlocked: String,
    /// When it was unlocked, as your clock showed it: seconds since 1970
    /// read as UTC (0 if Steam's text couldn't be read).
    pub unlocked_at: u64,
    pub url: String,
    /// Game badges: hours played and card drops left, as Steam writes them
    /// ("3,773 hrs on record", "3 card drops remaining").
    pub playtime: String,
    pub drops: String,
}

/// A trading card in a game badge's set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BadgeCard {
    pub name: String,
    pub image: String,
    /// How many you have (0 = missing).
    pub owned: u32,
    /// e.g. "3 of 8, Series 1".
    pub position: String,
}

/// A badge's own page: the badge, and for a game badge its card set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BadgeDetail {
    /// e.g. "Dota 2 Badge".
    pub title: String,
    /// The level you have now (empty title if none yet).
    pub current: Badge,
    pub cards: Vec<BadgeCard>,
    /// The game, for game badges.
    pub app_id: Option<AppId>,
}

/// The ways a Steam account's id is written: (name, value). The names are
/// the usual ones; "friend code" is the 32-bit account id Steam shows when
/// adding friends.
pub fn steam_id_formats(id: SteamId) -> [(&'static str, String); 5] {
    let account = id & 0xFFFF_FFFF;
    [
        ("SteamID64", id.to_string()),
        ("SteamID3", format!("[U:1:{account}]")),
        ("SteamID", format!("STEAM_0:{}:{}", account & 1, account >> 1)),
        ("friend_code", account.to_string()),
        ("link", format!("https://steamcommunity.com/profiles/{id}")),
    ]
}

/// Who can see a part of your profile: 1 only you, 2 friends, 3 everyone.
/// Comments: 0 friends, 1 everyone, 2 nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProfilePrivacy {
    pub profile: u8,
    pub inventory: u8,
    pub gifts: u8,
    pub games: u8,
    pub playtime: u8,
    pub friends: u8,
    pub comments: u8,
}

/// A profile item you own (background, frame...).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileItemChoice {
    pub id: u64,
    pub name: String,
    pub image: String,
    pub app_id: AppId,
    /// The full-size picture (backgrounds), when there's one.
    #[serde(default)]
    pub large: String,
    /// Its video (MP4) for animated items.
    #[serde(default)]
    pub movie: String,
    /// The game (or Steam collection) it comes from.
    #[serde(default)]
    pub app_name: String,
}

/// Your profile as Steam's Edit Profile pages show it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileEdit {
    pub persona_name: String,
    pub real_name: String,
    pub custom_url: String,
    pub summary: String,
    /// Location codes as Steam stores them (country "KZ", state, city id).
    pub country: String,
    pub state: String,
    pub city: String,
    /// "City, State, Country" as Steam names them.
    pub location_text: String,
    pub privacy: ProfilePrivacy,
    pub hide_awards: bool,
    pub theme: String,
    /// (theme id, title)
    pub themes: Vec<(String, String)>,
    pub backgrounds: Vec<ProfileItemChoice>,
    pub mini_backgrounds: Vec<ProfileItemChoice>,
    pub frames: Vec<ProfileItemChoice>,
    pub animated_avatars: Vec<ProfileItemChoice>,
    /// Equipped item ids: background, mini profile background, frame,
    /// animated avatar (0 = none).
    pub equipped: [u64; 4],
    /// The equipped background fills the window (Steam's "Full Screen").
    #[serde(default)]
    pub background_full: bool,
    /// The featured badge (a community badge's id; 0 = none).
    #[serde(default)]
    pub favorite_badge: u32,
}

/// Which part of the profile a save was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileSave {
    Info,
    Privacy,
    Avatar,
    Item,
    Theme,
}

/// A Steam group chat (Friends & Chat's chat room groups).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatGroup {
    pub id: u64,
    pub name: String,
    pub tagline: String,
    pub avatar: String,
    /// The Steam group it belongs to (0 = a group chat of friends).
    pub clan_id: u32,
    pub app_id: u32,
    /// The room it opens on.
    pub default_room: u64,
    pub active_members: u32,
    /// Text channels.
    pub rooms: Vec<ChatRoom>,
    /// Something new since you last read (and not muted).
    pub unread: bool,
    pub muted: bool,
}

/// A text channel in a group chat.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatRoom {
    pub id: u64,
    pub name: String,
    pub last_message: String,
    /// Unix seconds.
    pub last_time: u32,
    pub last_sender: SteamId,
    pub unread: bool,
    pub voice: bool,
    pub sort: u32,
    /// Who's in its voice channel now.
    #[serde(default)]
    pub in_voice: Vec<SteamId>,
}

/// A message in a group chat room.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GroupMessage {
    pub group: u64,
    pub room: u64,
    pub sender: SteamId,
    /// With Steam's markup (mentions, links, invites).
    pub text: String,
    /// Unix seconds.
    pub time: u32,
    /// Orders messages sent in the same second.
    pub ordinal: u32,
    /// Reactions under it.
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

/// Days since 1970-01-01 for a calendar date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Badges {
    pub level: u32,
    pub xp: String,
    /// e.g. "85 XP to reach Level 19".
    pub next: String,
    /// 0..1 progress to the next level.
    pub progress: f32,
    pub items: Vec<Badge>,
}

/// Your Quick Invite link: anyone who opens it becomes your friend (once).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickInvite {
    /// `https://s.team/p/<code>/<token>`.
    pub link: String,
    /// Unix time it stops working.
    pub expires: u32,
}

/// A page of profiles found by name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserSearch {
    pub text: String,
    pub page: u32,
    /// Matches in all, across pages (Steam shows 20 a page).
    pub total: u32,
    pub users: Vec<FoundUser>,
}

/// A profile in search results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FoundUser {
    pub steam_id: SteamId,
    pub name: String,
    pub avatar: String,
    pub profile: String,
    /// Real name, country and old names, one per line, when shown.
    pub details: String,
}

/// A collection to write to Steam: its whole new content, or deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionEdit {
    /// "favorite", "hidden", or "uc-…".
    pub id: String,
    pub name: String,
    pub games: Vec<AppId>,
    /// A dynamic collection's filters, as Steam stores them. When set, `games`
    /// is ignored: the games added and removed by hand stay as they are.
    #[serde(default)]
    pub filter: Option<String>,
    /// The filters this edit started from (none for a new collection). If
    /// Steam's copy has moved on, the edit isn't saved.
    #[serde(default)]
    pub based_on: Option<String>,
    pub deleted: bool,
}

/// A review on someone's profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileReview {
    pub app_id: AppId,
    pub voted_up: bool,
    /// e.g. "3,773.1 hrs on record".
    pub hours: String,
    /// e.g. "Posted 12 May, 2021."
    pub posted: String,
    pub text: String,
    /// e.g. "12 people found this review helpful".
    pub helpful: String,
}

/// A Steam group someone is in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileGroup {
    pub id: u64,
    pub name: String,
    /// The group's address name (steamcommunity.com/groups/<url>).
    pub url: String,
    pub headline: String,
    pub avatar: String,
    pub members: u32,
    pub online: u32,
    /// Their primary group.
    pub primary: bool,
}

/// One result of a Community Market search.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MarketResult {
    pub app_id: AppId,
    pub app_name: String,
    pub name: String,
    pub hash_name: String,
    pub icon: String,
    /// Name color (hex, no #), for rarity.
    pub color: String,
    pub listings: u32,
    /// Lowest price, as Steam formats it in your currency.
    pub price: String,
}

/// Your review of a game (`id` 0: not written yet).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MyReview {
    pub id: u64,
    pub text: String,
    /// Recommended (thumbs up) or not.
    pub voted_up: bool,
    /// Public, or friends only.
    pub public: bool,
    pub comments_disabled: bool,
    /// "I received this game for free".
    pub received_compensation: bool,
    /// Steam's language name ("english", "russian"…).
    pub language: String,
    /// Unix seconds.
    pub updated: u64,
    pub votes_up: u32,
    pub votes_funny: u32,
    pub comments: u32,
    /// Minutes played in total, and when it was written.
    #[serde(default)]
    pub playtime: u32,
    #[serde(default)]
    pub playtime_at_review: u32,
}

/// A vote on someone's review: "Was this review helpful?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReviewVote {
    Yes,
    No,
    Funny,
}

/// A library collection from the official Steam client (synced by Steam).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SteamCollection {
    /// "favorite", "hidden", or Steam's id for one you made ("uc-…").
    pub id: String,
    pub name: String,
    pub games: Vec<AppId>,
    /// Built from filters in Steam (its games aren't listed).
    pub dynamic: bool,
    /// A dynamic collection's filters, exactly as Steam stores them
    /// (`filterSpec`).
    #[serde(default)]
    pub filter: Option<String>,
    /// Games put in by hand (on a dynamic collection, on top of its filters).
    #[serde(default)]
    pub added: Vec<AppId>,
    /// Games taken out by hand.
    #[serde(default)]
    pub removed: Vec<AppId>,
}

/// A Steam download region with the connection time measured from this PC.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DownloadRegion {
    /// 0 for a partner CDN (pick it by `host`).
    pub cell_id: u32,
    pub name: String,
    pub code: String,
    pub host: String,
    /// Server load, 0-100.
    pub load: u32,
    /// Best TCP connect time; `None` if unreachable.
    pub ping_ms: Option<u32>,
}

impl DownloadRegion {
    /// Rough download speed from ping and load: 0 slow .. 3 fastest.
    pub fn speed_tier(&self) -> u8 {
        let base: u8 = match self.ping_ms {
            Some(p) if p < 30 => 3,
            Some(p) if p < 80 => 2,
            Some(p) if p < 160 => 1,
            _ => 0,
        };
        if self.load > 85 { base.saturating_sub(1) } else { base }
    }
}

/// One of your Steam Support requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SupportTicket {
    pub id: String,
    pub title: String,
    /// What it's about, usually a game's name.
    pub area: String,
    pub icon: Option<String>,
    pub status: String,
    pub open: bool,
    /// As Steam shows it, e.g. "Oct 7".
    pub updated: String,
    pub url: String,
}

/// One message in a support request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SupportMessage {
    pub from_steam: bool,
    /// "Message from Steam Support on Oct 9 @ 2:43pm | 4 weeks ago"
    pub header: String,
    /// The message body as Steam's HTML.
    pub html: String,
}

/// Your account's security details (Steam's "Account details" page).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountSecurity {
    pub steam_guard: bool,
    /// Unix time Steam Guard was turned on.
    pub steam_guard_since: Option<u64>,
    /// The Steam Mobile authenticator.
    pub authenticator: bool,
    pub authenticator_since: Option<u64>,
    pub phone_verified: bool,
    pub password_changed: Option<u64>,
    pub email_changed: Option<u64>,
    pub password_reset: Option<u64>,
    /// Computers that Steam Guard remembers for this account.
    pub devices: Vec<GuardDevice>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuardDevice {
    pub name: String,
    /// Unix time it was authorized.
    pub since: Option<u64>,
    pub ip: String,
}

/// One row of your purchase history (store purchases, gifts, wallet and
/// Market transactions).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Purchase {
    /// As Steam writes it ("8 Nov, 2026").
    pub date: String,
    pub items: Vec<String>,
    /// "Purchase", "Gift Purchase", "Refund", "Market Transaction", ...
    pub kind: String,
    /// Payment method under the type, when shown.
    pub payment: String,
    pub total: String,
    pub wallet_change: String,
    pub wallet_balance: String,
    /// Refunded or otherwise struck through.
    pub refunded: bool,
}

/// A page of purchase history; `more` continues it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PurchasePage {
    pub rows: Vec<Purchase>,
    /// Opaque cursor for the next page (None at the end).
    pub more: Option<String>,
}

/// A license on your account (Steam's "Licenses and product key activations").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct License {
    pub date: String,
    pub name: String,
    /// "Steam Store", "Retail", "Complimentary", "Gift/Guest Pass", ...
    pub method: String,
    /// Package id, when Steam shows a remove link for it.
    pub package_id: Option<u32>,
}

/// Your Steam Family.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Family {
    pub id: u64,
    pub name: String,
    pub members: Vec<FamilyMember>,
    /// People invited but not in yet: (steam id, role).
    pub invites: Vec<(SteamId, FamilyRole)>,
    pub free_spots: u32,
    pub country: String,
    /// Seconds until a new member can join after someone left.
    pub slot_cooldown: u32,
    /// Games shared in the family, with who owns them.
    pub shared: Vec<SharedApp>,
    /// People who left: they can be invited back during a slot cooldown.
    #[serde(default)]
    pub former: Vec<SteamId>,
    /// Exceptions Steam granted to the slot cooldown (it doesn't apply
    /// while there are any).
    #[serde(default)]
    pub slot_overrides: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FamilyRole {
    Adult,
    Child,
    #[default]
    Unknown,
}

impl FamilyRole {
    /// Steam's `EFamilyGroupRole`: 1 adult, 2 child.
    pub fn from_steam(v: i32) -> Self {
        match v {
            1 => FamilyRole::Adult,
            2 => FamilyRole::Child,
            _ => FamilyRole::Unknown,
        }
    }

    /// Back to `EFamilyGroupRole`; `None` for a role Steam can't be sent.
    pub fn to_steam(self) -> Option<i32> {
        match self {
            FamilyRole::Adult => Some(1),
            FamilyRole::Child => Some(2),
            FamilyRole::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FamilyMember {
    pub steam_id: SteamId,
    pub role: FamilyRole,
    pub joined: u64,
    /// Seconds until they can join another family after leaving.
    pub cooldown: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharedApp {
    pub app_id: AppId,
    pub name: String,
    pub owners: Vec<SteamId>,
    /// Can't be shared (Steam's exclude reason is not 0).
    pub excluded: bool,
}

/// Whether you're in a family, and which.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FamilyStatus {
    pub family: Option<Family>,
    /// Your own role in it.
    pub role: FamilyRole,
    /// Seconds before you can join a family again.
    pub cooldown: u32,
    /// Families that invited you.
    pub invites: Vec<FamilyInvite>,
}

/// A family's invitation to you.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyInvite {
    pub family: u64,
    pub inviter: SteamId,
    /// What you'd join as.
    pub role: FamilyRole,
    pub invite_id: u64,
    /// You accepted and Steam is waiting for your phone or email code.
    pub awaiting_confirmation: bool,
}

/// Something done to a Steam Family. Each is one call to Steam's
/// FamilyGroups service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FamilyAction {
    Invite { family: u64, steam_id: SteamId, role: FamilyRole },
    /// Take back an invitation you sent.
    CancelInvite { family: u64, steam_id: SteamId },
    /// Accept an invitation to you.
    Accept { family: u64 },
    /// The code Steam emailed you after accepting.
    ConfirmJoin { family: u64, invite_id: u64, code: u64 },
    /// Turn down an invitation to you.
    Decline { family: u64 },
    Remove { family: u64, steam_id: SteamId },
    /// Remove yourself.
    Leave { family: u64 },
}

/// How Steam answered a family action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FamilyOutcome {
    Done,
    /// Waiting for approval in the Steam Mobile app.
    ConfirmOnPhone,
    /// Waiting for the email: a link (inviting) or a code (joining).
    ConfirmByEmail,
    /// Already accepted: a confirmation request is waiting.
    AlreadyAccepted,
    /// Steam asked for a confirmation this app doesn't know.
    ConfirmOther,
}

/// Why you can't invite someone new right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteBlock {
    /// Only adults can invite.
    NotAdult,
    /// All six places are taken.
    Full,
    /// A place was left recently: only former members can be invited,
    /// for this many more seconds.
    SlotCooldown(u32),
}

impl FamilyStatus {
    /// What stops you inviting someone who was never in this family.
    pub fn invite_block(&self) -> Option<InviteBlock> {
        let f = self.family.as_ref()?;
        if self.role != FamilyRole::Adult {
            return Some(InviteBlock::NotAdult);
        }
        if f.free_spots == 0 {
            return Some(InviteBlock::Full);
        }
        (f.slot_cooldown > 0 && f.slot_overrides == 0).then_some(InviteBlock::SlotCooldown(f.slot_cooldown))
    }

    /// Whether `who` can be invited now: not already in or invited, and
    /// allowed by [`Self::invite_block`] (a slot cooldown still lets former
    /// members back in).
    pub fn can_invite(&self, who: SteamId) -> bool {
        let Some(f) = &self.family else { return false };
        if who == 0 || f.members.iter().any(|m| m.steam_id == who) || f.invites.iter().any(|i| i.0 == who) {
            return false;
        }
        match self.invite_block() {
            None => true,
            Some(InviteBlock::SlotCooldown(_)) => f.former.contains(&who),
            Some(_) => false,
        }
    }

    /// Whether you can remove `who` (yourself = leaving). As on Steam's
    /// page: adults only, and not the last member (that's deleting the
    /// family, which stays on Steam's site).
    pub fn can_remove(&self, who: SteamId) -> bool {
        let Some(f) = &self.family else { return false };
        self.role == FamilyRole::Adult && f.members.len() > 1 && f.members.iter().any(|m| m.steam_id == who)
    }
}

/// What happened in a row of your Community Market history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryKind {
    Bought,
    Sold,
    Listed,
    Canceled,
    #[default]
    Other,
}

/// One row of your Community Market history, as Steam shows it (price and
/// dates already written out in your currency and language).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryRow {
    pub id: String,
    pub kind: HistoryKind,
    pub name: String,
    pub name_color: Option<String>,
    pub game: String,
    /// Full picture link.
    pub image: String,
    pub price: String,
    /// When it happened, and when it was listed.
    pub acted: String,
    pub listed: String,
    /// Who bought or sold it (empty for listings you made or took down).
    pub other: String,
    /// What Steam writes in that column when nobody else was involved
    /// ("Listing created", "Listing canceled"…).
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MyListings {
    pub listings: Vec<MyListing>,
    pub buy_orders: Vec<BuyOrder>,
}

/// Community Market fee math, matching Steam's own: a Steam fee (5%) plus the
/// game's fee (usually 10%), each rounded down and at least the wallet
/// currency's minimum fee (1 cent in USD, more in some currencies).
pub mod fees {
    use serde::{Deserialize, Serialize};

    pub const DEFAULT_GAME: f64 = 0.10;

    /// Steam's fee rules for one wallet currency (`g_rgWalletInfo`).
    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    pub struct Rules {
        pub minimum: u64,
        pub percent: f64,
        pub base: u64,
        pub game_default: f64,
    }

    impl Default for Rules {
        fn default() -> Self {
            Self { minimum: 1, percent: 0.05, base: 0, game_default: DEFAULT_GAME }
        }
    }

    /// (Steam fee, game fee) when you receive `receive`.
    pub fn split(receive: u64, game_fee: f64, r: &Rules) -> (u64, u64) {
        let steam = (receive as f64 * r.percent).floor().max(r.minimum as f64) as u64 + r.base;
        let game = if game_fee > 0.0 { (receive as f64 * game_fee).floor().max(r.minimum as f64) as u64 } else { 0 };
        (steam, game)
    }

    /// What the buyer pays when you ask to receive `receive`.
    pub fn buyer_pays(receive: u64, game_fee: f64, r: &Rules) -> u64 {
        if receive == 0 {
            return 0;
        }
        let (a, b) = split(receive, game_fee, r);
        receive + a + b
    }

    /// The most you receive while the buyer pays at most `pays`.
    pub fn you_receive(pays: u64, game_fee: f64, r: &Rules) -> u64 {
        if pays <= buyer_pays(1, game_fee, r) {
            return if pays == buyer_pays(1, game_fee, r) { 1 } else { 0 };
        }
        let mut x = ((pays.saturating_sub(r.base)) as f64 / (1.0 + r.percent + game_fee)).floor() as u64;
        while x > 0 && buyer_pays(x, game_fee, r) > pays {
            x -= 1;
        }
        while buyer_pays(x + 1, game_fee, r) <= pays {
            x += 1;
        }
        x
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn matches_steam_usd() {
            let r = Rules::default();
            assert_eq!(buyer_pays(1, DEFAULT_GAME, &r), 3);
            assert_eq!(you_receive(3, DEFAULT_GAME, &r), 1);
            assert_eq!(buyer_pays(100, DEFAULT_GAME, &r), 115);
            assert_eq!(you_receive(115, DEFAULT_GAME, &r), 100);
            for pays in 3..5000 {
                assert!(buyer_pays(you_receive(pays, DEFAULT_GAME, &r), DEFAULT_GAME, &r) <= pays);
            }
        }
        #[test]
        fn matches_steam_with_a_currency_minimum() {
            // Real tenge listings: receive 5.00 -> buyer pays 13.00; 21.00 -> 29.00.
            let r = Rules { minimum: 400, ..Rules::default() };
            assert_eq!(buyer_pays(500, 0.1, &r), 1300);
            assert_eq!(buyer_pays(2100, 0.1, &r), 2900);
            assert_eq!(you_receive(2900, 0.1, &r), 2100);
        }
    }
}

/// "$1,234.56", "1 234,56€", "¥ 120" -> cents (for currencies with cents).
pub fn parse_money(s: &str) -> Option<u64> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    let digits = digits.trim_matches(|c| c == '.' || c == ',');
    if digits.is_empty() {
        return None;
    }
    // A separator followed by exactly two digits at the end is the decimal point.
    let bytes = digits.as_bytes();
    let n = bytes.len();
    let has_cents = n > 3 && (bytes[n - 3] == b'.' || bytes[n - 3] == b',');
    let whole: String = digits.chars().filter(|c| c.is_ascii_digit()).collect();
    let v: u64 = whole.parse().ok()?;
    Some(if has_cents { v } else { v * 100 })
}

/// Where a trade offer stands, as Steam reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TradeState {
    /// Waiting for the other person to answer.
    #[default]
    Active,
    Accepted,
    /// Replaced by a counter offer.
    Countered,
    Expired,
    Cancelled,
    Declined,
    /// Some items aren't available any more.
    InvalidItems,
    /// Made, but waiting for confirmation in the Steam Mobile app.
    NeedsConfirmation,
    /// Accepted, with the items held by Steam for a while.
    InEscrow,
    Other,
}

impl TradeState {
    pub fn from_steam(n: u64) -> TradeState {
        match n {
            2 => TradeState::Active,
            3 => TradeState::Accepted,
            4 => TradeState::Countered,
            5 => TradeState::Expired,
            6 | 10 => TradeState::Cancelled,
            7 => TradeState::Declined,
            8 => TradeState::InvalidItems,
            9 => TradeState::NeedsConfirmation,
            11 => TradeState::InEscrow,
            _ => TradeState::Other,
        }
    }

    /// Still waiting on someone (you, them, or a phone confirmation).
    pub fn open(self) -> bool {
        matches!(self, TradeState::Active | TradeState::NeedsConfirmation)
    }
}

/// One item in a trade offer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TradeItem {
    pub app_id: AppId,
    /// Name, picture, context and asset id like an inventory item.
    pub item: InventoryItem,
    /// No longer in the owner's inventory.
    pub missing: bool,
}

/// A trade offer you sent or got.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TradeOffer {
    pub id: u64,
    pub partner: SteamId,
    pub message: String,
    /// Unix seconds.
    pub created: u64,
    pub updated: u64,
    pub expires: u64,
    /// When held items arrive, if Steam holds them (0 = no hold).
    pub escrow_end: u64,
    pub state: TradeState,
    /// You made it.
    pub ours: bool,
    /// Items you give.
    pub give: Vec<TradeItem>,
    /// Items you get.
    pub get: Vec<TradeItem>,
}

/// Your trade offers: active ones and those that changed recently.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TradeOffers {
    pub received: Vec<TradeOffer>,
    pub sent: Vec<TradeOffer>,
}

/// An answer to a trade offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeAction {
    Accept,
    Decline,
    /// Take back one you sent.
    Cancel,
}

/// An item to put in a new trade offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TradeAsset {
    pub app_id: AppId,
    pub context_id: u64,
    pub asset_id: u64,
    pub amount: u64,
}

/// A game that has an inventory on someone's account, as Steam lists it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InventoryApp {
    pub app_id: AppId,
    pub name: String,
    pub icon: Option<String>,
    /// Sections of the inventory: (context id, name, item count).
    pub contexts: Vec<(u64, String, u32)>,
}

/// One game's inventory for one account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub steam_id: SteamId,
    pub app_id: AppId,
    pub context_id: u64,
    pub items: Vec<InventoryItem>,
    pub total: u32,
    /// Set when the inventory couldn't be read ("private" for hidden ones).
    pub error: Option<String>,
}

/// One of a game's achievements, with whether you have it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Achievement {
    pub name: String,
    pub description: String,
    pub icon: String,
    pub icon_gray: String,
    pub hidden: bool,
    pub unlocked: bool,
    /// Unix seconds, 0 if locked.
    pub unlock_time: u64,
    /// Share of players who have it, 0..100.
    pub global_percent: f32,
}

/// A game on someone's profile, with their playtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerGame {
    pub app_id: AppId,
    pub name: String,
    pub playtime_minutes: u64,
    pub playtime_2weeks: u64,
    pub last_played: Option<u64>,
    pub icon: Option<String>,
}

/// Steam currency code -> (ISO code, symbol, symbol goes after the number).
pub fn currency(code: i32) -> (&'static str, &'static str, bool) {
    match code {
        1 => ("USD", "$", false),
        2 => ("GBP", "£", false),
        3 => ("EUR", "€", true),
        4 => ("CHF", "CHF ", false),
        5 => ("RUB", " ₽", true),
        6 => ("PLN", "zł", true),
        7 => ("BRL", "R$ ", false),
        8 => ("JPY", "¥ ", false),
        17 => ("TRY", "₺", false),
        18 => ("UAH", "₴", true),
        20 => ("CAD", "CDN$ ", false),
        21 => ("AUD", "A$ ", false),
        23 => ("CNY", "¥ ", false),
        24 => ("INR", "₹ ", false),
        37 => ("KZT", "₸", true),
        _ => ("", "", false),
    }
}

/// A country whose store prices are in this Steam currency (lowercase
/// two-letter code), so the store can price like your wallet.
pub fn currency_country(code: i32) -> Option<&'static str> {
    Some(match code {
        1 => "us",
        2 => "gb",
        3 => "de",
        4 => "ch",
        5 => "ru",
        6 => "pl",
        7 => "br",
        8 => "jp",
        9 => "no",
        10 => "id",
        11 => "my",
        12 => "ph",
        13 => "sg",
        14 => "th",
        15 => "vn",
        16 => "kr",
        17 => "tr",
        18 => "ua",
        19 => "mx",
        20 => "ca",
        21 => "au",
        22 => "nz",
        23 => "cn",
        24 => "in",
        25 => "cl",
        26 => "pe",
        27 => "co",
        28 => "za",
        29 => "hk",
        30 => "tw",
        31 => "sa",
        32 => "ae",
        33 => "se",
        34 => "ar",
        35 => "il",
        36 => "by",
        37 => "kz",
        38 => "kw",
        39 => "qa",
        40 => "cr",
        41 => "uy",
        42 => "bg",
        43 => "hr",
        44 => "cz",
        45 => "dk",
        46 => "hu",
        47 => "ro",
        _ => return None,
    })
}

/// The `cc` to load the store with: your wallet's country, like Steam's own
/// client, so a VPN doesn't change the currency. The IP country (from
/// sign-in) is only a fallback for accounts without a wallet. `None` = not
/// known yet.
pub fn store_country(wallet_currency: Option<i32>, ip_country: &str) -> Option<String> {
    let wallet = wallet_currency.and_then(currency_country).map(str::to_string);
    wallet.or_else(|| Some(ip_country.to_lowercase()).filter(|c| !c.is_empty()))
}

#[cfg(test)]
mod store_country_tests {
    use super::*;

    #[test]
    fn wallet_currency_picks_the_country() {
        assert_eq!(currency_country(37), Some("kz"));
        assert_eq!(currency_country(5), Some("ru"));
        assert_eq!(currency_country(1), Some("us"));
        assert_eq!(currency_country(18), Some("ua"));
        assert_eq!(currency_country(3), Some("de"));
        assert_eq!(currency_country(0), None);
        assert_eq!(currency_country(999), None);
    }

    #[test]
    fn every_known_currency_has_a_country() {
        for code in 1..=47 {
            if !currency(code).0.is_empty() {
                assert!(currency_country(code).is_some(), "currency {code}");
            }
        }
    }

    #[test]
    fn wallet_beats_the_vpn_country() {
        // KZT wallet, signed in through a Dutch VPN.
        assert_eq!(store_country(Some(37), "NL").as_deref(), Some("kz"));
        // No wallet: the IP country.
        assert_eq!(store_country(None, "NL").as_deref(), Some("nl"));
        // Unknown currency code: the IP country.
        assert_eq!(store_country(Some(0), "DE").as_deref(), Some("de"));
        // Nothing yet: wait.
        assert_eq!(store_country(None, ""), None);
    }
}

pub fn format_money(cents: i64, code: i32) -> String {
    let (iso, _, _) = currency(code);
    format_price(cents, iso)
}

/// A price the way Steam's store writes it in that currency: "$19.99",
/// "19,99€", "4 480₸", "1 999 ₽", "R$ 19,99", "¥ 1,980"…
pub fn format_price(cents: i64, iso: &str) -> String {
    // (before, after, decimal mark, thousands mark, whole numbers only)
    let (pre, post, dec, th, whole) = match iso {
        "USD" | "" => ("$", "", '.', ",", false),
        "EUR" => ("", "€", ',', " ", false),
        "GBP" => ("£", "", '.', ",", false),
        "RUB" => ("", " ₽", ',', " ", true),
        "KZT" => ("", "₸", ',', " ", true),
        "UAH" => ("", "₴", ',', " ", true),
        "PLN" => ("", "zł", ',', " ", false),
        "BRL" => ("R$ ", "", ',', ".", false),
        "TRY" => ("₺", "", ',', ".", false),
        "JPY" => ("¥ ", "", '.', ",", true),
        "CNY" => ("¥ ", "", '.', ",", false),
        "KRW" => ("₩ ", "", '.', ",", true),
        "INR" => ("₹ ", "", '.', ",", true),
        "CAD" => ("CDN$ ", "", '.', ",", false),
        "AUD" => ("A$ ", "", '.', ",", false),
        "NZD" => ("NZ$ ", "", '.', ",", false),
        "CHF" => ("CHF ", "", '.', "'", false),
        "NOK" => ("", " kr", ',', " ", false),
        "MXN" => ("Mex$ ", "", '.', ",", false),
        "IDR" => ("Rp ", "", ',', " ", true),
        "VND" => ("", "₫", ',', ".", true),
        "SGD" => ("S$", "", '.', ",", false),
        "HKD" => ("HK$ ", "", '.', ",", false),
        "ILS" => ("₪", "", '.', ",", false),
        "ZAR" => ("R ", "", '.', " ", false),
        other => return format!("{:.2} {other}", cents as f64 / 100.0),
    };
    let negative = cents < 0;
    let cents = cents.unsigned_abs();
    let (units, minor) = (cents / 100, cents % 100);
    // Group the whole part in threes.
    let digits = units.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push_str(th);
        }
        grouped.push(ch);
    }
    // Whole-number currencies still show kopecks/tiyn if a price has them.
    let frac = if whole && minor == 0 { String::new() } else { format!("{dec}{minor:02}") };
    format!("{}{pre}{grouped}{frac}{post}", if negative { "-" } else { "" })
}

#[cfg(test)]
mod price_tests {
    use super::format_price;

    #[test]
    fn prices_look_like_steams() {
        assert_eq!(format_price(448_000, "KZT"), "4 480₸");
        assert_eq!(format_price(640_000, "KZT"), "6 400₸");
        assert_eq!(format_price(1999, "USD"), "$19.99");
        assert_eq!(format_price(1999, "EUR"), "19,99€");
        assert_eq!(format_price(199_900, "RUB"), "1 999 ₽");
        assert_eq!(format_price(1_234_567, "USD"), "$12,345.67");
        assert_eq!(format_price(199_850, "KZT"), "1 998,50₸");
        assert_eq!(format_price(198_000, "JPY"), "¥ 1,980");
    }
}

/// One chat message in a 1:1 conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    /// The other person in the conversation.
    pub friend: SteamId,
    /// Who wrote it (the friend, or you).
    pub from: SteamId,
    pub text: String,
    /// Unix seconds.
    pub time: u64,
    /// A game invite (Steam's `[gameinvite]`, or an older SteamInRust
    /// "steam://run" link), shown as a card with a Join button.
    #[serde(default)]
    pub invite: Option<GameInvite>,
    /// The message as Steam sent it, with its markup (emoticons, spoilers,
    /// code, quotes, links, images); see [`chat::rich`]. Empty for messages
    /// without any.
    #[serde(default)]
    pub raw: String,
    /// Orders messages sent in the same second; with `time` it names the
    /// message for reactions and reports.
    #[serde(default)]
    pub ordinal: u32,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

/// A reaction under a chat message: an emoticon or a sticker, how many
/// people chose it, and whether you're one of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub name: String,
    pub sticker: bool,
    pub count: u32,
    pub mine: bool,
}

impl Reaction {
    /// Count one person adding (or taking back) `name`.
    pub fn apply(list: &mut Vec<Reaction>, name: &str, sticker: bool, add: bool, me: bool) {
        match list.iter_mut().position(|r| r.name == name && r.sticker == sticker) {
            Some(i) => {
                let r = &mut list[i];
                if add {
                    if !(me && r.mine) {
                        r.count += 1;
                    }
                } else if !me || r.mine {
                    r.count = r.count.saturating_sub(1);
                }
                if me {
                    r.mine = add;
                }
                if r.count == 0 {
                    list.remove(i);
                }
            }
            None if add => list.push(Reaction { name: name.to_string(), sticker, count: 1, mine: me }),
            None => {}
        }
    }
}

/// The chat emoticons, stickers and chat effects you own (Steam's
/// emoticon list).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Emoticons {
    pub emoticons: Vec<OwnedEmote>,
    pub stickers: Vec<OwnedEmote>,
    pub effects: Vec<OwnedEmote>,
}

/// One emoticon, sticker or effect you own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OwnedEmote {
    /// Without colons (`steamhappy`).
    pub name: String,
    /// The game it came from.
    pub app_id: AppId,
    /// When you last used it (unix seconds; 0 = never).
    pub last_used: u32,
    pub use_count: u32,
}

/// "Come play with me": which game, and how to join the inviter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInvite {
    pub app_id: AppId,
    /// Launch arguments that join the inviter (`+connect_lobby 123`,
    /// `+invite 456 …`); empty to just start the game.
    pub connect: String,
}

impl GameInvite {
    /// A link that starts the game with the join arguments, for invites in
    /// chat. (Steam strips `[gameinvite]` markup that doesn't come from its
    /// own client, so we can't send that; lobby invites go through the
    /// lobby itself and Steam writes their chat message.) Any Steam client
    /// opens it; ours shows it as an invite card.
    pub fn link(&self) -> String {
        if self.connect.is_empty() {
            return format!("steam://run/{}", self.app_id);
        }
        let mut args = String::new();
        for b in self.connect.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'+' | b':' => args.push(b as char),
                _ => args.push_str(&format!("%{b:02X}")),
            }
        }
        format!("steam://run/{}//{args}", self.app_id)
    }
}

/// Steam chat markup ("BBCode") to plain text plus a game invite, if any.
pub mod chat {
    use super::GameInvite;

    /// `name="value"` (or `name=value`) from inside a tag.
    fn attr(tag: &str, name: &str) -> Option<String> {
        let mut from = 0;
        while let Some(i) = tag[from..].find(name).map(|i| i + from) {
            from = i + name.len();
            let starts_word = tag[..i].chars().last().is_none_or(|c| c.is_whitespace());
            let Some(v) = tag[from..].strip_prefix('=') else { continue };
            if !starts_word {
                continue;
            }
            return Some(match v.strip_prefix('"') {
                Some(q) => q.split('"').next().unwrap_or("").to_string(),
                None => v.split(char::is_whitespace).next().unwrap_or("").to_string(),
            });
        }
        None
    }

    fn invite_from(tag: &str) -> Option<GameInvite> {
        let app_id = attr(tag, "appid")?.parse().ok()?;
        let mut connect = attr(tag, "connect").unwrap_or_default();
        if connect.is_empty() {
            if let Some(lobby) = attr(tag, "lobbyid").filter(|l| !l.is_empty() && l != "0") {
                connect = format!("+connect_lobby {lobby}");
            }
        }
        Some(GameInvite { app_id, connect })
    }

    /// Plain text for display, and the invite in it. Tags are dropped and
    /// their inside kept (emoticons become `:name:`).
    pub fn parse(raw: &str) -> (String, Option<GameInvite>) {
        let mut out = String::new();
        let mut invite = None;
        let mut rest = raw;
        while let Some(c) = rest.chars().next() {
            // Steam escapes a literal "[" as "\[".
            if let Some(after) = rest.strip_prefix("\\[").or_else(|| rest.strip_prefix("\\\\")) {
                out.push(rest.as_bytes()[1] as char);
                rest = after;
                continue;
            }
            if c == '[' {
                if let Some(end) = rest.find(']') {
                    let tag = &rest[1..end];
                    let closing = tag.starts_with('/');
                    let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '=').next().unwrap_or("");
                    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        match name {
                            "gameinvite" | "lobbyinvite" if !closing => invite = invite.or_else(|| invite_from(tag)),
                            "emoticon" => out.push(':'),
                            "sticker" if !closing => out.push_str(&format!("[{}]", attr(tag, "type").unwrap_or_else(|| "sticker".into()))),
                            _ => {}
                        }
                        rest = &rest[end + 1..];
                        continue;
                    }
                }
            }
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
        let text = out.trim().to_string();
        // SteamInRust's invites: "Play X with me! steam://run/123//+args".
        if invite.is_none() {
            invite = link_invite(&text);
        }
        (text, invite)
    }

    /// The invite in a `steam://run/<app>[//<args>]` link, if any.
    fn link_invite(text: &str) -> Option<GameInvite> {
        let at = text.find("steam://run/")?;
        let rest = &text[at + 12..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let app_id = digits.parse().ok()?;
        let args = rest[digits.len()..].strip_prefix("//").unwrap_or("");
        let args = args.split(char::is_whitespace).next().unwrap_or("");
        // Percent-decoded launch arguments.
        let mut bytes = Vec::new();
        let raw = args.as_bytes();
        let mut i = 0;
        while i < raw.len() {
            match (raw[i], raw.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())) {
                (b'%', Some(b)) => {
                    bytes.push(b);
                    i += 3;
                }
                (b, _) => {
                    bytes.push(b);
                    i += 1;
                }
            }
        }
        Some(GameInvite { app_id, connect: String::from_utf8_lossy(&bytes).into_owned() })
    }

    /// A piece of a chat message, as Steam's chat shows it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Part {
        Text(String),
        /// A Steam emoticon, by name (`steamhappy`).
        Emoticon(String),
        /// A sticker, by name.
        Sticker(String),
        /// A link: where it goes and what it shows.
        Link { url: String, text: String },
        /// `@name` for a member (by account id).
        Mention { account: u32, text: String },
        /// Hidden until clicked (`/spoiler`).
        Spoiler(Vec<Part>),
        /// One line of code (`/code`).
        Code(String),
        /// Preformatted text, as typed (`/pre`).
        Pre(String),
        /// A quote (`/quote`), with whom it quotes when known.
        Quote { author: String, parts: Vec<Part> },
        /// An action (`/me`).
        Me(Vec<Part>),
        /// A random number (`/random`); Steam picks the result.
        Random { min: i64, max: i64, result: Option<i64> },
        /// A coin toss (`/flip`): `Some(true)` heads.
        Flip(Option<bool>),
        /// A picture sent in chat.
        Image { src: String, width: u32, height: u32 },
        /// A chat effect (confetti and the like), by name.
        Effect(String),
    }

    /// Tags whose content is kept as typed.
    const VERBATIM: [&str; 2] = ["code", "pre"];

    /// `\[` back to `[`, `\\` back to `\`.
    fn unescape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' && matches!(chars.peek(), Some('[') | Some('\\')) {
                out.push(chars.next().unwrap_or('\\'));
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Where the matching `[/name]` starts in `s` (nested tags of the same
    /// name counted), and where it ends.
    fn closing(s: &str, name: &str) -> Option<(usize, usize)> {
        let (open, close) = (format!("[{name}"), format!("[/{name}]"));
        let lower = s.to_ascii_lowercase();
        let mut depth = 0;
        let mut i = 0;
        while i < lower.len() {
            if lower[i..].starts_with(&close) {
                if depth == 0 {
                    return Some((i, i + close.len()));
                }
                depth -= 1;
                i += close.len();
            } else if lower[i..].starts_with(&open) && lower[i + open.len()..].starts_with([']', ' ', '=']) {
                depth += 1;
                i += open.len();
            } else {
                i += lower[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            }
        }
        None
    }

    /// The value right after the tag name (`[url=…]`, `[quote=…]`).
    fn main_value(tag: &str, name: &str) -> Option<String> {
        let v = tag.get(name.len()..)?.strip_prefix('=')?;
        let v = v.strip_prefix('"').map(|q| q.split('"').next().unwrap_or("")).unwrap_or_else(|| v.split(char::is_whitespace).next().unwrap_or(""));
        Some(v.to_string())
    }

    /// A message's pieces, from Steam's markup: emoticons, stickers, links,
    /// mentions, spoilers, code, quotes, `/me`, `/random`, `/flip`, images
    /// and chat effects. Formatting the chat doesn't show (bold and the
    /// like) keeps its text; game invites are left to [`parse`].
    pub fn rich(raw: &str) -> Vec<Part> {
        let mut out = Vec::new();
        let mut text = String::new();
        let mut rest = raw;
        let flush = |text: &mut String, out: &mut Vec<Part>| {
            if !text.is_empty() {
                plain_parts(&std::mem::take(text), out);
            }
        };
        while let Some(c) = rest.chars().next() {
            if let Some(after) = rest.strip_prefix("\\[").or_else(|| rest.strip_prefix("\\\\")) {
                text.push(rest.as_bytes()[1] as char);
                rest = after;
                continue;
            }
            if c == '[' {
                if let Some(end) = rest.find(']') {
                    let tag = &rest[1..end];
                    let closing_tag = tag.starts_with('/');
                    let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '=').next().unwrap_or("").to_ascii_lowercase();
                    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        let after = &rest[end + 1..];
                        if closing_tag {
                            // A stray closing tag: drop it.
                            rest = after;
                            continue;
                        }
                        let (inner, next) = match closing(after, &name) {
                            Some((a, b)) => (&after[..a], &after[b..]),
                            None => ("", after),
                        };
                        flush(&mut text, &mut out);
                        part(&name, tag, inner, &mut out);
                        rest = next;
                        continue;
                    }
                }
            }
            text.push(c);
            rest = &rest[c.len_utf8()..];
        }
        flush(&mut text, &mut out);
        merge(out)
    }

    fn part(name: &str, tag: &str, inner: &str, out: &mut Vec<Part>) {
        let num = |k: &str| attr(tag, k).and_then(|v| v.trim().parse::<i64>().ok());
        let inner_text = || unescape(inner).trim().to_string();
        match name {
            "emoticon" => {
                let n = inner_text();
                let n = n.trim_matches(':').to_string();
                if !n.is_empty() {
                    out.push(Part::Emoticon(n));
                }
            }
            "sticker" => {
                if let Some(n) = attr(tag, "type").filter(|n| !n.is_empty()).or_else(|| Some(inner_text()).filter(|n| !n.is_empty())) {
                    out.push(Part::Sticker(n));
                }
            }
            "url" => {
                let url = main_value(tag, "url").filter(|u| !u.is_empty()).unwrap_or_else(inner_text);
                let shown = parse(inner).0;
                out.push(Part::Link { text: if shown.is_empty() { url.clone() } else { shown }, url });
            }
            "tradeofferlink" | "tradeoffer" | "og" => {
                let url = main_value(tag, name).or_else(|| attr(tag, "url")).filter(|u| u.starts_with("http")).unwrap_or_else(inner_text);
                if url.starts_with("http") {
                    out.push(Part::Link { text: url.clone(), url });
                } else if !url.is_empty() {
                    out.push(Part::Text(url));
                }
            }
            "mention" => {
                let account = main_value(tag, "mention").and_then(|v| v.parse().ok()).unwrap_or(0);
                out.push(Part::Mention { account, text: inner_text() });
            }
            "spoiler" => out.push(Part::Spoiler(rich(inner))),
            "code" => out.push(Part::Code(unescape(inner).trim_matches('\n').to_string())),
            "pre" => out.push(Part::Pre(unescape(inner).trim_matches('\n').to_string())),
            "quote" => {
                let author = main_value(tag, "quote").or_else(|| attr(tag, "author")).unwrap_or_default();
                out.push(Part::Quote { author, parts: rich(inner) });
            }
            "me" => out.push(Part::Me(rich(inner))),
            "random" => out.push(Part::Random { min: num("min").unwrap_or(1), max: num("max").unwrap_or(100), result: num("result").or_else(|| inner_text().parse().ok()) }),
            "flip" => {
                let r = attr(tag, "result").unwrap_or_else(inner_text).to_ascii_lowercase();
                out.push(Part::Flip(match r.as_str() {
                    "heads" | "1" | "true" => Some(true),
                    "tails" | "0" | "false" => Some(false),
                    _ => None,
                }));
            }
            "img" => {
                let src = attr(tag, "src").filter(|s| s.starts_with("http")).unwrap_or_else(inner_text);
                if src.starts_with("http") {
                    let n = |k: &str| attr(tag, k).and_then(|v| v.parse().ok()).unwrap_or(0);
                    out.push(Part::Image { src, width: n("width"), height: n("height") });
                }
            }
            "roomeffect" => {
                if let Some(n) = attr(tag, "type").or_else(|| Some(inner_text())).filter(|n| !n.is_empty()) {
                    out.push(Part::Effect(n));
                }
            }
            "gameinvite" | "lobbyinvite" => {}
            _ if VERBATIM.contains(&name) => {}
            // Formatting the chat doesn't draw: keep what's inside.
            _ => out.extend(rich(inner)),
        }
    }

    /// Plain text with its web links and `:emoticon:` names picked out.
    fn plain_parts(s: &str, out: &mut Vec<Part>) {
        let mut text = String::new();
        let mut rest = s;
        while !rest.is_empty() {
            let link = ["https://", "http://"].iter().filter_map(|p| rest.find(p)).min();
            let emote = emoticon_at(rest);
            let next = match (link, emote) {
                (Some(l), Some((e, _, _))) => l.min(e),
                (Some(l), None) => l,
                (None, Some((e, _, _))) => e,
                (None, None) => rest.len(),
            };
            text.push_str(&rest[..next]);
            rest = &rest[next..];
            if rest.is_empty() {
                break;
            }
            if let Some((0, len, name)) = emoticon_at(rest) {
                if !text.is_empty() {
                    out.push(Part::Text(std::mem::take(&mut text)));
                }
                out.push(Part::Emoticon(name));
                rest = &rest[len..];
                continue;
            }
            // A link runs to the next space; a closing bracket or full
            // stop right at its end isn't part of it.
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let mut url = &rest[..end];
            while url.ends_with(['.', ',', ')', '!', '?', ';', ':', '"', '\'']) {
                url = &url[..url.len() - 1];
            }
            if url.len() > 8 {
                if !text.is_empty() {
                    out.push(Part::Text(std::mem::take(&mut text)));
                }
                out.push(Part::Link { url: url.to_string(), text: url.to_string() });
                rest = &rest[url.len()..];
            } else {
                text.push_str(&rest[..1]);
                rest = &rest[1..];
            }
        }
        if !text.is_empty() {
            out.push(Part::Text(text));
        }
    }

    /// The first `:name:` in `s`: where, how long, and the name. Names are
    /// letters, digits and `_`, with at least one letter (so times like
    /// 12:30:45 stay text).
    fn emoticon_at(s: &str) -> Option<(usize, usize, String)> {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b':' {
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j < b.len() && b[j] == b':' && j - i > 2 && b[i + 1..j].iter().any(|c| c.is_ascii_alphabetic()) {
                    return Some((i, j + 1 - i, s[i + 1..j].to_string()));
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
        None
    }

    fn merge(parts: Vec<Part>) -> Vec<Part> {
        let mut out: Vec<Part> = Vec::with_capacity(parts.len());
        for p in parts {
            match (out.last_mut(), p) {
                (Some(Part::Text(a)), Part::Text(b)) => a.push_str(&b),
                (_, p) => out.push(p),
            }
        }
        out
    }

    /// Every link in a message (not inside code), for previews.
    pub fn links(parts: &[Part]) -> Vec<String> {
        let mut out = Vec::new();
        for p in parts {
            match p {
                Part::Link { url, .. } if !out.contains(url) => out.push(url.clone()),
                Part::Spoiler(inner) | Part::Me(inner) | Part::Quote { parts: inner, .. } => {
                    for u in links(inner) {
                        if !out.contains(&u) {
                            out.push(u);
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    const ECONOMY: &str = "https://community.akamai.steamstatic.com/economy";

    /// An emoticon's picture (small, or the large one).
    pub fn emoticon_url(name: &str, large: bool) -> String {
        format!("{ECONOMY}/{}/{}", if large { "emoticonlarge" } else { "emoticon" }, name.trim_matches(':'))
    }
    pub fn sticker_url(name: &str) -> String {
        format!("{ECONOMY}/sticker/{name}")
    }
    pub fn effect_url(name: &str) -> String {
        format!("{ECONOMY}/roomeffect/{name}")
    }

    /// Text to send as markup: `[` escaped so it shows as typed, and
    /// `:name:` made an emoticon when `owned(name)`. Steam turns slash
    /// commands (`/flip`, `/code …`) into their markup itself.
    pub fn compose(input: &str, owned: impl Fn(&str) -> bool) -> String {
        let input = input.trim();
        let mut out = String::with_capacity(input.len() + 8);
        let mut rest = input;
        while !rest.is_empty() {
            match emoticon_at(rest) {
                Some((at, len, name)) if owned(&name) => {
                    out.push_str(&escape(&rest[..at]));
                    out.push_str(&format!("[emoticon]{name}[/emoticon]"));
                    rest = &rest[at + len..];
                }
                Some((at, len, _)) => {
                    // Not one of yours: keep it as text, but look again
                    // from its closing colon (it may open the next one).
                    let keep = at + len - 1;
                    out.push_str(&escape(&rest[..keep]));
                    rest = &rest[keep..];
                }
                None => {
                    out.push_str(&escape(rest));
                    break;
                }
            }
        }
        out
    }

    /// `[` → `\[`, so Steam shows it instead of reading a tag.
    pub fn escape(s: &str) -> String {
        s.replace('[', "\\[")
    }

    /// A sticker message.
    pub fn sticker(name: &str) -> String {
        format!("[sticker type=\"{name}\" limit=\"0\"][/sticker]")
    }

    /// A chat effect message.
    pub fn effect(name: &str) -> String {
        format!("[roomeffect type=\"{name}\"][/roomeffect]")
    }

    /// A picture message (after it's uploaded).
    pub fn image(src: &str, width: u32, height: u32) -> String {
        format!("[img src=\"{src}\" width=\"{width}\" height=\"{height}\"][/img]")
    }

    /// A quote of someone's message, to put in the message box.
    pub fn quote(author: &str, text: &str) -> String {
        format!("[quote={}]{}[/quote]\n", author.replace([']', '['], ""), escape(text))
    }

    /// A `[quote…]…[/quote]` at the start of what's typed (from "Quote"),
    /// and the rest. The quote is markup; the rest is text to escape.
    pub fn split_quote(input: &str) -> (&str, &str) {
        if !input.starts_with("[quote") {
            return ("", input);
        }
        let Some(open) = input.find(']') else { return ("", input) };
        match closing(&input[open + 1..], "quote") {
            Some((_, end)) => input.split_at(open + 1 + end),
            None => ("", input),
        }
    }

    /// The slash commands the chat offers, Steam's first; `store` is
    /// SteamInRust's own (it sends a Store link).
    pub const COMMANDS: [&str; 8] = ["me", "code", "pre", "quote", "spoiler", "random", "flip", "store"];

    /// While typing a command name (`/co`): what's typed after the slash.
    pub fn typing_command(input: &str) -> Option<&str> {
        let rest = input.strip_prefix('/')?;
        (!rest.contains(char::is_whitespace)).then_some(rest)
    }

    /// `/store <game>`: the game's name.
    pub fn store_command(input: &str) -> Option<&str> {
        let rest = input.trim().strip_prefix("/store")?;
        (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rich_messages() {
            use Part::*;
            assert_eq!(rich("hi [emoticon]steamhappy[/emoticon]!"), vec![Text("hi ".into()), Emoticon("steamhappy".into()), Text("!".into())]);
            assert_eq!(rich("gg :steamthumbsup: at 12:30:45"), vec![Text("gg ".into()), Emoticon("steamthumbsup".into()), Text(" at 12:30:45".into())]);
            assert_eq!(rich("[spoiler]he dies[/spoiler]"), vec![Spoiler(vec![Text("he dies".into())])]);
            assert_eq!(rich("[code]let a = x\\[0];[/code]"), vec![Code("let a = x[0];".into())]);
            assert_eq!(rich("[pre]a\n  b[/pre]"), vec![Pre("a\n  b".into())]);
            assert_eq!(rich("[quote=Ferris]see [b]this[/b][/quote]ok"), vec![Quote { author: "Ferris".into(), parts: vec![Text("see this".into())] }, Text("ok".into())]);
            assert_eq!(rich(r#"[random min="1" max="100" result="39"][/random]"#), vec![Random { min: 1, max: 100, result: Some(39) }]);
            assert_eq!(rich(r#"[flip result="tails"][/flip]"#), vec![Flip(Some(false))]);
            assert_eq!(rich(r#"[sticker type="Sticker_Hello" limit="0"][/sticker]"#), vec![Sticker("Sticker_Hello".into())]);
            assert_eq!(rich(r#"[img src="https://images.example/a.png" width="640" height="360"][/img]"#), vec![Image { src: "https://images.example/a.png".into(), width: 640, height: 360 }]);
            assert_eq!(rich("[mention=22202]@Ferris[/mention] hi"), vec![Mention { account: 22202, text: "@Ferris".into() }, Text(" hi".into())]);
            assert_eq!(rich(r#"[roomeffect type="confetti"][/roomeffect]"#), vec![Effect("confetti".into())]);
            assert_eq!(rich("[me]waves[/me]"), vec![Me(vec![Text("waves".into())])]);
            // Escaped brackets and stray tags stay text.
            assert_eq!(rich(r"a \[b] c [/x] d"), vec![Text("a [b] c  d".into())]);
            assert_eq!(rich("1 [ 2"), vec![Text("1 [ 2".into())]);
        }

        #[test]
        fn message_links() {
            use Part::*;
            let parts = rich("look https://store.steampowered.com/app/620/Portal_2/. and [url=https://x.y/z]this[/url]");
            assert_eq!(parts[1], Link { url: "https://store.steampowered.com/app/620/Portal_2/".into(), text: "https://store.steampowered.com/app/620/Portal_2/".into() });
            assert_eq!(super::links(&parts), vec!["https://store.steampowered.com/app/620/Portal_2/".to_string(), "https://x.y/z".to_string()]);
            // No previews for links inside code.
            assert!(super::links(&rich("[code]https://a.b/c[/code]")).is_empty());
        }

        #[test]
        fn composing() {
            let owned = |n: &str| n == "steamhappy";
            assert_eq!(compose("hi :steamhappy: [b]x[/b]", owned), r"hi [emoticon]steamhappy[/emoticon] \[b]x\[/b]");
            assert_eq!(compose("not mine :steamsad: at 1:2:3", owned), "not mine :steamsad: at 1:2:3");
            assert_eq!(compose("::steamhappy::", owned), ":[emoticon]steamhappy[/emoticon]:");
            assert_eq!(rich(&compose("a [x] :steamhappy:", owned)), vec![Part::Text("a [x] ".into()), Part::Emoticon("steamhappy".into())]);
            assert_eq!(typing_command("/co"), Some("co"));
            assert_eq!(typing_command("/code x"), None);
            assert_eq!(store_command("/store portal 2"), Some("portal 2"));
            assert_eq!(store_command("/storefront"), None);
            assert_eq!(rich(&quote("Ferris", "a [b]")), vec![Part::Quote { author: "Ferris".into(), parts: vec![Part::Text("a [b]".into())] }, Part::Text("\n".into())]);
            // A quote stays a quote when sent; what's typed after it is escaped.
            let typed = format!("{}my [reply]", quote("Ferris", "a [b]"));
            let (q, rest) = split_quote(&typed);
            assert_eq!(rich(&format!("{q}{}", compose(rest, owned))).first(), Some(&Part::Quote { author: "Ferris".into(), parts: vec![Part::Text("a [b]".into())] }));
            assert_eq!(rest, "\nmy [reply]");
            assert_eq!(split_quote("no quote"), ("", "no quote"));
            assert_eq!(split_quote("[quote=x]never closed"), ("", "[quote=x]never closed"));
        }

        #[test]
        fn reactions_count() {
            let mut r = Vec::new();
            super::super::Reaction::apply(&mut r, "steamhappy", false, true, false);
            super::super::Reaction::apply(&mut r, "steamhappy", false, true, true);
            assert_eq!((r[0].count, r[0].mine), (2, true));
            // Adding yours twice counts once.
            super::super::Reaction::apply(&mut r, "steamhappy", false, true, true);
            assert_eq!(r[0].count, 2);
            super::super::Reaction::apply(&mut r, "steamhappy", false, false, true);
            assert_eq!((r[0].count, r[0].mine), (1, false));
            super::super::Reaction::apply(&mut r, "steamhappy", false, false, false);
            assert!(r.is_empty());
        }

        #[test]
        fn steam_invites() {
            let (text, inv) = parse(r#"[gameinvite  appid="570" connect="+invite 30095787509687845 -launchsource2"][/gameinvite]"#);
            assert_eq!(text, "");
            assert_eq!(inv, Some(GameInvite { app_id: 570, connect: "+invite 30095787509687845 -launchsource2".into() }));
            let (_, inv) = parse(r#"[lobbyinvite appid="477160" lobbyid="109775241000000001"][/lobbyinvite]"#);
            assert_eq!(inv.unwrap().connect, "+connect_lobby 109775241000000001");
        }

        #[test]
        fn links() {
            let me = GameInvite { app_id: 10, connect: "+connect_lobby 1234 -x \"a b\"".into() };
            assert_eq!(me.link(), "steam://run/10//+connect_lobby%201234%20-x%20%22a%20b%22");
            assert_eq!(parse(&format!("Join me! {}", me.link())).1, Some(me));
            let plain = GameInvite { app_id: 477160, connect: String::new() };
            assert_eq!(plain.link(), "steam://run/477160");
            assert_eq!(parse("Play it with me! steam://run/477160 now").1, Some(plain));
        }

        #[test]
        fn plain_text() {
            assert_eq!(parse("hi [emoticon]steamhappy[/emoticon] there").0, "hi :steamhappy: there");
            assert_eq!(parse(r"a \[b] c").0, "a [b] c");
            assert_eq!(parse("[url=https://x.y]link[/url]").0, "link");
            assert_eq!(parse("1 [ 2").0, "1 [ 2");
            assert_eq!(parse("no tags").1, None);
            let (text, inv) = parse("Сыграем в Human Fall Flat? steam://run/477160");
            assert_eq!(text, "Сыграем в Human Fall Flat? steam://run/477160");
            assert_eq!(inv, Some(GameInvite { app_id: 477160, connect: String::new() }));
        }
    }
}

/// Image URLs on Valve's public CDN.
pub mod art {
    use super::AppId;
    const CDN: &str = "https://cdn.akamai.steamstatic.com/steam/apps";
    const DEFAULT_AVATAR: &str = "fef49e7fa7e1997310d705b2a6158ff8dc1cdfeb";

    /// Art paths from app info, for apps whose art moved to Steam's newer
    /// store asset folders (`<hash>/header.jpg`, `<hash>/library_600x900.jpg`…).
    /// Newer games and DLC only have it there; the old fixed address is a 404.
    static PATHS: std::sync::RwLock<Option<std::collections::HashMap<(AppId, String), String>>> = std::sync::RwLock::new(None);
    const ASSETS: &str = "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps";

    /// Remember an art path from an app's info, like "<hash>/header.jpg".
    pub fn remember(id: AppId, path: &str) {
        let Some((_, file)) = path.rsplit_once('/') else { return };
        if let Ok(mut map) = PATHS.write() {
            map.get_or_insert_with(Default::default).insert((id, file.to_string()), path.to_string());
        }
    }

    /// The newer address when app info gave one, else the classic one.
    fn url(id: AppId, file: &str) -> String {
        let known = PATHS.read().ok().and_then(|m| m.as_ref()?.get(&(id, file.to_string())).cloned());
        match known {
            Some(path) => format!("{ASSETS}/{id}/{path}"),
            None => format!("{CDN}/{id}/{file}"),
        }
    }

    pub fn header(id: AppId) -> String {
        url(id, "header.jpg")
    }
    pub fn capsule(id: AppId) -> String {
        url(id, "library_600x900.jpg")
    }
    pub fn hero(id: AppId) -> String {
        url(id, "library_hero.jpg")
    }
    pub fn logo(id: AppId) -> String {
        url(id, "logo.png")
    }
    pub fn icon(id: AppId, hash: &str) -> String {
        format!("https://shared.akamai.steamstatic.com/community_assets/images/apps/{id}/{hash}.jpg")
    }
    pub fn avatar_full(hash: Option<&str>) -> String {
        avatar(hash).replace("_medium.jpg", "_full.jpg")
    }
    /// Community item images (profile backgrounds, frames).
    pub fn community_item(path: &str) -> String {
        format!("https://cdn.akamai.steamstatic.com/steamcommunity/public/images/{path}")
    }
    /// Inventory item image, at most `size` × `size` with its own shape.
    /// (The "…fx…f" form pads it into a transparent square, which made
    /// items look small; this one doesn't.)
    pub fn econ(icon: &str, size: u32) -> String {
        format!("https://community.akamai.steamstatic.com/economy/image/{icon}/{size}x{size}")
    }
    pub fn avatar(hash: Option<&str>) -> String {
        let h = hash
            .filter(|h| !h.is_empty() && !h.chars().all(|c| c == '0'))
            .unwrap_or(DEFAULT_AVATAR);
        format!("https://avatars.steamstatic.com/{h}_medium.jpg")
    }
}

/// This computer's system, as Steam names it in `oslist` ("windows",
/// "macos", "linux").
pub const THIS_OS: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};

/// Does a Steam `oslist` ("windows,macos"; empty = every system) include `os`?
pub fn os_matches(os_list: &str, os: &str) -> bool {
    os_list.trim().is_empty() || os_list.split(',').any(|o| o.trim().eq_ignore_ascii_case(os))
}

#[cfg(test)]
mod os_tests {
    use super::*;

    #[test]
    fn system_lists() {
        assert!(os_matches("", "macos"));
        assert!(os_matches("windows, macos", "macos"));
        assert!(!os_matches("windows", "macos"));
        let launch = |os: &str| LaunchOption { executable: "game".into(), arguments: String::new(), working_dir: String::new(), description: String::new(), os_list: os.into() };
        let app = |os: &str, l: Vec<LaunchOption>| OwnedApp { os_list: os.into(), launch: l, ..Default::default() };
        // The app's own list wins.
        assert_eq!(app("windows,macos", vec![]).runs_on_this_os(), os_matches("windows,macos", THIS_OS));
        // Else its launch options; nothing known counts as yes.
        assert_eq!(app("", vec![launch("windows")]).runs_on_this_os(), THIS_OS == "windows");
        assert!(app("", vec![]).runs_on_this_os());
    }
}

#[cfg(test)]
mod steam_id_tests {
    #[test]
    fn writes_steam_ids_every_way() {
        let f = super::steam_id_formats(76561197960287930);
        assert_eq!(f[1].1, "[U:1:22202]");
        assert_eq!(f[2].1, "STEAM_0:0:11101");
        assert_eq!(f[3].1, "22202");
    }

    #[test]
    fn persona_join_args_hardens_server_and_connect() {
        use super::Persona;
        let ok_srv = Persona { server: Some("192.168.1.5:27015".into()), ..Default::default() };
        assert_eq!(ok_srv.join_args().as_deref(), Some("+connect 192.168.1.5:27015"));
        let ok_v6 = Persona { server: Some("[2001:db8::1]:27015".into()), ..Default::default() };
        assert_eq!(ok_v6.join_args().as_deref(), Some("+connect [2001:db8::1]:27015"));

        for bad in [
            "1.2.3.4:27015 --gpu-launcher=/bin/sh",
            "-mono-assembly-path=/tmp/x",
            "+exec evil.cfg",
            "LD_PRELOAD=/tmp/x.so %command%",
            "1.2.3.4:27015\n+quit",
            "",
        ] {
            let p = Persona { server: Some(bad.into()), ..Default::default() };
            assert_eq!(p.join_args(), None, "unsafe server must be rejected: {bad:?}");
        }

        for bad_connect in [
            "LD_PRELOAD=/tmp/x.so /bin/sh %command%",
            "%command%",
            "--gpu-launcher=/bin/sh",
            "+exec autoexec.cfg",
        ] {
            let p = Persona { connect: Some(bad_connect.into()), ..Default::default() };
            assert_eq!(p.join_args(), None, "unsafe connect must be rejected: {bad_connect:?}");
        }

        let fallback = Persona {
            connect: Some("%command%".into()),
            server: Some("-bad".into()),
            lobby: Some(42),
            ..Default::default()
        };
        assert_eq!(fallback.join_args().as_deref(), Some("+connect_lobby 42"));
    }
}

/// Managing group chats (roles, invites, bans, notifications).
pub mod chat_admin;
pub mod economy;

/// Voice chat: calls, who's in them, and audio settings.
pub mod voice;

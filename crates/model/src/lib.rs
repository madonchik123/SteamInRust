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
    /// Steam Deck: 0 unknown, 1 unsupported, 2 playable, 3 verified.
    #[serde(default)]
    pub deck: u8,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DownloadPhase {
    #[default]
    Queued,
    Preparing,
    Checking,
    Downloading,
    Finishing,
    Done,
    Failed,
}

impl OwnedApp {
    pub fn is_dlc(&self) -> bool {
        self.kind == "dlc"
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
        let os = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        self.os_list.is_empty() || self.os_list.split(',').any(|o| o.trim() == os)
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
}

impl Persona {
    pub fn in_game(&self) -> bool {
        self.game_app_id.is_some() || self.game_name.is_some()
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
    pub url: String,
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

/// A library collection from the official Steam client (synced by Steam).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SteamCollection {
    /// "favorite", "hidden", or Steam's id for one you made ("uc-…").
    pub id: String,
    pub name: String,
    pub games: Vec<AppId>,
    /// Built from filters in Steam (its games aren't listed).
    pub dynamic: bool,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// The other person in the conversation.
    pub friend: SteamId,
    /// Who wrote it (the friend, or you).
    pub from: SteamId,
    pub text: String,
    /// Unix seconds.
    pub time: u64,
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
        format!("https://cdn.akamai.steamstatic.com/steamcommunity/public/images/apps/{id}/{hash}.jpg")
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

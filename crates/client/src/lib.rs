//! The Steam client "engine". It runs on a tokio runtime and talks to the rest
//! of the app through two plain message types:
//!
//! - [`Command`]: what the UI (or an extension) asks for.
//! - [`Event`]: what happened. Every event goes to one [`Sink`] callback; the
//!   desktop app fans it out to the UI state and to all extensions.
//!
//! [`State`] is a ready-made reducer that folds events into a snapshot the UI
//! can render, so extensions see exactly the same data as built-in pages.

pub mod download;
pub mod workshop;
pub mod hub;
pub mod ingame;
pub mod ipc;
mod launch;
pub mod market;
pub mod social;
pub mod cloud;
pub mod regions;
pub mod a2s;
pub mod account;
mod notify;
pub mod offline;
pub mod support;
mod pics;
mod session;
pub mod state;
pub mod store;

pub use auth::Guard;
pub use launch::{LaunchMode, mode as launch_mode, mode_cached as launch_mode_cached};
pub use state::State;
pub use store::SavedAccount;

use model::{AppId, ChatMessage, OwnedApp, Persona, PersonaState, ProfileInfo, Relationship, SteamId};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum Command {
    /// Log in with account name and password.
    LoginPassword { account: String, password: String },
    /// Log in by scanning a QR code with the Steam Mobile app.
    LoginQr,
    /// Log in with a refresh token saved from an earlier login.
    LoginSaved { account: String },
    /// Start a saved account in offline mode, from what was cached at its
    /// last sign-in (no network needed).
    StartOffline { account: String },
    /// Leave offline mode / reconnect now.
    GoOnline,
    /// Disconnect from Steam and keep going in offline mode.
    GoOffline,
    /// Answer a Steam Guard prompt.
    SubmitCode(String),
    /// Give up on the login in progress.
    CancelLogin,
    /// Remove a saved login from this machine.
    ForgetAccount { account: String },
    SetPersonaState(PersonaState),
    /// Start an installed game from its folder.
    Launch { app_id: AppId, install_dir: PathBuf, args: String },
    /// Start a game without the Steam Cloud check (after a conflict, "play anyway").
    LaunchWithoutCloud { app_id: AppId, install_dir: PathBuf, args: String },
    /// Where Steam keeps `userdata`, and games with Steam Cloud turned off.
    CloudSettings { steam_root: Option<PathBuf>, disabled: Vec<AppId> },
    /// Sync a game's saves now. Conflicts listed in `keep_local` are uploaded,
    /// in `keep_cloud` downloaded. `then_launch` starts the game afterwards.
    CloudSync { app_id: AppId, install_dir: PathBuf, mode: cloud::Mode, keep_local: Vec<String>, keep_cloud: Vec<String>, then_launch: Option<String> },
    /// Close a running game (and the processes it started).
    StopGame { app_id: AppId },
    SendMessage { to: SteamId, text: String },
    /// Invite a friend into the running game's lobby: lets them in (private
    /// lobbies) and sends a game invite that joins it.
    InviteToLobby { app_id: AppId, lobby: u64, friend: SteamId },
    /// Send a friend request, or accept one you received.
    AddFriend { steam_id: SteamId },
    /// Remove a friend, or ignore a friend request.
    RemoveFriend { steam_id: SteamId },
    /// Set (or clear, with `None`) your nickname for someone.
    SetNickname { steam_id: SteamId, nickname: Option<String> },
    /// Block (or unblock) all communication with someone.
    BlockFriend { steam_id: SteamId, block: bool },
    /// Ask a game server about itself ("ip:port").
    QueryServer { addr: String },
    /// Tell a friend you're typing.
    Typing { to: SteamId },
    /// Fetch recent chat history with a friend.
    LoadHistory { friend: SteamId },
    /// Fetch someone's profile background, frame and bio.
    LoadProfile { steam_id: SteamId },
    /// Fetch one game's inventory for an account (753/6 is the Steam
    /// inventory: cards, backgrounds, emoticons). `language` is Steam's name.
    LoadInventory { steam_id: SteamId, app_id: AppId, context_id: u64, language: String },
    /// Find every game that has items in an account's inventory.
    LoadInventoryApps { steam_id: SteamId },
    /// Community Market price of an item in your wallet currency. `full` also
    /// reads the order book (highest buy order, cheapest listing in cents).
    MarketPrice { app_id: AppId, hash_name: String, currency: i32, country: String, full: bool },
    /// List an item on the Community Market. `receive` is what you get, in cents.
    /// Steam then asks for confirmation in the Steam Mobile app.
    SellItem { app_id: AppId, context_id: u64, asset_id: u64, amount: u64, receive: u64 },
    LoadMyListings,
    /// Your library collections from Steam (Favorites, Hidden, your own).
    LoadSteamCollections,
    /// Comments on a profile (newest first), from `start`.
    LoadComments { steam_id: SteamId, start: u32 },
    /// Leave a comment on a profile.
    PostComment { steam_id: SteamId, text: String },
    /// Delete a comment (on your profile, or one you wrote).
    DeleteComment { steam_id: SteamId, comment_id: u64 },
    LoadBadges { steam_id: SteamId },
    LoadNotifications,
    /// Mark notifications as read (empty = all).
    MarkNotificationsRead { ids: Vec<u64> },
    /// Your Steam Support requests.
    LoadSupportTickets,
    /// The messages in one support request.
    LoadSupportTicket { id: String, url: String },
    /// Steam Guard, authenticator, phone, remembered devices (read only).
    LoadAccountSecurity,
    /// Purchase history: the first page, or the next after `cursor`.
    LoadPurchases { language: String, cursor: Option<String> },
    /// Licenses and product key activations.
    LoadLicenses { language: String },
    /// Your Steam Family and its shared library.
    LoadFamily { language: String },
    CancelListing { listing_id: u64 },
    CancelBuyOrder { order_id: u64 },
    /// Fetch the games on someone's profile with their playtime.
    LoadPlayerGames { steam_id: SteamId, language: String },
    /// Your achievement progress (unlocked / total) for these games.
    LoadAchievementProgress { app_ids: Vec<AppId> },
    /// A game's achievements with which ones you've unlocked.
    LoadAchievements { app_id: AppId, language: String },
    /// Redeem a product key (CD key).
    ActivateKey { key: String },
    /// Install or update a game (queued; one download runs at a time).
    Install(download::InstallRequest),
    PauseDownload { app_id: AppId },
    ResumeDownload { app_id: AppId },
    CancelDownload { app_id: AppId },
    /// Measure every download region from this PC.
    ProbeRegions,
    /// Download from this Steam region (`None` = automatic), or prefer one
    /// content host (a partner CDN such as Akamai).
    DownloadRegion { cell_id: Option<u32>, host: Option<String> },
    /// Speed limit (KB/s, 0 = none) and parallel chunk downloads.
    DownloadLimits { limit_kbps: u64, concurrency: u32 },
    /// Unsubscribe from a workshop item (Steam removes it from the game).
    UnsubscribeWorkshop { app_id: AppId, file_id: u64 },
    /// Browse a game's Workshop. `query_type` is Steam's
    /// EPublishedFileQueryType (0 top rated, 1 newest, 3 trending, 9 most subscribed, 12 text search).
    WorkshopQuery { request_id: u64, app_id: AppId, query_type: u32, page: u32, search: String, days: u32, tags: Vec<String> },
    WorkshopSubscribe { app_id: AppId, file_id: u64 },
    /// Download every subscribed Workshop item of an installed game that
    /// is missing or outdated (also done before each launch, like Steam).
    SyncWorkshop { app_id: AppId },
    /// The game's own Workshop tag groups (for filters).
    LoadWorkshopTags { app_id: AppId, language: String },
    LoadWishlist,
    WishlistAdd { app_id: AppId },
    WishlistRemove { app_id: AppId },
    LoadCart,
    /// Add a package to the cart, optionally as a gift for a friend.
    CartAdd { package_id: u32, gift_to: Option<SteamId> },
    CartRemove { line_item_id: u64 },
    /// Community pages: Workshop items, guides, discussions, hub, achievements.
    Hub(hub::Request),
    LogOut,
}

#[derive(Debug, Clone)]
pub enum Event {
    Connecting,
    /// Show this URL as a QR code (it may be replaced by a newer one).
    QrChallenge { url: String },
    /// Steam Guard needs the user. `guards` lists allowed options, preferred first.
    GuardRequired { guards: Vec<Guard> },
    GuardCodeRejected(String),
    LoginFailed(String),
    /// A saved login couldn't reach Steam (no network, servers down). The
    /// login itself is fine; offline mode is possible if `offline_ready`.
    ConnectionError { account: String, message: String, offline_ready: bool },
    /// The connection to Steam changed (online, offline mode, reconnecting).
    Link(Link),
    /// Play sessions recorded while not connected: (game, seconds in all,
    /// seconds not on Steam yet). Replaces the earlier list.
    OfflinePlaytime(Vec<(AppId, u64, u64)>),
    /// Games whose Steam Cloud upload waits for a connection (full list).
    CloudPending(Vec<AppId>),
    /// A game started offline without its Steam Cloud download.
    CloudSkipped { app_id: AppId },
    LoggedIn { steam_id: SteamId, account: String },
    AccountInfo { persona_name: String, country: String },
    /// Number of licenses; owned apps follow in [`Event::Apps`].
    Licenses { count: usize },
    Apps(Vec<OwnedApp>),
    /// Full friends list (replaces any earlier one).
    Friends(Vec<(SteamId, Relationship)>),
    /// Personas whose details changed (friends and yourself).
    Personas(Vec<Persona>),
    /// Names of games outside your library (e.g. what friends are playing).
    AppNames(Vec<(AppId, String)>),
    GameStarted { app_id: AppId },
    /// How a game is being started: with SteamInRust's steamclient, through
    /// the official Steam client (anti-cheat), or without Steam features.
    LaunchMode { app_id: AppId, mode: LaunchMode },
    /// Steam Cloud is syncing a game's saves (before launch or after exit).
    CloudSyncing { app_id: AppId },
    /// Result of a Steam Cloud sync. `held_launch` is set when the game
    /// wasn't started because of conflicts (its launch arguments).
    CloudReport { report: model::CloudReport, held_launch: Option<String> },
    GameStopped { app_id: AppId },
    /// The running game entered (`Some`) or left a matchmaking lobby, so
    /// friends can be invited to it.
    GameLobby { app_id: AppId, lobby: Option<u64> },
    /// The running game's join string (rich presence `connect`), so friends
    /// can be invited to the game itself.
    GameConnect { app_id: AppId, connect: Option<String> },
    /// A chat message, incoming or sent by you (`from == your SteamID`).
    Message(ChatMessage),
    Typing { from: SteamId },
    History { friend: SteamId, messages: Vec<ChatMessage> },
    /// Your Steam level.
    Level(u32),
    /// Steam Wallet balance in cents, with Steam's currency code.
    /// `pending` is money Steam hasn't released yet (recent Market sales, refunds).
    Wallet { balance: i64, currency: i32, pending: i64 },
    Profile(ProfileInfo),
    /// Your account's email address (shown in Settings → Account).
    Email { address: String, validated: bool },
    Inventory(model::Inventory),
    MarketPrice { app_id: AppId, hash_name: String, result: Result<model::MarketPrice, String> },
    /// Result of [`Command::SellItem`]: `Ok(true)` = needs confirming on your phone.
    ItemListed { asset_id: u64, result: Result<bool, String> },
    MyListings(Result<model::MyListings, String>),
    SteamCollections(Result<Vec<model::SteamCollection>, String>),
    /// Comments on a profile, or why they can't be shown (private, turned off).
    Comments { steam_id: SteamId, result: Result<model::Comments, String> },
    CommentPosted { steam_id: SteamId, result: Result<(), String> },
    ServerInfo { addr: String, result: Result<crate::a2s::ServerInfo, String> },
    Blocked { steam_id: SteamId, result: Result<bool, String> },
    Badges { steam_id: SteamId, result: Result<model::Badges, String> },
    /// The full notification list (replaces the old one).
    Notifications(model::Notifications),
    /// New notifications just arrived (for a toast).
    NotificationsArrived(Vec<model::Notification>),
    SupportTickets(Result<Vec<model::SupportTicket>, String>),
    SupportTicket { id: String, result: Result<Vec<model::SupportMessage>, String> },
    AccountSecurity(Result<model::AccountSecurity, String>),
    /// A page of purchase history; `more` is true when it continues the list.
    Purchases { more: bool, result: Result<model::PurchasePage, String> },
    LicenseList(Result<Vec<model::License>, String>),
    Family(Result<model::FamilyStatus, String>),
    /// Community Market fee rules for your wallet currency.
    FeeRules(model::fees::Rules),
    ListingCancelled { listing_id: u64, result: Result<(), String> },
    BuyOrderCancelled { order_id: u64, result: Result<(), String> },
    /// Games with inventories for an account, or why it's unknown ("private").
    InventoryApps { steam_id: SteamId, result: Result<Vec<model::InventoryApp>, String> },
    /// Games on a profile, or why they can't be shown ("private").
    PlayerGames { steam_id: SteamId, result: Result<Vec<model::PlayerGame>, String> },
    /// Your achievement progress: (game, unlocked, total).
    AchievementProgress(Vec<(AppId, u32, u32)>),
    Achievements { app_id: AppId, result: Result<Vec<model::Achievement>, String> },
    /// Result of [`Command::ActivateKey`]: `Ok(items)` or `Err(reason)`.
    KeyActivated(Result<Vec<String>, String>),
    Download(model::DownloadStatus),
    /// Download regions, fastest first.
    Regions(Vec<model::DownloadRegion>),
    DownloadQueue(Vec<AppId>),
    DownloadFinished { app_id: AppId, result: Result<String, String> },
    WorkshopUnsubscribed { file_id: u64, result: Result<(), String> },
    WorkshopResults { request_id: u64, total: u64, items: Vec<model::WorkshopEntry> },
    WorkshopSubscribed { file_id: u64, result: Result<(), String> },
    /// Workshop items were downloaded (or failed) for a game.
    WorkshopSync { app_id: AppId, result: Result<crate::workshop::Report, String> },
    WorkshopTags { app_id: AppId, result: Result<Vec<model::WorkshopTagCategory>, String> },
    /// (app id, date added)
    Wishlist(Vec<(AppId, u64)>),
    Cart(Vec<model::CartLine>),
    /// A store action failed (cart, wishlist).
    StoreError(String),
    /// A message the client doesn't handle yet; useful for extension authors.
    Unhandled { emsg: u32, name: String },
    LoggedOut { reason: String },
    Error(String),
    /// Answer to a [`Command::Hub`] request.
    Hub(hub::Response),
}

/// How the client is connected to Steam right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Link {
    #[default]
    Online,
    /// Offline mode: everything from the local cache, no connection attempts.
    Offline,
    /// The connection dropped; trying again (`attempt` from 1, next try in
    /// `retry_in` seconds, 0 = trying now).
    Reconnecting { attempt: u32, retry_in: u32 },
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// Handle used to send commands to the running engine. Cheap to clone.
#[derive(Clone)]
pub struct Client {
    tx: mpsc::UnboundedSender<Command>,
}

impl Client {
    /// Start the engine on `rt`. `sink` is called for every event, from a
    /// background thread.
    pub fn spawn(rt: &tokio::runtime::Handle, sink: Sink) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        rt.spawn(session::run(rx, sink));
        Self { tx }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Net(#[from] net::Error),
    #[error(transparent)]
    Auth(#[from] auth::Error),
    #[error("{0}")]
    Other(String),
}

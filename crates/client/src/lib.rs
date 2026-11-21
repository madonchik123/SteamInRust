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
pub mod feed;
pub mod ingame;
pub mod ipc;
#[cfg(target_os = "macos")]
mod ipctool;
mod launch;
pub mod market;
pub mod trade;
pub mod groupchat;
pub mod chat;
pub mod profile_edit;
pub mod social;
pub mod cloud;
pub mod collections;
pub mod friends_add;
pub mod discovery;
pub mod wishlist;
pub mod installscript;
pub mod reviews;
pub mod regions;
pub mod a2s;
pub mod game_ugc;
pub mod game_inventory;
pub mod game_servers;
mod input_config;
pub mod account;
mod notify;
pub mod offline;
pub mod support;
mod pics;
mod session;
pub mod state;
pub mod store;
pub mod chat_admin;
pub mod voice;

pub use auth::Guard;
pub use launch::{LaunchArgs, LaunchMode, mode as launch_mode, mode_cached as launch_mode_cached, mode_peek as launch_mode_peek, safe_connect_args};
pub use state::State;
pub use store::SavedAccount;

use model::{AppId, ChatMessage, OwnedApp, Persona, PersonaState, ProfileInfo, Relationship, SteamId};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone)]
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
    Launch { app_id: AppId, install_dir: PathBuf, args: LaunchArgs },
    /// Start a game without the Steam Cloud check (after a conflict, "play anyway").
    LaunchWithoutCloud { app_id: AppId, install_dir: PathBuf, args: LaunchArgs },
    /// Where Steam keeps `userdata`, and games with Steam Cloud turned off.
    CloudSettings { steam_root: Option<PathBuf>, disabled: Vec<AppId> },
    /// Sync a game's saves now. Conflicts listed in `keep_local` are uploaded,
    /// in `keep_cloud` downloaded. `then_launch` starts the game afterwards.
    CloudSync { app_id: AppId, install_dir: PathBuf, mode: cloud::Mode, keep_local: Vec<String>, keep_cloud: Vec<String>, then_launch: Option<LaunchArgs> },
    /// Close a running game (and the processes it started).
    StopGame { app_id: AppId },
    /// For developers: let a game started by hand (a debugger, a test probe)
    /// use this session's Steam features. Answered with
    /// [`Event::GameAttached`]; nothing is reported to Steam as played.
    AttachGame { app_id: AppId, install_dir: PathBuf },
    /// The player accepted an invite (or "Join game") for a game that's
    /// running: hand it to the game like Steam does (GameLobbyJoinRequested_t
    /// for "+connect_lobby <id>", else GameRichPresenceJoinRequested_t).
    /// Start a game that isn't running with `connect` as its arguments instead.
    JoinRunningGame { app_id: AppId, friend: SteamId, connect: String },
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
    /// Many of one game's prices at once from the market search (fast);
    /// answers with [`Event::MarketPrice`] for each, then
    /// [`Event::MarketPricesDone`].
    MarketPrices { app_id: AppId, names: Vec<String>, currency: i32 },
    /// List an item on the Community Market. `receive` is what you get, in cents.
    /// Steam then asks for confirmation in the Steam Mobile app.
    SellItem { app_id: AppId, context_id: u64, asset_id: u64, amount: u64, receive: u64 },
    LoadMyListings,
    /// A page of your Community Market history (50 rows from `start`).
    LoadMarketHistory { start: u32, language: String },
    /// Your profile's editable details, privacy, owned items and themes.
    LoadProfileEdit { language: String },
    SaveProfileInfo(Box<model::ProfileEdit>),
    SaveProfilePrivacy(model::ProfilePrivacy),
    /// Upload a picture file as your avatar.
    UploadAvatar { path: std::path::PathBuf },
    /// Equip a profile item (0 background, 1 mini profile background, 2 avatar
    /// frame, 3 animated avatar); `id` 0 takes it off.
    EquipProfileItem { kind: u8, id: u64 },
    SetProfileTheme(String),
    /// Show the equipped background full screen or at its own size.
    SetBackgroundFullScreen { id: u64, full: bool },
    /// The badge featured on your profile (a community badge id; 0 = none).
    SetFavoriteBadge { badge: u32 },
    /// Countries, a country's states, or a state's cities.
    LoadLocations { country: Option<String>, state: Option<String> },
    /// Fetch names and avatars of people who aren't your friends.
    LoadPersonas { ids: Vec<SteamId> },
    /// Names for games (e.g. a wishlist's) that aren't in the library.
    LoadAppNames { ids: Vec<AppId> },
    /// Your trade offers (active ones and recent changes); item names in
    /// `language` (Steam's name for it).
    LoadTradeOffers { language: String },
    /// Your group chats (and start receiving their messages).
    LoadChatGroups,
    /// A room's messages; `before` = (time, ordinal) of the oldest shown, to page back.
    LoadGroupHistory { group: u64, room: u64, before: Option<(u32, u32)> },
    SendGroupMessage { group: u64, room: u64, text: String },
    /// Mark a room read up to a message's time.
    AckGroupRoom { group: u64, room: u64, time: u32 },
    /// Accept, decline or cancel a trade offer.
    RespondTradeOffer { id: u64, partner: SteamId, action: model::TradeAction },
    /// Send a trade offer. `counter` replaces an offer you got; `token` is the
    /// partner's trade link token (needed when they aren't your friend).
    SendTradeOffer { partner: SteamId, give: Vec<model::TradeAsset>, get: Vec<model::TradeAsset>, message: String, counter: Option<u64>, token: Option<String> },
    /// Your library collections from Steam (Favorites, Hidden, your own).
    LoadSteamCollections,
    /// Write changed or deleted collections to Steam, so they show in the
    /// official client and on other computers.
    SaveSteamCollections(Vec<model::CollectionEdit>),
    /// Comments on a profile (newest first), from `start`.
    LoadComments { steam_id: SteamId, start: u32 },
    /// Your Quick Invite link; `new` makes another one.
    LoadQuickInvite { new: bool },
    /// Become friends through someone's Quick Invite link.
    RedeemQuickInvite { steam_id: SteamId, token: String },
    /// Profiles whose name matches (`page` starts at 1).
    SearchUsers { text: String, page: u32 },
    /// Leave a comment on a profile.
    PostComment { steam_id: SteamId, text: String },
    /// Delete a comment (on your profile, or one you wrote).
    DeleteComment { steam_id: SteamId, comment_id: u64 },
    LoadBadges { steam_id: SteamId },
    /// A badge's own page (its link from the badges list).
    LoadBadge { url: String },
    /// Craft the next level of a game's badge (`foil`: the foil badge).
    CraftBadge { app_id: AppId, foil: bool },
    /// Someone's friends, latest reviews and groups (their profile pages).
    LoadProfileFriends { steam_id: SteamId },
    LoadProfileReviews { steam_id: SteamId },
    LoadProfileGroups { steam_id: SteamId },
    /// Search the Community Market (`app`: one game only; `start`: paging).
    /// Search the Community Market. A query of `cards:<appid>` lists that
    /// game's trading cards (Steam's own filters, like its Market page).
    MarketSearch { query: String, app: Option<AppId>, start: u32 },
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
    /// Move a queued download to place `to` in the queue (0 = next).
    MoveDownload { app_id: AppId, to: usize },
    /// Download this queued game now: the running download is set aside
    /// (kept on disk) and goes back in the queue right after it.
    DownloadNow { app_id: AppId },
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
    /// Check an installed game's files against Steam's manifests, shown on
    /// the Downloads page like Steam's "Verify integrity of game files".
    VerifyGame { game: model::InstalledGame, steam_root: PathBuf },
    CancelVerify { app_id: AppId },
    /// Add a free-to-play game to the account, like Steam's "Play Game" /
    /// "Add to Library" on a free game's store page.
    RequestFreeLicense { app_id: AppId },
    /// Download every subscribed Workshop item of an installed game that
    /// is missing or outdated (also done before each launch, like Steam).
    SyncWorkshop { app_id: AppId },
    /// The game's own Workshop tag groups (for filters).
    LoadWorkshopTags { app_id: AppId, language: String },
    LoadWishlist,
    WishlistAdd { app_id: AppId },
    /// Your store discovery queue; `rebuild` asks Steam for a fresh one.
    LoadDiscoveryQueue { rebuild: bool },
    /// A game in the discovery queue was seen ("Next in queue").
    DiscoverySeen { app_id: AppId },
    /// Your review of a game (answered with `Event::MyReview`).
    LoadMyReview { app_id: AppId },
    /// Post or change your review (`review.id` 0: a new one).
    SaveReview { app_id: AppId, review: model::MyReview },
    DeleteReview { app_id: AppId },
    /// Your friends' reviews of a game.
    LoadFriendReviews { app_id: AppId },
    /// These people's reviews of a game (to vote on them).
    LoadReviewsBy { app_id: AppId, people: Vec<SteamId> },
    /// "Was this review helpful?" on someone's review.
    VoteReview { review: u64, vote: model::ReviewVote },
    /// "Not interested" in a game (`ignore: false` undoes it).
    IgnoreApp { app_id: AppId, ignore: bool },
    WishlistRemove { app_id: AppId },
    /// Move a game to `to` (0-based) in the wishlist's own order ("Your rank").
    WishlistMove { app_id: AppId, to: usize },
    LoadCart,
    /// Add a package to the cart, optionally as a gift for a friend.
    /// Add a copy to the cart: for this account, or as a gift (optionally
    /// already for a friend).
    CartAdd { package_id: u32, gift: bool, gift_to: Option<SteamId> },
    /// Switch a cart line between "For my account" and "This is a gift"
    /// (optionally for a friend), like the dropdown in Steam's cart.
    CartModify { line_item_id: u64, gift: bool, gift_to: Option<SteamId> },
    CartRemove { line_item_id: u64 },
    /// Community pages: Workshop items, guides, discussions, hub, achievements.
    Hub(hub::Request),
    /// Send a chat message written as Steam's markup (see
    /// `model::chat::compose`); Steam turns slash commands into theirs.
    SendChat { to: ChatTarget, text: String },
    /// Add or take back a reaction (an emoticon or a sticker) under a message.
    ReactToMessage { to: ChatTarget, time: u32, ordinal: u32, reaction: String, sticker: bool, add: bool },
    /// Report a message to Steam's moderators (`reason`: Steam's
    /// EContentReportReason).
    ReportMessage { to: ChatTarget, from: SteamId, time: u32, ordinal: u32, reason: i32, text: String, language: String },
    /// Your emoticons, stickers and chat effects.
    LoadEmoticons,
    /// Upload a picture and send it in a chat.
    SendChatImage { to: ChatTarget, path: PathBuf },
    /// Your trade offer link (`new`: make a new one; the old one stops working).
    LoadTradeLink { new: bool },
    /// Make a group chat with friends. With `partner`, it grows the
    /// one-to-one chat with them into a group; an empty `name` makes an
    /// unnamed one.
    CreateChatGroup { name: String, partner: Option<SteamId>, invitees: Vec<SteamId> },
    /// Invite a friend into a group chat.
    InviteToChatGroup { group: u64, friend: SteamId },
    LeaveChatGroup { group: u64 },
    LogOut,
    /// Manage a group chat (`group` 0 for a new one): see [`model::chat_admin::AdminOp`].
    ChatAdmin { group: u64, op: model::chat_admin::AdminOp },
    /// Check that Valve's voice server accepts a connection. Rings nobody
    /// and joins nothing.
    VoiceConnectionTest,
    /// Calls, channels, mute, volumes and audio settings.
    Voice(model::voice::VoiceOp),
}

/// A conversation: one-to-one with a friend, or a room of a group chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatTarget {
    Friend(SteamId),
    Group { group: u64, room: u64 },
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::LoginPassword { account, .. } => f.debug_struct("LoginPassword").field("account", account).field("password", &"[REDACTED]").finish(),
            Command::LoginQr => f.write_str("LoginQr"),
            Command::LoginSaved { account } => f.debug_struct("LoginSaved").field("account", account).finish(),
            Command::StartOffline { account } => f.debug_struct("StartOffline").field("account", account).finish(),
            Command::GoOnline => f.write_str("GoOnline"),
            Command::GoOffline => f.write_str("GoOffline"),
            Command::SubmitCode(_) => f.debug_tuple("SubmitCode").field(&"[REDACTED]").finish(),
            Command::CancelLogin => f.write_str("CancelLogin"),
            Command::ForgetAccount { account } => f.debug_struct("ForgetAccount").field("account", account).finish(),
            Command::SetPersonaState(s) => f.debug_tuple("SetPersonaState").field(s).finish(),
            Command::Launch { app_id, install_dir, args } => f.debug_struct("Launch").field("app_id", app_id).field("install_dir", install_dir).field("args", args).finish(),
            Command::LaunchWithoutCloud { app_id, install_dir, args } => f.debug_struct("LaunchWithoutCloud").field("app_id", app_id).field("install_dir", install_dir).field("args", args).finish(),
            Command::CloudSettings { steam_root, disabled } => f.debug_struct("CloudSettings").field("steam_root", steam_root).field("disabled", disabled).finish(),
            Command::CloudSync { app_id, install_dir, mode, keep_local, keep_cloud, then_launch } => f.debug_struct("CloudSync").field("app_id", app_id).field("install_dir", install_dir).field("mode", mode).field("keep_local", keep_local).field("keep_cloud", keep_cloud).field("then_launch", then_launch).finish(),
            Command::StopGame { app_id } => f.debug_struct("StopGame").field("app_id", app_id).finish(),
            Command::AttachGame { app_id, install_dir } => f.debug_struct("AttachGame").field("app_id", app_id).field("install_dir", install_dir).finish(),
            Command::JoinRunningGame { app_id, friend, connect } => f.debug_struct("JoinRunningGame").field("app_id", app_id).field("friend", friend).field("connect", connect).finish(),
            Command::SendMessage { to, text } => f.debug_struct("SendMessage").field("to", to).field("text", text).finish(),
            Command::InviteToLobby { app_id, lobby, friend } => f.debug_struct("InviteToLobby").field("app_id", app_id).field("lobby", lobby).field("friend", friend).finish(),
            Command::AddFriend { steam_id } => f.debug_struct("AddFriend").field("steam_id", steam_id).finish(),
            Command::RemoveFriend { steam_id } => f.debug_struct("RemoveFriend").field("steam_id", steam_id).finish(),
            Command::SetNickname { steam_id, nickname } => f.debug_struct("SetNickname").field("steam_id", steam_id).field("nickname", nickname).finish(),
            Command::BlockFriend { steam_id, block } => f.debug_struct("BlockFriend").field("steam_id", steam_id).field("block", block).finish(),
            Command::QueryServer { addr } => f.debug_struct("QueryServer").field("addr", addr).finish(),
            Command::Typing { to } => f.debug_struct("Typing").field("to", to).finish(),
            Command::LoadHistory { friend } => f.debug_struct("LoadHistory").field("friend", friend).finish(),
            Command::LoadProfile { steam_id } => f.debug_struct("LoadProfile").field("steam_id", steam_id).finish(),
            Command::LoadInventory { steam_id, app_id, context_id, language } => f.debug_struct("LoadInventory").field("steam_id", steam_id).field("app_id", app_id).field("context_id", context_id).field("language", language).finish(),
            Command::LoadInventoryApps { steam_id } => f.debug_struct("LoadInventoryApps").field("steam_id", steam_id).finish(),
            Command::MarketPrice { app_id, hash_name, currency, country, full } => f.debug_struct("MarketPrice").field("app_id", app_id).field("hash_name", hash_name).field("currency", currency).field("country", country).field("full", full).finish(),
            Command::MarketPrices { app_id, names, currency } => f.debug_struct("MarketPrices").field("app_id", app_id).field("names", names).field("currency", currency).finish(),
            Command::SellItem { app_id, context_id, asset_id, amount, receive } => f.debug_struct("SellItem").field("app_id", app_id).field("context_id", context_id).field("asset_id", asset_id).field("amount", amount).field("receive", receive).finish(),
            Command::LoadMyListings => f.write_str("LoadMyListings"),
            Command::LoadMarketHistory { start, .. } => f.debug_struct("LoadMarketHistory").field("start", start).finish(),
            Command::LoadProfileEdit { language } => f.debug_struct("LoadProfileEdit").field("language", language).finish(),
            Command::SaveProfileInfo(p) => f.debug_tuple("SaveProfileInfo").field(p).finish(),
            Command::SaveProfilePrivacy(p) => f.debug_tuple("SaveProfilePrivacy").field(p).finish(),
            Command::UploadAvatar { path } => f.debug_struct("UploadAvatar").field("path", path).finish(),
            Command::EquipProfileItem { kind, id } => f.debug_struct("EquipProfileItem").field("kind", kind).field("id", id).finish(),
            Command::SetProfileTheme(s) => f.debug_tuple("SetProfileTheme").field(s).finish(),
            Command::SetBackgroundFullScreen { id, full } => f.debug_struct("SetBackgroundFullScreen").field("id", id).field("full", full).finish(),
            Command::SetFavoriteBadge { badge } => f.debug_struct("SetFavoriteBadge").field("badge", badge).finish(),
            Command::LoadLocations { country, state } => f.debug_struct("LoadLocations").field("country", country).field("state", state).finish(),
            Command::LoadPersonas { ids } => f.debug_struct("LoadPersonas").field("ids", ids).finish(),
            Command::LoadAppNames { ids } => f.debug_struct("LoadAppNames").field("ids", ids).finish(),
            Command::LoadTradeOffers { language } => f.debug_struct("LoadTradeOffers").field("language", language).finish(),
            Command::LoadChatGroups => f.write_str("LoadChatGroups"),
            Command::LoadGroupHistory { group, room, before } => f.debug_struct("LoadGroupHistory").field("group", group).field("room", room).field("before", before).finish(),
            Command::SendGroupMessage { group, room, .. } => f.debug_struct("SendGroupMessage").field("group", group).field("room", room).finish(),
            Command::AckGroupRoom { group, room, time } => f.debug_struct("AckGroupRoom").field("group", group).field("room", room).field("time", time).finish(),
            Command::RespondTradeOffer { id, partner, action } => f.debug_struct("RespondTradeOffer").field("id", id).field("partner", partner).field("action", action).finish(),
            Command::SendTradeOffer { partner, give, get, message, counter, token } => f.debug_struct("SendTradeOffer").field("partner", partner).field("give", give).field("get", get).field("message", message).field("counter", counter).field("token", &token.as_ref().map(|_| "[REDACTED]")).finish(),
            Command::LoadSteamCollections => f.write_str("LoadSteamCollections"),
            Command::SaveSteamCollections(edits) => f.debug_tuple("SaveSteamCollections").field(edits).finish(),
            Command::LoadComments { steam_id, start } => f.debug_struct("LoadComments").field("steam_id", steam_id).field("start", start).finish(),
            Command::LoadQuickInvite { new } => f.debug_struct("LoadQuickInvite").field("new", new).finish(),
            Command::RedeemQuickInvite { steam_id, .. } => f.debug_struct("RedeemQuickInvite").field("steam_id", steam_id).field("token", &"[REDACTED]").finish(),
            Command::SearchUsers { text, page } => f.debug_struct("SearchUsers").field("text", text).field("page", page).finish(),
            Command::PostComment { steam_id, text } => f.debug_struct("PostComment").field("steam_id", steam_id).field("text", text).finish(),
            Command::DeleteComment { steam_id, comment_id } => f.debug_struct("DeleteComment").field("steam_id", steam_id).field("comment_id", comment_id).finish(),
            Command::LoadBadges { steam_id } => f.debug_struct("LoadBadges").field("steam_id", steam_id).finish(),
            Command::LoadBadge { url } => f.debug_struct("LoadBadge").field("url", url).finish(),
            Command::CraftBadge { app_id, foil } => f.debug_struct("CraftBadge").field("app_id", app_id).field("foil", foil).finish(),
            Command::LoadProfileFriends { steam_id } => f.debug_struct("LoadProfileFriends").field("steam_id", steam_id).finish(),
            Command::LoadProfileReviews { steam_id } => f.debug_struct("LoadProfileReviews").field("steam_id", steam_id).finish(),
            Command::LoadProfileGroups { steam_id } => f.debug_struct("LoadProfileGroups").field("steam_id", steam_id).finish(),
            Command::MarketSearch { query, app, start } => f.debug_struct("MarketSearch").field("query", query).field("app", app).field("start", start).finish(),
            Command::LoadNotifications => f.write_str("LoadNotifications"),
            Command::MarkNotificationsRead { ids } => f.debug_struct("MarkNotificationsRead").field("ids", ids).finish(),
            Command::LoadSupportTickets => f.write_str("LoadSupportTickets"),
            Command::LoadSupportTicket { id, url } => f.debug_struct("LoadSupportTicket").field("id", id).field("url", url).finish(),
            Command::LoadAccountSecurity => f.write_str("LoadAccountSecurity"),
            Command::LoadPurchases { language, cursor } => f.debug_struct("LoadPurchases").field("language", language).field("cursor", cursor).finish(),
            Command::LoadLicenses { language } => f.debug_struct("LoadLicenses").field("language", language).finish(),
            Command::LoadFamily { language } => f.debug_struct("LoadFamily").field("language", language).finish(),
            Command::CancelListing { listing_id } => f.debug_struct("CancelListing").field("listing_id", listing_id).finish(),
            Command::CancelBuyOrder { order_id } => f.debug_struct("CancelBuyOrder").field("order_id", order_id).finish(),
            Command::LoadPlayerGames { steam_id, language } => f.debug_struct("LoadPlayerGames").field("steam_id", steam_id).field("language", language).finish(),
            Command::LoadAchievementProgress { app_ids } => f.debug_struct("LoadAchievementProgress").field("app_ids", app_ids).finish(),
            Command::LoadAchievements { app_id, language } => f.debug_struct("LoadAchievements").field("app_id", app_id).field("language", language).finish(),
            Command::ActivateKey { .. } => f.debug_struct("ActivateKey").field("key", &"[REDACTED]").finish(),
            Command::Install(req) => f.debug_tuple("Install").field(req).finish(),
            Command::PauseDownload { app_id } => f.debug_struct("PauseDownload").field("app_id", app_id).finish(),
            Command::ResumeDownload { app_id } => f.debug_struct("ResumeDownload").field("app_id", app_id).finish(),
            Command::CancelDownload { app_id } => f.debug_struct("CancelDownload").field("app_id", app_id).finish(),
            Command::MoveDownload { app_id, to } => f.debug_struct("MoveDownload").field("app_id", app_id).field("to", to).finish(),
            Command::DownloadNow { app_id } => f.debug_struct("DownloadNow").field("app_id", app_id).finish(),
            Command::ProbeRegions => f.write_str("ProbeRegions"),
            Command::DownloadRegion { cell_id, host } => f.debug_struct("DownloadRegion").field("cell_id", cell_id).field("host", host).finish(),
            Command::DownloadLimits { limit_kbps, concurrency } => f.debug_struct("DownloadLimits").field("limit_kbps", limit_kbps).field("concurrency", concurrency).finish(),
            Command::UnsubscribeWorkshop { app_id, file_id } => f.debug_struct("UnsubscribeWorkshop").field("app_id", app_id).field("file_id", file_id).finish(),
            Command::WorkshopQuery { request_id, app_id, query_type, page, search, days, tags } => f.debug_struct("WorkshopQuery").field("request_id", request_id).field("app_id", app_id).field("query_type", query_type).field("page", page).field("search", search).field("days", days).field("tags", tags).finish(),
            Command::WorkshopSubscribe { app_id, file_id } => f.debug_struct("WorkshopSubscribe").field("app_id", app_id).field("file_id", file_id).finish(),
            Command::VerifyGame { game, steam_root } => f.debug_struct("VerifyGame").field("game", game).field("steam_root", steam_root).finish(),
            Command::CancelVerify { app_id } => f.debug_struct("CancelVerify").field("app_id", app_id).finish(),
            Command::RequestFreeLicense { app_id } => f.debug_struct("RequestFreeLicense").field("app_id", app_id).finish(),
            Command::SyncWorkshop { app_id } => f.debug_struct("SyncWorkshop").field("app_id", app_id).finish(),
            Command::LoadWorkshopTags { app_id, language } => f.debug_struct("LoadWorkshopTags").field("app_id", app_id).field("language", language).finish(),
            Command::LoadWishlist => f.write_str("LoadWishlist"),
            Command::WishlistAdd { app_id } => f.debug_struct("WishlistAdd").field("app_id", app_id).finish(),
            Command::WishlistRemove { app_id } => f.debug_struct("WishlistRemove").field("app_id", app_id).finish(),
            Command::WishlistMove { app_id, to } => f.debug_struct("WishlistMove").field("app_id", app_id).field("to", to).finish(),
            Command::LoadDiscoveryQueue { rebuild } => f.debug_struct("LoadDiscoveryQueue").field("rebuild", rebuild).finish(),
            Command::DiscoverySeen { app_id } => f.debug_struct("DiscoverySeen").field("app_id", app_id).finish(),
            Command::LoadMyReview { app_id } => f.debug_struct("LoadMyReview").field("app_id", app_id).finish(),
            Command::SaveReview { app_id, review } => f.debug_struct("SaveReview").field("app_id", app_id).field("review", review).finish(),
            Command::DeleteReview { app_id } => f.debug_struct("DeleteReview").field("app_id", app_id).finish(),
            Command::LoadFriendReviews { app_id } => f.debug_struct("LoadFriendReviews").field("app_id", app_id).finish(),
            Command::LoadReviewsBy { app_id, people } => f.debug_struct("LoadReviewsBy").field("app_id", app_id).field("people", people).finish(),
            Command::VoteReview { review, vote } => f.debug_struct("VoteReview").field("review", review).field("vote", vote).finish(),
            Command::IgnoreApp { app_id, ignore } => f.debug_struct("IgnoreApp").field("app_id", app_id).field("ignore", ignore).finish(),
            Command::LoadCart => f.write_str("LoadCart"),
            Command::CartAdd { package_id, gift, gift_to } => f.debug_struct("CartAdd").field("package_id", package_id).field("gift", gift).field("gift_to", gift_to).finish(),
            Command::CartModify { line_item_id, gift, gift_to } => f.debug_struct("CartModify").field("line_item_id", line_item_id).field("gift", gift).field("gift_to", gift_to).finish(),
            Command::CartRemove { line_item_id } => f.debug_struct("CartRemove").field("line_item_id", line_item_id).finish(),
            Command::Hub(hub::Request::Reply { thread_url, .. }) => f.debug_struct("HubReply").field("thread_url", thread_url).finish_non_exhaustive(),
            Command::Hub(hub::Request::PostStatus { app_id, .. }) => f.debug_struct("HubPostStatus").field("app_id", app_id).finish_non_exhaustive(),
            Command::Hub(req) => f.debug_tuple("Hub").field(req).finish(),
            Command::SendChat { to, .. } => f.debug_struct("SendChat").field("to", to).finish_non_exhaustive(),
            Command::ReactToMessage { to, time, ordinal, reaction, sticker, add } => f.debug_struct("ReactToMessage").field("to", to).field("time", time).field("ordinal", ordinal).field("reaction", reaction).field("sticker", sticker).field("add", add).finish(),
            Command::ReportMessage { to, from, time, ordinal, reason, .. } => f.debug_struct("ReportMessage").field("to", to).field("from", from).field("time", time).field("ordinal", ordinal).field("reason", reason).finish_non_exhaustive(),
            Command::LoadEmoticons => f.write_str("LoadEmoticons"),
            Command::SendChatImage { to, path } => f.debug_struct("SendChatImage").field("to", to).field("path", path).finish(),
            Command::LoadTradeLink { new } => f.debug_struct("LoadTradeLink").field("new", new).finish(),
            Command::CreateChatGroup { name, partner, invitees } => f.debug_struct("CreateChatGroup").field("name", name).field("partner", partner).field("invitees", invitees).finish(),
            Command::InviteToChatGroup { group, friend } => f.debug_struct("InviteToChatGroup").field("group", group).field("friend", friend).finish(),
            Command::LeaveChatGroup { group } => f.debug_struct("LeaveChatGroup").field("group", group).finish(),
            Command::LogOut => f.write_str("LogOut"),
            Command::ChatAdmin { group, op } => f.debug_struct("ChatAdmin").field("group", group).field("op", op).finish(),
            Command::VoiceConnectionTest => f.write_str("VoiceConnectionTest"),
            Command::Voice(op) => f.debug_tuple("Voice").field(op).finish(),
        }
    }
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
    /// The `STEAMINRUST_IPC` value for a game started by hand ([`Command::AttachGame`]).
    GameAttached { app_id: AppId, env: String },
    /// A game asked for Steam's overlay (see `ipc::protocol::Request::Overlay`
    /// for `dialog` and `target`): SteamInRust's overlay should open there.
    GameOverlay { app_id: AppId, dialog: String, target: String },
    /// How a game is being started: with SteamInRust's steamclient, through
    /// the official Steam client (anti-cheat), or without Steam features.
    LaunchMode { app_id: AppId, mode: LaunchMode },
    /// Steam Cloud is syncing a game's saves (before launch or after exit).
    CloudSyncing { app_id: AppId },
    /// Before a launch: subscribed Workshop items downloading (`done` of
    /// `total`, the current one's title). `total` 0 = finished.
    WorkshopPreparing { app_id: AppId, done: u32, total: u32, title: String },
    /// Result of a Steam Cloud sync. `held_launch` is set when the game
    /// wasn't started because of conflicts (its launch arguments).
    CloudReport { report: model::CloudReport, held_launch: Option<LaunchArgs> },
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
    /// A [`Command::MarketPrices`] finished; names it didn't price weren't found.
    MarketPricesDone { app_id: AppId },
    /// Result of [`Command::SellItem`]: `Ok(true)` = needs confirming on your phone.
    ItemListed { asset_id: u64, result: Result<bool, String> },
    MyListings(Result<model::MyListings, String>),
    /// A page of market history: rows from `start`, and how many there are.
    MarketHistory { start: u32, result: Result<(Vec<model::HistoryRow>, u32), String> },
    TradeOffers(Result<model::TradeOffers, String>),
    ChatGroups(Result<Vec<model::ChatGroup>, String>),
    /// Who's in each group chat.
    ChatGroupMembers(Vec<(u64, Vec<SteamId>)>),
    /// A page of a room's messages (`older`: from paging back).
    GroupHistory { group: u64, room: u64, result: Result<(Vec<model::GroupMessage>, bool), String>, older: bool },
    /// A new message in a group chat (yours too, once Steam echoes it).
    GroupMessage(model::GroupMessage),
    GroupMessageFailed { group: u64, room: u64, error: String },
    /// A room was read here (its unread mark goes).
    GroupRoomRead { group: u64, room: u64 },
    ProfileEdit(Result<model::ProfileEdit, String>),
    ProfileSaved { what: model::ProfileSave, result: Result<(), String> },
    /// (code, name) for [`Command::LoadLocations`].
    Locations { country: Option<String>, state: Option<String>, result: Result<Vec<(String, String)>, String> },
    /// Result of [`Command::RespondTradeOffer`]: `Ok(true)` = confirm it in
    /// the Steam Mobile app.
    TradeOfferAnswered { id: u64, action: model::TradeAction, result: Result<bool, String> },
    /// Result of [`Command::SendTradeOffer`]: the new offer's id and whether
    /// to confirm it in the Steam Mobile app.
    TradeOfferSent { partner: SteamId, result: Result<(u64, bool), String> },
    SteamCollections(Result<Vec<model::SteamCollection>, String>),
    /// Result of [`Command::SaveSteamCollections`].
    SteamCollectionsSaved(Result<(), String>),
    /// Comments on a profile, or why they can't be shown (private, turned off).
    Comments { steam_id: SteamId, result: Result<model::Comments, String> },
    QuickInvite(Result<model::QuickInvite, String>),
    QuickInviteRedeemed { steam_id: SteamId, result: Result<(), String> },
    UserSearch(Result<model::UserSearch, String>),
    CommentPosted { steam_id: SteamId, result: Result<(), String> },
    ServerInfo { addr: String, result: Result<crate::a2s::ServerInfo, String> },
    Blocked { steam_id: SteamId, result: Result<bool, String> },
    Badges { steam_id: SteamId, result: Result<model::Badges, String> },
    Badge { url: String, result: Result<model::BadgeDetail, String> },
    /// The new badge's name, or why not.
    BadgeCrafted { app_id: AppId, result: Result<String, String> },
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
    /// A game's first-time setup (Steam's install scripts) before it starts:
    /// step `done` of `total` is running; `total` 0 = finished.
    FirstTimeSetup { app_id: AppId, done: usize, total: usize, step: String },
    /// The running download was set aside for another ("download now") and
    /// is back in the queue; it continues from what's on disk.
    DownloadStopped { app_id: AppId },
    WorkshopUnsubscribed { file_id: u64, result: Result<(), String> },
    WorkshopResults { request_id: u64, total: u64, items: Vec<model::WorkshopEntry> },
    WorkshopSubscribed { file_id: u64, result: Result<(), String> },
    /// A verification ended (its row leaves the Downloads page).
    VerifyFinished { app_id: AppId, result: Result<local::manage::VerifyReport, String> },
    /// Answer to [`Command::RequestFreeLicense`].
    FreeLicense { app_id: AppId, result: Result<(), String> },
    /// Workshop items were downloaded (or failed) for a game.
    WorkshopSync { app_id: AppId, result: Result<crate::workshop::Report, String> },
    WorkshopTags { app_id: AppId, result: Result<Vec<model::WorkshopTagCategory>, String> },
    /// (app id, date added)
    Wishlist(Vec<(AppId, u64)>),
    /// A wishlist change was asked for: shown right away, then corrected by
    /// the [`Event::Wishlist`] that follows if Steam refused it.
    WishlistChanging { app_id: AppId, add: bool },
    ProfileFriends { steam_id: SteamId, result: Result<Vec<SteamId>, String> },
    ProfileReviews { steam_id: SteamId, result: Result<Vec<model::ProfileReview>, String> },
    ProfileGroups { steam_id: SteamId, result: Result<Vec<model::ProfileGroup>, String> },
    /// Market search results: (query, game, start) they're for, then (results, total).
    MarketSearch { query: String, app: Option<AppId>, start: u32, result: Result<(Vec<model::MarketResult>, u32), String> },
    /// Your review of a game (`None`: not written).
    MyReview { app_id: AppId, result: Result<Option<model::MyReview>, String> },
    /// Friends' reviews of a game, (friend, review).
    FriendReviews { app_id: AppId, result: Result<Vec<(SteamId, model::MyReview)>, String> },
    /// Reviews of a game by the people asked about.
    ReviewsBy { app_id: AppId, reviews: Vec<(SteamId, model::MyReview)> },
    ReviewVoted { review: u64, vote: model::ReviewVote, result: Result<(), String> },
    /// A review was posted, changed or deleted.
    ReviewSaved { app_id: AppId, deleted: bool, result: Result<(), String> },
    /// The discovery queue's games, in order.
    DiscoveryQueue(Result<Vec<AppId>, String>),
    /// "Not interested" was set (or undone) for a game.
    AppIgnored { app_id: AppId, ignore: bool, result: Result<(), String> },
    Cart(Vec<model::CartLine>),
    /// A store action failed (cart, wishlist).
    StoreError(String),
    /// A message the client doesn't handle yet; useful for extension authors.
    Unhandled { emsg: u32, name: String },
    LoggedOut { reason: String },
    Error(String),
    /// Answer to a [`Command::Hub`] request.
    Hub(hub::Response),
    /// Someone added (or took back) a reaction under a message.
    MessageReaction { to: ChatTarget, time: u32, ordinal: u32, reactor: SteamId, reaction: String, sticker: bool, add: bool },
    /// Your emoticons, stickers and chat effects.
    Emoticons(model::Emoticons),
    /// Your trade offer link, or why it couldn't be had.
    TradeLink(Result<String, String>),
    /// A picture was sent in a chat, or why not.
    ChatImageSent { to: ChatTarget, result: Result<(), String> },
    /// A message was reported, or why not.
    MessageReported(Result<(), String>),
    /// A group chat was made (its id), or why not.
    ChatGroupCreated(Result<u64, String>),
    /// Inviting into, or leaving, a group chat failed.
    ChatGroupFailed(String),
    /// What a [`Command::ChatAdmin`] came back with.
    ChatAdmin(model::chat_admin::AdminEvent),
    /// What a [`Command::VoiceConnectionTest`] found.
    VoiceConnectionTest(Result<voice::ConnectionTest, String>),
    Voice(model::voice::VoiceEvent),
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

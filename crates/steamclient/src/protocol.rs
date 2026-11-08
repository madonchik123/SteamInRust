//! Messages between the steamclient DLL (inside a game) and the running
//! SteamInRust app. Shared source: the app includes this file with
//! `#[path]`, so both sides always agree.
//!
//! Transport: a named pipe. Each request opens a connection, writes one JSON
//! line (an [`Envelope`]), reads one JSON line (a [`Reply`]) and closes.
//! The token comes from the game's environment and tells the app which
//! launch (and game) is asking; requests without a known token are refused.

use serde::{Deserialize, Serialize};

/// Environment variable with `<pipe name>|<token>`, set only for games
/// SteamInRust starts with its own steamclient.
pub const ENV_IPC: &str = "STEAMINRUST_IPC";
/// Lobby chat messages starting with this are SteamInRust's own (P2P
/// set-up between players), never shown to the game.
pub const SIGNAL_PREFIX: &[u8] = b"\0SIR-P2P\0";
/// Optional log file for the DLL.
pub const ENV_LOG: &str = "STEAMINRUST_STEAMCLIENT_LOG";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub token: String,
    pub pid: u32,
    pub req: Request,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Request {
    /// First contact: who is signed in, what the game may know. Also loads
    /// the player's stats and achievements for this game.
    Hello,
    /// Stats and achievements of any user for this game.
    UserStats { steam_id: u64 },
    /// Save changed stats (achievements are bits in "achievement block" stats).
    StoreStats { stats: Vec<(u32, u32)>, crc: u32, reset: bool },
    /// The game's full rich presence (empty = cleared).
    RichPresence { pairs: Vec<(String, String)> },
    /// How many people play this game right now.
    CurrentPlayers,
    /// A new auth session ticket (for the game's servers / EOS). `identity`
    /// is the web API identity, empty for a plain session ticket.
    AuthTicket { identity: String },
    /// Answers once Steam has accepted ticket list `seq` (the ticket is
    /// valid for servers from then on).
    AuthTicketWait { seq: u32 },
    /// Withdraw one of our tickets (by CRC).
    CancelTicket { crc: u32 },
    /// Check another player's ticket with Steam (we're the host).
    BeginAuth { steam_id: u64, ticket: Vec<u8> },
    EndAuth { steam_id: u64 },
    /// An encrypted app ticket with the game's own data inside.
    EncryptedTicket { data: Vec<u8> },
    /// Wait (up to ~20 s) for things Steam pushed: lobby changes, chat,
    /// P2P signals. Answered with [`Reply::Events`].
    Poll,
    /// ELobbyType (0 private, 1 friends, 2 public, 3 invisible).
    LobbyCreate { kind: i32, max_members: i32 },
    LobbyJoin { lobby: u64 },
    LobbyLeave { lobby: u64 },
    LobbyList { filters: Vec<LobbyFilter>, count: i32 },
    /// Replace the lobby's data (owner), or with `member`, our own member data.
    LobbySetData { lobby: u64, member: bool, data: Vec<(String, String)>, max_members: i32, kind: i32, flags: i32 },
    LobbyRefresh { lobby: u64 },
    LobbyChat { lobby: u64, data: Vec<u8> },
    LobbySetOwner { lobby: u64, owner: u64 },
    LobbyInvite { lobby: u64, user: u64 },
    /// Send a small signalling message to another player's SteamInRust
    /// (P2P connection setup). With `lobby`, also as a lobby message
    /// addressed to that player.
    Signal { to: u64, data: Vec<u8>, lobby: u64 },
    /// Find (or create) a leaderboard by name.
    LbFind { name: String, create: bool, sort: i32, display: i32 },
    /// ELeaderboardDataRequest 0 global, 1 around me, 2 friends, 3 users.
    LbEntries { lb: i32, request: i32, start: i32, end: i32, users: Vec<u64> },
    /// ELeaderboardUploadScoreMethod 1 keep best, 2 force update.
    LbUpload { lb: i32, method: i32, score: i32, details: Vec<i32> },
    /// How many players have each achievement (API name, percent).
    GlobalPercentages,
    /// A player's avatar as RGBA. `size`: 0 small (32), 1 medium (64), 2 large (184).
    Avatar { steam_id: u64, size: u8 },
    /// An HTTP request for ISteamHTTP. `method`: EHTTPMethod (1 GET, 2 HEAD, 3 POST, 4 PUT, 5 DELETE…).
    Http { method: i32, url: String, headers: Vec<(String, String)>, params: Vec<(String, String)>, body: Option<(String, Vec<u8>)>, timeout_ms: u32 },
    /// A Workshop query (ISteamUGC): answered with [`Reply::UgcResults`].
    UgcQuery(Box<UgcQuery>),
    /// The player's subscribed Workshop items for this game.
    UgcSubscribed,
    /// Subscribe (or unsubscribe) the player; `list` 1 subscriptions, 2 favorites.
    UgcSubscribe { id: u64, list: u32, on: bool },
    UgcVote { id: u64, up: bool },
    /// The player's vote: answered with [`Reply::UgcVote`].
    UgcGetVote { id: u64 },
    /// Download (or update) an item into the game's Workshop folder.
    UgcDownload { id: u64 },
    /// Read or change the player's items for this game (ISteamInventory):
    /// answered with [`Reply::Inventory`].
    Inventory(InventoryOp),
    /// The game's item definitions: answered with [`Reply::ItemDefs`].
    ItemDefs,
    /// Promo items this player can still get.
    PromoItems,
    /// Game servers from Steam's master server: `filter` in Steam's
    /// "\key\value" form. Answered with [`Reply::Servers`].
    ServerList { filter: String, max: u32 },
    /// Steam Input bindings for an Xbox-style pad: answered with [`Reply::InputConfig`].
    InputConfig,
}

/// A game server as Steam's server browser lists it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameServer {
    pub ip: u32,
    pub query_port: u16,
    pub game_port: u16,
    pub steam_id: u64,
    pub app_id: u32,
    pub name: String,
    pub map: String,
    pub game_dir: String,
    pub description: String,
    pub tags: String,
    pub version: u32,
    pub players: u32,
    pub max_players: u32,
    pub bots: u32,
    pub password: bool,
    pub secure: bool,
    pub ping: u32,
}

/// What ISteamInventory asked for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InventoryOp {
    All,
    ByIds(Vec<u64>),
    Consume { item: u64, quantity: u32 },
    /// Destroy (item, quantity) to make one of `generate`.
    Exchange { generate: i32, destroy: Vec<(u64, u32)> },
    /// Move `quantity` to `dest` (another stack), or split into a new stack (dest 0).
    Transfer { source: u64, quantity: u32, dest: u64 },
    /// A playtime drop from this drop list.
    Drop { def: i32 },
    /// Grant these promo items (empty: every eligible one).
    Promo(Vec<i32>),
    /// Developers only: make items.
    Generate(Vec<(i32, u32)>),
    /// Dynamic property changes: (item, name, value; None removes it).
    Modify(Vec<(u64, String, Option<PropValue>)>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropValue {
    Str(String),
    Bool(bool),
    Int(i64),
    Float(f32),
}

/// One item as SteamItemDetails_t plus its JSON properties.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InvItem {
    pub id: u64,
    pub def: i32,
    pub quantity: u16,
    /// ESteamItemFlags: 1 no trade, 0x100 removed, 0x200 consumed.
    pub flags: u16,
    pub props: Vec<(String, String)>,
}

/// What ISteamUGC's query builder collected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UgcQuery {
    /// 0 a user's list, 1 all items (EUGCQuery), 2 these ids.
    pub kind: u8,
    pub account: u32,
    /// EUserUGCList and EUserUGCListSortOrder.
    pub list: i32,
    pub sort: i32,
    /// EUGCQuery.
    pub query: i32,
    /// EUGCMatchingUGCType.
    pub matching: i32,
    pub creator_app: u32,
    pub consumer_app: u32,
    pub page: u32,
    pub cursor: Option<String>,
    pub ids: Vec<u64>,
    pub required_tags: Vec<String>,
    pub excluded_tags: Vec<String>,
    pub tag_groups: Vec<Vec<String>>,
    pub match_any: bool,
    pub search: String,
    pub trend_days: u32,
    pub kv: Vec<(String, String)>,
    pub language: String,
    pub cloud_filename: String,
    pub only_ids: bool,
    pub total_only: bool,
    pub long_description: bool,
    pub metadata: bool,
    pub children: bool,
    pub previews: bool,
    pub kv_tags: bool,
    pub playtime_days: u32,
    pub created: (u32, u32),
    pub updated: (u32, u32),
    pub max_age: u32,
}

/// One Workshop item, everything SteamUGCDetails_t and the query getters return.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UgcItem {
    pub id: u64,
    pub result: i32,
    /// EWorkshopFileType.
    pub file_type: i32,
    pub creator_app: u32,
    pub consumer_app: u32,
    pub title: String,
    pub description: String,
    pub owner: u64,
    pub created: u32,
    pub updated: u32,
    pub visibility: i32,
    pub banned: bool,
    pub accepted: bool,
    /// (tag, display name).
    pub tags: Vec<(String, String)>,
    /// The content manifest (UGCHandle) and preview handle.
    pub file: u64,
    pub preview_file: u64,
    pub file_name: String,
    pub file_size: u64,
    pub preview_size: u64,
    pub url: String,
    pub votes_up: u32,
    pub votes_down: u32,
    pub score: f32,
    pub children: Vec<u64>,
    pub preview_url: String,
    pub metadata: String,
    pub kv: Vec<(String, String)>,
    /// Additional previews: (URL or video id, EItemPreviewType, original file name).
    pub previews: Vec<(String, i32, String)>,
    /// (EItemStatistic, value).
    pub stats: Vec<(i32, u64)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LbEntry {
    pub steam_id: u64,
    pub rank: i32,
    pub score: i32,
    pub details: Vec<i32>,
    pub ugc: u64,
}

/// One RequestLobbyList filter, as Steam's matchmaking server takes it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LobbyFilter {
    pub key: String,
    pub value: String,
    /// ELobbyComparison (-2 ..= 3).
    pub cmp: i32,
    /// 0 string, 1 number, 2 open slots, 3 near value, 4 distance.
    pub kind: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LobbyMember {
    pub steam_id: u64,
    pub name: String,
    pub data: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lobby {
    pub id: u64,
    pub owner: u64,
    pub kind: i32,
    pub flags: i32,
    pub max_members: i32,
    pub num_members: i32,
    pub data: Vec<(String, String)>,
    /// Empty in lobby lists (only members see members).
    pub members: Vec<LobbyMember>,
}

/// Something Steam told SteamInRust that the game needs to hear.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "e")]
pub enum GameEvent {
    LobbyData(Lobby),
    LobbyMember { lobby: u64, user: u64, name: String, joined: bool },
    LobbyChat { lobby: u64, sender: u64, data: Vec<u8> },
    LobbyGameServer { lobby: u64, ip: u32, port: u16, server: u64 },
    /// A P2P signalling message from another player.
    Signal { from: u64, data: Vec<u8> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Reply {
    Hello(Box<Session>),
    Stats { result: Result<Stats, String> },
    Stored { eresult: i32, crc: u32 },
    Players { count: Option<i32> },
    /// A session ticket, its CRC (its id with Steam) and ticket-list number.
    Ticket { ticket: Vec<u8>, crc: u32, seq: u32 },
    /// EAuthSessionResponse (0 = OK) and the game's owner.
    AuthResult { response: u32, owner: u64 },
    Bytes { data: Vec<u8> },
    Events { events: Vec<GameEvent> },
    LobbyCreated { eresult: i32, lobby: u64 },
    /// EChatRoomEnterResponse (1 = success) and the lobby when joined.
    LobbyJoined { response: i32, lobby: Option<Lobby> },
    LobbyList { lobbies: Vec<Lobby> },
    Result { eresult: i32 },
    UgcResults { eresult: i32, total: u32, items: Vec<UgcItem>, cursor: String },
    UgcIds { ids: Vec<u64> },
    UgcVote { eresult: i32, up: bool, down: bool, skipped: bool },
    Inventory { eresult: i32, items: Vec<InvItem>, ticket: Vec<u8> },
    /// Item definitions: (id, properties).
    ItemDefs { defs: Vec<(i32, Vec<(String, String)>)> },
    PromoItems { eresult: i32, defs: Vec<i32> },
    Servers { servers: Vec<GameServer> },
    /// A controller_mappings VDF (empty: none found) and where it came from.
    InputConfig { bindings: String, source: String },
    /// EResult, the item's folder, size on disk and last update time.
    UgcDownloaded { eresult: i32, folder: String, size: u64, updated: u32 },
    LbFound { found: bool, id: i32, count: i32, sort: i32, display: i32, name: String },
    LbEntries { total: i32, entries: Vec<LbEntry> },
    LbUploaded { ok: bool, changed: bool, rank_new: i32, rank_prev: i32 },
    Percentages { list: Vec<(String, f32)> },
    Image { width: u32, height: u32, rgba: Vec<u8> },
    HttpDone { status: i32, headers: Vec<(String, String)>, body: Vec<u8>, timed_out: bool },
    Ok,
    Err { message: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    pub app_id: u32,
    pub steam_id: u64,
    pub account: String,
    pub persona_name: String,
    /// Two-letter country of the connection ("US").
    pub country: String,
    /// Steam's language names ("english", "russian").
    pub ui_language: String,
    pub game_language: String,
    pub level: u32,
    /// Every app and DLC the account owns.
    pub owned: Vec<u32>,
    /// DLC of this game the account owns: (app id, name).
    pub dlc: Vec<(u32, String)>,
    pub friends: Vec<Friend>,
    pub install_dir: String,
    /// `<Steam root>`: `userdata/<account id>/<app>/` lives under it.
    pub steam_root: String,
    pub build_id: u32,
    /// Beta branch name, empty for the public branch.
    pub beta: String,
    /// The player's stats for this game, when they could be loaded.
    pub stats: Option<Stats>,
    /// SteamInRust isn't connected to Steam (offline mode): `BLoggedOn`
    /// is false, stats come from the local cache and are sent later.
    #[serde(default)]
    pub offline: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Friend {
    pub steam_id: u64,
    pub name: String,
    /// EPersonaState (0 offline, 1 online, 2 busy, 3 away…).
    pub state: u32,
    /// EFriendRelationship (3 = friend, 2 = request received, 4 = request sent).
    pub relationship: u32,
    /// Game they're playing (0 = none).
    pub game: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub crc: u32,
    pub stats: Vec<Stat>,
    pub achievements: Vec<Achievement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stat {
    pub id: u32,
    pub name: String,
    /// 1 int, 2 float, 3 average rate, 4 achievement block.
    pub kind: u8,
    /// Raw 32 bits (float stats hold `f32::to_bits`).
    pub value: u32,
    pub default: u32,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub increment_only: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Achievement {
    /// API name, e.g. "ACH_WIN_ONE_GAME".
    pub name: String,
    /// Stat id of the block holding its bit, and the bit.
    pub block: u32,
    pub bit: u32,
    pub display_name: String,
    pub description: String,
    pub hidden: bool,
    pub icon: String,
    pub icon_gray: String,
    pub unlocked: bool,
    pub unlock_time: u32,
}

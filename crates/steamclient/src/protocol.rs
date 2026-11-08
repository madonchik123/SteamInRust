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

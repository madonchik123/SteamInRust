# Steam features in games started by SteamInRust

Games built with Steamworks call into Steam while they run: who is signed
in, achievements, cloud saves, friends. Before this, games started by
SteamInRust found no Steam and said "no connection to Steam" (or quit).

SteamInRust now starts every game in one of three ways. The game page
shows which one ("Steam features: SteamInRust" / "Runs through Steam
(anti-cheat)" / "Starts without Steam").

| Game | How it starts | Steam features from |
|---|---|---|
| No anti-cheat, 64-bit | SteamInRust's own `steamclient` DLL | SteamInRust |
| No anti-cheat, 32-bit | The same DLL built for 32-bit (`steamclient32.dll`) | SteamInRust |
| Any anti-cheat (VAC, Easy Anti-Cheat, BattlEye, …) | Official Steam client, started silently | Valve's Steam |
| No anti-cheat, our DLL missing | Plain start, as before | none |

## 1. Our steamclient (games without anti-cheat)

A game's `steam_api64.dll` never talks to Steam itself. It loads the
`steamclient64.dll` named in
`HKCU\Software\Valve\Steam\ActiveProcess\SteamClientDll64`, checks that the
process in `ActiveProcess\pid` is alive, then uses Steam's C++ interfaces.

`crates/steamclient` is a Rust replacement for that DLL (same 41 exports as
Valve's). For a launch, SteamInRust:

1. registers a one-time token on its pipe (`\\.\pipe\steaminrust-<pid>-…`);
2. points `SteamClientDll64` (64-bit games) or `SteamClientDll` (32-bit
   games) at its DLL, sets `pid` to itself and
   `ActiveUser` to your account (old values are saved);
3. starts the game with `SteamAppId`, `SteamGameId`, `SteamClientLaunch=1`
   and `STEAMINRUST_IPC=<pipe>|<token>`;
4. puts Valve's DLL path back as soon as the game has connected (or after
   5 minutes, for games that never use Steam), and the rest when the game
   closes. Values the official client
   changed in between are left alone. A crash is repaired at the next
   sign-in.

The DLL answers from what the app sends over the pipe, using the app's own
connection to Steam. Loaded into any process **without** the token (for
example a game the official client starts while the registry briefly points
at us), it loads Valve's real `steamclient64.dll` and forwards every call, so
that game talks to real Steam.

Interface layouts were read from the flat-API wrappers
(`SteamAPI_ISteamX_Method` → `call [vtable+offset]`) of the `steam_api.dll`
and `steam_api64.dll` of 22 Steamworks SDK releases (the redistributables
Steamworks.NET ships with each release, SDK 1.3x to 1.6x) and games on the
dev PC: 91 interface versions, every one with the same slots on both
bitnesses and, for 32-bit, the bytes each method pops. Two details matter:
MSVC puts overloads in reverse order (`GetStatFloat` before
`GetStatInt32`), and a few exports are inline helpers that call another
method (`InitRelayNetworkAccess` calls `CheckPingDataUpToDate`,
`SetGlobalConfigValue*` call `SetConfigValue`), so the slot belongs to the
method they call. Valve sometimes appended methods without a new version
number; the tables keep the longest list. Versions older than all of these
(`SteamClient014–016`) use the nearest layout.

### 32-bit games

The same crate builds for `i686-pc-windows-gnu` (needs a 32-bit MinGW, e.g.
WinLibs i686; `tools/package.ps1` ships it as `steamclient32.dll` when one is
on PATH or in `SIR_MINGW32`). 32-bit games call interface methods with
`thiscall`: `this` in ECX, arguments on the stack, popped by the method. The
DLL keeps one C implementation for both: on x86 every vtable slot is a small
trampoline generated at start-up that copies the arguments, calls the C
function with `this` first and pops exactly the bytes the real method pops.
Those byte counts come from the flat-API thunks of a 32-bit `steam_api.dll`
(`tables32.rs`, read from Human Fall Flat's copy). Tested: Human Fall Flat
(32-bit, SDK ~1.50) starts signed in, with its stats, and the Spacewar probe
built for 32-bit passes the same checks as 64-bit (stats, auth tickets
validated by Steam, encrypted tickets, lobbies).

Build it by hand:

```
rustup target add i686-pc-windows-gnu
set CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER=<mingw32>\bin\i686-w64-mingw32-gcc.exe
cargo build -p steamclient --target i686-pc-windows-gnu
```

While developing, the launcher also finds
`target/i686-pc-windows-gnu/<profile>/steamclient.dll` next to
`target/<profile>/steam-rs.exe`.

### What works

- **Signing in**: `SteamAPI_Init` succeeds; your SteamID, persona name,
  level, country, Steam UI language; `BIsSubscribed`, owned apps and your
  DLC for the game; app id, install folder, build id and beta branch.
- **Stats and achievements**: loaded with the game's schema
  (`ClientGetUserStats`), read, set, cleared, stored
  (`ClientStoreUserStats2`). `UserStatsReceived`, `UserStatsStored` and
  `UserAchievementStored` callbacks fire like on Steam. Other players' stats
  (`RequestUserStats`), current player count.
- **Steam Cloud (API files)**: `ISteamRemoteStorage` reads and writes
  `<Steam>/userdata/<account>/<app>/remote/`, which SteamInRust syncs with
  Steam Cloud before the game starts and after it closes (Auto-Cloud games
  were already covered).
- **Friends**: friends list, names, online state, what they're playing.
- **Rich presence**: what the game sets is sent to Steam, so friends see it.
- **Playing status and playtime**: SteamInRust reports the game as played.
- **In-game overlay**: in D3D11 games our steamclient draws SteamInRust's
  Shift+Tab overlay inside the game, like Steam's, and sends the game
  `GameOverlayActivated_t` when it opens and closes. While it's open the game
  doesn't get mouse or keyboard input. Other games get a separate overlay
  window. Set `STEAMINRUST_NO_INGAME_OVERLAY=1` to turn the in-game one off.
- **Invites**: the game's lobby and its rich presence `connect` decide what
  the friend menus offer ("Invite to lobby", "Invite to game", or just
  "Invite to play"). All are sent as Steam's own chat invites.
- **Overlay links**: "open web page" and "open store page" open in your
  browser.
- **Multiplayer sign-in (auth tickets)**: `GetAuthSessionTicket` builds
  tickets like Valve's client (a game connect token Steam sends at sign-in,
  a session header, the game's ownership ticket) and registers them with
  Steam (`ClientAuthList`), so game servers and Epic Online Services can
  check them; `GetAuthSessionTicketResponse` fires once Steam accepted it.
  As a host, `BeginAuthSession` asks Steam about another player's ticket
  (`ValidateAuthTicketResponse`). `CancelAuthTicket` / `EndAuthSession`
  withdraw them, and they're dropped when the game closes. Encrypted app
  tickets (`RequestEncryptedAppTicket`) come from Steam. Tested with
  Spacewar: Steam validated our ticket ("OK", owner = the account).
  `GetAuthTicketForWebApi` returns the same kind of ticket; services that
  check the identity string may still refuse it (not tested yet).
- **Lobbies and matchmaking** (`ISteamMatchmaking`): Steam's real lobbies,
  through SteamInRust's connection to Steam's matchmaking service (the
  messages carry the game's app id in `routing_appid`, which is how Steam
  routes them). Create, join, leave, search with string/number/near/slot/
  distance filters and result counts, lobby and member data, owner, member
  limit and type, invites, lobby chat. What Steam pushes (lobby data,
  members joining and leaving, chat, game server) reaches the game through
  a long-poll on the pipe as the usual callbacks (`LobbyEnter_t`,
  `LobbyDataUpdate_t`, `LobbyChatUpdate_t`, `LobbyChatMsg_t`,
  `LobbyGameCreated_t`). These are the same lobbies players on the official
  client see. Tested on Spacewar: create → set data → Steam's push → found
  by a filtered search → leave.
- **P2P with players on the official client** (`ISteamNetworking`:
  `SendP2PPacket`, `ReadP2PPacket`, `IsP2PPacketAvailable`,
  `AcceptP2PSessionWithUser`, session state). Valve's client still runs
  this API on libjingle 0.4.0 (the Google Talk P2P library; its sources are
  in Steam's client build as `src/external/libjingle-0.4.0`), not on
  SteamNetworkingSockets, and SteamInRust speaks the same protocol
  (`crates/steamclient/src/jingle.rs`, `pseudotcp.rs`):
  one `ClientP2PConnectionInfo` per candidate in Valve's serialization,
  libjingle's STUN connectivity checks (RFC 3489, username fragments),
  Valve's "mixed" socket (`[0]` unreliable / `[1]` PseudoTcp segment,
  `0xBEEFFACE` to close), a port of libjingle's PseudoTcp (conversation 0)
  for reliable packets with `[u32 length]` framing, and Valve's packet
  header (`[0][data]` on channel 0, `[1][i32 channel][data]`). Official
  players behind NAT offer a candidate on Valve's libjingle relay servers;
  those forward to the player anything carrying their binding's name, so
  SteamInRust reaches them through the relay with no relay account of its
  own, and it works behind any NAT on our side. Each official peer gets
  its own UDP socket (players on one relay share its address). Tested
  live: Human Fall Flat joined a public lobby hosted by an official-client
  player; the session opened through Valve's relay in Chicago and the game
  loaded the host's level ("Siege", 2 players). Hosting for official
  players uses the same code (tested against a stand-in libjingle peer
  over real UDP in both directions; not yet live).
- **P2P between SteamInRust players** (the same API). Each side learns its LAN and
  public (STUN) addresses, the two exchange them with a random session
  token through Steam (a lobby message addressed to that player when both
  are in a lobby, plus Steam's `ClientP2PConnectionInfo`), then connect
  directly with UDP hole punching. Reliable sends get sequence numbers,
  acks, retransmission, in-order delivery and fragmentation (up to 1 MB);
  unreliable ones go as they are (up to 1200 bytes, like Steam).
  `P2PSessionRequest_t` on the first packet, `P2PSessionConnectFail_t`
  after 15 s without an answer. Tested in-process with two endpoints; a
  real two-player test needs a second account.
- **Leaderboards**: find / find-or-create, name, entry count, sort and
  display type, download (global, around the player, friends, given users)
  with details, upload scores. Tested: Spacewar's "Feet Traveled" (top 5).
- **Global achievement percentages**: `RequestGlobalAchievementPercentages`,
  `GetAchievementAchievedPercent`, most-achieved iteration.
- **Avatars**: `GetSmall/Medium/LargeFriendAvatar` (32/64/184 px) with
  `GetImageSize` / `GetImageRGBA`; `AvatarImageLoaded_t` when one arrives.
- **HTTP** (`ISteamHTTP`): requests made by SteamInRust for the game (GET,
  POST, …; headers, parameters, raw bodies, timeouts; body and headers of
  the answer; `HTTPRequestCompleted_t`).
- **Networking sockets and messages** (`ISteamNetworkingSockets`,
  `ISteamNetworkingMessages`, the networking utils games wait on): P2P
  connections by Steam ID and virtual port (listen, connect, accept, close,
  status callbacks, reliable and unreliable messages up to 512 KB, poll
  groups, real-time status with measured ping, socket pairs, connecting to
  yourself) and per-user message sessions with their request/failed
  callbacks, over the same direct UDP link as `ISteamNetworking`.
  GameNetworkingSockets-style config callbacks (function pointers set with
  `SetConfigValue`) run during `SteamAPI_RunCallbacks`. Tested with two
  processes and two made-up players (`p2p_pair` example), including a
  200 KB message.
- **Workshop** (`ISteamUGC`): queries (a user's lists, all items ranked or
  by text, tags, tag groups, key-value tags, dates, page or cursor, details
  by id) with every result getter; the player's subscriptions;
  subscribe/unsubscribe, favorites and votes; item downloads into
  `steamapps/workshop/content/<app>/<item>` from the content servers (or a
  legacy item's file URL) with `DownloadItemResult_t` / `ItemInstalled_t`.
  Subscribed items missing on disk download when the game first asks for
  its subscriptions, like Steam does. Tested on Spacewar's Workshop: 50 of
  6325 items with details, one downloaded.
- **Inventory** (`ISteamInventory`): the player's items (all, or by id) with
  their properties, playtime drops, promo items, consuming, crafting
  (`ExchangeItems`), stack splits and merges, dynamic property updates,
  `SerializeResult`. Tested: Spacewar's 7 items on the test account.
- **Server browser** (`ISteamMatchmakingServers`): internet lists from
  Steam's master server, each server then asked for its state and ping with
  A2S_INFO; LAN lists by broadcast; ping, player and rules queries. Results
  reach the game's response objects on its thread. Tested: 2185 Team
  Fortress 2 servers, 1770 answering with real pings; players and rules of
  one server.
- **Screenshots** (`ISteamScreenshots`): `WriteScreenshot` and
  `AddScreenshotToLibrary` save a JPEG and thumbnail into
  `userdata/<account>/760/remote/<app>/screenshots`; `TriggerScreenshot`
  asks the game when it hooked screenshots, else captures the game's window.
- **Steam Input** (`ISteamInput`, `ISteamController`): Xbox-style pads
  through XInput, mapped to the game's actions by its Steam Input bindings:
  the player's own config, their Workshop choice per controller type, a
  config the game ships, or the game's official configs from its app info.
  Action sets and layers, digital and analog actions, origins, rumble.
  Without bindings no controller is reported, so the game keeps its own
  controller support. Tested: Spacewar's bindings found and parsed (no pad
  was connected for a button test).
- **Callbacks and call results**: both the `Steam_BGetCallback` path
  (steam_api's manual dispatch) and `ISteamUtils::GetAPICallResult`.

### What doesn't (yet)

- **Networking sockets and messages with players on the official client**
  (`ISteamNetworkingSockets`, `ISteamNetworkingMessages`, used by newer
  games; `ISteamNetworking` above works). For these Valve's client uses
  SteamNetworkingSockets: signals in `ClientP2PConnectionInfo.rendezvous`,
  Valve-signed certificates (`ClientNetworkingCertRequest`, EMsg 5621),
  Ed25519/X25519/AES-GCM, the SNP framing, and either direct ICE (open
  source in GameNetworkingSockets) or the Steam Datagram Relay. Not done
  yet: SteamInRust doesn't answer those signals, so such a connection ends
  with a timeout. It isn't ruled out: the SDR client code isn't open
  source, but Valve's `steamclient64.dll` carries all of it with its debug
  strings, the relay list is public (`ISteamApps/GetSDRConfig`), and the
  official client with default settings relays rather than sharing IPs
  with strangers, so the relay path is the one that matters.
  Between two SteamInRust players everything works directly; behind two
  strict (symmetric) NATs hole punching can fail, with no relay to fall
  back on (official players' libjingle relays carry `ISteamNetworking`
  traffic to them, not between two SteamInRust players).
- **Connecting by IP address** (`ConnectByIPAddress`, dedicated servers):
  Steam's sockets speak Valve's encrypted UDP protocol to those servers;
  SteamInRust doesn't implement it, so these connections fail at once with
  a clear end reason instead of hanging. Listen sockets by IP get only P2P
  connections.
- **Game servers** (`ISteamGameServer`): hosting a server Steam lists needs
  its own anonymous game-server sign-in to Steam and callbacks routed to the
  server's pipe; not done yet, so in-process servers stay anonymous and
  offline.
- **Workshop publishing** (`CreateItem`, `SubmitItemUpdate`) fails with
  k_EResultFail.
- **Inventory**: item definitions need a digest Steam's Inventory service
  refuses to a client session (tested while playing too), so
  `GetItemDefinitionIDs` finds none yet; buying items needs Steam's overlay
  checkout; another player's serialized result can't be checked (Valve's
  signature), so `DeserializeResult` reports k_EResultFail.
- **Screenshots** stay on this PC: Steam's client uploads them and lists
  them in its own index.
- **Steam Input**: gyro, trackpads, mouse modes, haptics other than rumble
  and the binding panel (Steam's overlay configurator).
- **Server browser**: friends, favorites, history and spectator lists come
  back empty.
- **The overlay**: SteamInRust's own Shift+Tab overlay, not Steam's. Global
  stats (`RequestGlobalStats`): "failed".
- **Joining from the friends list** (`GameLobbyJoinRequested_t`,
  `+connect_lobby`) isn't wired to SteamInRust's friends list yet; join from
  the game's own lobby browser.

## 2. Official Steam (games with anti-cheat)

Games with anti-cheat never get our DLL. Their anti-cheat expects Valve's
client, and replacing it would mean the anti-cheat doesn't run as intended:
that's bypassing it and risks a ban. Two more reasons this covers every
anti-cheat, not only VAC:

- Easy Anti-Cheat and BattlEye check which DLLs are loaded into the game.
  Our DLL isn't signed by Valve and can be refused or flagged.
- Their servers usually check the game with the anti-cheat's own service
  too, which expects Valve's client next to it.

Detection (`launch::anti_cheat`), safe side when unsure:

- Steam's store category 8 "Valve Anti-Cheat enabled";
- launch options or files in the install folder (three levels deep) named
  like `EasyAntiCheat`, `start_protected_game.exe`, `eac_launcher`,
  `BattlEye`, `BEService`, `EAAntiCheat`, PunkBuster, XIGNCODE, GameGuard,
  EQU8, ACE, FACEIT, RICOCHET and other known names, or anything with
  "anticheat" in it;
- an install folder that can't be read.

How it runs:

1. Valve's DLL path is put back in the registry first, if a swap is active.
2. Steam is found through `HKCU\Software\Valve\Steam\SteamExe`. If it isn't
   running, `steam.exe -silent` starts it in the background and SteamInRust
   waits for it (`ActiveProcess\pid` alive).
3. **Auto sign-in.** Before starting Steam, SteamInRust makes it sign in to
   the same account as SteamInRust, the way Steam does after you pick an
   account in its list: `AutoLoginUser` in the registry, `AutoLogin "1"` for
   that account in `config/loginusers.vdf`, and Steam's "Ask which account
   to use each time Steam starts" off (`AlwaysShowUserChooser` in
   `config/config.vdf`). These changes are temporary. The values they
   replace are saved first (`steam-sign-in-restore.json` in SteamInRust's
   config folder). `AutoLoginUser` goes back as soon as Steam has signed in.
   The two files go back once Steam has closed, because Steam rewrites them
   when it exits: a background watcher does it while SteamInRust runs, and
   the next SteamInRust sign-in does it otherwise. A value you changed
   yourself in the meantime is left alone. This only works for accounts the
   official client already remembers ("Remember me"); SteamInRust never
   passes your password or tokens to it. If Steam is already open at its
   account picker or sign-in window, nobody is signed in, so it's restarted
   signed in to the account. If it still isn't signed in
   (`ActiveProcess\ActiveUser` stays 0), you get a message asking you to sign
   in to the Steam window.
4. `steam.exe -applaunch <appid> <args>` starts the game.
5. SteamInRust follows `HKCU\Software\Valve\Steam\Apps\<appid>\Running` to
   show the game as running and when it closes. It doesn't send its own
   "playing" status (Steam does, and two would clash) and doesn't sync the
   cloud (Steam does). Stop closes processes running from the game folder.

The official client keeps running afterwards, like after any Steam launch.

Messages you may see: "Steam isn't installed" (install it from
store.steampowered.com), "Steam is running but not signed in", "Steam didn't
start the game" (Steam may be showing an update or EULA prompt).

## Testing

- `cargo test --workspace` covers layouts, schema parsing, anti-cheat
  detection and callback structs.
- `crates/steamclient/examples/probe.rs`: a fake game that loads a real
  `steam_api64.dll` (or `steam_api.dll` when built for 32-bit) and prints
  what Steam answers. `--lobby`, `--ticket`, `--extras` (leaderboards,
  percentages, avatar, HTTP), `--unlock`, `--stat`, `--cloud`,
  `--sockets`. With `--direct <steamclient.dll> --test ugc|inventory|servers|input|lobbies`
  it loads our DLL without steam_api and checks one area through the
  vtables (read-only on the account, except one Workshop item download).
- `crates/client/examples/steamclient_attach.rs <appid> <dir> <probe> [args]`:
  signs in and attaches the probe with `Command::AttachGame`, without
  launching a game or reporting one as played (so a game running on
  another PC isn't disturbed).
- `crates/client/examples/p2p_pair.rs <probe> <dll>`: plays Steam for two
  made-up players and runs two probes against each other over real UDP
  (networking sockets and messages); no account needed.
- `crates/client/examples/steamclient_test.rs <appid> <install dir>`: signs
  in with the saved account and starts a game exactly like the Play button.
  `STEAMINRUST_STEAMCLIENT64=<dll>` picks the DLL,
  `STEAMINRUST_STEAMCLIENT_LOG=<file>` logs every interface the game asks for.

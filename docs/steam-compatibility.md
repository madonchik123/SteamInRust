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

Interface layouts were taken from the flat-API thunks of real
`steam_api64.dll` files (SDK 1.57 and 1.62), slot by slot, including MSVC's
reversed order for overloads (`GetStatFloat` before `GetStatInt32`).
Supported versions: `SteamClient017/020/021`, `SteamUser021/023`,
`SteamFriends017/018`, `SteamUtils010`, `STEAMAPPS…008`,
`STEAMUSERSTATS…012/013`, `STEAMREMOTESTORAGE…014/016`,
`SteamMatchMaking009`, `SteamNetworking006`, `SteamNetworkingUtils003/004`,
`STEAMHTTP…003`, `STEAMUGC…015/016/021` and the rest of those SDKs as stubs.
Other versions of a known interface use the nearest known layout.

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
- **Overlay links**: "open web page" and "open store page" open in your
  browser (there is no in-game overlay).
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
- **P2P between SteamInRust players** (`ISteamNetworking`:
  `SendP2PPacket`, `ReadP2PPacket`, `IsP2PPacketAvailable`,
  `AcceptP2PSessionWithUser`, session state). Each side learns its LAN and
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
- **Installed Workshop items** (`ISteamUGC`): the subscribed list, item
  state and install folder from `steamapps/workshop/content/<app>`, so games
  that load mods that way find them.
- **Callbacks and call results**: both the `Steam_BGetCallback` path
  (steam_api's manual dispatch) and `ISteamUtils::GetAPICallResult`.

### What doesn't (yet)

- **P2P with players on the official client**: Valve's client carries P2P
  traffic (`ISteamNetworking`, `ISteamNetworkingSockets`,
  `ISteamNetworkingMessages`) over the Steam Datagram Relay, whose protocol
  isn't public (the open-source GameNetworkingSockets leaves it out). So a
  SteamInRust player and an official-client player can share a lobby but
  not exchange game packets; the connection attempt ends with
  `P2PSessionConnectFail_t` (timeout). Between two SteamInRust players it
  works directly; behind two strict (symmetric) NATs hole punching can fail,
  and there's no relay to fall back on.
- **ISteamNetworkingSockets / ISteamNetworkingMessages** (the newer
  networking APIs): stubs ("invalid handle"). Games built only on these
  can't connect yet.
- **Workshop queries and publishing, inventory, screenshots, Steam Input,
  the overlay**: stubs. Global stats (`RequestGlobalStats`): "failed".
- **Game servers** started in-process get an anonymous, offline server.
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
  percentages, avatar, HTTP), `--unlock`, `--stat`, `--cloud`.
- `crates/client/examples/steamclient_test.rs <appid> <install dir>`: signs
  in with the saved account and starts a game exactly like the Play button.
  `STEAMINRUST_STEAMCLIENT64=<dll>` picks the DLL,
  `STEAMINRUST_STEAMCLIENT_LOG=<file>` logs every interface the game asks for.

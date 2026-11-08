# Steam parity

How much of the official Steam desktop client SteamInRust covers, area by
area. Last checked against the code on 8 November 2026 (v0.3.0).

Scores: **0** not started · **0.25** started · **0.5** usable, big parts
missing · **0.75** mostly there · **1** complete. Anything marked "opens
Steam's website" still works; it just isn't native yet.

| Area | Score | Main gaps |
| --- | ---: | --- |
| Sign-in and accounts | 0.75 | Tokens are in a per-user file, not the Windows credential store; changing email, password or phone opens Steam's website |
| Offline mode and reconnecting | 1 | Whether Steam counts playtime recorded offline is unconfirmed |
| Library | 0.75 | Collection edits aren't saved back to Steam; dynamic collections are skipped |
| Downloads and installs | 0.75 | No drag to reorder the queue; no taskbar progress |
| Store | 0.75 | Checkout happens on Steam's website (Steam requires it); no discovery queue; can't write reviews |
| Friends and chat | 0.75 | No group chats or voice chat |
| Community | 0.5 | Reading only for discussions; badge pages, groups and someone's reviews open Steam's website |
| Inventory and Market | 0.75 | Trade offers open Steam's website; no Market search page |
| Steam Cloud | 1 | Can't delete files from the cloud |
| Steam features inside games | 0.75 | P2P with players on the official client; `ISteamNetworkingSockets`; game servers |
| Shift+Tab overlay | 0.75 | Drawn inside D3D11 games only (others get a window overlay); no in-game web browser or screenshots |
| Notifications and tray | 1 | |
| Settings, languages, updates | 0.75 | English and Russian only; Windows only |
| Big Picture, Remote Play, Steam Input | 0 | Not planned for now |

---

## Sign-in and accounts

| | |
| --- | --- |
| Password with Steam Guard (email code, mobile code, approve on the phone) | Done |
| QR code from the Steam Mobile app | Done |
| Remember me, several saved accounts, switch account, ask at startup | Done |
| Refresh token storage | Per-user file in app data. Moving it to the Windows credential store is planned |
| Account details: email, phone, Steam Guard and authenticator with dates, password and email change dates, store country, wallet, VAC | Done, native (Credentials service) |
| Purchase history with "load more", refunds marked | Done, native (read from Steam's history page, like Steam's client) |
| Licenses and product key activations, searchable | Done, native |
| Steam Families: members, roles, cooldowns, invites, shared library with owners | Done, native (FamilyGroups service). Inviting, removing and limits open Steam's website |
| Changing email, password, phone, Steam Guard | Open Steam's website (they need Steam's own confirmation) |

## Offline mode and reconnecting

| | |
| --- | --- |
| "Connection error" window at startup with Retry and Start offline | Done. A rejected sign-in still goes to the sign-in screen |
| Offline mode sticks until you choose Go online, like Steam | Done |
| Library, installed games and Steam features in games while offline | Done (cached user and stats, stats queued until online) |
| Reconnect after the connection drops, without signing out | Done. Retries after 2, 5, 10, 20, 40 and 60 s, then every minute. A silent drop is noticed within about 100 s |
| Downloads pause while offline and resume after reconnecting | Done |
| Playtime while offline | Recorded and sent with `Player.RecordDisconnectedPlaytime`, the call Steam uses. Steam accepted it in a test, but the profile's playtime didn't change, so it's unconfirmed |
| Anti-cheat games offline | The official client is started in its own offline mode. Not tested live |
| Proxy | `STEAMINRUST_PROXY=http://host:port` for the Steam connection, `HTTPS_PROXY` for the rest |

## Library

| | |
| --- | --- |
| Every owned game (all licenses through PICS) and installed games | Done |
| Search, filters (genre, features, Steam Deck status, installed, played), sort by name, recent, hours, size | Done |
| Home with recent games and shelves, game page with art, play bar, news and achievements | Done |
| Collections, including your Steam Favorites and Hidden | Read from Steam. Edits stay local for now |
| Non-Steam games | Done |
| Install states everywhere: list row with progress, game page, right-click menu, Properties | Done |
| Properties: general, launch options, language, updates, installed files, versions and betas (with private beta codes), DLC, Steam Cloud, Workshop, controller, privacy | Done |
| Verify files, uninstall, desktop shortcuts, browse local files (also while installing) | Done |
| Achievements with rarity, sorting and comparing with a friend | Done |

## Downloads and installs

| | |
| --- | --- |
| SteamInRust's own depot downloader (manifests, chunks, decryption, decompression) | Done |
| Queue, pause, resume, cancel; the download survives a restart and resumes | Done |
| Separate network and disk speeds, and what it's doing (downloading, writing to disk, verifying) | Done |
| Every Steam download region and partner CDNs, sorted by measured speed | Done |
| Bandwidth limit, update schedule, downloads while playing, automatic updates | Done |
| Ask before quitting during a download | Done |
| Reorder the queue by dragging, taskbar progress | Not yet |
| Workshop items | Downloaded when you subscribe (installed games) and before each launch, like Steam; legacy single-file items too |

## Store

All native, no web view.

| | |
| --- | --- |
| Front page, search, tags and genres with filters, wishlist | Done |
| Game pages: screenshots, trailers, description, requirements, languages, review score, tags, DLC | Done |
| Trailers | Native player (`crates/video`): HLS streaming, Media Foundation H.264 and AAC, full screen, quality and volume |
| Prices in your wallet's currency, "In library" marks | Done |
| Cart (the same cart as the website), product keys, wallet balance | Done |
| Gifts, including games you already own, with a check that Steam will deliver to the friend's region | Done |
| Price history | Through IsThereAnyDeal, with your own free key |
| Checkout | Opens Steam's website. Steam only takes payment there |
| Discovery queue, writing reviews, microtrailers on hover | Not yet |

## Friends and chat

| | |
| --- | --- |
| Friends list with live status, the game being played and rich presence | Done |
| One-to-one chat with history, typing indicator and unread counts | Done |
| Separate Friends & Chat window that works while the main window is in the tray | Done |
| Right-click menu: join game, view server info (live A2S query), watch game, view game info, invite to play, nickname, favorites, remove, block | Done |
| Add friends, friend code, accept or ignore requests | Done |
| Group chats, voice chat | Not yet |

## Community

| | |
| --- | --- |
| Profiles: background, avatar frame, level, badges, showcases, comments (read and post) | Done |
| Someone's games, inventory, screenshots, artwork, videos, Workshop items and guides | Done |
| Community hub per game: screenshots, artwork, videos, news | Done |
| Workshop item pages: media, description, required items, change notes, comments; subscribe, rate, favorite | Done. Rating and favoriting weren't tested on a real account |
| Guides: list, sorts, search, reader with contents | Done |
| Discussions: forums, topics, threads with quotes and spoilers | Reading only |
| Steam Support: your requests and replies | Done. New requests open Steam's help site |
| Badge pages, groups, someone's friends and reviews | Open Steam's website |

## Inventory and Market

| | |
| --- | --- |
| Every game's inventory with the game's own filters and rarity colors | Done |
| Market price, order book, price chart, estimated value, in your currency | Done |
| Sell, quick sell, sell to the highest buy order, your listings and buy orders | Done. Steam's fees per currency; you confirm in the Steam Mobile app |
| Trade offers | Open Steam's website |
| Market search and history pages | Open Steam's website |

## Steam Cloud

| | |
| --- | --- |
| Per-game toggle, sync before launch and after quit, usage | Done |
| Conflicts, with a local backup before anything is replaced | Done |
| Saves changed offline upload by themselves once online | Done |
| Deleting cloud files | Not yet |

## Steam features inside games

Details: [steam-compatibility.md](steam-compatibility.md).

| | |
| --- | --- |
| Games without anti-cheat use SteamInRust's own `steamclient` DLL, 64-bit and 32-bit | Done |
| Games with VAC, Easy Anti-Cheat or BattlEye start through the official client, silently | Done. Our DLL is never used for them |
| User, apps, achievements, stats, cloud, friends, auth tickets, encrypted app tickets | Done |
| Lobbies (`ISteamMatchmaking`) through Steam's matchmaking service | Done |
| P2P (`ISteamNetworking`) between SteamInRust players | Done, with UDP hole punching |
| Leaderboards, global achievement percentages, avatars, `ISteamHTTP`, installed Workshop items | Done |
| P2P with players on the official client | Not possible: Valve routes it over the Steam Datagram Relay, whose protocol isn't public |
| `ISteamNetworkingSockets`, `ISteamNetworkingMessages`, game servers | Stubs |
| Inventory, screenshots, Steam Input, Workshop queries | Stubs |

## Shift+Tab overlay

| | |
| --- | --- |
| Opens over the running game with Shift+Tab, only while the game's window is in front | Done |
| Drawn inside the game, like Steam's, so it works in exclusive full screen and the game keeps focus | Done for D3D11 games SteamInRust starts with its steamclient (32- and 64-bit). D3D9, D3D12, OpenGL and Vulkan games get the window overlay, which shows over windowed and borderless games only |
| Mouse and keyboard go to the overlay while it's open; the cursor shows and the game gets `GameOverlayActivated_t` | Done |
| Game overview, achievements, friends, chat, notes, guides, discussions, hub, store page | Done, in floating windows |
| Pin windows over the game with adjustable opacity | Done |
| Games with anti-cheat | They start through the official client, so they get Steam's own overlay |
| Game invites in chat show as cards with a Join button (Steam's `[gameinvite]` markup). Friend menus offer "Invite to lobby" or "Invite to game" from what the game reports, else "Invite to play", all sent as real Steam invites | Done |
| In-game web browser, screenshots (F12), FPS counter | Not yet |

## Notifications and tray

| | |
| --- | --- |
| All Steam notification types with game art or avatars, toasts | Done |
| Tray icon with Steam's menu, close to tray | Done |

## Settings, languages, updates

| | |
| --- | --- |
| Settings: account, friends, security, notifications, interface, appearance, store, library, downloads, storage, cloud, in game, accessibility, plugins | Done |
| Themes, accent color, UI scale | Done |
| English and Russian, picked from the system language | Done. Other languages aren't translated yet |
| Installer, uninstaller, background updates checked with SHA-256 | Done |
| Plugins in sandboxed [Rhai](https://rhai.rs) scripts with permissions | Done. Network access for plugins is planned |
| Linux and macOS | Not yet |

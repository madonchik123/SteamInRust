# Steam parity

How much of the official Steam desktop client SteamInRust covers, area by
area. Last checked against the code on 18 November 2026 (v0.7.0 and the changes after it).

Scores: **0** not started · **0.25** started · **0.5** usable, big parts
missing · **0.75** mostly there · **1** complete. Anything marked "opens
Steam's website" still works; it just isn't native yet.

| Area | Score | Main gaps |
| --- | ---: | --- |
| Sign-in and accounts | 0.75 | Changing email, password or phone opens Steam's website |
| Offline mode and reconnecting | 1 | Whether Steam counts playtime recorded offline is unconfirmed |
| Library | 0.75 | Dynamic collections show and stay up to date, but their filters can only be changed in Steam |
| Downloads and installs | 1 | |
| Store | 1 | Checkout happens on Steam's website (Steam requires it) |
| Friends and chat | 0.75 | No voice chat; group chats are text only (no pictures, polls or managing rooms) |
| Community | 0.75 | Starting a new discussion topic, and joining or leaving a group, open Steam's website |
| Inventory and Market | 1 | |
| Steam Cloud | 1 | |
| Steam features inside games | 0.75 | Networking sockets / messages through Valve's relays (strangers on the official client); hosting game servers |
| Shift+Tab overlay | 0.75 | Drawn inside D3D11 games on Windows only (other games, and every game on a Mac, get a window overlay); no in-game web browser. F12 screenshots and the FPS counter only in games it's drawn inside |
| Notifications and tray | 1 | |
| Settings, languages, updates | 0.75 | Seven languages (Steam has about 30); no Linux version; Mac builds aren't signed with an Apple developer ID |
| Big Picture | 0.75 | Buying and most settings move to the normal window |
| Remote Play | 0 | Not planned for now. Steam Input for games is under "Steam features inside games" |

---

## Sign-in and accounts

| | |
| --- | --- |
| Password with Steam Guard (email code, mobile code, approve on the phone) | Done |
| QR code from the Steam Mobile app | Done |
| Remember me, several saved accounts, switch account, ask at startup | Done |
| Saved sign-ins | In the system's password store: Windows Credential Manager, or the Keychain on a Mac. Sign-ins saved by older versions move there on first start |
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
| Which systems a game runs on: a **Runs on** filter, a game-page line, and "Not available on macOS" for Windows-only games on a Mac | Done |
| Search, filters (genre, features, Steam Deck status, installed, played), sort by name, recent, hours, size | Done |
| Home with recent games and shelves, game page with art, play bar, news and achievements | Done |
| Collections, including your Steam Favorites and Hidden | Done, in step with Steam both ways. Edits made before Steam answers (or offline) are kept and sent once it does |
| Dynamic collections (made in Steam from filters) | Shown with their games worked out from Steam's filters, so they stay up to date. Their filters are changed in Steam. Filters on friends who own a game, languages and subscriptions aren't checked, so such a collection can show a few extra games |
| Non-Steam games | Done |
| Install states everywhere: list row with progress, game page, right-click menu, Properties | Done |
| Properties: general, launch options, language, updates, installed files, versions and betas (with private beta codes), DLC, Steam Cloud, Workshop, controller, privacy | Done |
| Verify files, uninstall, desktop shortcuts, browse local files (also while installing) | Done. Uninstalling also removes the game's Workshop items and unused download manifests, like Steam |
| Achievements with rarity, sorting and comparing with a friend | Done |

## Downloads and installs

| | |
| --- | --- |
| SteamInRust's own depot downloader (manifests, chunks, decryption, decompression) | Done |
| Queue, pause, resume, cancel; the download survives a restart and resumes | Done. Cancelling frees the disk space it used |
| The right version for your system: the macOS package on a Mac, with app bundles and run permissions restored | Done |
| Separate network and disk speeds, and what it's doing (downloading, writing to disk, verifying) | Done |
| Every Steam download region and partner CDNs, sorted by measured speed | Done |
| Bandwidth limit, update schedule, downloads while playing, automatic updates | Done |
| Ask before quitting during a download | Done |
| Reorder the queue (up, down, download now) | Done. "Download now" sets the running download aside; it goes back in the queue and continues from what it had |
| Progress on the taskbar button (Windows) or the Dock icon (Mac) | Done |
| Steam's shared redistributables (DirectX, Visual C++, .NET) a game uses | Downloaded into `Steamworks Shared` with the game, recorded in the app manifests like Steam |
| First-time setup: the game's install scripts (installers, registry values, firewall rules) before its first launch | Done on Windows, with progress on the Play button. Steps Steam (or we) already did are skipped by the same registry markers; one UAC prompt when an administrator is needed. On a Mac Steam runs none either (only permissions and symlinks, which the downloader restores) |
| Workshop items | Downloaded when you subscribe (installed games) and before each launch, like Steam; legacy single-file items too |

## Store

All native, no web view.

| | |
| --- | --- |
| Front page, search, tags and genres with filters, wishlist (the heart fills at once and shows what's already on it) | Done |
| Game pages: screenshots, trailers, description, requirements (tabs per system), languages (interface, full audio, subtitles), review score, tags, DLC | Done |
| Reviews on game pages: friends' reviews first, most helpful and recently posted, hours on record and at review time, helpful/funny votes, positive share for the past month and all time | Done |
| Demos: a "Demo available" tag in lists and Steam's "Download demo" box on the game page; unreleased games show their expected release date | Done |
| Mature content warning with "View page" and "Don't ask again" (switch back in store settings) | Done |
| Trailers | Native player (`crates/video`): HLS streaming, full screen, quality and volume. Decoded with Media Foundation on Windows and VideoToolbox/AudioToolbox on a Mac |
| Prices in your wallet's currency, "In library" marks | Done |
| Cart (the same cart as the website), Steam's "Added to your cart" popup, discount price tags, product keys, wallet balance | Done |
| Gifts, including games you already own, with a check that Steam will deliver to the friend's region | Done |
| Price history | Through IsThereAnyDeal, with your own free key |
| Checkout | Opens Steam's website. Steam only takes payment there |
| Discovery queue: each game's store page with Not interested, Wishlist and Next | Done (Steam's own queue, so it matches the website and the official client) |
| Your review of a game you own: write, change (recommended or not, public or friends only, comments, received for free) and delete | Done, on the game's store page |
| Microtrailers: a card's art plays the game's short clip while the pointer rests on it | Done |

## Friends and chat

| | |
| --- | --- |
| Friends list with live status, the game being played and rich presence | Done |
| One-to-one chat with history, typing indicator and unread counts | Done |
| Separate Friends & Chat window that works while the main window is in the tray | Done |
| Right-click menu: join game, view server info (live A2S query), watch game, view game info, invite to play, nickname, favorites, remove, block | Done |
| Add friends: friend code, Quick Invite links (make, copy, or open someone's), search profiles by name; accept or ignore requests | Done |
| Group chats: your groups with unread marks, rooms, history, sending, live messages, who's online in a members list | Done, in the app. Text only: pictures, polls, and creating or managing rooms aren't there yet |
| Voice chat | Not yet |

## Community

| | |
| --- | --- |
| Profiles: background, avatar frame, level, badges, showcases, comments (read and post) | Done |
| Someone's games, inventory, screenshots, artwork, videos, Workshop items and guides | Done |
| Community hub per game: screenshots, artwork, videos, news | Done |
| Workshop item pages: media, description, required items, change notes, comments; subscribe, rate, favorite | Done. Rating and favoriting weren't tested on a real account |
| Guides: list, sorts, search, reader with contents | Done |
| Discussions: forums, topics, threads with quotes and spoilers; reply to a thread | Done, in the app. Starting a new topic opens Steam's website. Sending a reply wasn't tested on a real account |
| Steam Support: your requests and replies | Done. New requests open Steam's help site |
| Someone's friends (online first, searchable), their latest reviews, their groups, and the badges list | Done, in the app |
| Group pages: name, headline, member counts, description, announcements, members (friends first), a button to the group's chat | Done, in the app, also for steamcommunity.com/groups links. Joining or leaving opens Steam's website |

## Inventory and Market

| | |
| --- | --- |
| Every game's inventory with the game's own filters and rarity colors | Done |
| Market price, order book, price chart, estimated value, in your currency | Done. Prices come from Steam's market search up to 100 at a time, only for items on screen |
| Sell, quick sell, sell to the highest buy order, your listings and buy orders | Done. Steam's fees per currency; you confirm in the Steam Mobile app |
| Trade offers: incoming and sent, with both sides' items and market value; accept, decline, cancel, counter | Done, in the app. Giving items away needs your confirmation in the Steam Mobile app, like on Steam |
| New trade offers: to a friend or by trade link, items from both inventories with a game picker, search, tag filters, sorting and prices | Done, in the app |
| Market search: by name and game, most popular first, with each item's price history and order book | Done, in the app |
| Your Market history: sold, bought, listed and removed, with prices and dates, filters, 50 at a time | Done, in the app (My listings → History) |

## Steam Cloud

| | |
| --- | --- |
| Per-game toggle, sync before launch and after quit, usage | Done |
| Conflicts, with a local backup before anything is replaced | Done |
| Saves changed offline upload by themselves once online | Done |

## Steam features inside games

Details: [steam-compatibility.md](steam-compatibility.md).

| | |
| --- | --- |
| Games without anti-cheat use SteamInRust's own `steamclient` DLL, 64-bit and 32-bit | Done |
| The same on a Mac: one `steamclient.dylib` for Apple and Intel games (Intel games run under Rosetta) | Done. Tested with Human: Fall Flat: Workshop, stats, and joining a stranger's lobby on the official client |
| Games with VAC, Easy Anti-Cheat or BattlEye start through the official client, silently | Done. Our DLL is never used for them |
| User, apps, achievements, stats, cloud, friends, auth tickets, encrypted app tickets | Done |
| Lobbies (`ISteamMatchmaking`) through Steam's matchmaking service | Done |
| P2P (`ISteamNetworking`) between SteamInRust players | Done, with UDP hole punching |
| P2P (`ISteamNetworking`) with players on the official client | Done: Valve's libjingle protocol, direct or through Valve's relays. Tested joining an official player's Human Fall Flat lobby |
| Leaderboards, global achievement percentages, avatars, `ISteamHTTP` | Done |
| Exact interface layouts for SDK 1.3x–1.6x (91 versions, both bitnesses) | Done |
| `ISteamNetworkingSockets` / `ISteamNetworkingMessages` with players on the official client | Partly: Valve's protocol (Steam-signed certificates, rendezvous, encrypted SNP) over direct ICE connections, which official players allow for friends. No Steam Datagram Relay, which the official client uses for strangers. Not yet tested against an official player |
| `ISteamNetworkingSockets`, `ISteamNetworkingMessages` between SteamInRust players | Done, on Valve's protocol over ICE (P2P; connecting by IP to dedicated servers isn't possible) |
| Workshop (`ISteamUGC`): queries, subscriptions, votes, favorites, downloads | Done. Publishing items isn't supported |
| Inventory (`ISteamInventory`): items, drops, promos, crafting, properties | Done, including item definitions (through the web session when Steam won't give them to the game's session). No purchases |
| Server browser (`ISteamMatchmakingServers`): internet and LAN lists, ping, players, rules | Done |
| Screenshots (`ISteamScreenshots`) into the Steam screenshot folder | Done. Not uploaded |
| Steam Input on Xbox-style pads with the game's bindings | Done on Windows. No gyro, trackpads or binding panel; no controllers in games on a Mac yet |
| Hosting game servers (`ISteamGameServer`) | Not yet: needs a game-server sign-in to Steam |

## Shift+Tab overlay

| | |
| --- | --- |
| Opens over the running game with Shift+Tab, only while the game's window is in front | Done |
| Drawn inside the game, like Steam's, so it works in exclusive full screen and the game keeps focus | Done for D3D11 games SteamInRust starts with its steamclient (32- and 64-bit). D3D9, D3D12, OpenGL and Vulkan games get the window overlay, which shows over windowed and borderless games only |
| On a Mac | A window over the game that also works over full-screen games. Shift+Tab needs the Input Monitoring permission; SteamInRust explains it on first start |
| Mouse and keyboard go to the overlay while it's open; the cursor shows and the game gets `GameOverlayActivated_t` | Done |
| Game overview, achievements, friends, chat, notes, guides, discussions, hub, store page | Done, in floating windows |
| Pin windows over the game with adjustable opacity | Done |
| Games with anti-cheat | They start through the official client, so they get Steam's own overlay |
| Game invites in chat show as cards with a Join button (Steam's `[gameinvite]` markup). Friend menus offer "Invite to lobby" or "Invite to game" from what the game reports, else "Invite to play", all sent as real Steam invites | Done |
| Screenshots with F12 into Steam's screenshot folder, with a "Screenshot saved" notice; FPS counter in a corner you pick (Settings → In game) | Done for games the overlay is drawn inside (D3D11). The frame is copied before the overlay is drawn on it; games that take their own screenshots get Steam's request instead. Not tested in a real game yet |
| In-game web browser | Not yet |

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
| English, Russian, Simplified Chinese, Spanish, Brazilian Portuguese, German and French, picked from the system language | Done. Japanese, Korean, Traditional Chinese, Polish, Turkish and others aren't translated yet |
| Installer, uninstaller, background updates checked with SHA-256 | Done. One download at a time; it continues after a restart or a dropped connection, and old installers are deleted. On a Mac: a disk image, and the app replaces itself on update |
| Copy, cut and paste in every text field, with a right-click menu | Done |
| Plugins in sandboxed [Rhai](https://rhai.rs) scripts with permissions | Done: pages with inputs, switches, tables and images; buttons on game pages; items in friends' menus; timers; web requests to the hosts a plugin lists (https, with your approval); launching games; downloads and wishlist; clipboard; trade offers, inventories and market prices (plugins can decline or cancel offers, and open a trade or an offer's review, but only you can accept or send) |
| macOS | Done (macOS 11 or later). Builds aren't signed with an Apple developer ID, so macOS asks for the Keychain password and Input Monitoring again after each update |
| Linux | Not yet |

## Big Picture

| | |
| --- | --- |
| Full screen for the TV, driven by a controller, the keyboard or the mouse | Done. Open it from the menu, the top bar, the controller's Guide button or `--bigpicture` |
| Home: continue playing, recently played, what's new | Done |
| Library: cover grid with Installed and All games | Done |
| Game page: Play, Install, Update, Pause and Resume, play time, size, achievements | Done |
| Store: front page, sections, search with an on-screen keyboard (English and Russian), store pages with screenshots, wishlist | Done. Buying moves to the normal window, because Steam only takes payment on its website |
| Downloads: progress, pause, resume, cancel, queue | Done |
| Friends | Done: a page per friend (join their game, invite to yours, profile) and chat with the on-screen keyboard or a real one |
| Settings | Interface size, start in Big Picture, language. Everything else moves to the normal window |

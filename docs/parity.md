# Steam parity tracker

Legend: ✅ done · 🟡 partial · ⬜ not started · 🚫 not possible natively (explained)

## Account & sign-in
| Feature | Status | Notes |
|---|---|---|
| Password sign-in with Steam Guard (email, mobile code, approve on phone) | ✅ | |
| QR code sign-in | ✅ | Verified against Steam's servers |
| Remember me / auto sign-in | ✅ | |
| Multiple saved accounts, switch account | ✅ | App menu → Switch account |
| Ask which account at startup | ✅ | Settings → Interface |
| Refresh token in the OS keychain | ⬜ | Stored in a per-user file today |
| Account details (email, phone, Steam Guard status, purchase history) | ⬜ | Account service calls |
| Store preferences (content filters, ignored tags) | ⬜ | |
| Family sharing / Steam Families | ⬜ | |

## Offline mode & connection
| Feature | Status | Notes |
|---|---|---|
| Last sign-in kept per account (name, avatar, owned apps with launch options and cloud settings, friends) | ✅ | `<config>/offline/<account id>/account.json`, written from live data only; no credentials in it |
| "Connection error" at startup with Retry / Start in offline mode | ✅ | Only for network failures (no connection, timeout, Steam down); a rejected login still goes to sign-in. Auto-login is never cleared by a network error |
| Start offline by itself when Steam can't be reached | ✅ | Toggle in the Connection error window |
| Offline mode sticks until "Go online" | ✅ | Like Steam: exit while offline, start offline next time |
| Offline badge in the top bar, Go online / Go offline in the account menu | ✅ | |
| Library and installed games offline | ✅ | Verified with Knights End through our steamclient: cold start offline, play, quit |
| Store, Community, Profile, Inventory, Workshop, Support offline | ✅ | Clean "You're offline" page with Go online |
| Friends offline | ✅ | Shown as offline with a note that their status is unknown and chat is unavailable |
| Downloads paused while offline or reconnecting | ✅ | A running download pauses itself and resumes after reconnecting |
| Reconnect after a dropped connection without signing out | ✅ | UI and running games stay; retries after 2, 5, 10, 20, 40, 60 s (then every minute); same logon id, so the new session replaces only our own old one. A connection that goes silent is noticed within about 100 s. Verified by cutting the network mid-session (reset and silent drop) |
| Maintenance / busy server log-off reconnects instead of signing out | ✅ | `ServiceUnavailable`, `TryAnotherCM` |
| Play time while offline | 🟡 | Recorded locally per session and added to the game page's Play time (with a note). Once back online (on reconnect, or at the next online start) it's sent with `Player.RecordDisconnectedPlaytime`, the call Steam's own client uses. Live test: Steam accepted 4 min of Knights End, but the profile's playtime (`Player.GetOwnedGames`) still showed the old 15 min about 5 minutes later, so it's unconfirmed whether Steam counts it |
| Steam Cloud after offline play | ✅ | Offline launch skips the download (with a note); saves changed offline become an upload (the last-sync record tells them apart from conflicts) and run by themselves once online. Verified live: a save added offline (Knights End slot 2) uploaded by itself on going online; the cloud went from 2 files / 7.8 KB to 3 files / 13.0 KB |
| Steam Cloud names for Auto-Cloud paths ending in `\` | ✅ | Fixed: rule folders are normalized (`SaveGames\` gives `SaveGames/file.sav`, as Steam names them). One test save from before the fix is still in Knights End's cloud as `SaveGames//file.sav`; deleting cloud files isn't supported yet |
| Games through our steamclient offline | ✅ | Cached user, `BLoggedOn` false, ownership kept; stats come from the local cache, stats stored offline are kept and sent when online (sending not live-tested) |
| Anti-cheat games offline | 🟡 | The official client is started in its own offline mode (`WantsOfflineMode` / `SkipOfflineModeWarning` for that account, put back after Steam closes). Needs Steam to remember the account; if Steam is already running it keeps its own state. Unit-tested, not live-tested. Many anti-cheat games need to be online anyway |
| Proxy for the Steam connection | ✅ | `STEAMINRUST_PROXY=http://host:port` (HTTP CONNECT); the rest of the app follows `HTTPS_PROXY` |

## Library
| Feature | Status | Notes |
|---|---|---|
| Owned games (all licenses via PICS) + installed games | ✅ | |
| Sidebar list, search, filters, sort by name/recent/hours/size | ✅ | |
| Library home: continue playing, recent shelf, grid | ✅ | |
| Game page: hero, logo, play bar, details, launch options | ✅ | |
| Launch games, show "playing" to friends | ✅ | DRM-free; see [compatibility](steam-compatibility.md) |
| Stop a running game | ✅ | Ends the whole process tree |
| Verify game files | ✅ | Against Steam's depot manifests (SHA-1) |
| Uninstall | ✅ | With confirmation |
| DLC list with on/off toggles | 🟡 | Saved; applied by the download manager |
| Custom launch options per game | ✅ | |
| Game news / patch notes on game page | 🟡 | API wired; UI in progress |
| Add a non-Steam game | 🟡 | In progress |
| Favorites, hidden games, collections | 🟡 | Favorites/hidden in progress; dynamic collections ⬜ |
| Achievements | ✅ | Native page per game: filter, sort by rarity, compare with a friend (Player service) |
| Cloud save status | ⬜ | |
| Game properties: betas, language, compatibility tool | ⬜ | |
| Install / update (depot downloader) | ⬜ | **Next big milestone** |

## Downloads
| Feature | Status | Notes |
|---|---|---|
| Downloads page with update queue and recently updated | ✅ | |
| Download settings (region, bandwidth, schedule, while playing) | 🟡 | Saved; used by the downloader |
| Actual downloading, pause/resume, reorder queue | ⬜ | Depot downloader |
| Taskbar progress | ⬜ | With the downloader |

## Store (native, no web view)
| Feature | Status | Notes |
|---|---|---|
| Front page: carousel, specials, top sellers, new, upcoming | ✅ | |
| Search with results page | ✅ | |
| Game page: media strip, description, requirements, languages | ✅ | |
| Review score, tags, features, "Is this relevant to you?" | ✅ | IStoreBrowseService |
| Prices in your currency, text in your language | ✅ | Follows your wallet currency (even through a VPN) + UI language |
| Trailers | ✅ | Native player (`crates/video`): HLS/fMP4 streaming, Media Foundation H.264 + AAC, WASAPI audio clock; autoplays muted until you turn sound on (remembered), seek bar with buffered range, volume, quality, full screen, advances to the next item at the end, pauses when the page is hidden |
| Microtrailers on hover over store capsules | ⬜ | The player already plays Steam's microtrailer MP4s (looping, silent) |
| Wallet balance, pending funds | ✅ | Top bar; click for pending money, add funds, purchase history |
| Product key activation | ✅ | App menu → Activate a product |
| Wishlist, tag/genre browsing with filters | ✅ | Wishlist service, IStoreQueryService |
| Discovery queue, recommendations | ⬜ | |
| Price history | ✅ | IsThereAnyDeal (your own free key) + prices you've seen |
| Cart, gifts for friends | ✅ | AccountCart service, same cart as the website |
| Checkout | 🚫 | Steam only accepts payment on its website; opens in the browser |
| User reviews (read/write) | ⬜ | |

## Community, friends & chat
| Feature | Status | Notes |
|---|---|---|
| Friends list with live status and in-game info | ✅ | |
| 1:1 chat with history, typing indicator, unread counts | ✅ | |
| Undocked Friends & Chat window | ✅ | Works while the main window is in the tray |
| Profiles: background, avatar frame, bio, level | ✅ | |
| Profile links: games, inventory, friends, badges, screenshots, reviews, groups | ✅ | Games and inventory native; private/friends-only profiles respected |
| Friend requests: accept / ignore | ✅ | Community page; add by name still to do |
| Group chats, voice chat | ⬜ | |
| Notifications bell (all 30 Steam types), toasts | ✅ | SteamNotification service; game art / avatars, opens the right page |
| Inventory: every game, per-game tag filters, rarity colors | ✅ | Econ service; game list from Steam's inventory data |
| Market prices, order book, price chart, estimated value | ✅ | In your wallet currency |
| Sell / quick sell / sell to highest buy order, your listings, buy orders | ✅ | Steam's real fee rules per currency; confirm in Steam Mobile app |
| Trade offers | 🟡 | Opens Steam's trade page; native trading next |
| Steam Support: your requests and replies, per-game help | ✅ | New requests and replies open Steam's help site |
| Community hub per game: popular this week, screenshots, artwork, videos, news | ✅ | PublishedFile.QueryFiles; mature content left out |
| Workshop item page: pictures/videos, description, required items, change notes, stats, tags, comments | ✅ | PublishedFile.GetDetails / GetChangeHistory; subscribe, unsubscribe |
| Rate and favorite Workshop items and guides | 🟡 | Buttons work through Steam's services; not tested on a real account (they write to it) |
| Guides: list with Steam's sorts, time ranges, categories, search; guide reader with index | ✅ | Sections read from the guide page |
| Discussions: forums, topic list with paging, threads with pages, quotes, spoilers | ✅ | Reading only; replying and new topics open nowhere yet |
| Someone's screenshots, artwork, videos, Workshop items, guides (profile links) | ✅ | PublishedFile.GetUserFiles |
| Profile comments: all pages | ✅ | Older / newer pages instead of Steam's "all comments" page |
| Badge details, someone's friends list, reviews, groups | ⬜ | Still open Steam's pages |
| Community Market browsing, market history | ⬜ | Still open Steam's pages |

## App
| Feature | Status | Notes |
|---|---|---|
| Tray icon with menu, close to tray | ✅ | |
| Shift+Tab overlay (friends, quick chat, notes) | ✅ | No injection; windowed/borderless games |
| Overlay: guides, discussions, community hub, store page, achievements in its own windows | ✅ | Nothing opens the browser; links inside go back and forth in the window |
| English / Russian, auto-detected | ✅ | `crates/i18n/locales` |
| Themes, accent color, UI scale | ✅ | |
| Plugins (sandboxed, permissioned) | ✅ | Rhai; see `crates/plugins` |
| Plugin network access with per-domain permission | ⬜ | Unlocks plugins like OpenDota stats or price history |
| Big Picture / controller UI | ⬜ | |
| Remote Play, Steam Input | ⬜ | |

## Community-requested ideas we plan to include
From r/Steam threads, Steam Client Beta discussions and popular add-ons
(SteamDB extension, Augmented Steam, Millennium plugins):
- Queue all pending updates at once; drag to reorder downloads; per-weekday schedules.
- Library filters by language, accessibility, Deck status, hours, last played.
- Price history and lowest price on store pages (plugin with network permission).
- Player counts and latest game updates on store/game pages.
- Lower memory use and fast startup (native Rust UI already helps here).

## Recently added
| Feature | Status | Notes |
|---|---|---|
| Steam Cloud: per-game toggle, sync before play / after quit, conflicts, usage | ✅ | Cloud service; downloads verified live, local backups before replacing |
| Library collections, Steam's Favorites and Hidden imported | ✅ | CloudConfigStore (read); edits aren't written back to Steam yet |
| Right-click game menu with Properties, Manage, collections | ✅ | |
| Workshop: game's own tag groups, 6 sorts, batch unsubscribe, size filter | ✅ | |
| Profile: two columns, badges and level, comments (read/post), item showcase | ✅ | Posting comments not live-tested |
| Steam-style sign-in window, custom title bar, custom tray menu, app icon | ✅ | |
| Shift+Tab overlay redesign (dimmed game, top strip, floating windows, invites) | ✅ | Invites send a play link; no injection |
| Desktop shortcut removed on uninstall | ✅ | |
| Native community pages instead of the browser (hub, Workshop items, guides, discussions, achievements) | ✅ | Steam links inside posts and guides open natively; other websites in the browser |

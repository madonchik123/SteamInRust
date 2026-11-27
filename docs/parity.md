# Steam parity

How much of the official Steam desktop client SteamInRust covers, area by
area. Last checked against the code on 21 November 2026 (v0.14.0 and the community actions after it).
area. Last checked against the code on 21 November 2026 (v0.14.0 and the dynamic collection changes after it).

Scores: **0** not started · **0.25** started · **0.5** usable, big parts
missing · **0.75** mostly there · **1** complete. Anything marked "opens
Steam's website" still works; it just isn't native yet.

| Area | Score | Main gaps |
| --- | ---: | --- |
| Sign-in and accounts | 0.75 | Changing email, password or phone opens Steam's website |
| Offline mode and reconnecting | 1 | Whether Steam counts playtime recorded offline is unconfirmed |
| Library | 0.75 | Dynamic collections can be made and edited here, but saving them hasn't been tried on a real Steam account yet |
| Downloads and installs | 1 | |
| Store | 1 | Checkout happens on Steam's website (Steam requires it) |
| Friends and chat | 0.75 | Voice chat is built but hasn't been tried in a real call; no polls in group chats |
| Community | 0.75 | No Showcase Manager. New discussions, joining, leaving and group invites are in the app but haven't been tried on a real account |
| Inventory and Market | 0.75 | Buying, buy orders, gems, booster packs and pending gifts are in the app but haven't been tried on a real account |
| Steam Cloud | 1 | |
| Steam features inside games | 0.75 | Networking sockets / messages through Valve's relays (strangers on the official client); parts of hosting game servers aren't proven yet (login tokens, players connecting over networking sockets) |
| Shift+Tab overlay | 0.75 | Drawn inside D3D11 and D3D12 games on Windows (Vulkan, OpenGL and D3D9 games, and every game on a Mac, get a window overlay); no in-game web browser. F12 screenshots and the FPS counter only in games it's drawn inside |
| Notifications and tray | 1 | |
| Settings, languages, updates | 0.75 | No Linux version; Mac builds aren't signed with an Apple developer ID |
| Big Picture | 0.75 | Buying, trade offers and some settings move to the normal window |
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
| Steam Families: members, roles, cooldowns, invites, shared library with owners | Done, native (FamilyGroups service) |
| Steam Families: invite and cancel, accept (with Steam's phone or email confirmation) and decline, remove a member, leave | Done, native (FamilyGroups service), following the calls Steam's own family page makes. The rules and cooldowns show first, and accepting, removing and leaving need an "I understand" tick. **Not tried on a real account**, because these can lock an account out of a family for a year: requests and replies are unit-tested and the screens were checked on the demo account |
| Steam Families: creating, renaming or deleting a family, its history, a child's playtime, game, store and community limits | Open Steam's website. Limits aren't part of the FamilyGroups service (they're Steam's separate Parental service) |
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
| Which systems a game runs on: a **Runs on** filter, a game-page line, and "Not available on macOS" for Windows-only games on a Mac without CrossOver or Wine | Done |
| Steam Play on a Mac: Windows games through CrossOver or Wine, **Settings → Compatibility** (tools found, default, "all other titles") and **Properties → Compatibility** ("Force the use of a specific Steam Play compatibility tool") | Built. Big Picture too (button states, reason, Library settings). Windows-only games without a runner offer to install Wine (pinned, checksummed download) and Rosetta 2. **Screens not seen on a Mac yet** |
| Search, filters (genre, features, Steam Deck status, installed, played), sort by name, recent, hours, size | Done |
| Home with recent games and shelves, game page with art, play bar, news and achievements | Done |
| Collections, including your Steam Favorites and Hidden | Done, in step with Steam both ways. Edits made before Steam answers (or offline) are kept and sent once it does |
| Dynamic collections | Shown with their games worked out from Steam's filters, including games added or removed by hand. Made from the library's filters (**Create Collection**) or edited here (right-click one, or its filter button), saved in Steam's own format. Only what you change is rewritten, and a save is refused if Steam's copy changed meanwhile. Filters this app can't write back exactly stay as they are: you can rename that collection, and change its filters in Steam |
| Dynamic collections: how far they're tested | The format is unit-tested byte for byte on saved samples, and on read-only copies of the 23 collections (1 dynamic) cached on the test PC. The editor was tried in the demo account. **Not tried live: saving to a real Steam account, or seeing a collection made here in the official client** |
| Dynamic collection filters | Checked: play state, features, Steam Deck, Steam Machine and Steam Frame status, genres and store tags, store categories, languages, EA Play, and friends who own a game. Not checked, so a collection can show a few extra games: private games, Steam OS compatibility, friends whose game lists are private, and a game borrowed from the friend you filter on. Steam's fuzzy name search and its search suggestions aren't copied, so a text filter can show fewer games. Library filters Steam has no equivalent for (such as Not installed or Type) are named in the editor and left out |
| Non-Steam games: the same right-click menu as Steam games, collections, a game page with the play bar, and Steam's shortcut properties (name, target, start in, launch options, icon) | Done. Games added in Steam (`shortcuts.vdf`) show up too |
| Custom artwork for any game, like Steam's: icon, cover, background (hero), logo with its position, and wide header, picked from files on disk | Done. Steam's own custom art in `config/grid` shows up too |
| Install states everywhere: list row with progress, game page, right-click menu, Properties | Done |
| Properties: general, launch options, language, updates, installed files, versions and betas (with private beta codes), DLC, Steam Cloud, Workshop, controller, privacy | Done |
| Verify files, uninstall, desktop shortcuts, browse local files (also while installing) | Done. Uninstalling also removes the game's Workshop items and unused download manifests, like Steam |
| Move a game to another library folder (Properties → Installed files, the game's Manage menu, Settings → Storage): several games at once, space needed and free, progress with Cancel | Done. Files are copied, checked by hash, and the originals deleted only after the switch; a move cut off by a crash is finished or undone at the next start. Workshop items, the Proton prefix and shader cache move too. Tested only on made-up libraries in a temp folder, never on a real game, a second physical drive, or with Steam's own app open (it may need a restart to see the new location). Moving within one drive still copies, so it needs the free space |
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
| Cart (the same cart as the website), Steam's "Added to your cart" popup, discount price tags, product keys, wallet balance | Done. Each copy is for your account or a gift to a friend you pick, then one Checkout button |
| Points Shop: your points, items by category (avatars, frames, backgrounds, mini-profile backgrounds, stickers, chat effects, emoticons…) and items from your games. Get items with your points in the app: a large preview (items that move play), whether you own it, a confirmation with the item and its cost, Steam's errors in plain words, then an offer to wear it (avatar, frame, background, mini-profile) | Done, with Steam's own `LoyaltyRewards` calls. **Redeeming was not tried live**: no points were spent on a real account, so it's covered by unit tests and the demo account only; reading the real balance and owned items wasn't tried live either. Bundles and seasonal badges still open Steam's website. Videos in H.265 (every startup movie checked and a few backgrounds) can't play in the app: the item's window says so and links to Steam's page |
| Bundle and package pages: what's included, what you already own, the bundle discount, "Complete your collection" and add to cart. Game pages list their bundles and editions, and Steam's bundle, package and curator links open in the app | Done. **Adding to a real cart wasn't tried live**: it's covered by unit tests and the demo account. The "Complete your collection" price is worked out with Steam's rounding and marked as approximate, because Steam's cart sets the final price |
| Curators: the ones you follow, the most followed, a curator's page with its reviews, follow and unfollow, and "Recommended by curators you follow" on game pages | Done. **Following and unfollowing weren't tried live**, and neither was reading a real account's follows: they're covered by unit tests and the demo account |
| Discovery feed: the queue as a feed, "New picks" for a fresh queue | Done. "New picks" wasn't tested on a real account |
| Search suggestions while you type (picture, name, price), with the arrow keys and Enter | Done |
| Wishlist in your order: drag games by the handle or type a place, saved to Steam, when sorted by "Your rank" | Done. Saving wasn't tested on a real wishlist |
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
| One Friends & Chat window, like Steam's: the friends list docked on the right, the open chat beside it on the left, conversations switched inside it. It works while the main window is in the tray | Done. It replaced the friends panel, the hover preview and the friends page in Community |
| Right-click menu: join game, view server info (live A2S query), watch game, view game info, invite to play, nickname, favorites, remove, block | Done |
| Add friends: friend code, Quick Invite links (make, copy, or open someone's), search profiles by name; accept or ignore requests | Done |
| Group chats: your groups with unread marks, rooms, history, sending, live messages, who's online in a members list | Done, in the app. Polls aren't there yet |
| Chat like Steam's: Steam emoticons and emoji with a picker (recent, emoticons, stickers, effects, search), slash commands (`/me`, `/code`, `/pre`, `/quote`, `/spoiler`, `/random`, `/flip`, `/store`), reactions, a menu on each message ("⋮" or a right-click: copy, react, quote, report), message text you select with the mouse and copy with Ctrl+C, a right-click menu on links (open, open in browser, copy link), links shown whole (never cut with "…"; a link whose text says something else shows its real address under it), cards for Store, Workshop and community links, pictures shown in the chat and opened large, MP4 videos played in the chat. Pictures and videos from sites other than Steam wait behind "Load preview" (loading them shows that site your IP address). Sending pictures: "+", Ctrl+V or dropping a file, each shown first with Send and Cancel | Done. Sending pictures goes through Steam's own chat upload but **wasn't tried live** (it would post a real picture) |
| Sending more than text: a picture, your latest screenshot, your trade offer link; inviting a friend into the conversation | Done. These weren't tried on a real account |
| Group chat management, like Steam's: notifications per group and per channel, invite links and inviting friends, the settings window (name, tagline, icon, channels, roles and what they allow, invite links, invited accounts, bans, leave), a member's right-click Manage menu (roles, make admin, kick, ban), making and deleting group chats | Done. Everything you're not allowed to do is greyed out with the reason. Voice channels can be made and joined |
| Voice chat: one-to-one calls and group voice channels with join, leave, answer, decline, mute, deafen, who's talking, volume per person, microphone and speaker choice, and push-to-talk | Built, **not yet proven in a real call**. Proven live: Valve's voice server accepts our encrypted connection and Steam confirms it (nobody was called, no channel joined, no audio sent). Proven only with stand-ins: sending, receiving and mixing voice. Not tried: a real microphone or speakers, a call with someone on the official client, a real voice channel, and macOS audio |

## Community

| | |
| --- | --- |
| Post a status for your friends from Friends activity, tagged with a game, like Steam's activity page | Done. Posting wasn't tried on a real account |
| Community home with three sections: **Friends activity** (Steam's activity feed: screenshots, artwork, reviews, achievements, new games, posts), **Explore** (artwork, screenshots, broadcasts, guides, Workshop) and **Developers** (news from games you own) | Done |
| Profiles: background (animated ones play), avatar frame, level, badges, showcases, comments (read and post), Steam's profile themes | Done. Animated avatar frames show as still pictures |
| On someone's profile: Add friend (or accept, cancel, remove), Message, and a More menu (copy the link, view on Steam, inventory, remove friend, block, report on Steam) | Done. Adding, removing and blocking weren't tested on a real account |
| Edit your profile: General, Avatar (animated avatars and frames), Profile background (full screen or original size), Mini profile, Theme, Featured badge, and Steam's full Privacy page with its checkboxes | Done, with a preview before saving. The Showcase Manager isn't there yet. Saving wasn't tested on a real account |
| Someone's games, inventory, screenshots, artwork, videos, Workshop items and guides | Done |
| Screenshot manager (View menu, a game's page, the in-game "Screenshot saved" notice): your screenshots per game, on this PC and uploaded, a large viewer, caption, spoiler tag, visibility, upload, delete from this PC or from Steam (always asked first), show on disk, copy | Done, tested on made-up screenshots with the demo account. **Never run against Steam:** the uploaded list, uploading, changing an uploaded screenshot and deleting from Steam, so no real screenshot was touched. They follow Steam's published message formats. Also not tried: clicking the notice in a real game, a real `screenshots.vdf`, and Steam's own app being open (it may overwrite captions edited here). Non-Steam games' screenshots can't be uploaded |
| Community hub per game: screenshots, artwork, videos, news | Done |
| Watching broadcasts | Native page with the app's own player. Opens from Community → Broadcasts, a game's hub, a "live now" strip on store pages, a friend's "Watch game" and broadcast links. Quality (or automatic), volume, full screen, pause and "Back to live", title, broadcaster, game, viewer count, and the chat beside it (read-only: you can't post). Played with public broadcasts on Windows. Not tried yet: a Mac, and friends-only or private broadcasts. Steam gives one answer for ended, private and friends-only, so the app says it may be any of them |
| Workshop item pages: media, description, required items, change notes, comments; subscribe, rate, favorite | Done. Rating and favoriting weren't tested on a real account |
| Guides: list, sorts, search, reader with contents | Done |
| Discussions: forums, topics, threads with quotes and spoilers; reply to a thread | Done, in the app. Sending a reply wasn't tested on a real account |
| Start a new discussion in a game hub or group forum: title, text, the forum's formatting buttons, a preview, which section to post in, and Steam's errors shown in the box | Done, in the app. **Not tried live:** nothing was posted on a real account. The request and Steam's answers are tested against saved samples, and the screens on the demo account |
| Steam Support: your requests and replies | Done. New requests open Steam's help site |
| Someone's friends (online first, searchable), their latest reviews, their groups, and the badges list | Done, in the app |
| Group pages: name, headline, member counts, description, announcements, members (friends first), a button to the group's chat, and the group's own discussions | Done, in the app, also for steamcommunity.com/groups links |
| Join or leave a group from its page: open groups, request-to-join groups and invitation-only groups, with the button showing where you really stand. Leaving asks first | Done, in the app. **Not tried live:** no group was joined or left on a real account, and the test account is in no groups, so reading your real standing from Steam's friends list hasn't been seen working either |
| Accept or ignore group invites, on the group's page and under Invites in Friends | Done, in the app. **Not tried live:** no real invite was answered. The message is built from a public reference and unit-tested |

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
| Buy from the Market: the cheapest listing, with the seller's price, Steam's fee, the game's fee, any tax, the total and your wallet after it, then a confirmation | In the app. **Not tried on a real account**: no real purchase was made. The requests follow Steam's current Market pages and are tested against saved samples. If Steam needs a billing address first, it sends you to its website |
| Buy orders: price, quantity, the most you can pay, a confirmation; cancel them in your lists | In the app. **Placing one wasn't tried on a real account**. Approval in the Steam Mobile app is handled, also untried |
| Gems: turn an item into gems (shows what you get, and that the item is destroyed), pack gems into sacks, open sacks | In the app. **Not tried on a real account**: nothing was ground, packed or opened |
| Booster packs: which you can make, their gem cost and when they're next available, make one, open one and see the cards | In the app. **Not tried on a real account**. A pack that would have to use gems you can't trade opens Steam's page, which asks how you want to pay |
| Pending gifts: see them, add to your library, keep in your inventory, or decline with a note | In the app. **Not tried on a real gift**. The list is read from Steam's inventory page, and that part of the page couldn't be seen without a real gift, so a gift's name or picture may be missing (its buttons still work from Steam's own ids) |

## Steam Cloud

| | |
| --- | --- |
| Per-game toggle, sync before launch and after quit, usage | Done |
| Conflicts, with a local backup before anything is replaced | Done. The question brings the window forward even when a game starts from a desktop shortcut while SteamInRust is in the tray |
| Saves changed offline upload by themselves once online | Done |

## Steam features inside games

Details: [steam-compatibility.md](steam-compatibility.md).

| | |
| --- | --- |
| Games without anti-cheat use SteamInRust's own `steamclient` DLL, 64-bit and 32-bit | Done |
| The same on a Mac: one `steamclient.dylib` for Apple and Intel games (Intel games run under Rosetta) | Done. Tested with Human: Fall Flat: Workshop, stats, and joining a stranger's lobby on the official client |
| Games with VAC, Easy Anti-Cheat or BattlEye start through the official client, silently | Done. Our DLL is never used for them |
| Steam Play on a Mac: the Windows version of a game downloads (Windows depots, recorded as `platform_override_source` like Steam), gets a Wine prefix with Steam's redistributables, and starts through the runner with our Windows `steamclient64.dll` (localhost connection to the app, same token check). Anti-cheat games never start this way | Done with Wine Devel 11.21 on an M4 Mac (macOS 27): MineSweeper Rush (32-bit) installed, started, read stats, used leaderboards and unlocked an achievement through our DLL; a Steam Cloud file written under Wine landed in the Mac's userdata folder. Not tried: CrossOver, a 64-bit game, Auto-Cloud rules under Wine (unit-tested). See [steam-compatibility.md](steam-compatibility.md#1b-steam-play-on-a-mac-windows-games-through-crossover-or-wine) |
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
| Screenshots (`ISteamScreenshots`) into the Steam screenshot folder | Done. Uploading them is in the screenshot manager (see Community) |
| Steam Input on Xbox-style pads with the game's bindings | Done on Windows. No gyro, trackpads or binding panel; no controllers in games on a Mac yet |
| Hosting game servers (`ISteamGameServer`, `ISteamGameServerStats`), listen and dedicated | Partly. Tested on Spacewar through Valve's `steam_api`: anonymous sign-in, listed in Steam's server browser, answering server queries, Steam checking a player's ticket, reading a player's stats, and two game servers connecting over networking sockets and messages. Not proven: signing in with a real login token, saving stats, a player connecting to a server over networking sockets |

## Shift+Tab overlay

| | |
| --- | --- |
| Opens over the running game with Shift+Tab, only while the game's window is in front | Done |
| Drawn inside the game, like Steam's, so it works in exclusive full screen and the game keeps focus | Done for D3D11 and D3D12 games SteamInRust starts with its steamclient (32- and 64-bit). D3D12 tested in Knights End (Unreal, 64-bit), Grr Grr Gnomes Demo (Unity 6, 64-bit) and Mechanica (Unity 2019, 32-bit). Vulkan, OpenGL and D3D9 aren't done (no installed game without anti-cheat could test them): those games get the window overlay, which shows over windowed and borderless games only. If drawing keeps failing in a game, the overlay stops drawing there, the game carries on, and the window overlay takes over |
| On a Mac | A window over the game that also works over full-screen games. Shift+Tab needs the Input Monitoring permission; SteamInRust explains it on first start |
| While it's open the game gets no mouse, keyboard or controller input (window messages on any of its windows, including the pointer input Unity 6 uses, raw input, XInput) and can't hide or lock the cursor; it gets `GameOverlayActivated_t` | Done. DirectInput and Windows.Gaming.Input controllers still reach the game |
| Open and close with a controller: the Guide (Xbox) button, like Steam; B also closes it | Done for XInput controllers. Windows' Game Bar may open on the Guide button too unless it's turned off for controllers |
| Game overview, achievements, friends, chat, notes, guides, discussions, hub, store page | Done, in floating windows |
| Pin windows over the game with adjustable opacity | Done |
| Games with anti-cheat | They start through the official client, so they get Steam's own overlay |
| Game invites in chat show as cards with a Join button (Steam's `[gameinvite]` markup). Friend menus offer "Invite to lobby" or "Invite to game" from what the game reports, else "Invite to play", all sent as real Steam invites | Done |
| Screenshots with F12 into Steam's screenshot folder, with a "Screenshot saved" notice; FPS counter in a corner you pick (Settings → In game) | Done for games the overlay is drawn inside (D3D11 and D3D12). The frame is copied before the overlay is drawn on it; games that take their own screenshots get Steam's request instead. Tested on D3D11 in Mechanica and Grr Grr Gnomes Demo, and on D3D12 in those two and Knights End |
| In-game web browser | Not yet |

## Notifications and tray

| | |
| --- | --- |
| All Steam notification types with game art or avatars, toasts | Done |
| Tray icon with Steam's menu (drawn in the current theme), close to tray | Done |
| Pick the app icon (taskbar, window and tray) from the SteamInRust logos | Done |

## Settings, languages, updates

| | |
| --- | --- |
| Settings: account, friends, security, notifications, interface, appearance, store, library, downloads, storage, cloud, in game, accessibility, plugins | Done |
| Themes: full skins with their own colors, fonts, images, icons, control styles and layout. Built in: **SteamInRust** (the default, after [Juxtopposed's Steam Redesign](https://www.figma.com/community/file/1302616100790619521/steam-redesign)) and **Classic** (the look before it). Your own themes load from a folder ([docs/themes.md](themes.md)) | Done. People upgrading are asked once whether to try the new look. There's no theme store yet |
| Accent color, UI scale | Done |
| English, Russian, Simplified Chinese, Spanish, Brazilian Portuguese, German and French, picked from the system language | Done |
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
| Store: front page, sections, search with an on-screen keyboard (English and Russian), store pages with screenshots you can flip with the mouse wheel, wishlist | Done. Buying moves to the normal window, because Steam only takes payment on its website |
| Downloads: progress, pause, resume, cancel, queue | Done |
| Friends and chat | Done: a page per friend (join their game, invite to yours, profile), group chats with their rooms, and a full-screen chat for a controller, keyboard or mouse. Pick a message to reply with a quote, react, copy it, show a spoiler, load a picture or video from another site, open a picture large (Back closes it), play a video, open a link (its whole address shown first), open it in the browser or copy it, join a game or report it. Emoticons, stickers, chat effects, link cards and slash commands work, and you can send a screenshot or your trade link, or paste a picture with Ctrl+V or drop one on the window (shown before it's sent). Store, Workshop, community and profile links open inside Big Picture (their pages are read with up and down; their buttons need a mouse). Trade offers, voice rooms, inviting people and managing a group stay in the normal window |
| Settings | Done for what you'd change from the couch: interface size, start in Big Picture, language; downloads (auto-updates, region, speed limit, schedule, connections); library (what it opens on, Steam Cloud); notifications; controller (swap A and B, repeat speed, stick sensitivity); in-game overlay and FPS counter; theme; status, account picker, close to tray and sign out. The rest stays in the normal window because it needs typing, a file dialog or a browser: profile and privacy, security, family, wallet and purchases, Store content filters, library folders, custom colors, plugins. SteamInRust has no sounds, so there's no audio setting |

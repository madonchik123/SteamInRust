<p align="center">
  <img src="docs/media/en/banner.png" alt="SteamInRust: the Steam client, rebuilt in Rust" width="100%">
</p>

<p align="center">
  <a href="#download"><b>Download</b></a> ·
  <a href="#performance"><b>Benchmarks</b></a> ·
  <a href="docs/parity.md"><b>Parity</b></a> ·
  <a href="docs/steam-compatibility.md"><b>Games and anti-cheat</b></a> ·
  <a href="CONTRIBUTING.md"><b>Contributing</b></a> ·
  <a href="README.ru.md"><b>Русский</b></a>
</p>

<p align="center">
  <a href="https://github.com/madonchik123/SteamInRust/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/madonchik123/SteamInRust?style=flat-square&color=4F8CFF"></a>
  <a href="https://github.com/madonchik123/SteamInRust/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/madonchik123/SteamInRust/ci.yml?branch=main&style=flat-square&label=CI"></a>
  <img alt="Windows 10 and 11" src="https://img.shields.io/badge/Windows%2010%20%C2%B7%2011-4F8CFF?style=flat-square">
  <img alt="macOS 11 and later" src="https://img.shields.io/badge/macOS%2011%2B-4F8CFF?style=flat-square">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-2FB36B?style=flat-square">
</p>

# SteamInRust

**The Steam desktop client, rewritten in Rust.** Store, library, downloads,
friends, chat, inventory, Market, Steam Cloud and the Shift+Tab overlay, in one
native process with no embedded browser. It signs in to your own Steam account
and talks to Steam's servers directly. Free, open source (MIT) and unofficial.

The official client is a Chromium app: a dozen `steamwebhelper.exe` processes
drawing web pages. SteamInRust draws everything itself on the GPU with
[egui](https://github.com/emilk/egui), so its window is up in about two and a half
seconds and it idles at about 160 MB.

<p align="center">
  <img src="docs/media/en/store.png" alt="A store page in SteamInRust with the Hades trailer playing" width="100%">
</p>

## What works today

Everything below is in the current release. Area-by-area status:
[parity](docs/parity.md).

- **Store.** Front page, search, tags with filters, wishlist and cart. Game
  pages play trailers natively. Prices are in your wallet's currency, and games
  you own are marked. You can gift a game, even one you own, and SteamInRust
  checks first whether Steam will deliver it to that friend's region. Payment
  happens on Steam's website, as Steam requires.
- **Library.** Every game you own, collections (your Steam Favorites and Hidden
  included), non-Steam games, filters, achievements and full Properties:
  launch options, language, betas, DLC, Steam Cloud, verify files.
- **Downloads.** SteamInRust's own depot downloader. It has a queue, pause and
  resume, and downloads that survive a restart. It shows network and disk
  speed separately, and picks the fastest of Steam's download regions.
- **Friends and chat.** Live status with the game and rich presence, chat with
  history, and a separate Friends window. Right-click a friend to join their
  game or look up their server.
- **Community.** Profiles, comments, game hubs, Workshop pages, guides and
  discussions, all inside the app.
- **Workshop.** Browse a game's Workshop with its own tag filters. Subscribing
  downloads the item right away, and missing or updated items download before
  the game starts, like in Steam.
- **Account.** Account details, purchase history, licenses and your Steam
  Family, as native pages.
- **Inventory and Market.** Every game's inventory, market prices, order book and
  price chart. Selling uses Steam's real fees; you confirm in the Steam Mobile app.
- **Steam Cloud.** Sync before you play and after you quit, conflicts with
  local backups, and saves made offline upload once you're back online.
- **Games.** Games without anti-cheat get achievements, stats, cloud saves,
  lobbies, P2P, Workshop, inventory, the server browser, screenshots and Steam
  Input from SteamInRust's own `steamclient`, 32-bit games included. Older
  multiplayer games (like Human Fall Flat) play with friends and strangers on
  the official client.
  Games with VAC, Easy Anti-Cheat or BattlEye always start through the official
  client, silently, so their anti-cheat runs as intended
  ([how](docs/steam-compatibility.md)).
- **Overlay.** Shift+Tab in the running game opens friends, chat,
  achievements, guides and notes. Like Steam's, it's drawn inside the game
  (D3D11 games), so it works in full screen. You can pin any of them over the
  game.
- **Offline mode.** It works like Steam's. If the connection drops, SteamInRust
  reconnects without signing you out, and your running games keep going.
- **Big Picture.** A full-screen mode for the TV, driven by a controller or the
  keyboard: home, library, game pages, store with an on-screen keyboard,
  downloads and friends. Open it from the menu, with the controller's Guide
  button, or start with `--bigpicture`.
- **Plugins.** Small [Rhai](https://rhai.rs) scripts can add pages,
  buttons on game pages, items in friends' menus, timers, notifications and
  web requests to the sites they list, with only the permissions you approve.
- **Seven languages**: English, Russian, Simplified Chinese, Spanish, Brazilian
  Portuguese, German and French, picked from your system language.

<table>
  <tr>
    <td width="50%"><img src="docs/media/en/library.png" alt="The library: game grid, collections in the sidebar"></td>
    <td width="50%"><img src="docs/media/en/inventory.png" alt="Inventory: a Dota 2 item with its market price and price chart"></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/media/en/profile.png" alt="A profile: level, recent activity, most played and comments"></td>
    <td width="50%"><img src="docs/media/en/startup-race.gif" alt="SteamInRust and Steam starting side by side"></td>
  </tr>
</table>

<p align="center"><img src="docs/media/en/bigpicture.gif" alt="Big Picture: the home screen and library, driven with the keyboard" width="720"></p>

<sub>Screenshots from <code>--demo</code> mode, which uses sample account data.</sub>

## Performance

**SteamInRust shows its window about 2.5 times sooner than Steam, has your
library fully drawn about 3 times sooner, and uses about a sixth of the
memory**, on the same PC, account and network. Measured on 17 November 2026 on
a Ryzen 5 5600 with 16 GB of RAM, Windows 11, SteamInRust 0.7.1 against Steam
11.23.65.88. Both clients started on the library. Every figure is the middle
result of 3 runs:

| | Steam | SteamInRust |
| --- | ---: | ---: |
| Start → first window | 6.4 s | 2.6 s |
| Start → main window | 9.0 s | 2.4 s |
| Start → library fully drawn | 14.7 s | 4.8 s |
| Memory when idle (private bytes) | 1,019 MB | 163 MB |
| CPU when idle | 7.6 % of a core | 0.00 % |
| Processes | 8 | 1 |
| Program size | 1,084 MB | 29.8 MB |
| Open the Store | 1.33 s | 0.75 s |
| Open a game page in the library | 0.70 s | 0.05 s |
| Open Friends & Chat | 1.20 s | 0.06 s |

Start times are cold starts (nothing in the file cache). 0.7.1 also cut
SteamInRust's own idle memory from 253 MB to 163 MB and its idle CPU to zero,
compared with 0.7.0 on the same run. Method, ranges, warm starts and what was
left out: [docs/benchmarks.md](docs/benchmarks.md).

## Download

Get it from [the latest release](https://github.com/madonchik123/SteamInRust/releases/latest):

| File | What it is |
| --- | --- |
| `SteamInRust-Setup-<version>.exe` | Installer. Installs for your user only, without administrator rights, into `%LOCALAPPDATA%\Programs\SteamInRust`, and updates itself |
| `SteamInRust-<version>-portable.zip` | The same app with no installer. Doesn't update itself |
| `SHA256SUMS.txt` | Checksums for both |
| `SteamInRust-<version>-macos.dmg` | macOS (Apple Silicon and Intel games, macOS 11 or later). Drag SteamInRust to Applications; it updates itself |
| `SHA256SUMS-macos.txt` | Checksum for the disk image |

Needs Windows 10 or 11 (64-bit), or macOS 11 or later on Apple Silicon, and a
Steam account. Sign in with your password and Steam Guard, or scan the QR code
with the Steam Mobile app. Windows SmartScreen may warn about a new unsigned
app: **More info → Run anyway**. Uninstall from **Settings → Apps**.

On macOS the app isn't signed by an Apple developer account, so the first time
macOS says it can't check it for malicious software. Open it once with
**right-click → Open → Open**, or allow it in **System Settings → Privacy &
Security → Open Anyway**. On a Mac, games without a macOS version show as not
available, like in Steam, and new installs go to Steam's usual folder
(`~/Library/Application Support/Steam`) unless you pick another. To remove it,
drag SteamInRust from Applications to the Bin.

## Is it safe to use?

- **Your account.** SteamInRust signs in the way the official client and the
  Steam Mobile app do. Your sign-in stays on your PC, in your user's app data
  folder, and is only sent to Steam.
- **Anti-cheat.** SteamInRust never touches anti-cheat. Games that use it
  always run through Valve's client.
- **Other connections.** Apart from Steam, the app only contacts GitHub (for
  updates) and an exchange-rate service (when you check a gift's region). If
  you add your own key, it also contacts IsThereAnyDeal (price history). There
  is no telemetry.
- **Not official.** SteamInRust isn't made or approved by Valve.

## Plugins

A plugin is a folder with a manifest and a script. This one adds a page that
counts your online friends:

```toml
# plugins/friends-online/plugin.toml
id = "friends-online"
name = "Friends online"
version = "1.0.0"
permissions = ["friends.read"]

[page]
title = "Friends online"
icon = "users"
```

```rust
// plugins/friends-online/main.rhai
fn page() {
    let online = friends().filter(|f| f.status != "Offline");
    [
        #{ type: "heading", text: "Friends online" },
        #{ type: "stat", label: "Right now", value: online.len().to_string() },
    ]
}
```

Scripts can't reach files or other programs, only the API they were granted.
The network is opt-in per plugin: `network` plus a `hosts` list in the manifest
(https only), which you see before enabling it. Plugins can also run on a
timer (`tick`), add buttons to game pages and items to friends' menus, launch
installed games, read downloads and the wishlist, and draw inputs, switches,
tables and images on their page. The API is in
[`crates/plugins`](crates/plugins/src/lib.rs), with examples in
[`crates/plugins/examples`](crates/plugins/examples) (Wishlist Sales checks
your wishlist's prices every hour and tells you about discounts).

## Build from source

Needs [Rust](https://rustup.rs). On Windows also MinGW-w64: the project uses the
GNU toolchain there (see `rust-toolchain.toml`). On macOS, install the Xcode
command line tools (`xcode-select --install`), then tell rustup to use the Mac's
own toolchain in the project folder, once:

```sh
rustup override set stable
cargo xtask steamclient-mac  # the steamclient games load, for Apple and Intel games
```

```sh
git clone https://github.com/madonchik123/SteamInRust.git
cd SteamInRust
cargo run                  # sign in with your account
cargo run -- --demo        # sample data, no sign-in
cargo test --workspace
cargo xtask steamclient32   # the DLL for 32-bit games (needs a 32-bit MinGW)
cargo xtask package        # installer and portable zip (macOS: a .dmg) in target/package/dist
```

<details>
<summary>How the code is laid out</summary>

```
apps/desktop     the app: window, sign-in, overlay, tray, updater, pages (builtin/)
apps/installer   installer, updater and uninstaller
xtask            release packaging (cargo xtask package)
crates/
  client         the engine: commands in, events out; PICS, chat, cloud, market
  net            connection to Steam: WebSocket, framing, jobs, service calls
  auth           sign-in: RSA password, QR, Steam Guard, refresh tokens
  depot          depot downloader
  steamclient    the steamclient DLL that games load
  store          store APIs: browse, reviews, tags, trailers, news
  video          trailer player: HLS; Media Foundation and WASAPI (Windows),
                 VideoToolbox and AudioToolbox (macOS)
  local          the Steam folders on disk: libraries, manifests, verify
  plugins        Rhai plugins
  extensions     the interface every page implements
  i18n           translations (locales/ru.toml)
  ui             theme, fonts, widgets
  proto          Steam's .proto files, compiled with protox
  vdf, model     KeyValues parser, shared types
```
</details>

## Contributing

Issues and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md)
first. Pull request titles follow Conventional Commits (`feat: …`, `fix: …`),
since that's how releases and the changelog are made. Areas that still need
work are in [docs/parity.md](docs/parity.md).

## Project status

| Area | Score |
| --- | ---: |
| Sign-in and accounts | 0.75 |
| Offline mode and reconnecting | 1 |
| Library | 0.75 |
| Downloads and installs | 0.75 |
| Store | 0.75 |
| Friends and chat | 0.75 |
| Community | 0.5 |
| Inventory and Market | 0.75 |
| Steam Cloud | 1 |
| Steam features inside games | 0.75 |
| Shift+Tab overlay | 0.75 |
| Notifications and tray | 1 |
| Settings, languages, updates | 0.75 |
| Big Picture | 0.75 |
| Remote Play | 0 |

0 is not started, 1 is complete. What's missing in each area:
[docs/parity.md](docs/parity.md).

## Credits

Fonts: [Inter](https://rsms.me/inter/) (OFL). Icons: [Phosphor](https://phosphoricons.com) (MIT).
Protocol definitions: [SteamDatabase/Protobufs](https://github.com/SteamDatabase/Protobufs).
Price history: [IsThereAnyDeal](https://isthereanydeal.com).

## License

[MIT](LICENSE). Steam and the Steam logo are trademarks of Valve Corporation.
SteamInRust is not affiliated with, endorsed or sponsored by Valve.

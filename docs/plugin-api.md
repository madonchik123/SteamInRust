# Plugin API

Plugins are folders in the app's `plugins` folder (Settings → Plugins → Open
plugins folder):

```text
plugins/friend-alerts/
  plugin.toml   id, name, version, permissions, optional page
  main.rhai     the script
```

Scripts are written in [Rhai](https://rhai.rs). They run in a sandbox:

- no file, network or process access, only the functions below;
- every function checks the permissions you approved;
- CPU time, recursion, string, array and map sizes are capped;
- a plugin that fails only stops itself, never the app.

The code is in [`crates/plugins`](../crates/plugins/src/lib.rs). The examples
are in [`crates/plugins/examples`](../crates/plugins/examples).

## The manifest

```toml
id = "friend-alerts"          # letters, digits, - and _
name = "Friend Alerts"
version = "1.0.0"
author = "You"
description = "Tells you when friends come online"
permissions = ["friends.read", "notifications"]
hosts = ["api.example.com"]   # only with "network"
tick = 60                     # seconds between on_tick() calls, at least 10

[page]                        # optional: the plugin gets its own page
title = "Friend Alerts"
icon = "users"
bigpicture = true             # optional: also show the page in Big Picture
```

`icon` is one of `bell`, `dice`, `chat`, `star`, `chart`, `trophy`, `fire`,
`clock`, `users`, `note`, `globe`, `controller`, `lightning`, `gear`, `swap`,
`backpack`, `coins`, `tag`, `palette`, `store`, `download`, `books` or `search`
(anything else shows a puzzle piece).

The plugin only starts after you approve every permission it lists. If a new
version asks for more, it stops and asks again. Turning a plugin off takes all
its permissions away.

## Permissions

| Permission | What it allows |
|---|---|
| `account.read` | See your profile name and SteamID |
| `friends.read` | See your friends and what they're playing |
| `library.read` | See your games and playtime |
| `achievements.read` | See your achievements |
| `chat.read` | Read incoming chat messages |
| `chat.write` | Send chat messages as you |
| `status.write` | Change your online status |
| `notifications` | Show notifications |
| `storage` | Save its own settings |
| `network` | Connect to the websites it lists |
| `games.launch` | Start your installed games |
| `downloads.read` | See your downloads |
| `downloads.write` | Pause, resume, cancel and reorder your downloads |
| `store.read` | See your wishlist, and Steam store pages and prices |
| `clipboard` | Copy text to your clipboard |
| `inventory.read` | See your inventory and items' market prices |
| `trades.read` | See your trade offers |
| `trades.write` | Decline or cancel trade offers, and suggest trades for you to review |

`chat.read`, `chat.write`, `status.write`, `network`, `downloads.write` and
`trades.write` are shown with a warning when you enable the plugin.

## Checking the API version

`api_version()` (no permission) returns `4` in this version. It didn't exist in
version 1, so check like this:

```rhai
let v = try { api_version() } catch { 1 };
if v >= 2 { /* game_info, achievements, store data... */ }
if v >= 3 { /* timers, storage helpers, http_fetch, on_unload... */ }
if v >= 4 { /* Big Picture pages, the new widgets */ }
```

| Version | Added |
|---|---|
| 1 | Everything not listed below |
| 2 | `api_version`, library info, achievements, download control, store data, inventory and trade helpers |
| 3 | Timers, storage helpers, `http_fetch` and `http_cancel`, response headers, `on_unload`, `plugin_info`, `log_warn` / `log_error`, install events |
| 4 | Pages in Big Picture (`bigpicture = true`, `bigpicture_page()`, `view_mode()`), the widgets badge, metric, card, section, grid, select, slider, tabs, avatar, empty_state, loading, chart, sparkline and tooltip, and the hooks `on_select` and `on_slider` |

## Functions

Functions that read data Steam has to send first return `()` while it loads.
Call them again later (from `page()`, `on_tick()` or the matching event).

### Basics

| Function | Permission | Returns |
|---|---|---|
| `log(text)` | none | Writes to the plugin's log (Settings → Plugins → Show log) |
| `log_warn(text)`, `log_error(text)` | none | The same, shown in yellow or red |
| `now()` | none | Unix seconds |
| `api_version()` | none | The API version (4) |
| `view_mode()` | none | Where the page is being shown: `"window"` or `"bigpicture"` |
| `plugin_info()` | none | `#{id, name, version, author, permissions, hosts}` from your manifest |
| `language()` | none | The app's language code ("en", "ru"...) |
| `platform()` | none | `"windows"`, `"macos"` or `"linux"` |
| `currency()` | none | Your wallet's currency code ("USD"...) |
| `json_parse(text)`, `json_string(value)` | none | JSON to Rhai values and back |
| `notify(text)` | `notifications` | Shows a notification |
| `copy(text)` | `clipboard` | Copies text |
| `navigate(page, arg)` | none | Opens `"store"`, `"library"`, `"community"`, `"profile"`, `"downloads"`, `"settings"` or `"plugin:<id>"` |

### Storage

Each plugin has its own saved data: up to 200 keys and 1 MB in all.

| Function | Permission | Returns |
|---|---|---|
| `storage_get(key)`, `storage_set(key, value)` | `storage` | Values are saved as text (at most 16 384 characters; longer ones are cut) |
| `storage_set_json(key, value)`, `storage_get_json(key)` | `storage` | Saves maps, arrays, numbers and so on as they are, and gives them back the same. Too big a value is refused, not cut. `()` for a missing key, or one saved as plain text |
| `storage_has(key)` | `storage` | `true` if the key is saved |
| `storage_delete(key)` | `storage` | `true` if there was something to delete |
| `storage_keys()` | `storage` | All keys, sorted |
| `storage_clear()` | `storage` | Deletes everything this plugin saved |
| `storage_size()` | `storage` | Bytes used (out of 1 MB) |

### Timers

No permission needed. A plugin can have 20 timers at once. Each is at least 5
seconds and at most 30 days away, and all of them stop when the plugin does. A
due timer calls `on_timer(id)`.

| Function | Returns |
|---|---|
| `schedule_in(seconds)` | A timer id. It runs once, `seconds` from now |
| `schedule_every(seconds)` | A timer id. It runs every `seconds` (runs missed while the PC slept are skipped, not piled up) |
| `schedule_at(unix_seconds)` | A timer id. It runs once at that time |
| `cancel_timer(id)` | `true` if the timer was still there |
| `timers()` | `[#{id, due, every}]` |

`tick` in the manifest still works. Timers are for when you need several, or
need to start and stop them.

### You and your friends

| Function | Permission | Returns |
|---|---|---|
| `me()` | `account.read` | `#{steam_id, name, status}` |
| `friends()` | `friends.read` | `[#{steam_id, name, status, game}]` |
| `set_status(s)` | `status.write` | `"online"`, `"away"`, `"busy"` or `"invisible"` |
| `send_message(steam_id, text)` | `chat.write` | Only to people on your friends list |

SteamIDs are strings, because they don't fit Rhai's numbers safely.

### Library

| Function | Permission | Returns |
|---|---|---|
| `library()` | `library.read` | `[#{app_id, name, installed, playtime_minutes}]` (unchanged from version 1) |
| `game_info(app_id)` | `library.read` | A game map (below), or `()` if it isn't in your library |
| `installed_games()` | `library.read` | Game maps of installed games |
| `recent_games(count)` | `library.read` | Up to `count` (1 to 100) game maps, last played first |
| `launch(app_id)` | `games.launch` | Starts an installed game |

A game map has:

- `app_id`, `name`, `type` ("game", "application"...)
- `installed`, `owned` (on your account, not just found on disk)
- `playtime_minutes`, `playtime_2weeks`, `last_played` (Unix seconds)
- `size_on_disk` (bytes), `last_updated` (Unix seconds), `update_required`
- `genres` and `categories`: Steam's id numbers (category 22 is achievements,
  23 Steam Cloud, 30 Workshop)
- `deck`: `"verified"`, `"playable"`, `"unsupported"` or `"unknown"`
- `platforms`, like `["windows", "macos"]` (empty if not known)
- `store_url`
- `art`: `#{header, capsule, hero, logo, icon}` picture URLs

Values that aren't known are `()`. For example, the size of a game that isn't
installed, or playtime from your profile before it has loaded. Developer,
publisher and release date come from the store: see `app_details`.

### Achievements

| Function | Permission | Returns |
|---|---|---|
| `achievement_progress(app_id)` | `achievements.read` | `#{unlocked, total, percent}`, or `()` while loading |
| `achievements(app_id)` | `achievements.read` | The game's achievements, or `()` while loading. Empty if the game has none |
| `achievement_info(app_id, name)` | `achievements.read` | One achievement by its name, or `()` |

An achievement is a map: `name`, `description`, `icon`, `icon_gray` (picture
URLs), `hidden`, `unlocked`, `unlock_time` (Unix seconds, 0 if locked) and
`global_percent` (share of players who have it).

The `achievement_unlocked` event comes when the game's list reloads after you
unlocked something: when the game closes, or when you open its achievements.
It doesn't come live during play.

### Downloads

| Function | Permission | Returns |
|---|---|---|
| `downloads()` | `downloads.read` | Running downloads, then the queue in order |
| `pause_download(app_id)` | `downloads.write` | Pauses a running download |
| `resume_download(app_id)` | `downloads.write` | Resumes a paused one |
| `cancel_download(app_id)` | `downloads.write` | Cancels a download, or takes it out of the queue |
| `move_download(app_id, to)` | `downloads.write` | Moves a queued download to place `to` (0 is next) |
| `download_now(app_id)` | `downloads.write` | Starts a queued download now; the running one waits |

A download is a map: `app_id`, `name`, `progress` (0 to 1), `bytes_done`,
`bytes_total`, `bytes_per_second`, `paused`, `queued`, `position` (place in the
queue, or `()`), `phase` (`"queued"`, `"preparing"`, `"checking"`,
`"downloading"`, `"finishing"` or `"verifying"` for a file check) and `error`
(or `()`).

The actions fail with an error when they don't fit: no download for that game,
pausing a queued one, moving a running one, or cancelling a file check.

There's no progress event, since progress changes many times a second. Call
`downloads()` from `on_tick()` or `page()` instead.

### Store

| Function | Permission | Returns |
|---|---|---|
| `wishlist()` | `store.read` | App ids on your wishlist |
| `wishlist_items()` | `store.read` | `[#{app_id, name, added, rank}]`: your wishlist in your order. `added` is Unix seconds, `rank` starts at 1 |
| `app_details(app_id)` | `store.read` | The game's store page (below), `#{error}` if Steam has none, or `()` while loading |
| `store_price(app_id)` | `store.read` | `#{initial, final, discount, currency, text, free}`, `#{error}`, or `()` while loading |
| `store_search(term)` | `store.read` | `[#{app_id, name, header, price}]`, or `()` while loading |

A store page is a map: `app_id`, `name`, `type`, `short_description`,
`developers`, `publishers`, `genres`, `categories` (as names), `release_date`
(as the store writes it), `coming_soon`, `free`, `price` (as `store_price`, or
`()`), `metacritic`, `recommendations`, `platforms`, `website`, `header`,
`background`, `dlc` (app ids), `demo` (an app id or `()`) and `store_url`.

Store prices are in cents of the store's currency for your country (`initial`
before the discount, `final` now). They are **not** Market prices: those come
from `market_price` and are in your wallet currency.

Store pages stay cached for an hour and searches for 10 minutes. A plugin can
start 30 new lookups a minute; past that, the rest return `()` and load later.
When one arrives you get `app_details_loaded` or `store_search_loaded`.

### Inventory and Market

| Function | Permission | Returns |
|---|---|---|
| `inventory(app_id, context_id)` | `inventory.read` | Your items there (753/6 is Steam's: cards, backgrounds), or `()` while loading |
| `market_price(app_id, market_hash_name)` | `inventory.read` | Cheapest listing in cents of your wallet currency, or `()` while it's looked up |
| `market_prices(app_id, names)` | `inventory.read` | A map of each name to its price in cents, or `()` while it's looked up. Up to 500 names, looked up together |
| `item_info(app_id, context_id, asset_id)` | `inventory.read` | One of your items, or `()` (while the inventory loads, or if it isn't there) |
| `tradable_items(app_id, context_id)` | `inventory.read` | Your items there that can be traded now, or `()` while loading |
| `marketable_items(app_id, context_id)` | `inventory.read` | Your items there that can be sold on the Market, or `()` while loading |
| `inventory_value(app_id, context_id)` | `inventory.read` | `#{total, priced, unpriced, currency}`: what the marketable items are worth, or `()` while loading. Missing prices are looked up, so call it again later |

Items are maps: `app_id`, `context_id`, `asset_id`, `class_id`, `amount`,
`name`, `type`, `market_hash_name`, `tradable`, `marketable`, `commodity`,
`color`, `icon` and `icon_large` (picture URLs), `description` (its text lines),
`tags` (`[#{category, name}]`), `trade_hold_days`, `market_hold_days` and
`price` (cents, or `()` if not known yet).

Market prices are the cheapest listing on the Community Market, in cents of
your wallet currency (`currency()`). Prices already looked up are kept, so
asking again is cheap. Store prices are separate: see `store_price`.

### Trades

| Function | Permission | Returns |
|---|---|---|
| `trade_offers()` | `trades.read` | Your offers: `id`, `partner`, `partner_name`, `message`, `state`, `ours`, `created`, `expires`, `held_until`, `give`, `get` |
| `trade_offer_info(id)` | `trades.read` | One offer, as in `trade_offers()`, or `()` |
| `evaluate_trade(id)` | `trades.read` | What both sides are worth on the Market (below), or `()` if there's no such offer |
| `refresh_trades()` | `trades.read` | Asks Steam for them again (also done every 2 minutes) |
| `decline_trade(id)`, `cancel_trade(id)` | `trades.write` | |
| `review_trade(id)` | `trades.write` | Opens the app's "Accept this trade?" review. Only you can accept |
| `propose_trade(partner, give, get, message)` | `trades.write` | Opens a new offer with these items (`[#{app_id, context_id, asset_id}]`) for you to check and send |

`state` is `"active"`, `"accepted"`, `"countered"`, `"expired"`,
`"cancelled"`, `"declined"`, `"invalid_items"`, `"needs_confirmation"`,
`"on_hold"` or `"other"`. Plugins can never accept a trade or confirm it with
Steam Guard.

`evaluate_trade` returns `give` and `get` (cents), `difference` (`get - give`),
`give_unpriced` and `get_unpriced` (marketable items with no price yet; they're
looked up, so ask again later), `not_marketable` (items that can't have a
Market price), `complete` (every marketable item is priced) and `currency`. It
only informs: what to do with the offer is still up to you.

### Web requests

| Function | Permission | Returns |
|---|---|---|
| `http_get(url)`, `http_post(url, body)` | `network` | A request id |
| `http_request(method, url, body, headers)` | `network` | A request id. `GET`, `POST`, `PUT`, `PATCH` or `DELETE` |
| `http_fetch(#{url, method, body, json, headers, timeout})` | `network` | A request id. Only `url` is needed. `json` sends a value as JSON (with its `Content-Type`). `method` is `GET`, or `POST` when there's a body. `timeout` is 1 to 20 seconds (20 if not given) |
| `http_cancel(id)` | `network` | `true` if it was still waiting; its answer never comes |

Rules for every request:

- Only `https://` addresses on the manifest's `hosts` work (`"*.example.com"`
  covers subdomains).
- Nothing on your own computer or local network: `localhost`, `.local` names,
  and private, link-local and similar addresses are refused, even if the
  manifest lists them. This is checked again after the name is looked up, so a
  public name that points to a local address is refused too.
- At most 30 a minute, and 8 waiting for an answer at once.
- Redirects aren't followed. Answers over 1 MB are cut off. Requests time out
  after 20 seconds at most.

The answer goes to `on_http(id, status, body)`, where `status` is 0 if the
request failed and `body` says why. Write it as `on_http(id, status, body,
headers)` to also get the response headers as a map (names in lower case, at
most 32, cookies left out).

## Hooks

Define any of these functions:

| Hook | When |
|---|---|
| `on_load()` | The plugin starts |
| `on_unload()` | The plugin is turned off, reloaded or the app closes it. It can still save with `storage_set`; other actions are dropped |
| `on_timer(id)` | A timer is due |
| `on_event(name, data)` | Something happened (see below) |
| `on_tick()` | Every `tick` seconds from the manifest |
| `on_http(id, status, body)` or `on_http(id, status, body, headers)` | A web request's answer |
| `page()` | Returns the widgets of the plugin's page (needs `[page]`) |
| `bigpicture_page()` | Returns the page's widgets for Big Picture, if it should differ there (otherwise `page()` is used) |
| `on_select(id, value)` | A select or tabs widget changed (without this hook, `on_input(id, value)` is called) |
| `on_slider(id, value)` | A slider was let go at a new number (without it, `on_input(id, "value")`) |
| `on_click(id)`, `on_input(id, text)`, `on_toggle(id, on)` | A button, text field or switch on the page |
| `game_actions(app_id)` | Returns buttons for a game's library page: `[#{id, text}]`; a click calls `on_game_action(id, app_id)` |
| `friend_menu(steam_id)` | Returns items for a friend's right-click menu: `[#{id, text}]`; a click calls `on_friend_menu(id, steam_id)` |

### Events

| Event | Data | Permission |
|---|---|---|
| `logged_in` | | none |
| `game_started`, `game_stopped` | `app_id` | none |
| `game_installed`, `game_uninstalled` | `app_id`, `name` | `library.read` |
| `friend_online`, `friend_offline` | `steam_id`, `name` | `friends.read` |
| `friend_game` | `steam_id`, `name`, `game` | `friends.read` |
| `message` | `steam_id`, `name`, `text` | `chat.read` |
| `download_started` | `app_id` (it left the queue) | `downloads.read` |
| `download_paused`, `download_resumed` | `app_id` | `downloads.read` |
| `download_finished` | `app_id`, `ok`, `error` (why it failed, or `()`) | `downloads.read` |
| `trade_offer` | `id`, `partner`, `name` (someone sent you one) | `trades.read` |
| `trade_changed` | `id`, `state` | `trades.read` |
| `achievement_unlocked` | `app_id`, `name`, `description`, `unlock_time` | `achievements.read` |
| `wishlist_changed` | `added`, `removed` (app ids) | `store.read` |
| `app_details_loaded` | `app_id` | `store.read` |
| `store_search_loaded` | `term` | `store.read` |

## Widgets

`page()` returns an array of maps. Every widget works in the normal window
and in Big Picture.

**Text and labels**

- `#{type: "heading", text}`, `"text"`, `"muted"`
- `#{type: "pill", text}`
- `#{type: "badge", text, tone}`: a colored label. `tone` is `"good"`,
  `"warn"`, `"bad"`, `"accent"` or left out
- `#{type: "link", text, page, arg}`: opens a page of the app (see `navigate`)
- `#{type: "tooltip", text, item: #{...}}`: `text` shows when pointing at
  `item` (in Big Picture, when it's focused)
- `#{type: "separator"}` (or `"divider"`)
- `#{type: "loading", text}`

**Things you press or change**

- `#{type: "button", id, text}`: calls `on_click(id)`
- `#{type: "input", id, value, hint}`: calls `on_input(id, text)` when you're
  done typing
- `#{type: "toggle", id, text, value}` (or `"checkbox"`): calls `on_toggle(id, on)`
- `#{type: "select", id, label, options, value}`: `options` are strings or
  `#{value, label}`. Calls `on_select(id, value)`
- `#{type: "slider", id, label, min, max, step, value}`: calls
  `on_slider(id, value)`
- `#{type: "tabs", id, items, value}`: a row of tabs, `items` like `options`.
  Calls `on_select(id, value)`; show what goes with the picked tab yourself
- `#{type: "game", app_id}`: a game's picture; opening it shows the game

**Numbers and data**

- `#{type: "stat", label, value}`
- `#{type: "metric", label, value, change, tone, hint}`: a big number. A
  `change` starting with `+` is green and with `-` red, unless `tone` says
  otherwise
- `#{type: "progress", value}` (0 to 1)
- `#{type: "chart", kind, values, labels, height}`: `kind` is `"line"` or
  `"bar"`; up to 500 values
- `#{type: "sparkline", values}`: a small line chart
- `#{type: "table", columns: [...], rows: [[...]]}`
- `#{type: "image", url, width, height}`. Pictures come from Steam's image
  servers or the plugin's hosts.
- `#{type: "avatar", steam_id, size}`: a friend's (or your) picture

**Layout**

- `#{type: "row", items: [...]}`: side by side
- `#{type: "grid", columns, items: [...]}`: equal columns (1 to 6), wrapping
- `#{type: "card", title, items: [...]}`: a box around other widgets
- `#{type: "section", title, items: [...]}`: a titled group
- `#{type: "empty_state", icon, title, text}`: for an empty list

A page shows at most 500 widgets, and containers nest at most 8 deep.

## Big Picture

A plugin's page is in Big Picture only if the manifest says so, because a page
made for a mouse may not work well with a controller:

```toml
[page]
title = "Backlog"
icon = "dice"
bigpicture = true
```

Pages that opt in are listed in Big Picture's **Plugins** tab, which only
appears when there is at least one. There:

- buttons, switches, choices, tabs, links, games and table rows take focus,
  and A (or Enter) uses them;
- a select steps to its next option with A;
- a slider is picked with A, moved with left and right, and put down with A
  (B leaves it as it was);
- a text field opens a page with the on-screen keyboard. A real keyboard works
  too, and the plugin gets `on_input` when you press Done or Enter;
- a page with nothing to focus below scrolls with up and down;
- links and `navigate()` open Big Picture's own pages (a game, the store,
  downloads...), or the normal window's page inside Big Picture.

To lay the page out differently for the TV, check `view_mode()` in `page()`,
or define `bigpicture_page()` and it's used there instead.

Buttons from `game_actions` show on Big Picture's game pages, and items from
`friend_menu` on its friend pages, for every plugin (no opt-in needed).

The **Widget Gallery** example shows every widget in both modes.

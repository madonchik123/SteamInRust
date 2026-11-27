# Plugin API plan

Where the Rhai plugin API goes next. It started from an AI-written roadmap; this
file checks each item against the real code and says what we build, change or
drop.

## What we have today

The plugin code is in `crates/plugins/src/lib.rs` (sandbox, permissions, script
API, widgets) and `apps/desktop/src/builtin/plugins.rs` (the host: it builds the
data plugins see, runs their requests, draws their pages and the consent
dialog).

- **Sandbox:** Rhai with no modules, no `eval`, capped operations, call depth,
  string, array and map sizes. A failing hook only sets the plugin's error.
- **Permissions:** 16, declared in `plugin.toml`. All must be approved before
  the script runs. Turning the plugin off revokes them all. If the manifest asks
  for more later, the plugin stops and asks again.
- **Functions:** `me`, `friends`, `library` (id, name, installed, playtime),
  `downloads`, `wishlist` (ids only), `inventory`, `market_price`,
  `trade_offers`, `decline_trade`, `cancel_trade`, `review_trade`,
  `propose_trade`, `notify`, `set_status`, `send_message`, `launch`,
  `storage_get` / `storage_set`, `http_get` / `http_post` / `http_request`
  (async, https only, only to the manifest's hosts, 30 a minute, no redirects),
  `copy`, `navigate`, `language`, `platform`, `currency`, JSON helpers.
- **Hooks:** `on_load`, `on_event`, `on_tick` (10 s minimum), `on_http`,
  `page`, `on_click` / `on_input` / `on_toggle`, `game_actions`, `friend_menu`.
- **Widgets:** heading, text, muted, button, stat, separator, row, progress,
  game, input, toggle, link, table, pill, image. Nesting is capped at 8 and the
  count at 500.
- **Big Picture:** it has **no plugin pages at all**. The Settings list in
  `docs/parity.md` keeps plugins in the normal window.

Client data that is already loaded, so plugins can get it without new Steam
calls:

| Data | Where |
|---|---|
| Owned apps with genres, categories, Deck rating, OS list, icon | `State::apps` (`model::OwnedApp`) |
| Installed games with size, last played, playtime, last update | `LocalState::games` (`model::InstalledGame`) |
| Achievement progress and full lists (asked for per game) | `State::achievements`, `State::achievement_list`; `Command::LoadAchievementProgress`, `LoadAchievements` |
| Downloads and queue; pause, resume, cancel, move, download now | `State::downloads`, `download_queue`; `Command::PauseDownload` etc. |
| Wishlist with the date each game was added, in your order | `State::wishlist` |
| Store pages (developer, publisher, genres, release date, price) | `store::StoreClient::details` / `search` (public store endpoints; no login) |
| Inventories and Market prices | `State::inventories`, `State::market_prices`; `Command::MarketPrices` (one search per game) |
| Trade offers | `State::trade_offers` |
| Friends, groups, profiles, badges, Workshop browsing | `State::personas`, `friends`, `profiles`, `badges`; `Command::WorkshopQuery` |

## Roadmap items

### P0

**1. Richer library info (`library.read`): keep, but change.**
- `game_info(app_id)`, `installed_games()` and `recent_games(count)`: keep.
  They come from `OwnedApp` + `InstalledGame`: playtime, last played, size on
  disk, last update, update needed, genre and category ids, Deck rating, OS
  list, store URL and art URLs. No new Steam calls.
- Developer, publisher, genre names and release date are **not** in the
  library data; they come from the store page. So they live in `app_details`
  (`store.read`) instead of copying store data into the library call.
- `game_stats`: **dropped** as a separate call. It would be `game_info` plus
  `achievement_progress`, so plugins just call both.
- `favorite_games`: **moved to P1**. Favorites live inside the library page's
  own saved state, which other parts of the app can't reach. Reaching it needs
  a small shared hook in `library.rs`, which another open PR is changing right
  now.
- `library()` keeps its shape for old plugins.
- Complexity: low.

**2. Achievements (new `achievements.read`): keep.**
- `achievement_progress(app_id)`, `achievements(app_id)` and
  `achievement_info(app_id, name)`. They work like `inventory()`: they return
  `()` while loading and ask Steam once. No new Steam calls.
- `achievement_unlocked` event: **changed**. The app isn't told live when a
  game unlocks something (the in-process Steam API posts it to the game only).
  So the host reloads a game's list when it starts and again when it closes,
  and sends one event per new unlock. The docs say it arrives when the game
  closes, not during play.
- Complexity: low to medium.

**3. Download control (new `downloads.write`): keep, with changes.**
- `pause_download`, `resume_download`, `cancel_download`, `move_download(app,
  to)` and `download_now`. They map one-to-one to existing commands. They only
  act on downloads that exist. `cancel_download` refuses a running file check
  (verify).
- Events: `download_started`, `download_paused` and `download_resumed`; the
  existing `download_finished` gains an `error` field.
- **Dropped:** a separate `download_failed` event, because
  `download_finished` with `ok: false` already says it. Also dropped: a
  `download_progress` event. Progress changes several times a second and would
  flood every plugin; `downloads()` (now with bytes, phase and error) in
  `on_tick` is the way to show progress.
- `downloads.write` counts as sensitive (it acts for you), so it gets the
  warning in the consent dialog.
- Complexity: low.

**4. Store data (`store.read`): keep.**
- `app_details(app_id)`, `store_price(app_id)`, `store_search(term)` and
  `wishlist_items()`.
- The first three use the public store endpoints the app already calls: they
  are async, cached in the host (1 hour for pages, 10 minutes for searches) and
  limited to 30 new lookups a minute per plugin. They return `()` while
  loading, and `app_details_loaded` / `store_search_loaded` events say when to
  ask again.
- `wishlist_items` uses the loaded wishlist (added date, your order).
- `wishlist_changed` event (`added`, `removed`): from the wishlist updates the
  app already gets.
- Store prices are in their own `store_price` map with the store's currency.
  Market prices stay in `market_price` / item `price` in cents of your wallet
  currency. The two are never mixed.
- The permission text changes to say it also sees store pages and prices.
- Complexity: medium (async cache, rate limit).

**5. Inventory (`inventory.read`): keep, but change.**
- `market_prices(app_id, names)` (batch): keep. It uses the existing price
  cache and the one-search-per-game lookup.
- `item_info`, `tradable_items`, `marketable_items` and `inventory_value`:
  keep. All are built from data that is already loaded.
- Richer item metadata: items also get `class_id`, `description` (text lines),
  `trade_hold_days`, `market_hold_days`, `commodity` and a large `icon_large`.
  That's everything Steam's inventory answer has that's useful.
- "With caching": the client's `market_prices` map already is the cache, so no
  second cache.
- Complexity: low.

**6. Trades: keep in part.**
- `trade_offer_info(id)` and `evaluate_trade(id)` (`trades.read`): keep.
  `evaluate_trade` adds up Market prices of both sides and says how many items
  have no price yet. It is read-only and only informs.
- `trade_history`: **moved to P1**. The client can't load it today; it needs a
  new Steam call (`IEconService/GetTradeHistory` with the session's access
  token) plus parsing and paging.
- No auto-accept and no automatic Steam Guard confirmations. Accepting still
  only goes through the app's review dialog. `trades.read` and `trades.write`
  stay separate.
- Complexity: low.

**7. More widgets: moved to P1, after Big Picture can show plugin pages.**
- Every new widget has to work in Big Picture in the same change, but Big
  Picture has no plugin pages yet. Adding 18 widgets now would mean building
  the whole plugin page there first. That is its own piece of work: a
  controller-friendly page renderer for all widgets, with focus order for
  buttons, inputs and switches.
- So P1 starts with "plugin pages in Big Picture", then adds widgets in small
  groups that work in both modes.
- From the list:
  - Keep: `metric` (a bigger `stat`), `select`, `checkbox` (same as `toggle`, so
    just an alias), `slider`, `section`, `tabs`, `grid`, `card`, `badge`,
    `tooltip`, `divider` (alias of `separator`), `avatar`, `empty_state` and
    `loading`.
  - Change: `chart` and `sparkline`, as line or bar only.
  - Drop: `search` (it's an `input` with a hint) and `modal` (a plugin popup
    over the whole app is easy to misuse for fake prompts; use the page).

### P1

- **Plugin pages in Big Picture**, then the widgets above. This one comes
  first.
- **Notification actions** (`notify_action`, `on_notification_action`): keep.
  The buttons must work in the Big Picture toast too.
- **Timers** (`schedule_in`, `schedule_at`, `cancel_timer`, `on_timer`): keep.
  At most 20 per plugin, at least 5 seconds apart, all cleared on stop.
- **HTTP:**
  - Features: response headers in `on_http`, a per-request timeout (at most
    20 s), cancel by id, a cap on requests in flight (4).
  - Security: block localhost and private, link-local and CGNAT addresses after
    DNS, using a custom resolver on the plugin client. Redirects are off today,
    which is safer than re-checking them. Allow them only if each hop is
    checked against the hosts and the blocked ranges.
  - JSON helpers already exist.
- **Storage:** `storage_delete`, `storage_has`, `storage_keys`, `storage_clear`.
  Keep the 200 keys, and add a total size limit (1 MB).
- **Lifecycle:** `on_unload` (called on stop) and `on_settings_changed`: keep.
  `on_enable` / `on_disable`: drop. They are the same as `on_load` /
  `on_unload` here, because a disabled plugin doesn't run.
- **`favorite_games`** through a small read-only hook in the library page.
- **`trade_history`** with the new Steam call.

### P2

- **Workshop read:** keep. `WorkshopQuery` exists. A plugin version needs its
  own request ids and answers; you can read only, and subscribing stays in the
  app.
- **Friends info** (`friend_info`, `friend_groups`, relationship): keep. The
  data is in `State` already. Needs `friends.read`.
- **Account info** (level, badges, profile): keep under `account.read`. Level
  is loaded; badges load per profile.
- **Richer `game_actions`** (separators, submenus, icons, confirm, disabled
  with a reason) and a game context menu hook: keep, in both modes.

### P3

- `api_version()` is **moved to P0**. With new functions, plugins need a way to
  check what the app has. `capabilities()` is dropped: `api_version()` covers
  it, and an unknown function already gives a clear error.
- `plugin_version()`, log levels, better errors and a dev mode (reload on
  save): keep.
- Marketplace prep: keep. Installing must never grant permissions. The consent
  dialog stays the only way in.

### Dropped by the user's decision

- **Plugin secrets** (`secret_get`...).
- **Plugin-to-plugin calls** (`plugin_call`...).
- **Screenshot API.** Not worth adding.

## Gaps the roadmap missed

- **Feature checks:** `api_version()` (done in P0).
- **Per-permission revoke:** today you can only turn the whole plugin off.
  "Revocable" is met that way, but switching off one permission (for example
  `network`) and having its calls fail would help.
- **Install and uninstall events** (`game_installed`, `game_uninstalled`): the
  local scan already knows.
- **Storage holds strings only:** `storage_set` stores `to_string()`.
  `storage_set_json` / `storage_get_json` would save plugins a lot of parsing.
- **The one-second snapshot** is cloned for every plugin. Big lists are now
  shared (`Arc`), but a plugin doing lots of calls still copies data each time.
  A lazy, per-permission snapshot would scale better.

## P0 in this change

1. `api_version()`.
2. Library: `game_info`, `installed_games`, `recent_games`.
3. Achievements: `achievements.read`, three functions, the `achievement_unlocked`
   event.
4. Downloads: `downloads.write`, five actions, richer `downloads()`, three new
   events.
5. Store: `app_details`, `store_price`, `store_search`, `wishlist_items`,
   `wishlist_changed`, and the loaded events.
6. Inventory: `market_prices`, `item_info`, `tradable_items`,
   `marketable_items`, `inventory_value`, more item fields.
7. Trades: `trade_offer_info`, `evaluate_trade`.

None of these add new UI. Plugin actions show through the app's existing
toasts and pages, so Big Picture needs no change for them. The consent dialog
is only in normal-mode Settings, as before.

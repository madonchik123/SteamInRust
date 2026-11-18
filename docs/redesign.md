# The redesign: shared foundation

SteamInRust's look follows the Figma file "Steam Redesign (Community)". This
page is for whoever restyles a page: what the shared parts are, what each
one is for, how to call it, and which code draws which screen today.

The design itself lives outside the repository (it's never committed):

- `C:\SteamInRust\Steam_Redesign_Full_AI_Export\` is the full Figma export.
  `pages/style-guide.md` lists the style guide frames; `decoded/nodes.jsonl`
  and `decoded/all_nodes.csv` have exact sizes, paddings, radii and colors per
  node; `decoded/frame_inventory.csv` lists every screen.
- `C:\SteamInRust\media\Steam Redesign (Community)\` has full-size renders of
  the screens (`library home.png`, `client store.png`, `store page.png`…).

Node IDs below (`84:791`) are Figma's. In the export, a node's fill can come
from a shared style (`styleIdForFill`); the style's color wins over the
node's own cached fill, so read both. The Play button's master, for example,
caches a blue fill but uses the "Accent / Green" style, and renders green.

Never use Steam's logo, the word "STEAM" or other Valve branding: the top
bar shows SteamInRust's own logo (`ui::brand`), and only the logo (no name
beside it).

## Seeing your work

```bash
cargo run -p steam-rs -- --separate --demo --no-local --lang=en --page=gallery
```

- `--page=gallery` opens the design gallery: every shared control in each of
  its states, plus a sample page bar (`apps/desktop/src/builtin/gallery.rs`).
  It's only there when asked for.
- `--theme=Slate` (or `Midnight`, `Classic`…) shows a built-in theme without
  touching your saved one. `--window-size=1920x1080` sets the window size.
- `--offscreen` opens the window far off the screen, for screenshots of a test
  copy that never shows on the desktop. Capture it with `PrintWindow` and the
  `PW_RENDERFULLCONTENT` flag. Windows ignores real mouse messages for an
  off-screen window, so `--pointer=x,y` keeps a pretend pointer there (to see
  hover states and hover menus) and `--click=x,y` clicks once, 4 seconds in.
- `--demo --no-local` shows a made-up account and library: use it for any
  screenshot you share.

## Theme tokens

`ui::Theme` (`crates/ui/src/theme.rs`). The default theme for new users is
**Slate**, the style guide's colors (frame `11:103`, "colors"). The older
themes (Midnight, Aurora, Classic, OLED, Daylight) are still there, and the
accent color and roundness settings still apply. Always take colors from the
theme, never write a hex value in a page, so every theme works.

| Token | Slate | Style guide name | Use it for |
| --- | --- | --- | --- |
| `chrome` | `#0E141B` | Bg / Main | The top bar; the darkest fills (sort chips, dropdowns, search fields) |
| `bg` | `#181D24` | — | Page background where no section gradient applies |
| `raised` | `#1E2329` | Bg / Highlight | Menus, secondary buttons, tags, the top bar's buttons |
| `surface` | `#262C35` | (`#313843` at 50%) | Content cards ("Friends who play", "Activity") |
| `surface_hover` | `#313843` | Bg / Hover | Hover fills, selected chips and tags, pause button |
| `tertiary` | `#212B45` | Bg / Tertiary | The page bar under the top bar; the toggle's track when on |
| `secondary` | `#4B619B` | Color / Secondary | Primary buttons, the selected tab in a page bar |
| `accent` | `#66C0F4` | Color / Primary | Links, the open section, the primary button on hover, Install, badges, the toggle's knob |
| `accent_2` | `#4B619B` | Color / Secondary | Second accent stop (gradients, Big Picture) |
| `text` | `#F3F3F3` | Text / Main | Main text |
| `text_dim` | `#76808C` | Text / Dim | Secondary text, icons, labels |
| `text_faint` | `#525B66` | — | Hints, disabled text, captions |
| `play` | `#A1CD44` | Accent / Green | The Play button, in-game friends |
| `discount` | `#A1CD44` | Accent / Green | The discount block, discounted prices |
| `danger` | `#CD5444` | Accent / Red | Removing, errors, negative reviews |
| `away` | `#C1B15F` | Accent / Yellow | Away status, pending money, warnings |
| `online` | `#66C0F4` | — | Online friends |
| `in_game` | `#A1CD44` | — | Friends in a game |
| `on_accent` | `#0E141B` | — | Text on `accent` fills |
| `border` | `#2A313B` | — | The rare line or outline (the design uses almost none) |
| `radius` | `5` | — | `t.corner()` is 5 (cards), `t.corner_small()` is 3 (buttons, tags, menus) |

Helpers:

- `t.text_on(fill)` gives near-black or near-white text, whichever reads on
  `fill` (the Play button's green and Install's blue take dark text).
- `t.paint_store_background(painter, rect)` paints the store gradient
  (`--gpGradient-StoreBackground`: `#2A475E` to `#1B2838`, reached 80% down).
- `t.paint_library_background(painter, rect)` paints the library gradient
  (`--gpGradient-LibraryBackground`: a radial glow from `#3D4450` 40 px in
  from the top-left corner to `#23262E`).
- The shell already paints these behind the Store and Library pages; a page
  that fills its own background covers them.
- `ui::widgets::radial_gradient` and `ui::widgets::gradient` draw any radial
  or two-color gradient.
- The style guide's other values (`--gpSystem…` greys, `--gpStore…` blue
  greys, `--gpColor-Blue #1A9FFF`, the translucent `--gpBackground-…` fills)
  are Valve's own client variables, copied onto the guide for reference. The
  screens use the "colors" frame above, so they aren't tokens. If a page needs
  one, add a token rather than a hex value in the page.

Themes saved before these tokens existed get them filled in by
`Theme::upgrade` (`rev` 0 → 1), so a user's own theme keeps its look.

## Type

The design is set in Motiva Sans, which is Valve's and can't be shipped.
Inter (bundled in `crates/ui/assets/fonts`) stands in at the same sizes and
weights; it runs a little wider. The scale is in `ui::theme::type_scale`,
with text helpers in `ui::widgets`:

| Style guide | Size, weight | Helper |
| --- | --- | --- |
| Heading Large | 26, bold | `w::heading_l(text, color)` |
| Heading Medium | 22, bold | `w::heading_m` (also egui's `Heading` style) |
| Heading Small | 18, bold | `w::heading_s` (big buttons, the top bar's sections at 17) |
| Body Large | 16, regular | `w::body_l` (page-bar tabs, secondary buttons) |
| Body Medium | 14, regular | `w::body_m` (egui's `Body` style) |
| Body Small | 12, regular | `w::body_s` (tags, review bars, small notes) |

`w::text`, `w::text_medium`, `w::text_semibold` and `w::text_bold` still take
any size.

## Shared controls

All in `crates/ui/src/widgets.rs` (`use ui::widgets as w`). Every one takes
`&Theme`, has hover and pressed states, and dims when the `Ui` is disabled
(`ui.add_enabled_ui(false, …)`). Existing calls kept their signatures.

| Control | Figma | Call |
| --- | --- | --- |
| Primary button | "primary button" `84:791` | `w::button(ui, t, icon, "Add to Cart", w::Kind::Primary, w::Size::Lg)` |
| Secondary button | "secondary button" `84:787` | `Kind::Secondary` |
| Tertiary button | "tertiary button 2" `400:13118` (outline at 20%, 40% on hover); `324:1431` is the same with a Bg / Highlight outline | `Kind::Tertiary` |
| Play | "Play button" `335:1183` | `w::play_button(ui, t, "Play")` or `Kind::Play` |
| Install | "install button" `375:4453` | `w::install_button(ui, t, "Install")` or `Kind::Install` |
| Pause | "pause button" `379:8204` | `w::pause_button(ui, t, "Pause")` or `Kind::Pause` |
| Other buttons | — | `Kind::Ghost` (no fill until hovered), `Kind::Danger` |
| Button sizes | 53 px in the guide | `Size::Lg` 53, `Size::Md` 40, `Size::Sm` 32; any height with `w::button_sized` |
| Main CTA | "main-cta" `426:4355` | `w::main_cta(ui, t, "$23.09", Some("$34.99"), 34, Some("Until October 19"), "Add to Cart")` returns the button's response |
| Price | "price" `426:4419` | `w::price(ui, t, label, original, discount)` (44 px); `w::price_tag` is the same at 36 px for cards and rows |
| Tag | "tag" `280:203` | `w::tag(ui, t, "Souls-like", Some("5,000"), w::TagState::Checked)`; states `Plain`, `Unchecked`, `Checked`, `Current`, `Filtered` |
| Toggle | "toggle" `477:7301` | `w::toggle(ui, t, &mut on)`; `w::toggle_row(ui, t, &mut on, label)` puts a label before it |
| Check box | "Check box" `757:10005` | `w::check(ui, t, &mut on, "label")`; `w::checkbox(ui, t, &mut on, 30.0)` without a label |
| Sort filter | "sort filter" `296:579`, "sorting" `457:12113` | `w::sort_chip(ui, t, label, selected)`; `w::sort_by(ui, t, &mut value, &[(v, "New and Trending"), …])` adds "Sort by" before the chips |
| Dropdown | "sort filter" with its caret, "menu dropdown" `18:273` | `let r = w::dropdown(ui, t, label, min_width); egui::Popup::menu(&r).show(\|ui\| …)` |
| Menu | "menu dropdown" `18:273` | egui popups and context menus already take the theme's menu look (Bg / Highlight, soft shadow) |
| Menu item | "Menu items" `18:248` | `w::menu_item(ui, t, icon, label)`, `w::menu_item_tinted` (e.g. red), `w::submenu_item` |
| Wishlist | "wishlist" `280:345`, "wishlist page" `302:1745` | `w::wishlist_button(ui, t, wishlisted, Some("Wishlist"))`; `None` for the square heart. Wishlisted turns red on hover (a click removes it) |
| Reviews | "reviews" `289:270` | `w::review_bar(ui, t, positive_percent, 138.0)` |
| Page-bar tab | "menu" `82:322` | `w::sub_tab(ui, t, "Home", selected)` |
| Top-bar section | "store menu" `18:294` | `w::section_tab(ui, t, label, selected, height)` (the shell draws these) |
| All-caps tab | — | `w::nav_tab(ui, t, label, selected, height)`: dim, accent when selected (in-page tabs) |
| Search | "Search" `280:57` | `w::search(ui, t, &mut text, hint, width)` (34 px); `w::bar_search` (41 px) for a page bar |
| Icon box | "icon box" `280:175`, "icon select" `346:2627` | `w::icon_box(ui, t, icon, tooltip, selected)`: view switches, filters, the action row under a game's art |
| Top-bar buttons | "friends" `447:3849`, "notifications" `766:10160` | `w::bar_button(ui, t, icon, tooltip, active, badge)` |
| Badge | the blue count circle `447:3845` | `w::badge(painter, t, center, n)`; `w::icon_button` shows one too |
| Download ring | "download status" `383:10133` | `w::progress_ring(painter, t, center, 7.5, fraction)` |
| Card | `523:6593` and the other content boxes | `w::card(t).show(ui, …)`: flat, rounded 5, no border |
| Chips | — | `w::chip`, `w::tinted_chip`, `w::toggle_chip`, `w::image_chip`, `w::segmented` now share the tag's shape and colors |

`icons::PLAY` and `icons::PAUSE` are drawn as solid shapes in buttons (the
icon font only has outlines); `w::paint_icon` draws any icon the same way.
`w::paint_heart` draws the filled heart.

Not matched exactly, on purpose:

- Menu rows are 38 px tall with 14.5 px text, not the guide's 49 px with
  16–18 px text: the app's right-click menus are long, and 49 px rows made
  them run off the screen. The colors and padding are the guide's.
- The Wishlist button is 53 px tall like the other large buttons (the guide
  draws it 58 px), so it lines up next to "Add to Cart".
- The guide's primary button keeps white text on its light-blue hover; so
  does ours. Install turns its text white on hover, as the guide shows.
- The check box when on is filled in Text / Dim with a dark check, as the
  guide shows (not the accent color).

## The app shell

`apps/desktop/src/shell.rs` draws the window; `chrome.rs` the window buttons
and resize edges.

```
┌ top bar, 59 px, chrome ───────────────────────────────────────────────────────┐
│ ‹ ›  [logo]  STORE  COMMUNITY  <YOU>  LIBRARY  DOWNLOADS ◔ 46%   [👥4][🔔1][avatar name $5.25 ▾] Help View ▾  – ▢ ✕ │
├ page bar, 57 px, tertiary ────────────────────────────────────────────────────┤
│ [Home] Collections  News …    [Search…        🔍] [⧩][◷][▷]     View [≡][▦][▢]  Activate a Product … │
├───────────────────────────────────────────────────────────┬ friends drawer, 290 px ┤
│ the page (store / library gradient behind it)             │                         │
└───────────────────────────────────────────────────────────┴─────────────────────────┘
```

Top bar (Figma `333:2433` "client", top 59 px; `header.png`):

- Back and forward arrows, then the logo, which is the app menu (Settings,
  Activate a product, Redeem a wallet code, Plugins, Switch account, Exit).
- The sections, in the design's order: Store, Community, your name (your
  profile), Library, Downloads; then pages extensions add as main pages, then
  the Plugins menu. The open section is in `accent`. They space out up to the
  design's 66 px as the window widens, and drop to a smaller size on narrow
  windows. `shell::section_of(page)` says which section a page belongs to.
- Resting the pointer on a section opens its menu of shortcuts (Figma
  `18:295`, `18:798`, `18:847`, `18:894`), e.g. Store: Home, Browse,
  Discovery queue, Wishlist, Cart.
- While something downloads, Downloads shows a ring and a percentage, with
  the details on hover (this replaces the old status bar at the bottom, which
  the design doesn't have).
- On the right: the offline badge (when offline), Friends (opens the friends
  drawer; rest the pointer on it for the quick list), Notifications, the
  account button (avatar, name and wallet; its menu has your status, profile,
  settings, the wallet with its actions, and sign-out), Help (Support,
  updates), View (Big Picture, the main places, Friends & Chat, Settings),
  and the window buttons. The account button hides the balance on windows
  under 1400 px wide.

Page bar (Figma `375:2459` "library header", `header.png`): each page fills
it by implementing `Extension::page_bar`:

```rust
fn page_bar(&mut self, _page: &str, ui: &mut Ui, ctx: &mut Ctx) -> bool {
    let t = ctx.theme.clone();
    ui.spacing_mut().item_spacing.x = 7.0;
    for (i, label) in [t!("Home"), t!("Collections")].iter().enumerate() {
        if w::sub_tab(ui, &t, label, self.tab == i).clicked() {
            self.tab = i;
        }
    }
    ui.add_space(30.0);
    w::bar_search(ui, &t, &mut self.search, t!("Search"), 374.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // Buttons on the right, last one first.
    });
    true
}
```

`page_bar` runs before `page` in the same frame; the `Ui` is laid out left to
right, centered vertically, with 10 px in from each end. Return `false` (the
default) and the bar shows the app-wide search (games, friends, the store).
A page that takes the bar over should have its own search there. The sample
in `builtin/gallery.rs` matches the library's bar.

The friends drawer, toasts, dialogs and the notifications menu are unchanged
apart from taking the new tokens and controls.

Big Picture (`apps/desktop/src/bigpicture`) keeps its own look; it only picks
up the theme's colors.

## Screens and who restyles them

From `decoded/frame_inventory.csv` (the UI page, `18:946`). "Today" is the
code that draws the closest screen now.

### Store session

| Frame | Node | Today |
| --- | --- | --- |
| client store | `426:3487`, `835:10477` | `builtin/store.rs` (`home`, `section`, `search_results`, the store's own bar in `subnav`), `store_micro.rs` (hover clips) |
| Group 136 (store carousel, 7 states) | `634:15231` | `builtin/store.rs` (`home`'s spotlight) |
| store page | `411:2833` | `builtin/store.rs` (`app_page`), `store_specs.rs`, `store_review.rs`, `store_reviews_list.rs` |
| wishlist | `300:321` | `builtin/store_more.rs` (`wishlist_page`) |
| cart | `397:11087` | `builtin/store_more.rs` (`cart_page`) |
| discover | `306:2010` | `builtin/store_discovery.rs` |
| browse | `660:10019` | `builtin/store_more.rs` (`browse_page`) |
| point shop | `317:1001` | nothing yet (new screen) |

### Library session

| Frame | Node | Today |
| --- | --- | --- |
| library home | `324:1652` | `builtin/library.rs` (`sidebar`, `home`, `game_page`, `side_panels`) |
| library home filters / recent / ready to play | `727:39670`, `727:40268`, `727:41218` | `builtin/library.rs` (`sidebar`, `adv_panel`, `filtered`) |
| library home grid view | `363:5243`, `368:13794` | `builtin/library.rs` (`home`), `ui::widgets::game_tile` |
| library home detail view | `375:2803`, `375:2966` | `builtin/library.rs` (`game_page`, `dlc_strip`), `library_news.rs` |
| library collections | `360:4997` | `builtin/library.rs` (collections, `custom_page`) |
| Component 3 (sidebar group, closed and open) | `368:14735` | `builtin/library.rs` (`sidebar`) |
| game card | `363:5954` | `ui::widgets::game_tile` |
| icon select | `346:2628` | `ui::widgets::icon_box` (shared, done) |
| download | `375:5665` | `builtin/downloads.rs` |
| download status | `383:10134` | `shell.rs` top bar (done) with `w::progress_ring` |

The game's Properties window (`library_props.rs`) has no frame in the design.

### Community session

| Frame | Node | Today |
| --- | --- | --- |
| community home | `487:5076` | nothing matching yet: `builtin/community.rs` is the friends and chat page |
| game community hub | `495:6863`, `801:10167` | `builtin/hub/mod.rs`, `hub/files.rs` |
| game community hub discussions | `505:8934` | `builtin/hub/forum.rs` |
| game community hub workshop, workshop 2 | `512:11944`, `517:14057` | `builtin/hub/files.rs` (Workshop items) |
| discussions | `495:5839` | `builtin/hub/forum.rs` |
| forum | `503:7385` | `builtin/hub/forum.rs` |
| game forum | `505:9758` | `builtin/hub/forum.rs` |
| discussion | `504:7960` | `builtin/hub/forum.rs` (a thread) |
| workshop | `509:10969` | `builtin/workshop.rs` |
| market | `532:6677` | `builtin/inventory_search.rs`, `inventory_market.rs` |

Groups (`hub/group.rs`), achievements (`hub/achievements.rs`) and the
inventory (`inventory.rs`, `inventory_trades.rs`) have no frame of their own.

### Profile and settings session

| Frame | Node | Today |
| --- | --- | --- |
| profile | `435:7035` | `builtin/profile.rs` |
| games | `455:9708` | `builtin/profile.rs` (`games_page`) |
| screenshots | `457:12553` | `builtin/hub/files.rs` (`user_files`) |
| settings | `475:6189` | `builtin/settings.rs`, `settings_account.rs` (its own `settings::toggle` can become `w::toggle`) |
| privacy settings | `477:7327` | `builtin/profile_edit.rs` (privacy) |
| profile settings | `476:6952` | `builtin/profile_edit.rs` |
| friends (popup) | `447:3850` | `shell.rs` (`friends_hover`, `friends_drawer`, `friend_list`), `friends_window.rs` |
| notifications (popup) | `766:10159` | `shell.rs` (`notifications_button`) |
| account (menu) | `18:1282` | `shell.rs` (`account_menu`; the button is done, the menu's rows can follow the frame) |

Also: badges, friends, reviews and groups lists in `profile_badges.rs` and
`profile_lists.rs`.

### Done in the foundation

| Frame | Node | Code |
| --- | --- | --- |
| Style guide (all frames on page `10:5`) | | `crates/ui/src/theme.rs`, `widgets.rs` |
| client (top bar of every screen) | `333:2433` | `shell.rs` (`top_bar`) |
| store menu ×4 (section menus) | `18:295`, `18:798`, `18:847`, `18:894` | `shell.rs` (`section_menu`) |
| header / library header (page bar) | `375:2459` | `shell.rs` (`page_bar`) and `Extension::page_bar` |
| Group (Steam's wordmark) | `435:10461` | not used: SteamInRust's own logo instead |

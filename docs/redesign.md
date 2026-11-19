# The redesign

SteamInRust's default look follows [Steam Redesign](https://www.figma.com/community/file/1302616100790619521/steam-redesign) by
**Juxtopposed**, shared on Figma Community. This page is for whoever works
on a page: the rules, what the shared parts are, what each one is for, how to
call it, and which code draws which screen.

The design isn't in the repository. Open the Figma file above to see it. For
exact numbers, a local export of the file is handy (never commit it; the
repository's `.gitignore` already keeps `Steam_Redesign_Full_AI_Export/` and
`media/` out):

- In the export, `pages/style-guide.md` lists the style guide frames;
  `decoded/nodes.jsonl` and `decoded/all_nodes.csv` have exact sizes,
  paddings, radii and colors per node; `decoded/frame_inventory.csv` lists
  every screen.
- Renders of the screens (`library home.png`, `client store.png`,
  `store page.png`…) can be exported from Figma next to it.

The design is drawn on a 1920 px canvas at a large size, so the default
theme shows it at 85% (`scale = 0.85` in its `[layout]`). Keep the Figma
numbers in the theme as they are; the scale does the rest.

Node IDs below (`84:791`) are Figma's. In the export, a node's fill can come
from a shared style (`styleIdForFill`); the style's color wins over the
node's own cached fill, so read both. The Play button's master, for example,
caches a blue fill but uses the "Accent / Green" style, and renders green.

## The one rule: everything comes from the theme

The app's look is a theme package (`docs/themes.md`). The redesign is the
default theme, **SteamInRust**; **Classic** is the look from before. Users
will be able to make their own. So a page must never hard-code its look:

- No color values in a page (no `Color32::from_rgb`, no hex). Use a token
  (`t.accent`, `t.text_dim`, `t.surface`…), `t.overlay_dark(i)` /
  `t.overlay_light(i)` over pictures, or `t.color(&"accent@0.5".into())`.
- No font sizes or weights of your own: text helpers (`w::heading_s(t, …)`,
  `w::body_m(t, …)`) or `t.typography.*` (`w::styled(text, style, color)`).
- No radii, paddings or spacing numbers where a token exists:
  `t.corner()`, `t.corner_small()`, `t.spacing.*`, `t.dense(x)` for sizes
  (it shrinks them in the compact density).
- Draw with the shared widgets. If a page needs a control the widgets don't
  have, add it to `ui::widgets` with a component style in
  `ui::theme::Components` (and in `crates/ui/themes/steaminrust/theme.toml`,
  plus Classic if it should differ), so themes can style it too.
- Backgrounds: `w::paint_background(ui, t, rect, "store")`, or leave it to
  the shell, which paints each area's background behind the page.
- Layout choices a theme makes are in `t.layout` (top bar and page bar
  heights, navigation on top or left, friends list side, game card style,
  density, corners): read them rather than assuming the default.
- Never use Steam's logo, the word "STEAM" or other Valve branding.

Check a page with `--theme=classic` and with the example theme
(`--theme-dir docs/examples/minimal-theme`) as well as the default.

## Seeing your work

```bash
cargo run -p steam-rs -- --separate --demo --no-local --lang=en --page=gallery
```

- `--page=gallery` opens the design gallery: every shared control in each of
  its states, plus a sample page bar (`crates/ui/src/gallery.rs`). It's only
  there when asked for.
- `--theme=classic` (any built-in theme's id or name) shows that theme;
  `--theme-dir <folder>` loads a theme package from a folder. Neither is
  saved. `--window-size=1920x1080` sets the window size.
- `--offscreen` opens the window far off the screen, for screenshots of a test
  copy that never shows on the desktop. Capture it with `PrintWindow` and the
  `PW_RENDERFULLCONTENT` flag. Windows ignores real mouse messages for an
  off-screen window, so `--pointer=x,y` keeps a pretend pointer there (to see
  hover states and hover menus) and `--click=x,y` clicks once, 4 seconds in; `--type=<text>` then types into what it focused (`--type-submit` adds Ctrl+Enter).
- `--demo --no-local` shows a made-up account and library: use it for any
  screenshot you share.
- `--new-look-prompt` shows the one-time "A new look for SteamInRust"
  question that people who used the app before the redesign get (new
  installs start on SteamInRust and are never asked).

## Theme tokens

`ui::Theme` (`crates/ui/src/theme.rs`), read straight off the theme
(`t.accent`). The values below are the SteamInRust theme's, from the style
guide's "colors" frame (`11:103`); `docs/themes.md` has the full format.

| Token | SteamInRust | Style guide name | Use it for |
| --- | --- | --- | --- |
| `chrome` | `#0E141B` | Bg / Main | The top bar; the darkest fills |
| `bg` | `#181D24` | — | Page background where no area background applies |
| `raised` | `#1E2329` | Bg / Highlight | Menus, secondary buttons, tags, the top bar's buttons |
| `surface` | `#262C35` | (`#313843` at 50%) | Content cards ("Friends who play", "Activity") |
| `surface_hover` | `#313843` | Bg / Hover | Hover fills, selected chips and tags |
| `tertiary` | `#212B45` | Bg / Tertiary | The page bar under the top bar |
| `accent` | `#66C0F4` | Color / Primary | Links, the open section, focus (the user can change it) |
| `accent_2` | `#4B619B` | Color / Secondary | Primary buttons, the selected page-bar tab (the user can change it) |
| `on_accent` | `#0E141B` | — | Text on `accent` fills |
| `text` | `#F3F3F3` | Text / Main | Main text |
| `text_dim` | `#76808C` | Text / Dim | Secondary text, icons, labels |
| `text_faint` | `#525B66` | — | Hints, disabled text, captions |
| `play` | `#A1CD44` | Accent / Green | Positive: Play, in-game |
| `online` | `#66C0F4` | — | Online friends |
| `in_game` | `#A1CD44` | — | Friends in a game |
| `away` | `#C1B15F` | Accent / Yellow | Warnings, away, pending money |
| `danger` | `#CD5444` | Accent / Red | Removing, errors, negative reviews |
| `price` | `#F3F3F3` | — | A full price |
| `price_new` | `#A1CD44` | Accent / Green | A price after a discount |
| `price_old` | `#76808C` | Text / Dim | The struck-through old price |
| `discount` | `#A1CD44` | Accent / Green | The discount box |
| `discount_text` | `#0E141B` | Bg / Main | Text in the discount box |
| `border` | `#2A313B` | — | The rare line or outline (the design uses almost none) |
| `radius` | `5` | — | `t.corner()`: cards, panels |
| `radius_small` | `3` | — | `t.corner_small()`: buttons, tags, menus, inputs |

Beyond the colors:

- `t.backgrounds` per area: the store's is the style guide's
  `--gpGradient-StoreBackground` (`#2A475E` to `#1B2838`, reached 80% down),
  the library's `--gpGradient-LibraryBackground` (a radial glow from
  `#3D4450` 40 px in from the top-left to `#23262E`). Paint one with
  `w::paint_background`.
- `t.overlay_light(0..5)` and `t.overlay_dark(0..4)`: the guide's
  translucent `--gpBackground-…` washes, softest first.
- `t.spacing` (`xs` 4, `sm` 8, `md` 12, `lg` 16, `xl` 24, `item`, `card`).
- `t.text_on(fill)` gives near-black or near-white text for any fill.
- `t.components.*`: each widget's style; `t.look(component, state)` resolves
  its colors for a state, `w::paint_box` draws it.
- The guide's other values (`--gpSystem…` greys, `--gpStore…` blue greys,
  `--gpColor-Blue #1A9FFF`) are Valve's own client variables, copied onto the
  guide for reference. The screens use the "colors" frame, so they aren't
  tokens. If a page needs one, add a token rather than a hex value.

## Type

The design is set in Motiva Sans, which is Valve's and can't be shipped.
Inter stands in at the same sizes and weights (a theme can bring its own
fonts). The styles are `t.typography.*`, with text helpers in `ui::widgets`:

| Style guide | SteamInRust | Helper |
| --- | --- | --- |
| Heading Large | 26, bold | `w::heading_l(t, text, color)` |
| Heading Medium | 22, bold | `w::heading_m(t, …)` (also egui's `Heading` style) |
| Heading Small | 18, bold | `w::heading_s(t, …)` |
| Body Large | 16, regular | `w::body_l(t, …)` |
| Body Medium | 14, regular | `w::body_m(t, …)` (egui's `Body` style) |
| Body Small | 12, regular | `w::body_s(t, …)` |

Also `button`, `button_large`, `section`, `tab`, `caption`, `menu` and
`price` styles; `w::styled(text, t.typography.caption, color)` for any of
them.

## Shared controls

All in `crates/ui/src/widgets.rs` (`use ui::widgets as w`). Every one takes
`&Theme` and draws from its component style (named in brackets below; see
`docs/themes.md`), with hover, pressed, selected and disabled states; it
dims when the `Ui` is disabled (`ui.add_enabled_ui(false, …)`).

| Control | Figma | Call |
| --- | --- | --- |
| Primary button (`button_primary`) | "primary button" `84:791` | `w::button(ui, t, icon, "Add to Cart", w::Kind::Primary, w::Size::Lg)` |
| Secondary button (`button_secondary`) | "secondary button" `84:787` | `Kind::Secondary` |
| Tertiary button (`button_tertiary`) | "tertiary button 2" `400:13118` (outline at 20%, 40% on hover); `324:1431` is the same with a Bg / Highlight outline | `Kind::Tertiary` |
| Play (`button_play`) | "Play button" `335:1183` | `w::play_button(ui, t, "Play")` or `Kind::Play` |
| Install (`button_install`) | "install button" `375:4453` | `w::install_button(ui, t, "Install")` or `Kind::Install` |
| Pause (`button_pause`) | "pause button" `379:8204` | `w::pause_button(ui, t, "Pause")` or `Kind::Pause` |
| Other buttons (`button_ghost`, `button_danger`) | — | `Kind::Ghost` (no fill until hovered), `Kind::Danger` |
| Button sizes | 53 px in the guide | `Size::Md` is the component's height (40), `Size::Lg` a third taller (53), `Size::Sm` 80% (32); any height with `w::button_sized` |
| Main CTA (`button_cta`) | "main-cta" `426:4355` | `w::main_cta(ui, t, "$23.09", Some("$34.99"), 34, Some("Until October 19"), "Add to Cart")` returns the button's response |
| Price (`price`, the `price…` tokens) | "price" `426:4419` | `w::price(ui, t, label, original, discount)`; `w::price_tag` is smaller, for cards and rows |
| Tag (`tag`) | "tag" `280:203` | `w::tag(ui, t, "Souls-like", Some("5,000"), w::TagState::Checked)`; states `Plain`, `Unchecked`, `Checked`, `Current`, `Filtered` |
| Toggle | "toggle" `477:7301` | `w::toggle(ui, t, &mut on)`; `w::toggle_row(ui, t, &mut on, label)` puts a label before it |
| Check box | "Check box" `757:10005` | `w::check(ui, t, &mut on, "label")`; `w::checkbox(ui, t, &mut on, 30.0)` without a label |
| Sort filter | "sort filter" `296:579`, "sorting" `457:12113` | `w::sort_chip(ui, t, label, selected)`; `w::sort_by(ui, t, &mut value, &[(v, "New and Trending"), …])` adds "Sort by" before the chips |
| Dropdown | "sort filter" with its caret, "menu dropdown" `18:273` | `let r = w::dropdown(ui, t, label, min_width); egui::Popup::menu(&r).show(\|ui\| …)` |
| Menu | "menu dropdown" `18:273` | egui popups and context menus already take the theme's menu look (Bg / Highlight, soft shadow) |
| Menu item | "Menu items" `18:248` | `w::menu_item(ui, t, icon, label)`, `w::menu_item_tinted` (e.g. red), `w::submenu_item` |
| Wishlist | "wishlist" `280:345`, "wishlist page" `302:1745` | `w::wishlist_button(ui, t, wishlisted, Some("Wishlist"))`; `None` for the square heart |
| Reviews | "reviews" `289:270` | `w::review_bar(ui, t, positive_percent, 138.0)` |
| Page-bar tab | "menu" `82:322` | `w::sub_tab(ui, t, "Home", selected)` |
| Top-bar section | "store menu" `18:294` | `w::section_tab(ui, t, label, selected, height)` (the shell draws these) |
| All-caps tab | — | `w::nav_tab(ui, t, label, selected, height)`: dim, accent when selected (in-page tabs) |
| Search | "Search" `280:57` | `w::search(ui, t, &mut text, hint, width)` (34 px); `w::bar_search` (41 px) for a page bar |
| Icon box | "icon box" `280:175`, "icon select" `346:2627` | `w::icon_box(ui, t, icon, tooltip, selected)`: view switches, filters, the action row under a game's art |
| Top-bar buttons | "friends" `447:3849`, "notifications" `766:10160` | `w::bar_button(ui, t, icon, tooltip, active, badge)` |
| Badge | the blue count circle `447:3845` | `w::badge(painter, t, center, n)`; `w::icon_button` shows one too |
| Download ring | "download status" `383:10133` | `w::progress_ring(painter, t, center, 7.5, fraction)` |
| Card (`card`) | `523:6593` and the other content boxes | `w::card(t).show(ui, …)` (flat and borderless in SteamInRust); `w::component_frame(t, &t.components.x)` for any component as a frame |
| Chips (`chip`, `segmented`) | — | `w::chip`, `w::tinted_chip`, `w::toggle_chip`, `w::image_chip`, `w::segmented` |
| Game card (`game_card`) | "game card" `363:5954` | `w::game_card(ui, t, cover_url, header_url, title, installed, width)` follows the theme's card style (cover or header); `w::game_tile` draws a given size |
| Post card (`post_card`) | `494:5547` | `w::post_card(t).show(ui, …)`; a whole post: `builtin::hub::post::card`, two to a row: `post::masonry` |
| Filter bar (`filter_bar`) | "sorting" `489:5317` | `w::filter_bar(t).show(ui, …)` with `w::sort_chip`s inside |
| Count chip (`stat_chip`) | `494:5552` | `w::stat_chip(ui, t, icons::THUMBS_UP, Some("Yes"), Some("374"))` |
| Large search (`search_large`) | `487:5333` | `w::search_large(ui, t, &mut text, hint, width)` (55 px) |
| Page numbers (`sort`) | `505:8934` | `w::page_numbers(ui, t, page, pages)` returns the page clicked |
| Art behind a page | `494:5450` | `w::paint_cover_fading(ui, rect, url, opacity)`: fades out to nothing over any background |

`icons::PLAY` and `icons::PAUSE` are drawn as solid shapes in buttons (the
icon font only has outlines); `w::paint_icon` draws any icon the same way.
`w::paint_heart` draws the filled heart.

Not matched exactly, on purpose:

- Menu rows are 38 px tall with 14.5 px text, not the guide's 49 px with
  16–18 px text: the app's right-click menus are long, and 49 px rows made
  them run off the screen. The colors and padding are the guide's.
- The Wishlist button is 53 px tall like the other large buttons (the guide
  draws it 58 px), so it lines up next to "Add to Cart". The guide's red
  "remove" look on hover isn't there: a component has no "selected and
  hovered" state.
- The guide's primary button keeps white text on its light-blue hover; so
  does ours. Install turns its text white on hover, as the guide shows.
- The check box when on is filled in Text / Dim with a dark check, as the
  guide shows (not the accent color).

## The app shell

`apps/desktop/src/shell.rs` draws the window; `chrome.rs` the window buttons
and resize edges. Its look and parts of its layout come from the theme
(`t.layout` and the `top_bar`, `page_bar`, `status_bar`, `sidebar` and
`sidebar_item` components): Classic puts the search in the top bar, has no
page bar and keeps a status bar; a theme can move the sections into a left
column or the friends list to the left. This is the default theme's layout:

```
┌ top bar, 59 px, chrome ───────────────────────────────────────────────────────┐
│ ‹ ›  [logo]  STORE  COMMUNITY  <YOU>  LIBRARY  DOWNLOADS ◔ 46%   [👥4][🔔1][avatar name $5.25 ▾] Help View ▾  – ▢ ✕ │
├ page bar, 57 px, tertiary ────────────────────────────────────────────────────┤
│ [Home] Collections  News …    [Search…        🔍] [⧩][◷][▷]     View [≡][▦][▢]  Activate a Product … │
├───────────────────────────────────────────────────────────────────────────────┤
│ the page (the area's background behind it)                                    │
└───────────────────────────────────────────────────────────────────────────────┘
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
- On the right: the offline badge (when offline), the search (when the
  theme puts it in the top bar), Friends (opens the Friends & Chat
  window, see below), Notifications, the
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
right, centered vertically, with the `page_bar` component's padding in from
each end. A theme can turn the bar off (`t.layout.page_bar`), so a page's
main navigation shouldn't live only there. Return `false` (the default) and
the bar shows the app-wide search (games, friends, the store) when the theme
puts the search there.
A page that takes the bar over should have its own search there. The sample
in `builtin/gallery.rs` matches the library's bar.

Toasts, dialogs and the notifications menu are unchanged apart from taking
the new tokens and controls.

Friends and chat are one window, Friends & Chat (`friends_window.rs`, Figma
"friends" `447:3850`), like Steam's: there's no friends panel in the main
window and no friends page. The friends list is docked on the theme's
`friends_side` (the right), `friends_width` wide: you, your status and "Add a
friend"; Friends and Activity tabs; friends by Favorites, In game, Online and
Offline, then group chats. Opening a conversation widens the window toward
the other side (the list stays where it was), with the open conversations as
tabs. Chats, group chats and the Add a friend / Invites / Blocked pages are
drawn by the `friends` extension (`builtin/friends.rs`,
`builtin/friends_groups.rs`). The top bar's Friends button, View → Friends &
Chat, the tray and the status bar open it. Links open it too:
`Action::OpenChat(id)`, or navigating to the page `friends` with an argument
(a SteamID, `group:<id>`, `groups`, `activity`, `add`, `invites`,
`blocked`); `community` with one of those arguments still works. For
previews, `--friends` opens it and `--friends=chat`, `=group`, `=add`,
`=chat,activity`… open it on something.

Big Picture (`apps/desktop/src/bigpicture`) keeps its own look; it only picks
up the theme's colors.

## Screens and the code that draws them

From `decoded/frame_inventory.csv` (the UI page, `18:946`). "Today" is the
code that draws the closest screen now.

### Store

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

### Library and Downloads

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

Restyled: the page bar, the game list, the home,
grid and list views, collections, a game's page and Downloads. Pieces other
pages can use too: `w::game_card_info` / `w::game_tile_info` (the card's
hover state from `363:5954`: title, Play or Install, last played and time
played, given in `w::TileInfo`), `w::game_card_size`, and
`w::paint_cover_fade` (a picture that fades into the page, for heroes).
New components: `game_row` (the library's list), `shelf` (its blocks and list
rows) and `download_row`. `--no-friends-panel` starts a preview without the
friends list, at the design's width.

### Community

| Frame | Node | Today |
| --- | --- | --- |
| community home | `487:5076` | `builtin/community_home.rs`: Friends activity (`client::feed`, Steam's hidden activity feed), Explore (the community-wide feed), Developers (news of your games, announcements of followed ones), the Discussions landing, and the Community page bar every Community page shares |
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

### Profile, settings, friends

| Frame | Node | Today |
| --- | --- | --- |
| profile | `435:7035` | `builtin/profile.rs` |
| games | `455:9708` | `builtin/profile.rs` (`games_page`) |
| screenshots | `457:12553` | `builtin/hub/files.rs` (`user_files`) |
| settings | `475:6189` | `builtin/settings.rs`, `settings_account.rs` (its own `settings::toggle` can become `w::toggle`) |
| privacy settings | `477:7327` | `builtin/profile_edit.rs` (privacy) |
| profile settings | `476:6952` | `builtin/profile_edit.rs` |
| friends (popup) | `447:3850` | `friends_window.rs`, `builtin/friends.rs` (done: the Friends & Chat window) |
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

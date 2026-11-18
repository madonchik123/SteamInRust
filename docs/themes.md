# Making a theme

A SteamInRust theme is a skin: it decides the app's colors, backgrounds,
fonts, pictures, icons, the look of every button, tab, menu and card, and
parts of the layout (where the navigation sits, which side the friends list
is on, how dense things are). This page is everything you need to make one.

The two built-in themes are made the same way, and are good to read next to
this page:

- [`crates/ui/themes/steaminrust/theme.toml`](../crates/ui/themes/steaminrust/theme.toml)
  is **SteamInRust**, the default. It's written out in full: every other
  theme falls back to its values.
- [`crates/ui/themes/classic/theme.toml`](../crates/ui/themes/classic/theme.toml)
  is **Classic**, the app's look before the redesign. Aurora, Blue, OLED and
  Daylight (in the same folder) are Classic in other colors.

A small working example is in [`docs/examples/minimal-theme`](examples/minimal-theme).

## Trying a theme

Put your theme in a folder and start the app with it:

```bash
steam-rs --theme-dir "C:\path\to\my-theme"
```

Add `--demo --no-local` to see it with a made-up account and library, and
`--page=gallery` to see every control in each state. The theme is used for
that run only; nothing is saved. Problems are listed in the app's log
(start it from a terminal, or set `STEAMINRUST_LOG_FILE`), and the app never
stops over a theme: a file it can't read means the default theme, with the
reason in the log.

There's no theme picker for your own themes yet; for now themes are tried
with `--theme-dir`.

## The package

```
my-theme/
├── theme.toml        the theme (required)
├── preview.png       a picture of it, for pickers (optional)
├── fonts/            font files the theme uses (optional)
└── images/           backgrounds, textures, a logo (optional)
```

The folder names inside are up to you; `theme.toml` names each file by its
path from the theme's folder, with `/` between folders.

Rules for files:

- Only files inside the theme's folder can be used: no `..`, no full paths
  like `C:\…` or `/…`, no web addresses. Anything else is left out, with a
  warning.
- Pictures: PNG, JPEG, WebP, GIF or SVG. Fonts: TrueType (`.ttf`) or
  OpenType (`.otf`).
- `theme.toml` can be up to 1 MB and each other file up to 32 MB.
- A file that's missing or can't be used is left out with a warning; what
  needed it falls back (to the app's own font, or to no picture).

## `theme.toml`

The file is [TOML](https://toml.io). Only `format`, `id` and `name` are
required: everything you leave out comes from the theme named in `extends`,
or from the default theme. Tables merge key by key, so changing one color in
one state of one button is one line.

```toml
format = 1                 # the theme format this file is written for
id = "my-theme"            # lowercase, unique, stays the same across versions
name = "My Theme"
author = "Your name"
version = "1.0.0"          # your theme's own version
preview = "preview.png"    # optional
extends = "classic"        # optional: start from a built-in theme

[tokens]
accent = "#FF7A45"
```

`extends` takes a built-in theme's id: `steaminrust` (the default),
`classic`, `aurora`, `blue`, `oled` or `daylight`.

A theme written for a newer `format` than the app knows still loads, with a
warning; fields the app doesn't know are skipped.

The user's own accent colors and roundness (Settings → Appearance) go on top
of whichever theme is active.

### Colors

A color is written one of these ways:

| Written | Means |
| --- | --- |
| `"#66C0F4"` | a color |
| `"#0E141B99"` | a color with transparency (the last two digits: `00` clear, `FF` solid) |
| `"#FFF"` | short form |
| `"accent"` | a token's color (see below), also `"white"`, `"black"`, `"transparent"` |
| `"accent@0.5"` | a token (or any color) at 50% opacity |
| `"mix(accent, white, 0.2)"` | 20% of the way from the first color to the second |
| `"lighten(play, 0.1)"` | 10% toward white |
| `"darken(accent, 0.14)"` | 14% toward black |
| `"alpha(accent, 0.5)"` | the same as `accent@0.5` |
| `"text_on(accent)"` | near-black or near-white, whichever reads on that color |

These nest: `"lighten(darken(accent, 0.5), 0.1)"`. Use token names where you
can, so the accent color a user picks flows into your buttons. An unknown
name shows as bright magenta and is listed in the log.

### `[tokens]`: base colors and corners

Everything else refers to these by name.

| Token | For |
| --- | --- |
| `dark` | `true` for a dark theme (egui's dark defaults, the white logo) |
| `bg` | page background (where a section has no background of its own) |
| `chrome` | the top bar; the darkest fills |
| `surface` | content cards |
| `surface_hover` | hover fills, selected chips |
| `raised` | menus, inputs, secondary buttons, tags |
| `tertiary` | the page bar under the top bar |
| `border` | lines and outlines |
| `text` | main text |
| `text_dim` | secondary text, icons |
| `text_faint` | hints, captions, disabled text |
| `accent` | links, the open section, focus; the user can change it |
| `accent_2` | the second accent (primary buttons in the default theme); the user can change it |
| `on_accent` | text on the accent |
| `play` | positive: Play, in-game |
| `online` | friends online |
| `in_game` | friends in a game |
| `away` | warnings, away, pending money |
| `danger` | removing, errors, negative reviews |
| `price` | a full price |
| `price_new` | a price after a discount |
| `price_old` | the struck-through old price |
| `discount` | the discount box's fill |
| `discount_text` | the discount box's text |
| `radius` | large corner radius (cards, panels), in points |
| `radius_small` | small corner radius (buttons, tags, menus, inputs) |

### `[backgrounds]`: behind each part of the app

```toml
[backgrounds]
page = "bg"                                                   # settings, downloads…
store = { linear = ["#2A475E", "#1B2838"], end = 0.8 }
library = { radial = ["#3D4450", "#23262E"], center = [40, 40], end = 0.8 }
community = { image = "images/community.jpg", fit = "cover", tint = "#0E141B99" }
profile = "bg"
```

Each is a **fill**, which can be:

| Fill | Written |
| --- | --- |
| A color | `"#181D24"` or any color above |
| Top-to-bottom gradient | `{ linear = ["from", "to"], end = 0.8 }`: reaches the second color `end` of the way down (1 = at the bottom) |
| Left-to-right gradient | `{ linear = ["from", "to"], horizontal = true }` |
| Glow from a point | `{ radial = ["center", "edge"], center = [40, 40], end = 0.8 }`: `center` in points from the top-left corner; reaches the edge color `end` of the way to the farthest corner |
| A picture | `{ image = "images/x.jpg", fit = "cover" }`, `fit` is `cover` (fill, crop the rest), `stretch` or `tile` (repeat at its own size, for textures); `tint = "color"` lays a color over it |

Pages can still draw their own backgrounds over these (a game's art, say).

### `[overlays]`: washes over pictures

```toml
[overlays]
light = ["#EBF6FF1A", "#EBF6FF33", "#EBF6FF4D", "#EBF6FF66", "#EBF6FF80"]
dark = ["#0E141B33", "#0E141B66", "#0E141B99", "#0E141BCC"]
```

From softest to hardest. Used to dim art (a game not installed, a hover).

### `[spacing]`

Points: `xs` 4, `sm` 8, `md` 12, `lg` 16, `xl` 24, plus `item` (between
widgets in a row or column) and `card` (inside a content card).

### `[typography]`: text styles

Each is `{ size = 16, weight = "regular" }`; weights are `regular`,
`medium`, `semibold` and `bold`.

| Style | Default | For |
| --- | --- | --- |
| `heading_l` | 26 bold | page titles |
| `heading_m` | 22 bold | headings |
| `heading_s` | 18 bold | section titles |
| `body_l` | 16 | large text, page-bar tabs |
| `body_m` | 14 | most text |
| `body_s` | 12 | small notes, tags |
| `button` | 15 medium | buttons |
| `button_large` | 18 bold | large buttons (Play, Add to Cart) |
| `section` | 17 bold | the top bar's sections |
| `tab` | 16 | page-bar tabs |
| `caption` | 11 bold | small all-caps labels |
| `menu` | 14.5 | menu rows |
| `price` | 18 bold | prices |

### `[fonts]`

```toml
[fonts]
regular = "fonts/MyFont-Regular.ttf"
medium = "fonts/MyFont-Medium.ttf"
semibold = "fonts/MyFont-SemiBold.ttf"
bold = "fonts/MyFont-Bold.ttf"
```

Any weight you leave out uses the app's own font (Inter). Characters your
font doesn't have (Chinese, Japanese, Korean, Thai, Arabic…) fall back to
the system's fonts, as usual. Check the font's license lets you share it.

### `[images]`

```toml
[images]
logo = "images/logo.png"
```

`logo` replaces the app's logo inside the app (the top bar). It never
changes the window, taskbar or tray icon. Don't use Steam's or Valve's logo
or name.

### `[icons]`: replacing icons

```toml
[icons]
font = "fonts/my-icons.ttf"
```

The app's icons are characters in an icon font (Phosphor). A theme can bring
its own icon font with glyphs at the same code points: the icons it has
replace the app's, and the rest stay. The code points (in the font's private
use area):

| Icon | Code point | Icon | Code point |
| --- | --- | --- | --- |
| `house` | U+E2C2 | `thumbs_down` | U+E48C |
| `books` | U+E758 | `smiley` | U+E436 |
| `lightning` | U+E2DE | `dots` | U+E1FE |
| `chart_bar` | U+E150 | `external` | U+E5DE |
| `gear` | U+E270 | `cart` | U+E41E |
| `search` | U+E30C | `qr` | U+E3E6 |
| `play` | U+E3D0 | `key` | U+E2D6 |
| `users` | U+E4D6 | `lock` | U+E2FA |
| `user` | U+E4C2 | `warning` | U+E4E0 |
| `shield` | U+E40C | `refresh` | U+E036 |
| `folder` | U+E256 | `trash` | U+E4A6 |
| `power` | U+E3DA | `note` | U+E348 |
| `download` | U+E20C | `globe` | U+E288 |
| `list` | U+E2F0 | `monitor` | U+E32E |
| `grid` | U+E464 | `crown` | U+E614 |
| `x` | U+E4F6 | `plugs` | U+EB56 |
| `plus` | U+E3D4 | `sliders` | U+E432 |
| `back` | U+E138 | `eye` | U+E220 |
| `copy` | U+E1CA | `circle` | U+E18A |
| `controller` | U+E26E | `stop` | U+E46C |
| `clock` | U+E19A | `caret_up` | U+E13C |
| `drives` | U+E2A0 | `cloud_check` | U+E1B0 |
| `puzzle` | U+E596 | `cube` | U+E1DA |
| `palette` | U+E6C8 | `desktop` | U+E560 |
| `info` | U+E2CE | `branch` | U+E278 |
| `sign_out` | U+E42A | `heart` | U+E2A8 |
| `bell` | U+E0CE | `image` | U+E2CA |
| `dice` | U+E1EE | `link` | U+E2E2 |
| `chat` | U+E16C | `paint` | U+E6F0 |
| `chats` | U+E17C | `stack` | U+E466 |
| `store` | U+E470 | `backpack` | U+E922 |
| `user_circle` | U+E4C4 | `swap` | U+E0A0 |
| `send` | U+E396 | `medal` | U+E320 |
| `caret_down` | U+E136 | `camera` | U+E10E |
| `caret_right` | U+E13A | `users_three` | U+E68E |
| `caret_left` | U+E138 | `package` | U+E390 |
| `star` | U+E46A | `storefront` | U+E470 |
| `windows` | U+E692 | `gift` | U+E276 |
| `apple` | U+E516 | `funnel` | U+E266 |
| `linux` | U+EB02 | `lock_simple` | U+E308 |
| `check` | U+E182 | `scroll` | U+EB7A |
| `trophy` | U+E67E | `sort` | U+E444 |
| `push_pin` | U+E3E2 | `user_plus` | U+E4D0 |
| `push_pin_slash` | U+E3E4 | `eye_slash` | U+E224 |
| `arrow_left` | U+E058 | `coins` | U+E78E |
| `arrow_right` | U+E06C | `minus` | U+E32A |
| `tag` | U+E478 | `square` | U+E45E |
| `fire` | U+E242 | `restore` | U+E1CC |
| `sparkle` | U+E6A2 | `user_switch` | U+E756 |
| `calendar` | U+E108 | `question` | U+E3E8 |
| `thumbs_up` | U+E48E | `pause` | U+E39E |

### `[layout]`

| Option | Values | Default | Does |
| --- | --- | --- | --- |
| `top_bar_height` | points | 59 | the top bar's height |
| `page_bar` | `true`, `false` | `true` | the bar under the top bar (a page's tabs, search and view buttons) |
| `page_bar_height` | points | 57 | its height |
| `nav` | `"top"`, `"left"` | `"top"` | the main sections across the top bar, or in a column on the left |
| `nav_width` | points | 220 | the left column's width |
| `search` | `"page_bar"`, `"top_bar"` | `"page_bar"` | where the app-wide search box sits (a page with its own bar shows its own search) |
| `friends_side` | `"left"`, `"right"` | `"right"` | which side the friends list docks on |
| `friends_width` | points | 290 | its width |
| `status_bar` | `true`, `false` | `false` | a bar along the bottom with downloads and Friends & Chat |
| `status_bar_height` | points | 34 | its height |
| `game_card` | `"cover"`, `"header"` | `"cover"` | game cards as tall cover art or the wide header picture (pages using the shared game card) |
| `density` | `"comfortable"`, `"compact"` | `"comfortable"` | compact makes controls and text about 15% smaller |
| `corners` | `"rounded"`, `"square"` | `"rounded"` | square ignores every radius |

### `[components.*]`: the look of each widget

Every shared widget has a style. All fields are optional:

```toml
[components.button_primary]
height = 40                 # points; 0 lets the widget decide
padding = [20, 0]           # [left and right, top and bottom]
radius = "small"            # "small", "large" (the tokens), "full" (round ends), "none", or points
border = 1                  # outline width, drawn when a state has a border color
indicator = "fill"          # "fill", or "underline": a bar under it when selected
shadow = { offset = [0, 2], blur = 4, spread = 0, color = "accent@0.3" }
text = "button"             # a text style's name, or { size = 15, weight = "bold" }
uppercase = false

normal = { fill = "accent_2", text = "text_on(accent_2)" }
hover = { fill = "accent" }
pressed = { fill = "darken(accent, 0.14)" }
selected = { }
disabled = { fill = "accent_2@0.45", text = "text_on(accent_2)@0.55" }
```

Each state can set `fill` (any fill: color, gradient or picture), `text`,
`border` and `icon` (the icon's color; the text color if left out). A state
only needs what changes: the widget starts from `normal`, takes `selected`
on top when it's selected, blends toward `hover` as the pointer arrives,
then `pressed` while clicked and `disabled` when it can't be used.

| Component | Is |
| --- | --- |
| `button_primary` | the main action ("Add to Cart") |
| `button_secondary` | other actions ("Add to Wishlist") |
| `button_tertiary` | quiet actions ("Ignore") |
| `button_ghost` | buttons with no fill until hovered |
| `button_play` | Play |
| `button_install` | Install |
| `button_pause` | Pause (a download) |
| `button_danger` | Uninstall, Remove… |
| `button_cta` | the store page's buy button, beside the price |
| `icon_button` | small square buttons with an icon |
| `bar_button` | the top bar's Friends, Notifications and account buttons |
| `icon_box` | view switches and filters (`selected` is the view showing) |
| `badge` | count badges (`normal` only) |
| `section_tab` | the top bar's sections (`selected` is the open one) |
| `page_tab` | tabs in the page bar |
| `nav_tab` | all-caps tabs inside pages |
| `chip` | filter chips |
| `tag` | tags (`normal` a game's tag, `selected` a filter that's on) |
| `sort` | "Sort by" choices and dropdown buttons |
| `segmented` | the block behind a row of choices (`normal` only; the choices use `sort`) |
| `card` | content cards (`normal` only; `padding` is the space inside) |
| `input` | search and text fields (`icon` is the hint and magnifier color, `selected` is focused) |
| `toggle` | on/off switches: the track is the fill, the knob the `text` color; `selected` is on |
| `check` | check boxes: `selected` is checked, its `text` the check mark |
| `menu` | popup menus' frame (`normal` only) |
| `menu_item` | rows in menus |
| `top_bar` | the top bar (`normal`; `border` draws a line under it) |
| `page_bar` | the page bar |
| `status_bar` | the bottom bar (`border` draws a line over it) |
| `sidebar` | the left navigation column and the friends list |
| `sidebar_item` | rows in the left navigation column |
| `game_card` | game cards (`hover` `border` outlines the one under the pointer) |
| `price` | the discount box ("-34%") |
| `wishlist` | the Wishlist button (`selected` is wishlisted) |
| `progress` | progress bars: `normal` the track, `selected` the filled part |
| `toast` | pop-up messages (`selected` `fill` is the bar at their side) |

## What a theme can't change

- The window, taskbar and tray icons.
- Pages' own pictures (game art, avatars, screenshots).
- Where things are inside a page, beyond the layout options above.

## For the app's developers

- A theme is `ui::Theme` (`crates/ui/src/theme.rs`); loading, built-ins and
  the user's changes are in `crates/ui/src/themes.rs`.
- Widgets take their style from `t.components.*` through `t.look(...)` and
  draw it with `w::paint_box`; pages use widgets and theme values only
  (`docs/redesign.md`).
- A new built-in theme is a folder under `crates/ui/themes/` added to
  `BUILTINS` in `themes.rs`. The tests check every built-in loads without
  problems, survives being written out and read back, and draws the gallery.
- A new field needs a default in the default theme's file (or a `Default`)
  so older themes keep loading; raise `FORMAT` when a change means older
  apps would draw a theme wrongly.

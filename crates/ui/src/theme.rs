//! Themes: everything about how the app looks, as data.
//!
//! A [`Theme`] is the parsed `theme.toml` of a theme package (see
//! `docs/themes.md`): base color tokens, background fills, overlays,
//! spacing, text styles, fonts, images, an icon font, per-widget styles
//! ([`Components`]) and layout options ([`Layout`]). Widgets and the app
//! shell draw only from it; pages use widgets and theme values, never a
//! color, size or font of their own. The built-in themes and user themes
//! are the same kind of package ([`crate::themes`]).
//!
//! Colors in component styles and fills are [`ColorRef`]s: a hex value or a
//! token name, with a little arithmetic (`accent@0.5`, `lighten(play, 0.1)`),
//! so the accent color the user picks flows into every widget.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The theme file format this app reads. A theme states the format it was
/// written for; newer formats only add fields.
pub const FORMAT: u32 = 1;

// ---------- the theme ----------

/// A theme package's `theme.toml`, ready to draw with. Base tokens are
/// reachable straight from the theme (`t.accent`, `t.radius`) through
/// [`Tokens`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    /// The format version the theme was written for ([`FORMAT`]).
    pub format: u32,
    /// Stable identifier, lowercase (`"steaminrust"`, `"classic"`, `"my-theme"`).
    pub id: String,
    /// Display name.
    pub name: String,
    #[serde(default)]
    pub author: String,
    /// The theme's own version, e.g. `"1.0.0"`.
    #[serde(default)]
    pub version: String,
    /// A picture of the theme for pickers, inside the package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// The built-in theme this one starts from (`"classic"`). Anything the
    /// file leaves out comes from it, or from the default theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,

    pub tokens: Tokens,
    #[serde(default)]
    pub backgrounds: Backgrounds,
    #[serde(default)]
    pub overlays: Overlays,
    #[serde(default)]
    pub spacing: Spacing,
    #[serde(default)]
    pub typography: TextStyles,
    #[serde(default)]
    pub fonts: Fonts,
    #[serde(default)]
    pub images: Images,
    #[serde(default)]
    pub icons: Icons,
    #[serde(default)]
    pub layout: Layout,
    /// Shared, so copying a theme stays cheap.
    #[serde(default)]
    pub components: Arc<Components>,

    /// The package's files (fonts, images), loaded from inside its folder.
    #[serde(skip)]
    pub assets: Assets,
}

impl std::ops::Deref for Theme {
    type Target = Tokens;
    fn deref(&self) -> &Tokens {
        &self.tokens
    }
}

impl std::ops::DerefMut for Theme {
    fn deref_mut(&mut self) -> &mut Tokens {
        &mut self.tokens
    }
}

impl Default for Theme {
    /// The default theme ("SteamInRust").
    fn default() -> Self {
        crate::themes::default_theme()
    }
}

/// Base colors and corner radii. Everything else can refer to these by
/// name. Read them straight off the theme: `t.bg`, `t.accent`, `t.radius`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    /// A dark theme (dark egui defaults, the white logo).
    pub dark: bool,
    /// Page background.
    #[serde(with = "hex")]
    pub bg: Color32,
    /// The top bar, and the darkest fills (search fields, sort chips).
    #[serde(with = "hex")]
    pub chrome: Color32,
    /// Content cards.
    #[serde(with = "hex")]
    pub surface: Color32,
    /// Hover fills, selected chips.
    #[serde(with = "hex")]
    pub surface_hover: Color32,
    /// Menus, inputs, secondary buttons, tags.
    #[serde(with = "hex")]
    pub raised: Color32,
    /// The page bar under the top bar.
    #[serde(with = "hex")]
    pub tertiary: Color32,
    /// Lines and outlines.
    #[serde(with = "hex")]
    pub border: Color32,
    /// Main text.
    #[serde(with = "hex")]
    pub text: Color32,
    /// Secondary text, icons.
    #[serde(with = "hex")]
    pub text_dim: Color32,
    /// Hints, captions, disabled text.
    #[serde(with = "hex")]
    pub text_faint: Color32,
    /// The accent: links, the open section, focus.
    #[serde(with = "hex")]
    pub accent: Color32,
    /// The second accent: primary buttons, the selected tab in a page bar.
    #[serde(with = "hex")]
    pub accent_2: Color32,
    /// Text on accent fills.
    #[serde(with = "hex")]
    pub on_accent: Color32,
    /// Positive: Play, in-game.
    #[serde(with = "hex")]
    pub play: Color32,
    /// Friends online.
    #[serde(with = "hex")]
    pub online: Color32,
    /// Friends in a game.
    #[serde(with = "hex")]
    pub in_game: Color32,
    /// Warning, away, money pending.
    #[serde(with = "hex")]
    pub away: Color32,
    /// Danger: removing, errors, negative reviews.
    #[serde(with = "hex")]
    pub danger: Color32,
    /// A full price.
    #[serde(with = "hex")]
    pub price: Color32,
    /// A price after a discount.
    #[serde(with = "hex")]
    pub price_new: Color32,
    /// The struck-through old price.
    #[serde(with = "hex")]
    pub price_old: Color32,
    /// The discount block's fill.
    #[serde(with = "hex")]
    pub discount: Color32,
    /// The discount block's text.
    #[serde(with = "hex")]
    pub discount_text: Color32,
    /// Large corner radius: cards, panels, windows.
    pub radius: u8,
    /// Small corner radius: buttons, tags, menus, inputs.
    pub radius_small: u8,
}

/// Backgrounds per area of the app, painted by the shell behind the page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Backgrounds {
    /// Pages with no area of their own (settings, downloads…).
    pub page: Fill,
    pub store: Fill,
    pub library: Fill,
    pub community: Fill,
    pub profile: Fill,
}

impl Default for Backgrounds {
    fn default() -> Self {
        let bg = Fill::Color(ColorRef::from("bg"));
        Self { page: bg.clone(), store: bg.clone(), library: bg.clone(), community: bg.clone(), profile: bg }
    }
}

/// See-through washes over pictures, from softest to hardest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlays {
    pub light: Vec<ColorRef>,
    pub dark: Vec<ColorRef>,
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            light: ["#ebf6ff1a", "#ebf6ff33", "#ebf6ff4d", "#ebf6ff66", "#ebf6ff80"].map(ColorRef::from).to_vec(),
            dark: ["#0e141b33", "#0e141b66", "#0e141b99", "#0e141bcc"].map(ColorRef::from).to_vec(),
        }
    }
}

/// Spacing steps, in points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spacing {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    /// Space between widgets in a row or column.
    pub item: f32,
    /// Space inside a content card.
    pub card: f32,
}

impl Default for Spacing {
    fn default() -> Self {
        Self { xs: 4.0, sm: 8.0, md: 12.0, lg: 16.0, xl: 24.0, item: 8.0, card: 22.0 }
    }
}

/// Font weights a theme can use. Each maps to a font file ([`Fonts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weight {
    #[default]
    Regular,
    Medium,
    Semibold,
    Bold,
}

impl Weight {
    pub fn family(self) -> FontFamily {
        match self {
            Weight::Regular => FontFamily::Proportional,
            Weight::Medium => medium(),
            Weight::Semibold => semibold(),
            Weight::Bold => bold(),
        }
    }
}

/// A size and weight.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub size: f32,
    #[serde(default)]
    pub weight: Weight,
}

impl TextStyle {
    pub const fn new(size: f32, weight: Weight) -> Self {
        Self { size, weight }
    }
    pub fn font(&self) -> FontId {
        FontId::new(self.size, self.weight.family())
    }
}

/// The text styles. Component styles name one of these ([`TextRef`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyles {
    pub heading_l: TextStyle,
    pub heading_m: TextStyle,
    pub heading_s: TextStyle,
    pub body_l: TextStyle,
    pub body_m: TextStyle,
    pub body_s: TextStyle,
    /// Buttons at the small and medium sizes.
    pub button: TextStyle,
    /// Large buttons (Play, Add to Cart).
    pub button_large: TextStyle,
    /// The top bar's sections.
    pub section: TextStyle,
    /// Tabs in a page bar.
    pub tab: TextStyle,
    /// Small all-caps group labels.
    pub caption: TextStyle,
    /// Menu rows.
    pub menu: TextStyle,
    /// Prices in the price block.
    pub price: TextStyle,
}

impl Default for TextStyles {
    fn default() -> Self {
        use Weight::*;
        Self {
            heading_l: TextStyle::new(26.0, Bold),
            heading_m: TextStyle::new(22.0, Bold),
            heading_s: TextStyle::new(18.0, Bold),
            body_l: TextStyle::new(16.0, Regular),
            body_m: TextStyle::new(14.0, Regular),
            body_s: TextStyle::new(12.0, Regular),
            button: TextStyle::new(15.0, Medium),
            button_large: TextStyle::new(18.0, Bold),
            section: TextStyle::new(17.0, Bold),
            tab: TextStyle::new(16.0, Regular),
            caption: TextStyle::new(11.0, Bold),
            menu: TextStyle::new(14.5, Regular),
            price: TextStyle::new(18.0, Bold),
        }
    }
}

impl TextStyles {
    /// A style by its name in a theme file (`"heading_s"`).
    pub fn get(&self, name: &str) -> Option<TextStyle> {
        Some(match name {
            "heading_l" => self.heading_l,
            "heading_m" => self.heading_m,
            "heading_s" => self.heading_s,
            "body_l" => self.body_l,
            "body_m" => self.body_m,
            "body_s" => self.body_s,
            "button" => self.button,
            "button_large" => self.button_large,
            "section" => self.section,
            "tab" => self.tab,
            "caption" => self.caption,
            "menu" => self.menu,
            "price" => self.price,
            _ => return None,
        })
    }
}

/// Font files inside the package, one per weight. Any left out use the
/// app's own font (Inter).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fonts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regular: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semibold: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<String>,
}

/// Pictures inside the package.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Images {
    /// Replaces the app's logo inside the app (the top bar). Never the
    /// tray or window icon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}

/// Icon replacements: a font with glyphs at the same code points as the
/// app's icons (`docs/themes.md` lists them). Icons it doesn't have fall
/// back to the built-in set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Icons {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

// ---------- layout ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavPosition {
    /// The main sections across the top bar.
    #[default]
    Top,
    /// A column down the left side.
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchPlace {
    /// In the page bar, when the page doesn't fill it itself.
    #[default]
    PageBar,
    /// In the top bar.
    TopBar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardStyle {
    /// Tall cover art (2:3).
    #[default]
    Cover,
    /// The wide header picture (460:215).
    Header,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    #[default]
    Comfortable,
    /// Controls and spacing about 15% smaller.
    Compact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corners {
    #[default]
    Rounded,
    /// Every corner square, whatever the radii say.
    Square,
}

/// Layout choices the app reads instead of fixed sizes and places.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub top_bar_height: f32,
    /// Show the page bar under the top bar.
    pub page_bar: bool,
    pub page_bar_height: f32,
    pub nav: NavPosition,
    /// Width of the left navigation column (`nav = "left"`).
    pub nav_width: f32,
    pub search: SearchPlace,
    /// Which side of the Friends & Chat window the friends list docks on (an
    /// open chat grows the window toward the other side).
    pub friends_side: Side,
    /// The friends list's width in that window.
    pub friends_width: f32,
    /// The bottom bar with downloads and Friends & Chat.
    pub status_bar: bool,
    pub status_bar_height: f32,
    pub game_card: CardStyle,
    pub density: Density,
    pub corners: Corners,
    /// The whole app's size for this theme, on top of the user's own zoom:
    /// 0.85 draws everything 15% smaller. Lets a theme keep its design's
    /// numbers while showing at a comfortable size.
    pub scale: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            top_bar_height: 59.0,
            page_bar: true,
            page_bar_height: 57.0,
            nav: NavPosition::Top,
            nav_width: 220.0,
            search: SearchPlace::PageBar,
            friends_side: Side::Right,
            friends_width: 338.0,
            status_bar: false,
            status_bar_height: 34.0,
            game_card: CardStyle::Cover,
            density: Density::Comfortable,
            corners: Corners::Rounded,
            scale: 1.0,
        }
    }
}

// ---------- colors and fills ----------

/// A color in a theme file: `"#66C0F4"`, `"#0E141B99"` (with alpha), a token
/// name (`"accent"`, `"surface_hover"`, also `"white"`, `"black"`,
/// `"transparent"`), `"token@0.5"` (that token at 50% opacity), or a
/// function: `mix(a, b, 0.3)`, `lighten(a, 0.1)`, `darken(a, 0.1)`,
/// `alpha(a, 0.5)`, `text_on(a)` (dark or light text that reads on `a`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ColorRef(pub String);

impl From<&str> for ColorRef {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// How a picture covers its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Fill the box, cropping the overflow.
    #[default]
    Cover,
    /// Stretch to the box.
    Stretch,
    /// Repeat at its own size (textures).
    Tile,
}

/// What fills a box: a color, a gradient, or a picture from the package.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Fill {
    /// `fill = "accent"`
    Color(ColorRef),
    /// `fill = { linear = ["#2A475E", "#1B2838"], end = 0.8 }`: top to bottom
    /// (or left to right with `horizontal = true`), reaching the second
    /// color `end` of the way.
    Linear {
        linear: [ColorRef; 2],
        #[serde(default)]
        horizontal: bool,
        #[serde(default = "one")]
        end: f32,
    },
    /// `fill = { radial = ["#3D4450", "#23262E"], center = [40, 40], end = 0.8 }`:
    /// from a point (`center`, in points from the top-left corner) out to
    /// `end` of the way to the farthest corner.
    Radial {
        radial: [ColorRef; 2],
        #[serde(default)]
        center: [f32; 2],
        #[serde(default = "one")]
        end: f32,
    },
    /// `fill = { image = "images/store.jpg", fit = "cover", tint = "#00000066" }`
    Image {
        image: String,
        #[serde(default)]
        fit: Fit,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tint: Option<ColorRef>,
    },
}

fn one() -> f32 {
    1.0
}

impl Default for Fill {
    fn default() -> Self {
        Fill::Color(ColorRef::from("transparent"))
    }
}

// ---------- component styles ----------

/// A corner radius: `"small"` or `"large"` (the theme's radii), `"full"`
/// (round ends), `"none"`, or a number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Radius {
    Px(f32),
    Named(String),
}

impl Default for Radius {
    fn default() -> Self {
        Radius::Named("small".into())
    }
}

/// How a component shows it's selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Indicator {
    /// The `selected` state's fill and colors.
    #[default]
    Fill,
    /// Also a bar under it, in the `selected` state's border color.
    Underline,
}

/// A text style by name (`"button"`) or written out (`{ size = 15, weight = "bold" }`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextRef {
    Named(String),
    Custom(TextStyle),
}

impl Default for TextRef {
    fn default() -> Self {
        TextRef::Named("body_m".into())
    }
}

/// A drop shadow; a transparent color means none.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shadow {
    pub offset: [f32; 2],
    pub blur: f32,
    pub spread: f32,
    pub color: ColorRef,
}

impl Default for Shadow {
    fn default() -> Self {
        Self { offset: [0.0, 0.0], blur: 0.0, spread: 0.0, color: ColorRef::from("transparent") }
    }
}

/// The colors of one state. Anything left out comes from `normal` (and,
/// while selected, from `selected`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StateStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<ColorRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<ColorRef>,
    /// Icon color; the text color when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<ColorRef>,
}

/// One widget's look: shape, size, text, and colors per state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Component {
    /// Height in points; 0 lets the widget decide.
    pub height: f32,
    /// Inner space: [left and right, top and bottom].
    pub padding: [f32; 2],
    pub radius: Radius,
    /// Outline width (drawn when the state has a `border` color).
    pub border: f32,
    pub indicator: Indicator,
    pub shadow: Shadow,
    pub text: TextRef,
    /// Show the label in capitals.
    pub uppercase: bool,
    pub normal: StateStyle,
    pub hover: StateStyle,
    pub pressed: StateStyle,
    pub selected: StateStyle,
    pub disabled: StateStyle,
}

impl Default for Component {
    fn default() -> Self {
        Self {
            height: 0.0,
            padding: [0.0, 0.0],
            radius: Radius::default(),
            border: 1.0,
            indicator: Indicator::Fill,
            shadow: Shadow::default(),
            text: TextRef::default(),
            uppercase: false,
            normal: StateStyle::default(),
            hover: StateStyle::default(),
            pressed: StateStyle::default(),
            selected: StateStyle::default(),
            disabled: StateStyle::default(),
        }
    }
}

/// Every widget a theme can style. `docs/themes.md` says what each is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Components {
    pub button_primary: Component,
    pub button_secondary: Component,
    pub button_tertiary: Component,
    pub button_ghost: Component,
    pub button_play: Component,
    pub button_install: Component,
    pub button_pause: Component,
    pub button_danger: Component,
    /// The store page's buy button (main CTA).
    pub button_cta: Component,
    pub icon_button: Component,
    /// The top bar's Friends and Notifications blocks, and the account button.
    pub bar_button: Component,
    /// Small square buttons (view switches, filters).
    pub icon_box: Component,
    /// Count badges.
    pub badge: Component,
    /// The top bar's sections.
    pub section_tab: Component,
    /// Tabs in a page bar.
    pub page_tab: Component,
    /// All-caps tabs inside pages.
    pub nav_tab: Component,
    pub chip: Component,
    pub tag: Component,
    /// Choices in a "Sort by" row and dropdown buttons.
    pub sort: Component,
    /// The block behind segmented choices.
    pub segmented: Component,
    pub card: Component,
    /// Text fields and search boxes.
    pub input: Component,
    /// Track: `normal` off, `selected` on. Knob: the state's `text` color.
    pub toggle: Component,
    /// Box: `normal` off, `selected` on. Check mark: the state's `text` color.
    pub check: Component,
    /// Popup menus' frame.
    pub menu: Component,
    pub menu_item: Component,
    pub top_bar: Component,
    pub page_bar: Component,
    pub status_bar: Component,
    /// The left navigation column.
    pub sidebar: Component,
    /// Rows in the left navigation column.
    pub sidebar_item: Component,
    pub game_card: Component,
    /// The price block's discount box.
    pub price: Component,
    pub wishlist: Component,
    /// The thin bar of `w::progress` (track `normal`, filled part `selected`).
    pub progress: Component,
    /// Toast messages.
    pub toast: Component,
    /// A game card on the store's shelves and carousel (Figma `426:4456`).
    pub store_card: Component,
    /// The block behind a store shelf or carousel (Figma `634:15211`).
    pub shelf: Component,
    /// Page dots under a carousel: `normal` a dot, `selected` the current
    /// page (filling up while it waits to move on; Figma `634:15220`).
    pub pager: Component,
    /// Game rows in the library's list beside the page (`selected` is the open game).
    pub game_row: Component,
    /// The library's blocks: a collection's games, a sidebar group, a game in the list view.
    pub library_shelf: Component,
    /// A game in the Downloads list.
    pub download_row: Component,
    /// A post in a social feed (the Community home, a game's hub; Figma
    /// `494:5547`).
    pub post_card: Component,
    /// The bar holding a feed's filters and sort (Figma "sorting" `489:5317`).
    pub filter_bar: Component,
    /// A small box with an icon and a count under a post (Figma `494:5552`):
    /// `icon` for the icon, `text` for the label, `selected.text` for the count.
    pub stat_chip: Component,
    /// The big search box of a landing page (Figma `487:5333`).
    pub search_large: Component,
    /// The profile's page-wide block behind its header and columns.
    pub profile_panel: Component,
    /// A titled box on a profile ("Recent Activity", "Comments"); its
    /// `text` is the title's style and color.
    pub profile_box: Component,
    /// The darker block inside a profile box, and the profile's side column.
    pub profile_inset: Component,
    /// A row on a profile: its links ("Games 350"), friends, games.
    pub profile_row: Component,
    /// The column of sections in Settings and Profile Settings.
    pub settings_nav: Component,
    /// A section in that column (`selected` is the open one).
    pub settings_nav_item: Component,
    /// A group of settings ("Profile", "Contact Info"); its `text` is the
    /// group title's style and color.
    pub settings_group: Component,
    /// One notification in the notifications popup (`selected` is unread).
    pub notification: Component,
    /// The Friends & Chat window's list column (`normal` fill, `padding`).
    pub friends_panel: Component,
    /// The box holding the friends list, under its Friends / Activity tabs.
    pub friends_box: Component,
    /// A row in the friends list: `height` (with the gap to the next row),
    /// `padding` around the row's content, `text` the name; `hover` and
    /// `selected` (the open conversation) fills.
    pub friend_row: Component,
    /// Avatars in the friends list and chat: `radius`, and `border` drawn in
    /// the friend's status color (0: a round status dot instead).
    pub friend_avatar: Component,
}

// ---------- assets ----------

/// The package's files, read once when the theme loads. Compared by
/// identity, not content.
#[derive(Clone, Debug, Default)]
pub struct Assets(pub Option<Arc<std::collections::HashMap<String, Arc<[u8]>>>>);

impl PartialEq for Assets {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Assets {
    pub fn get(&self, path: &str) -> Option<Arc<[u8]>> {
        self.0.as_ref()?.get(path).cloned()
    }
}

// ---------- reading the theme ----------

/// What a widget is doing, to pick its colors.
#[derive(Clone, Copy, Debug, Default)]
pub struct Interact {
    /// 0..1, animated.
    pub hover: f32,
    pub pressed: bool,
    pub selected: bool,
    pub disabled: bool,
}

/// A component's colors for one moment.
#[derive(Clone, Debug)]
pub struct Look {
    pub fill: ResolvedFill,
    pub text: Color32,
    pub border: Color32,
    pub icon: Color32,
}

/// A [`Fill`] with its colors worked out.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedFill {
    Color(Color32),
    Linear { a: Color32, b: Color32, horizontal: bool, end: f32 },
    Radial { a: Color32, b: Color32, center: [f32; 2], end: f32 },
    Image { path: String, fit: Fit, tint: Option<Color32> },
}

impl Theme {
    /// A [`ColorRef`] as a color. Unknown names give magenta, so a typo
    /// shows (and [`Theme::problems`] lists it).
    pub fn color(&self, c: &ColorRef) -> Color32 {
        self.parse_color(&c.0).unwrap_or(Color32::from_rgb(255, 0, 255))
    }

    fn token(&self, name: &str) -> Option<Color32> {
        let t = &self.tokens;
        Some(match name {
            "bg" => t.bg,
            "chrome" => t.chrome,
            "surface" => t.surface,
            "surface_hover" => t.surface_hover,
            "raised" => t.raised,
            "tertiary" => t.tertiary,
            "border" => t.border,
            "text" => t.text,
            "text_dim" => t.text_dim,
            "text_faint" => t.text_faint,
            "accent" => t.accent,
            "accent_2" => t.accent_2,
            "on_accent" => t.on_accent,
            "play" => t.play,
            "online" => t.online,
            "in_game" => t.in_game,
            "away" => t.away,
            "danger" => t.danger,
            "price" => t.price,
            "price_new" => t.price_new,
            "price_old" => t.price_old,
            "discount" => t.discount,
            "discount_text" => t.discount_text,
            "white" => Color32::WHITE,
            "black" => Color32::BLACK,
            "transparent" => Color32::TRANSPARENT,
            _ => return None,
        })
    }

    fn parse_color(&self, s: &str) -> Option<Color32> {
        let s = s.trim();
        if let Some((base, k)) = s.rsplit_once('@').filter(|(_, k)| !k.contains(')')) {
            return Some(self.parse_color(base)?.gamma_multiply(k.trim().parse::<f32>().ok()?.clamp(0.0, 1.0)));
        }
        if s.starts_with('#') {
            return parse_hex(s);
        }
        if let Some((func, rest)) = s.split_once('(') {
            let args = split_args(rest.strip_suffix(')')?);
            let num = |i: usize| args.get(i).and_then(|a| a.trim().parse::<f32>().ok());
            let col = |i: usize| args.get(i).and_then(|a| self.parse_color(a));
            return Some(match func.trim() {
                "mix" => crate::widgets::mix(col(0)?, col(1)?, num(2)?),
                "lighten" => crate::widgets::mix(col(0)?, Color32::WHITE, num(1)?),
                "darken" => crate::widgets::mix(col(0)?, Color32::BLACK, num(1)?),
                "alpha" => col(0)?.gamma_multiply(num(1)?.clamp(0.0, 1.0)),
                "text_on" => text_on(col(0)?),
                _ => return None,
            });
        }
        self.token(s)
    }

    /// Resolve a fill's colors.
    pub fn fill(&self, f: &Fill) -> ResolvedFill {
        match f {
            Fill::Color(c) => ResolvedFill::Color(self.color(c)),
            Fill::Linear { linear: [a, b], horizontal, end } => ResolvedFill::Linear { a: self.color(a), b: self.color(b), horizontal: *horizontal, end: *end },
            Fill::Radial { radial: [a, b], center, end } => ResolvedFill::Radial { a: self.color(a), b: self.color(b), center: *center, end: *end },
            Fill::Image { image, fit, tint } => ResolvedFill::Image { path: image.clone(), fit: *fit, tint: tint.as_ref().map(|c| self.color(c)) },
        }
    }

    /// A component's colors in a given state: `normal`, then `selected`
    /// (when selected), blended toward `hover` as the pointer arrives, then
    /// `pressed` and `disabled` on top.
    pub fn look(&self, c: &Component, s: Interact) -> Look {
        let layer = |base: &StateStyle, top: &StateStyle| StateStyle {
            fill: top.fill.clone().or_else(|| base.fill.clone()),
            text: top.text.clone().or_else(|| base.text.clone()),
            border: top.border.clone().or_else(|| base.border.clone()),
            icon: top.icon.clone().or_else(|| base.icon.clone()),
        };
        let mut base = c.normal.clone();
        if s.selected {
            base = layer(&base, &c.selected);
        }
        let hovered = layer(&base, &c.hover);
        let resolve = |st: &StateStyle| {
            let text = st.text.as_ref().map(|x| self.color(x)).unwrap_or(self.text);
            Look {
                fill: self.fill(&st.fill.clone().unwrap_or_default()),
                text,
                border: st.border.as_ref().map(|x| self.color(x)).unwrap_or(Color32::TRANSPARENT),
                icon: st.icon.as_ref().map(|x| self.color(x)).unwrap_or(text),
            }
        };
        let mut look = if s.hover > 0.0 {
            let (a, b) = (resolve(&base), resolve(&hovered));
            let k = s.hover;
            Look {
                fill: match (&a.fill, &b.fill) {
                    (ResolvedFill::Color(x), ResolvedFill::Color(y)) => ResolvedFill::Color(crate::widgets::mix(*x, *y, k)),
                    _ if k > 0.5 => b.fill.clone(),
                    _ => a.fill.clone(),
                },
                text: crate::widgets::mix(a.text, b.text, k),
                border: crate::widgets::mix(a.border, b.border, k),
                icon: crate::widgets::mix(a.icon, b.icon, k),
            }
        } else {
            resolve(&base)
        };
        if s.pressed {
            look = resolve(&layer(&layer(&base, &c.hover), &c.pressed));
        }
        if s.disabled {
            look = resolve(&layer(&base, &c.disabled));
        }
        look
    }

    /// A component's text style.
    pub fn text_style(&self, r: &TextRef) -> TextStyle {
        match r {
            TextRef::Named(n) => self.typography.get(n).unwrap_or(self.typography.body_m),
            TextRef::Custom(s) => *s,
        }
    }

    /// A component's text font, density applied.
    pub fn font_of(&self, c: &Component) -> FontId {
        let s = self.text_style(&c.text);
        FontId::new(s.size * self.density_text(), s.weight.family())
    }

    /// A component's corner radius.
    pub fn radius_of(&self, c: &Component) -> CornerRadius {
        if self.layout.corners == Corners::Square {
            return CornerRadius::ZERO;
        }
        CornerRadius::same(match &c.radius {
            Radius::Px(p) => p.round().clamp(0.0, 255.0) as u8,
            Radius::Named(n) => match n.as_str() {
                "large" => self.radius,
                "full" => 255,
                "none" => 0,
                _ => self.radius_small,
            },
        })
    }

    /// Scale a size by the theme's density (compact is 15% smaller).
    pub fn dense(&self, x: f32) -> f32 {
        match self.layout.density {
            Density::Comfortable => x,
            Density::Compact => (x * 0.85).round(),
        }
    }

    fn density_text(&self) -> f32 {
        match self.layout.density {
            Density::Comfortable => 1.0,
            Density::Compact => 0.93,
        }
    }

    /// A component's height, density applied (`fallback` when it sets none).
    pub fn height_of(&self, c: &Component, fallback: f32) -> f32 {
        self.dense(if c.height > 0.0 { c.height } else { fallback })
    }

    /// A component's padding, density applied.
    pub fn padding_of(&self, c: &Component) -> egui::Vec2 {
        egui::vec2(self.dense(c.padding[0]), self.dense(c.padding[1]))
    }

    /// The large radius (cards, panels).
    pub fn corner(&self) -> CornerRadius {
        if self.layout.corners == Corners::Square { CornerRadius::ZERO } else { CornerRadius::same(self.radius) }
    }

    /// The small radius (buttons, tags, menus).
    pub fn corner_small(&self) -> CornerRadius {
        if self.layout.corners == Corners::Square { CornerRadius::ZERO } else { CornerRadius::same(self.radius_small) }
    }

    /// Text that reads on `fill`: near-black on light fills, near-white on dark ones.
    pub fn text_on(&self, fill: Color32) -> Color32 {
        text_on(fill)
    }

    /// An overlay level: `light(0)` is the softest light wash.
    pub fn overlay_light(&self, i: usize) -> Color32 {
        self.overlays.light.get(i).or(self.overlays.light.last()).map(|c| self.color(c)).unwrap_or(Color32::TRANSPARENT)
    }

    /// A dark overlay level: `dark(0)` is the softest.
    pub fn overlay_dark(&self, i: usize) -> Color32 {
        self.overlays.dark.get(i).or(self.overlays.dark.last()).map(|c| self.color(c)).unwrap_or(Color32::TRANSPARENT)
    }

    /// Everything wrong with the theme that loading let through: unknown
    /// color names, text styles or radii, files that aren't in the package.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut check = |what: &str, c: &ColorRef| {
            if self.parse_color(&c.0).is_none() {
                out.push(format!("{what}: unknown color {:?}", c.0));
            }
        };
        for (name, f) in [("backgrounds.page", &self.backgrounds.page), ("backgrounds.store", &self.backgrounds.store), ("backgrounds.library", &self.backgrounds.library), ("backgrounds.community", &self.backgrounds.community), ("backgrounds.profile", &self.backgrounds.profile)] {
            fill_colors(f, &mut |c| check(name, c));
        }
        for c in self.overlays.light.iter().chain(&self.overlays.dark) {
            check("overlays", c);
        }
        for (name, comp) in self.components.all() {
            check(&format!("components.{name}.shadow"), &comp.shadow.color);
            for (sn, st) in [("normal", &comp.normal), ("hover", &comp.hover), ("pressed", &comp.pressed), ("selected", &comp.selected), ("disabled", &comp.disabled)] {
                let what = format!("components.{name}.{sn}");
                if let Some(f) = &st.fill {
                    fill_colors(f, &mut |c| check(&what, c));
                }
                for c in [&st.text, &st.border, &st.icon].into_iter().flatten() {
                    check(&what, c);
                }
            }
        }
        for (name, comp) in self.components.all() {
            if let TextRef::Named(n) = &comp.text
                && self.typography.get(n).is_none()
            {
                out.push(format!("components.{name}.text: unknown text style {n:?}"));
            }
            if let Radius::Named(n) = &comp.radius
                && !["small", "large", "full", "none"].contains(&n.as_str())
            {
                out.push(format!("components.{name}.radius: unknown radius {n:?}"));
            }
        }
        out
    }

    /// Push the theme into egui's global style, and its fonts when they
    /// changed. Call when the theme changes.
    pub fn apply(&self, ctx: &egui::Context) {
        let font_key = self.font_key();
        if ctx.data(|d| d.get_temp::<String>(egui::Id::unique("theme-fonts"))).as_deref() != Some(font_key.as_str()) {
            install_fonts_for(ctx, Some(self));
        }
        let mut v = if self.dark { Visuals::dark() } else { Visuals::light() };
        let menu = &self.components.menu;
        let menu_look = self.look(menu, Interact::default());
        let menu_fill = match menu_look.fill {
            ResolvedFill::Color(c) => c,
            _ => self.raised,
        };
        v.override_text_color = Some(self.text);
        v.panel_fill = self.bg;
        v.window_fill = menu_fill;
        v.window_stroke = Stroke::new(menu.border, menu_look.border);
        v.window_corner_radius = self.corner();
        v.window_shadow = egui::Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(140) };
        v.popup_shadow = shadow_of(self, &menu.shadow);
        v.menu_corner_radius = self.radius_of(menu);
        v.extreme_bg_color = self.raised;
        v.faint_bg_color = self.surface;
        v.code_bg_color = self.raised;
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent.linear_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.text_cursor.stroke = Stroke::new(2.0, self.accent);

        let r = self.corner_small();
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.surface;
        w.noninteractive.weak_bg_fill = self.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text_dim);
        w.inactive.bg_fill = self.raised;
        w.inactive.weak_bg_fill = self.raised;
        w.inactive.bg_stroke = Stroke::new(1.0, self.border);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.bg_fill = self.surface_hover;
        w.hovered.weak_bg_fill = self.surface_hover;
        w.hovered.bg_stroke = Stroke::new(1.0, self.accent.linear_multiply(0.5));
        w.hovered.fg_stroke = Stroke::new(1.5, self.text);
        w.hovered.expansion = 0.0;
        w.active.bg_fill = self.accent.linear_multiply(0.5);
        w.active.weak_bg_fill = self.accent.linear_multiply(0.5);
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(2.0, self.on_accent);
        w.active.expansion = 0.0;
        w.open.weak_bg_fill = self.surface_hover;
        w.open.bg_stroke = Stroke::new(1.0, self.border);
        for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            s.corner_radius = r;
        }

        ctx.set_visuals(v);
        let sp = self.spacing.clone();
        let text = self.typography.clone();
        let interact = self.dense(32.0);
        let k = self.density_text();
        let menu_pad = self.padding_of(menu);
        ctx.global_style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(sp.item, sp.item);
            s.spacing.button_padding = egui::vec2(sp.md + 2.0, sp.xs + 3.0);
            s.spacing.interact_size.y = interact;
            s.spacing.menu_margin = egui::Margin::symmetric(menu_pad.x as i8, menu_pad.y as i8);
            s.spacing.scroll.bar_width = 6.0;
            s.spacing.scroll.floating = true;
            s.spacing.scroll.bar_inner_margin = 2.0;
            s.interaction.selectable_labels = false;
            use egui::TextStyle::*;
            let f = |st: TextStyle| FontId::new(st.size * k, st.weight.family());
            s.text_styles = [
                (Heading, f(text.heading_m)),
                (Body, f(text.body_m)),
                (Button, f(TextStyle::new(text.body_m.size, Weight::Medium))),
                (Small, f(text.body_s)),
                (Monospace, FontId::new(13.0, FontFamily::Monospace)),
            ]
            .into();
        });
    }

    fn font_key(&self) -> String {
        format!("{}|{:?}|{:?}|{:?}|{:?}|{:?}", self.id, self.fonts.regular, self.fonts.medium, self.fonts.semibold, self.fonts.bold, self.icons.font)
    }
}

impl Components {
    /// Every component with its name in theme files.
    pub fn all(&self) -> Vec<(&'static str, &Component)> {
        vec![
            ("button_primary", &self.button_primary),
            ("button_secondary", &self.button_secondary),
            ("button_tertiary", &self.button_tertiary),
            ("button_ghost", &self.button_ghost),
            ("button_play", &self.button_play),
            ("button_install", &self.button_install),
            ("button_pause", &self.button_pause),
            ("button_danger", &self.button_danger),
            ("button_cta", &self.button_cta),
            ("icon_button", &self.icon_button),
            ("bar_button", &self.bar_button),
            ("icon_box", &self.icon_box),
            ("badge", &self.badge),
            ("section_tab", &self.section_tab),
            ("page_tab", &self.page_tab),
            ("nav_tab", &self.nav_tab),
            ("chip", &self.chip),
            ("tag", &self.tag),
            ("sort", &self.sort),
            ("segmented", &self.segmented),
            ("card", &self.card),
            ("input", &self.input),
            ("toggle", &self.toggle),
            ("check", &self.check),
            ("menu", &self.menu),
            ("menu_item", &self.menu_item),
            ("top_bar", &self.top_bar),
            ("page_bar", &self.page_bar),
            ("status_bar", &self.status_bar),
            ("sidebar", &self.sidebar),
            ("sidebar_item", &self.sidebar_item),
            ("game_card", &self.game_card),
            ("price", &self.price),
            ("wishlist", &self.wishlist),
            ("progress", &self.progress),
            ("toast", &self.toast),
            ("store_card", &self.store_card),
            ("shelf", &self.shelf),
            ("pager", &self.pager),
            ("game_row", &self.game_row),
            ("library_shelf", &self.library_shelf),
            ("download_row", &self.download_row),
            ("post_card", &self.post_card),
            ("filter_bar", &self.filter_bar),
            ("stat_chip", &self.stat_chip),
            ("search_large", &self.search_large),
            ("profile_panel", &self.profile_panel),
            ("profile_box", &self.profile_box),
            ("profile_inset", &self.profile_inset),
            ("profile_row", &self.profile_row),
            ("settings_nav", &self.settings_nav),
            ("settings_nav_item", &self.settings_nav_item),
            ("settings_group", &self.settings_group),
            ("notification", &self.notification),
            ("friends_panel", &self.friends_panel),
            ("friends_box", &self.friends_box),
            ("friend_row", &self.friend_row),
            ("friend_avatar", &self.friend_avatar),
        ]
    }
}

fn fill_colors(f: &Fill, check: &mut impl FnMut(&ColorRef)) {
    match f {
        Fill::Color(c) => check(c),
        Fill::Linear { linear, .. } => linear.iter().for_each(&mut *check),
        Fill::Radial { radial, .. } => radial.iter().for_each(&mut *check),
        Fill::Image { tint, .. } => tint.iter().for_each(&mut *check),
    }
}

/// egui's shadow from a theme shadow.
pub fn shadow_of(t: &Theme, s: &Shadow) -> egui::Shadow {
    egui::Shadow {
        offset: [s.offset[0].round() as i8, s.offset[1].round() as i8],
        blur: s.blur.round().clamp(0.0, 255.0) as u8,
        spread: s.spread.round().clamp(0.0, 255.0) as u8,
        color: t.color(&s.color),
    }
}

fn split_args(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Near-black text on light fills, near-white on dark ones.
pub fn text_on(fill: Color32) -> Color32 {
    let l = 0.2126 * fill.r() as f32 + 0.7152 * fill.g() as f32 + 0.0722 * fill.b() as f32;
    if l > 140.0 { Color32::from_rgb(0x0E, 0x14, 0x1B) } else { Color32::from_rgb(0xF3, 0xF3, 0xF3) }
}

/// `#RGB`, `#RRGGBB` or `#RRGGBBAA`.
pub fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        3 => {
            let n = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|v| v * 17);
            Some(Color32::from_rgb(n(0)?, n(1)?, n(2)?))
        }
        6 => Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

pub fn to_hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
}

/// Colors as `"#RRGGBB"` text in theme files (also reads egui's `[r, g, b, a]`).
pub mod hex {
    use egui::Color32;
    use serde::{Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(c: &Color32, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::to_hex(*c))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Color32, D::Error> {
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = Color32;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a color like \"#66C0F4\"")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Color32, E> {
                super::parse_hex(v).ok_or_else(|| E::custom(format!("not a color: {v:?}")))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Color32, A::Error> {
                let mut c = [0u8, 0, 0, 255];
                for (i, slot) in c.iter_mut().enumerate() {
                    match seq.next_element::<u8>()? {
                        Some(v) => *slot = v,
                        None if i == 3 => break,
                        None => return Err(de::Error::custom("a color needs 3 or 4 numbers")),
                    }
                }
                Ok(Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3]))
            }
        }
        d.deserialize_any(V)
    }
}

// ---------- fonts ----------

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
const PHOSPHOR: &[u8] = include_bytes!("../assets/Phosphor.ttf");

/// The operating system's fonts for Chinese, Japanese, Korean, Hindi, Thai,
/// Arabic and symbols (whichever exist), in fallback order.
fn system_font_files() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    if cfg!(target_os = "macos") {
        let dir = PathBuf::from("/System/Library/Fonts");
        ["Hiragino Sans GB.ttc", "ヒラギノ角ゴシック W3.ttc", "AppleSDGothicNeo.ttc", "Kohinoor.ttc", "ThonburiUI.ttc", "SFArabic.ttf", "Apple Symbols.ttf"]
            .iter()
            .map(|f| dir.join(f))
            .collect()
    } else if let Some(dir) = std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts")) {
        ["msyh.ttc", "YuGothM.ttc", "malgun.ttf", "Nirmala.ttc", "tahoma.ttf", "seguisym.ttf"].iter().map(|f| dir.join(f)).collect()
    } else {
        Vec::new()
    }
}

/// A system font file, mapped into memory instead of copied: only the parts
/// actually drawn get loaded, and they count as shared file cache rather than
/// the app's own memory (the CJK fonts are 70+ MB together).
fn map_font(path: &std::path::Path) -> Option<&'static [u8]> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: system font files don't change while the app runs; the mapping
    // lives for the whole program, like the font itself.
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    Some(&Box::leak(Box::new(map))[..])
}

/// Install Inter (four weights) plus the icon font. Call once at startup.
pub fn install_fonts(ctx: &egui::Context) {
    install_fonts_for(ctx, None);
}

/// The system fonts, mapped once for the whole run.
fn system_fonts() -> &'static [(String, &'static [u8])] {
    static FONTS: std::sync::OnceLock<Vec<(String, &'static [u8])>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut out = Vec::new();
        for path in system_font_files() {
            if let Some(bytes) = map_font(&path) {
                out.push((format!("system-{}", path.file_name().unwrap_or_default().to_string_lossy()), bytes));
            }
        }
        // Bold Chinese for headings and buttons (Windows ships it separately).
        if let Some(bytes) = std::env::var_os("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts").join("msyhbd.ttc")).and_then(|p| map_font(&p)) {
            out.push(("system-msyhbd".into(), bytes));
        }
        out
    })
}

/// Install the theme's fonts (or Inter for any weight it doesn't bring),
/// its icon font in front of the built-in icons, and the system fonts as
/// fallbacks.
pub fn install_fonts_for(ctx: &egui::Context, theme: Option<&Theme>) {
    let mut fonts = egui::FontDefinitions::default();
    let from_theme = |p: &Option<String>| -> Option<Arc<[u8]>> { theme.and_then(|t| t.assets.get(p.as_deref()?)) };
    let mut add = |name: &str, own: Option<Arc<[u8]>>, builtin: &'static [u8]| {
        let data = match own {
            Some(b) => egui::FontData::from_owned(b.to_vec()),
            None => egui::FontData::from_static(builtin),
        };
        fonts.font_data.insert(name.into(), Arc::new(data));
    };
    let f = theme.map(|t| t.fonts.clone()).unwrap_or_default();
    add("inter", from_theme(&f.regular), INTER_REGULAR);
    add("inter-medium", from_theme(&f.medium), INTER_MEDIUM);
    add("inter-semibold", from_theme(&f.semibold), INTER_SEMIBOLD);
    add("inter-bold", from_theme(&f.bold), INTER_BOLD);
    add("phosphor", None, PHOSPHOR);
    let icon_font = theme.and_then(|t| from_theme(&t.icons.font));
    if let Some(b) = icon_font.clone() {
        fonts.font_data.insert("theme-icons".into(), Arc::new(egui::FontData::from_owned(b.to_vec())));
    }
    // System fonts for scripts Inter doesn't cover, so game names in Chinese,
    // Japanese, Korean, Thai, Arabic, Hindi… show instead of empty boxes.
    let mut system: Vec<String> = Vec::new();
    let mut bold_cjk = None;
    for (name, bytes) in system_fonts() {
        fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_static(bytes)));
        if name == "system-msyhbd" {
            bold_cjk = Some(name.clone());
        } else {
            system.push(name.clone());
        }
    }
    // egui's bundled fonts stay as fallbacks for emoji and rare scripts.
    let mut fallbacks: Vec<String> = system;
    fallbacks.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    // A theme's icon font goes before the built-in icons, so the icons it
    // has replace ours and the rest stay.
    let icons: Vec<String> = icon_font.iter().map(|_| "theme-icons".to_string()).chain(["phosphor".to_string()]).collect();
    let family = |main: &str, heavy: bool| {
        let mut v = vec![main.to_string()];
        v.extend(icons.iter().cloned());
        if heavy {
            v.extend(bold_cjk.clone());
        }
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, family("inter", false));
    fonts.families.insert(medium(), family("inter-medium", false));
    fonts.families.insert(semibold(), family("inter-semibold", true));
    fonts.families.insert(bold(), family("inter-bold", true));
    fonts.families.entry(FontFamily::Monospace).or_default().extend(icons.iter().cloned());
    ctx.set_fonts(fonts);
    let key = theme.map(|t| t.font_key()).unwrap_or_default();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("theme-fonts"), key));
}
